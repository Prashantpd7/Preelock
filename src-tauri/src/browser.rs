//! Authenticated loopback transport. Never changes DNS, proxy settings, firewall or OS policy.
use crate::{
    clock,
    focus::{FocusConfig, Mode, WebsiteBlocking},
    platform,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PORT: u16 = 47832;
/// Extension behavior version the app shipped with. Bump together with
/// EXTENSION_VERSION in browser-extension/worker.js whenever extension
/// behavior changes, so a stale (never reloaded) copy can be spotted.
pub const EXTENSION_VERSION: u32 = 3;
#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    #[serde(skip)]
    browsers: Vec<String>,
    pub id: String,
    pub mode: String,
    pub domains: Vec<String>,
    pub expires_at: i64,
    pub observe_activity: bool,
}
impl Policy {
    fn applies_to(&self, browser: &str) -> bool {
        self.browsers.is_empty() || self.browsers.iter().any(|id| same_browser(id, browser))
    }
    pub fn new(id: &str, c: &FocusConfig, remaining: u64) -> Self {
        let policy_mode = match &c.website_blocking {
            WebsiteBlocking::Blocklist => "blocklist",
            WebsiteBlocking::Allowlist => "allowlist",
            WebsiteBlocking::None => "blocklist",
            WebsiteBlocking::Auto => match &c.mode {
                Mode::Standard => "blocklist",
                _ => "allowlist",
            },
        };
        let domains = match &c.website_blocking {
            WebsiteBlocking::None => Vec::new(),
            _ => c.domains.clone(),
        };
        Self {
            browsers: c.session_browsers(),
            id: id.into(),
            observe_activity: c.activity_consent && c.attention.enabled,
            mode: if !c.general_internet {
                "offline"
            } else {
                policy_mode
            }
            .into(),
            domains,
            expires_at: clock::now() + remaining.min(65_000) as i64,
        }
    }
}
/// Connection state of one paired browser. The bridge pairs any number
/// of browsers at once; each extension instance is keyed by its own
/// client UUID, so pairing a new browser never disturbs the others.
#[derive(Default)]
struct ClientState {
    /// Raw browser id reported by the extension's sync.
    browser: String,
    ext_v: u32,
    last_seen: u64,
    applied_id: String,
    error: String,
    blocks: u64,
    active_domain: String,
    browser_focused: bool,
}
#[derive(Default)]
struct State {
    policy: Option<Policy>,
    test_id: String,
    test_result: String,
    test_until: u64,
    clients: HashMap<String, ClientState>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BrowserStatus {
    pub available: bool,
    pub connected: bool,
    pub applied_id: String,
    pub client_browser: String,
    pub error: String,
    pub blocks: u64,
    pub cleanup_pending: bool,
    pub active_domain: String,
    pub browser_focused: bool,
    pub activity_fresh: bool,
    /// The connected extension copy predates the behavior the app shipped
    /// with (it was never reloaded after an app update).
    pub extension_stale: bool,
    /// Debug: the raw version reported by the client (for diagnosing
    /// stale-version issues).
    pub ext_v_debug: u32,
    /// Debug: the raw browser string reported by the client.
    pub raw_browser_debug: String,
}
#[derive(Deserialize)]
struct Heartbeat {
    #[serde(default)]
    test_id: String,
    #[serde(default)]
    test_result: String,
    client: String,
    #[serde(default)]
    browser: String,
    applied_id: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    blocks: u64,
    #[serde(default)]
    active_domain: String,
    #[serde(default)]
    browser_focused: bool,
    #[serde(default)]
    disconnect: bool,
    #[serde(default)]
    v: u32,
}
pub struct Bridge {
    state: Arc<Mutex<State>>,
    pub token: String,
    auth_key: Arc<Mutex<String>>,
    server: Arc<tiny_http::Server>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    /// Browser ids known to this machine, captured once at startup. Used to
    /// disambiguate extension reports (Comet looks exactly like Chrome).
    installed: Vec<String>,
}
impl Bridge {
    pub fn start(dir: &Path) -> Result<Self, String> {
        Self::start_at(dir, PORT)
    }
    fn start_at(dir: &Path, port: u16) -> Result<Self, String> {
        let path = dir.join("browser-token");
        let token = if path.exists() {
            std::fs::read_to_string(&path).map_err(|e| e.to_string())?
        } else {
            let token = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            opts.open(&path)
                .map_err(|e| e.to_string())?
                .write_all(token.as_bytes())
                .map_err(|e| e.to_string())?;
            token
        };
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Browser pairing token is invalid. Quit and remove only browser-token to reset pairing.".into());
        }
        let server = Arc::new(
            tiny_http::Server::http(("127.0.0.1", port))
                .map_err(|e| format!("Browser bridge unavailable: {e}"))?,
        );
        let port = server
            .server_addr()
            .to_ip()
            .ok_or("Missing bridge socket.")?
            .port();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let auth_key = Arc::new(Mutex::new(token.clone()));
        let (s, state2, key, stopped) = (
            server.clone(),
            state.clone(),
            auth_key.clone(),
            stop.clone(),
        );
        let installed: Vec<String> = platform::installed_browsers()
            .into_iter()
            .map(|b| b.id)
            .collect();
        let thread_browsers = installed.clone();
        std::thread::spawn(move || {
            while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok(Some(mut req)) = s.recv_timeout(Duration::from_secs(1)) else {
                    continue;
                };
                let header = |name: &'static str| {
                    req.headers()
                        .iter()
                        .find(|h| h.field.equiv(name))
                        .map(|h| h.value.as_str().to_owned())
                        .unwrap_or_default()
                };
                let origin = header("Origin");
                let host = header("Host");
                let authorized = key
                    .lock()
                    .is_ok_and(|token| header("Authorization") == format!("Bearer {token}"));
                let extension = origin
                    .strip_prefix("chrome-extension://")
                    .is_some_and(|id| {
                        id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
                    });
                if !extension || host != format!("127.0.0.1:{port}") || req.url() != "/sync" {
                    let _ = req.respond(tiny_http::Response::empty(403));
                    continue;
                }
                if req.method() == &tiny_http::Method::Options {
                    let mut res = tiny_http::Response::empty(204);
                    for (k, v) in [
                        ("Access-Control-Allow-Origin", origin.as_str()),
                        (
                            "Access-Control-Allow-Headers",
                            "Authorization, Content-Type",
                        ),
                        ("Access-Control-Allow-Methods", "POST"),
                        ("Access-Control-Allow-Private-Network", "true"),
                    ] {
                        res.add_header(tiny_http::Header::from_bytes(k, v).unwrap());
                    }
                    let _ = req.respond(res);
                    continue;
                }
                // A known Content-Length prevents chunked/oversized request allocation.
                if !authorized
                    || req.method() != &tiny_http::Method::Post
                    || req.body_length().is_none_or(|n| n > 4096)
                {
                    let _ = req.respond(tiny_http::Response::empty(403));
                    continue;
                }
                let mut body = String::new();
                if req.as_reader().read_to_string(&mut body).is_err() {
                    continue;
                }
                let Ok(heartbeat) = serde_json::from_str::<Heartbeat>(&body) else {
                    let _ = req.respond(tiny_http::Response::empty(400));
                    continue;
                };
                if heartbeat.test_result.len() > 240
                    || heartbeat.test_id.len() > 80
                    || heartbeat.client.len() != 36
                    || heartbeat.browser.len() > 40
                    || !heartbeat
                        .browser
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
                    || heartbeat.applied_id.len() > 80
                    || heartbeat.error.len() > 240
                    || heartbeat.active_domain.len() > 253
                    || !heartbeat
                        .active_domain
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                {
                    let _ = req.respond(tiny_http::Response::empty(400));
                    continue;
                }
                let Ok(mut state) = state2.lock() else {
                    break;
                };
                if heartbeat.disconnect {
                    // The extension disconnected on purpose: forget just
                    // this client at once, so the app stops showing it as
                    // connected instead of waiting out the heartbeat window.
                    // Other paired browsers are untouched.
                    state.clients.remove(&heartbeat.client);
                    let _ = req.respond(tiny_http::Response::empty(200));
                    continue;
                }
                let resolved_browser = resolve_browser(&heartbeat.browser, &thread_browsers);
                let observe = state
                    .policy
                    .as_ref()
                    .is_some_and(|p| p.observe_activity && p.applies_to(&resolved_browser));
                let client = state.clients.entry(heartbeat.client).or_default();
                client.browser = heartbeat.browser;
                client.ext_v = heartbeat.v;
                client.last_seen = clock::continuous_ms();
                client.applied_id = heartbeat.applied_id;
                client.error = heartbeat.error;
                client.blocks = heartbeat.blocks;
                client.active_domain = if observe {
                    heartbeat.active_domain.to_lowercase()
                } else {
                    String::new()
                };
                client.browser_focused = observe && heartbeat.browser_focused;
                if !heartbeat.test_id.is_empty()
                    && heartbeat.test_id == state.test_id
                    && !heartbeat.test_result.is_empty()
                {
                    state.test_result = heartbeat.test_result;
                    state.test_until = 0;
                }
                let test_id = if state.policy.is_none() && state.test_until > clock::continuous_ms()
                {
                    state.test_id.clone()
                } else {
                    String::new()
                };
                let policy = state
                    .policy
                    .clone()
                    .filter(|p| p.expires_at > clock::now() && p.applies_to(&resolved_browser));
                drop(state);
                let mut res = tiny_http::Response::from_string(
                    serde_json::json!({"policy":policy,"test_id":test_id}).to_string(),
                );
                for (k, v) in [
                    ("Content-Type", "application/json"),
                    ("Cache-Control", "no-store"),
                    ("Access-Control-Allow-Origin", origin.as_str()),
                ] {
                    res.add_header(tiny_http::Header::from_bytes(k, v).unwrap());
                }
                let _ = req.respond(res);
            }
        });
        Ok(Self {
            state,
            token,
            auth_key,
            server,
            stop,
            installed,
        })
    }
    pub fn request_test(&self) -> Result<(), String> {
        if !self.status().connected {
            return Err("Connect your browser first.".into());
        }
        let mut state = self.state.lock().map_err(|_| "Browser unavailable.")?;
        if state.policy.is_some() {
            return Err("Finish your focus session first.".into());
        }
        state.test_id = uuid::Uuid::new_v4().to_string();
        state.test_result = "Testing…".into();
        state.test_until = clock::continuous_ms() + 45_000;
        Ok(())
    }
    pub fn test_result(&self) -> String {
        self.state
            .lock()
            .map(|s| {
                if s.test_result == "Testing…" && s.test_until < clock::continuous_ms() {
                    "No response. Reload the extension, then try again.".into()
                } else {
                    s.test_result.clone()
                }
            })
            .unwrap_or_default()
    }
    pub fn rotate_token(&mut self, dir: &Path) -> Result<(), String> {
        use std::io::Write;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let temp = dir.join(format!(".browser-token-{}", uuid::Uuid::new_v4()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&temp).map_err(|e| e.to_string())?;
        file.write_all(token.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temp, dir.join("browser-token")).map_err(|e| e.to_string())?;
        *self.auth_key.lock().map_err(|_| "Pairing unavailable.")? = token.clone();
        self.token = token;
        *self.state.lock().map_err(|_| "Pairing unavailable.")? = State::default();
        Ok(())
    }
    #[cfg(test)]
    pub fn test_bridge(dir: &Path) -> Self {
        let mut bridge = Self::start_at(dir, 0).unwrap();
        // Deterministic test environment: the fake extension always reports
        // Chrome, so pin the installed list instead of reading the dev
        // machine (Comet-vs-Chrome disambiguation would otherwise rewrite
        // the report depending on which browsers happen to be installed).
        bridge.installed = vec!["com.google.Chrome".into()];
        bridge
    }
    #[cfg(test)]
    pub fn acknowledge(&self, client: &str, browser: &str, id: &str) {
        let mut s = self.state.lock().unwrap();
        let c = s.clients.entry(client.into()).or_default();
        c.browser = browser.into();
        c.ext_v = EXTENSION_VERSION;
        c.last_seen = clock::continuous_ms();
        c.applied_id = id.into();
        c.error.clear();
    }
    pub fn publish(&self, policy: Option<Policy>) {
        if let Ok(mut state) = self.state.lock() {
            state.policy = policy;
        }
    }
    /// Status of the most recently seen browser connection, for callers
    /// that don't care which browser it is.
    pub fn status(&self) -> BrowserStatus {
        self.status_for("")
    }
    /// Status of the connection in the given browser (platform browser
    /// id). An empty `browser_id` falls back to the most recently seen
    /// connection. When no paired extension reports that browser, the
    /// status comes back not-connected.
    pub fn status_for(&self, browser_id: &str) -> BrowserStatus {
        match self.state.lock() {
            Ok(s) => {
                let mut best: Option<&ClientState> = None;
                for c in s.clients.values() {
                    let resolved = resolve_browser(&c.browser, &self.installed);
                    let matches = browser_id.is_empty() || same_browser(browser_id, &resolved);
                    // Prefer a client running the current extension version: a
                    // stale worker (one that didn't restart after an update)
                    // must not shadow a healthy one that synced slightly
                    // earlier.
                    let stale = c.ext_v != EXTENSION_VERSION;
                    let better = match &best {
                        None => true,
                        Some(b) => {
                            let b_stale = b.ext_v != EXTENSION_VERSION;
                            (!stale && b_stale) || (stale == b_stale && c.last_seen > b.last_seen)
                        }
                    };
                    if matches && better {
                        best = Some(c);
                    }
                }
                match best {
                    Some(c) => {
                        let connected = c.last_seen != 0
                            && clock::continuous_ms().saturating_sub(c.last_seen) < 45_000;
                        BrowserStatus {
                            active_domain: c.active_domain.clone(),
                            browser_focused: c.browser_focused,
                            activity_fresh: c.last_seen != 0
                                && clock::continuous_ms().saturating_sub(c.last_seen) < 6_000,
                            available: true,
                            connected,
                            applied_id: c.applied_id.clone(),
                            client_browser: resolve_browser(&c.browser, &self.installed),
                            extension_stale: connected && c.ext_v != EXTENSION_VERSION,
                            error: c.error.clone(),
                            blocks: c.blocks,
                            cleanup_pending: s.policy.is_none() && !c.applied_id.is_empty(),
                            ext_v_debug: c.ext_v,
                            raw_browser_debug: c.browser.clone(),
                        }
                    }
                    None => BrowserStatus {
                        active_domain: String::new(),
                        browser_focused: false,
                        activity_fresh: false,
                        available: true,
                        connected: false,
                        applied_id: String::new(),
                        client_browser: String::new(),
                        extension_stale: false,
                        error: String::new(),
                        blocks: 0,
                        cleanup_pending: false,
                        ext_v_debug: 0,
                        raw_browser_debug: String::new(),
                    },
                }
            }
            Err(_) => unavailable("Browser bridge failed."),
        }
    }
    /// Resolved browser ids with a live connection, for the Settings panel.
    pub fn connected_browsers(&self) -> Vec<String> {
        match self.state.lock() {
            Ok(s) => {
                let now = clock::continuous_ms();
                let mut ids: Vec<String> = s
                    .clients
                    .values()
                    .filter(|c| c.last_seen != 0 && now.saturating_sub(c.last_seen) < 45_000)
                    .map(|c| resolve_browser(&c.browser, &self.installed))
                    .collect();
                ids.sort();
                ids.dedup();
                ids
            }
            Err(_) => Vec::new(),
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.publish(None);
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.server.unblock();
    }
}
pub fn unavailable(reason: &str) -> BrowserStatus {
    BrowserStatus {
        active_domain: String::new(),
        browser_focused: false,
        activity_fresh: false,
        available: false,
        connected: false,
        applied_id: String::new(),
        client_browser: String::new(),
        error: reason.into(),
        blocks: 0,
        cleanup_pending: false,
        extension_stale: false,
        ext_v_debug: 0,
        raw_browser_debug: String::new(),
    }
}
/// True when `id` — a platform browser id (macOS bundle id, or a Windows
/// executable path) — refers to the browser the extension reported in its
/// sync (`reported`, a canonical bundle id). Empty values never match; an
/// empty `reported` means the extension could not identify its browser and
/// does not count as connected to the chosen browser.
pub fn same_browser(id: &str, reported: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let reported = reported.to_ascii_lowercase();
    if id.is_empty() || reported.is_empty() {
        return false;
    }
    if id == reported {
        return true;
    }
    let exe = match reported.as_str() {
        "com.google.chrome" => "chrome.exe",
        "com.microsoft.edgemac" => "msedge.exe",
        "com.brave.browser" => "brave.exe",
        "com.vivaldi.vivaldi" => "vivaldi.exe",
        "org.chromium.chromium" => "chromium.exe",
        "ai.perplexity.comet" => "comet.exe",
        _ => return false,
    };
    id.ends_with(&format!("\\{exe}"))
}
/// Comet ships a Chrome-identical user agent and brand list, so the
/// extension reports it as Chrome. When this machine has Comet installed
/// but no real Chrome, such a report really belongs to Comet. The
/// installed list is captured once at startup; installing Chrome later
/// needs an app restart to take effect.
fn resolve_browser(reported: &str, installed: &[String]) -> String {
    if same_browser("com.google.chrome", reported)
        && installed
            .iter()
            .any(|id| same_browser(id, "ai.perplexity.comet"))
        && !installed
            .iter()
            .any(|id| same_browser(id, "com.google.chrome"))
    {
        return "ai.perplexity.comet".into();
    }
    reported.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policies_only_apply_to_selected_browsers() {
        let c = FocusConfig {
            browser_app: r"C:\Apps\Comet\comet.exe".into(),
            ..Default::default()
        };
        let policy = Policy::new("test", &c, 60_000);
        assert!(policy.applies_to("ai.perplexity.comet"));
        assert!(!policy.applies_to("com.google.chrome"));
    }
    #[test]
    fn policies_are_short_lived_and_mode_specific() {
        let mut c = FocusConfig::default();
        assert_eq!(Policy::new("s", &c, 500_000).mode, "blocklist");
        c.mode = Mode::Strict;
        assert_eq!(Policy::new("s", &c, 500_000).mode, "allowlist");
        c.general_internet = false;
        let p = Policy::new("s", &c, 500_000);
        assert_eq!(p.mode, "offline");
        assert!(p.expires_at - clock::now() <= 65_000);
    }
    #[test]
    fn loopback_auth_activation_acknowledgement_and_cleanup() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let mut bridge = Bridge::test_bridge(dir.path());
        let port = bridge.server.server_addr().to_ip().unwrap().port();
        let request = |key: &str, id: &str| {
            let body=serde_json::json!({"client":"11111111-1111-1111-1111-111111111111","applied_id":id}).to_string();
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(stream,"POST /sync HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\nAuthorization: Bearer {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        assert!(request("invalid", "").starts_with("HTTP/1.1 403"));
        bridge.publish(Some(Policy::new(
            "session",
            &FocusConfig::default(),
            60_000,
        )));
        let response = request(&bridge.token, "");
        assert!(response.contains("session"));
        assert!(bridge.status().connected);
        request(&bridge.token, "session");
        assert_eq!(bridge.status().applied_id, "session");
        bridge.publish(None);
        assert!(bridge.status().cleanup_pending);
        assert!(request(&bridge.token, "").contains("\"policy\":null"));
        assert!(!bridge.status().cleanup_pending);
        let old_token = bridge.token.clone();
        bridge.rotate_token(dir.path()).unwrap();
        assert!(request(&old_token, "").starts_with("HTTP/1.1 403"));
        assert!(request(&bridge.token, "").starts_with("HTTP/1.1 200"));
    }
    #[test]
    fn two_browsers_share_one_code_without_disturbing_each_other() {
        use std::io::{Read, Write};
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::test_bridge(dir.path());
        let port = bridge.server.server_addr().to_ip().unwrap().port();
        let sync = |key: &str, client: &str, browser: &str, disconnect: bool| {
            let body = serde_json::json!({
                "client": client,
                "browser": browser,
                "applied_id": "",
                "disconnect": disconnect,
            })
            .to_string();
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(stream,"POST /sync HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\nAuthorization: Bearer {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let comet = "11111111-1111-1111-1111-111111111111";
        let brave = "22222222-2222-2222-2222-222222222222";
        // No 409 anymore: both browsers pair with the same code.
        assert!(
            sync(&bridge.token, comet, "ai.perplexity.comet", false).starts_with("HTTP/1.1 200")
        );
        assert!(sync(&bridge.token, brave, "com.brave.browser", false).starts_with("HTTP/1.1 200"));
        assert!(bridge.status_for("ai.perplexity.comet").connected);
        assert!(bridge.status_for("com.brave.browser").connected);
        assert_eq!(
            bridge.status_for("com.brave.browser").client_browser,
            "com.brave.browser"
        );
        assert!(!bridge.status_for("com.google.chrome").connected);
        assert_eq!(bridge.connected_browsers().len(), 2);
        // Disconnecting one browser leaves the other alone.
        assert!(sync(&bridge.token, brave, "com.brave.browser", true).starts_with("HTTP/1.1 200"));
        assert!(!bridge.status_for("com.brave.browser").connected);
        assert!(bridge.status_for("ai.perplexity.comet").connected);
        assert_eq!(bridge.connected_browsers(), vec!["ai.perplexity.comet"]);
    }
}
