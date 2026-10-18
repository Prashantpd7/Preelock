//! Authenticated loopback transport. Never changes DNS, proxy settings, firewall or OS policy.
use crate::{
    clock,
    focus::{FocusConfig, Mode},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PORT: u16 = 47832;
#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    pub id: String,
    pub mode: String,
    pub domains: Vec<String>,
    pub expires_at: i64,
    pub observe_activity: bool,
}
impl Policy {
    pub fn new(id: &str, c: &FocusConfig, remaining: u64) -> Self {
        Self {
            id: id.into(),
            observe_activity: c.activity_consent && c.attention.enabled,
            mode: if !c.general_internet {
                "offline"
            } else if c.mode == Mode::Standard {
                "blocklist"
            } else {
                "allowlist"
            }
            .into(),
            domains: c.domains.clone(),
            expires_at: clock::now() + remaining.min(65_000) as i64,
        }
    }
}
#[derive(Default)]
struct State {
    policy: Option<Policy>,
    last_seen: u64,
    applied_id: String,
    error: String,
    client: String,
    blocks: u64,
    active_domain: String,
    browser_focused: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct BrowserStatus {
    pub available: bool,
    pub connected: bool,
    pub applied_id: String,
    pub error: String,
    pub blocks: u64,
    pub cleanup_pending: bool,
    pub active_domain: String,
    pub browser_focused: bool,
    pub activity_fresh: bool,
}
#[derive(Deserialize)]
struct Heartbeat {
    client: String,
    applied_id: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    blocks: u64,
    #[serde(default)]
    active_domain: String,
    #[serde(default)]
    browser_focused: bool,
}
pub struct Bridge {
    state: Arc<Mutex<State>>,
    pub token: String,
    auth_key: Arc<Mutex<String>>,
    server: Arc<tiny_http::Server>,
    stop: Arc<std::sync::atomic::AtomicBool>,
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
                if heartbeat.client.len() != 36
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
                if !state.client.is_empty() && state.client != heartbeat.client {
                    let _ = req.respond(tiny_http::Response::empty(409));
                    continue;
                }
                state.client = heartbeat.client;
                state.last_seen = clock::continuous_ms();
                state.applied_id = heartbeat.applied_id;
                state.error = heartbeat.error;
                state.blocks = heartbeat.blocks;
                let observe = state.policy.as_ref().is_some_and(|p| p.observe_activity);
                state.active_domain = if observe {
                    heartbeat.active_domain.to_lowercase()
                } else {
                    String::new()
                };
                state.browser_focused = observe && heartbeat.browser_focused;
                let policy = state.policy.clone().filter(|p| p.expires_at > clock::now());
                drop(state);
                let mut res = tiny_http::Response::from_string(
                    serde_json::json!({"policy":policy}).to_string(),
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
        })
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
        Self::start_at(dir, 0).unwrap()
    }
    #[cfg(test)]
    pub fn acknowledge(&self, id: &str) {
        let mut s = self.state.lock().unwrap();
        s.last_seen = clock::continuous_ms();
        s.applied_id = id.into();
        s.error.clear();
    }
    pub fn publish(&self, policy: Option<Policy>) {
        if let Ok(mut state) = self.state.lock() {
            state.policy = policy;
        }
    }
    pub fn status(&self) -> BrowserStatus {
        match self.state.lock() {
            Ok(s) => BrowserStatus {
                active_domain: s.active_domain.clone(),
                browser_focused: s.browser_focused,
                activity_fresh: s.last_seen != 0
                    && clock::continuous_ms().saturating_sub(s.last_seen) < 6_000,
                available: true,
                connected: s.last_seen != 0
                    && clock::continuous_ms().saturating_sub(s.last_seen) < 45_000,
                applied_id: s.applied_id.clone(),
                error: s.error.clone(),
                blocks: s.blocks,
                cleanup_pending: s.policy.is_none() && !s.applied_id.is_empty(),
            },
            Err(_) => unavailable("Browser bridge failed."),
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
        error: reason.into(),
        blocks: 0,
        cleanup_pending: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
