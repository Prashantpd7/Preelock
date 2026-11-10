//! Authenticated loopback transport. Never changes DNS, proxy settings, firewall or OS policy.
use crate::{
    blockreasons::{BlockCause, RuleCause},
    clock,
    focus::{FocusConfig, Mode, WebsiteBlocking},
    platform,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PORT: u16 = 47832;
/// Extension behavior version the app shipped with. Bump together with
/// EXTENSION_VERSION in browser-extension/worker.js whenever extension
/// behavior changes, so a stale (never reloaded) copy can be spotted.
pub const EXTENSION_VERSION: u32 = 6;
#[derive(Clone, Debug, Serialize)]
pub struct Policy {
    #[serde(skip)]
    browsers: Vec<String>,
    pub id: String,
    pub mode: String,
    pub domains: Vec<String>,
    /// Per-block exceptions (step 2): host/path rules that stay
    /// accessible even when they match a block rule. Present only for
    /// effective blocklist policies; empty in allowlist and offline
    /// mode, where exceptions do not apply (see the precedence note in
    /// focus.rs and browser-extension/rules.js). The extension checks
    /// them after the adult overlay and before block/keyword rules.
    pub exceptions: Vec<String>,
    /// Keyword rules, as bare keywords (the `search:` prefix stripped).
    /// The extension blocks search-engine result pages whose query
    /// contains one of these; it never sees them as domains.
    pub keywords: Vec<String>,
    /// Adult-category domains, present only when the session's "Block
    /// adult content" toggle is on. A block overlay: the extension
    /// blocks them even in allowlist mode, and the engine counts them
    /// as not relevant even when allowlisted. Empty in offline mode
    /// (everything is blocked there anyway) and when blocking is off.
    pub extra_blocked: Vec<String>,
    /// YouTube channel rules (step 3), normalized stored keys (see
    /// focus::normalize_youtube_channel). Carried whenever website
    /// blocking is on; empty when blocking is off or offline (in
    /// offline mode everything is blocked anyway). Channel pages are
    /// enforced by DNR rules, watch pages by the youtube.js content
    /// script; see browser-extension/rules.js for the full decision.
    pub youtube_blocked_channels: Vec<String>,
    pub youtube_allowed_channels: Vec<String>,
    /// Study mode: only `youtube_allowed_channels` play, and the
    /// extension blocks the YouTube homepage, Shorts, feeds and
    /// search results. False unless blocking is on.
    pub youtube_study_mode: bool,
    /// Watch-Later exception (list=WL pages stay open). False unless
    /// blocking is on. Extension-enforced only: query parameters are
    /// invisible to the engine (see the note on FocusConfig in
    /// focus.rs).
    pub youtube_allow_watch_later: bool,
    pub expires_at: i64,
    pub observe_activity: bool,
    /// Screen Time-only observation; does not enable or alter blocking rules.
    pub track_activity: bool,
    /// Why this session's own rules block (domains, keywords, adult
    /// overlay entries from the session config). The extension's
    /// blocked page shows this when the blocked site matched a
    /// session rule rather than a per-rule cause below. Optional:
    /// v6 clients ignore unknown fields, so the protocol stays 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_cause: Option<BlockCause>,
    /// Per-rule causes for `extra_blocked` entries (allowance
    /// exhaustion, app locks, adult overlay). The extension looks
    /// the matched rule up here; rules without an entry fall back
    /// to `session_cause`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_causes: Vec<RuleCause>,
}
/// Normalize a channel list for a policy, dropping invalid entries
/// (policies come from validated configs, so this is normally the
/// identity; see `Policy::new`).
fn normalized_channels(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values
        .iter()
        .filter_map(|v| crate::focus::normalize_youtube_channel(v).ok())
        .collect();
    out.sort();
    out.dedup();
    out
}
impl Policy {
    pub fn with_activity_tracking(mut self, enabled: bool) -> Self {
        self.track_activity = enabled;
        self
    }
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
        let has_youtube_rules = !c.youtube_blocked_channels.is_empty() || c.youtube_study_mode;
        // DEBUG 2026-11-03: log YouTube channels to diagnose empty policy
        eprintln!(
            "[preelock-debug] Policy::new youtube_blocked_channels={:?} study_mode={} has_rules={}",
            c.youtube_blocked_channels, c.youtube_study_mode, has_youtube_rules
        );
        let blocking = (!matches!(&c.website_blocking, WebsiteBlocking::None)
            && c.general_internet)
            || has_youtube_rules;
        let domains = match &c.website_blocking {
            WebsiteBlocking::None => Vec::new(),
            _ => c.website_host_rules(),
        };
        // Step 12 Frozen Turkey: a frozen policy is an allowlist
        // whose domains are exactly the frozen allowed websites
        // (default: none, so every site is blocked). This reuses
        // the existing allowlist machinery — the extension needs
        // no change — and it overrides the session's own website
        // mode, exceptions, and keywords: while frozen, only the
        // frozen list is open. The adult overlay below still
        // applies on top, exactly as in ordinary allowlist mode.
        let (policy_mode, domains, exceptions, keywords) = if c.frozen.enabled {
            (
                "allowlist",
                c.frozen.website_host_rules(),
                Vec::new(),
                Vec::new(),
            )
        } else {
            (
                policy_mode,
                domains,
                if blocking && policy_mode == "blocklist" {
                    c.exceptions.clone()
                } else {
                    Vec::new()
                },
                if blocking {
                    c.website_keywords()
                } else {
                    Vec::new()
                },
            )
        };
        Self {
            browsers: c.session_browsers(),
            id: id.into(),
            observe_activity: c.activity_consent && c.attention.enabled,
            track_activity: false,
            mode: if !c.general_internet && !has_youtube_rules {
                "offline"
            } else {
                policy_mode
            }
            .into(),
            domains,
            exceptions,
            keywords,
            extra_blocked: if blocking && c.block_adult_content {
                crate::focus::ADULT_DOMAINS
                    .iter()
                    .map(|d| d.to_string())
                    .collect()
            } else {
                Vec::new()
            },
            // The adult overlay blocks with its own cause, so the
            // blocked page can name it.
            rule_causes: if blocking && c.block_adult_content {
                crate::focus::ADULT_DOMAINS
                    .iter()
                    .map(|d| RuleCause {
                        rule: d.to_string(),
                        cause: BlockCause::adult(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            session_cause: None,
            // Policies are built from validated configs, so this is
            // normally the identity; invalid entries are dropped
            // defensively rather than breaking the whole policy.
            youtube_blocked_channels: if blocking {
                normalized_channels(&c.youtube_blocked_channels)
            } else {
                Vec::new()
            },
            youtube_allowed_channels: if blocking {
                normalized_channels(&c.youtube_allowed_channels)
            } else {
                Vec::new()
            },
            youtube_study_mode: blocking && c.youtube_study_mode,
            youtube_allow_watch_later: blocking && c.youtube_allow_watch_later,
            expires_at: clock::now() + remaining.min(65_000) as i64,
        }
    }
    /// Append allowance-exhausted site rules to `extra_blocked`
    /// (step 8). The extension already treats `extra_blocked` as a
    /// block overlay that outranks allowlist rules and per-block
    /// exceptions (the step-1 adult mechanism), so no extension
    /// change is needed — and exhausted rules block even when the
    /// session itself has no website blocking or adult toggle on.
    /// Adult entries already present are never duplicated.
    pub fn with_exhausted_rules(mut self, rules: &[String]) -> Self {
        for rule in rules {
            if !self.extra_blocked.iter().any(|e| e == rule) {
                self.extra_blocked.push(rule.clone());
            }
        }
        self
    }
    /// Set why this session's own rules block (see `session_cause`).
    pub fn with_session_cause(mut self, cause: BlockCause) -> Self {
        self.session_cause = Some(cause);
        self
    }
    /// Set per-rule causes for `extra_blocked` entries (see
    /// `rule_causes`). When a rule already has a cause (e.g. the
    /// adult overlay from `new`), the more specific one wins:
    /// timed/daily lock > allowance > adult overlay (see
    /// `blockreasons::cause_rank`).
    pub fn with_rule_causes(mut self, causes: &[RuleCause]) -> Self {
        for rc in causes {
            match self.rule_causes.iter_mut().find(|e| e.rule == rc.rule) {
                Some(existing)
                    if crate::blockreasons::cause_rank(&rc.cause)
                        < crate::blockreasons::cause_rank(&existing.cause) =>
                {
                    *existing = rc.clone();
                }
                None => self.rule_causes.push(rc.clone()),
                _ => {}
            }
        }
        self
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
    search_distracted: bool,
    channel_blocked: bool,
}
#[derive(Default)]
struct State {
    policy: Option<Policy>,
    tracking_policy: Option<Policy>,
    tracking_blocked_rules: Vec<String>,
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
    /// The extension reports the active tab is a search page whose query
    /// matches a keyword rule. The engine counts this as not relevant;
    /// the host alone would not reveal the query.
    pub search_distracted: bool,
    /// The extension reports the current tab is showing a channel-rule
    /// block page (a DNR-blocked channel/surface page, or a watch page
    /// the content script sent to the block page). The engine counts
    /// this as not relevant, exactly like `search_distracted`. An
    /// excepted page never reports this: exceptions override channel
    /// rules, so the extension never flags one.
    pub channel_blocked: bool,
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
    search_distracted: bool,
    #[serde(default)]
    channel_blocked: bool,
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
    _ownership: File,
    /// Browser ids known to this machine, captured once at startup. Used to
    /// disambiguate extension reports (Comet looks exactly like Chrome).
    installed: Vec<String>,
}
impl Bridge {
    pub fn start(dir: &Path) -> Result<Self, String> {
        Self::start_at(dir, PORT)
    }

    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.server.unblock();
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
        let ownership = File::create(dir.join("bridge.lock")).map_err(|e| e.to_string())?;
        let mut ownership_error = String::new();
        let mut ownership_acquired = false;
        for _ in 0..20 {
            match ownership.try_lock_exclusive() {
                Ok(()) => {
                    ownership_acquired = true;
                    break;
                }
                Err(error) => {
                    ownership_error = error.to_string();
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        if !ownership_acquired {
            return Err(format!(
                "Browser bridge ownership unavailable: {ownership_error}"
            ));
        }
        let mut last_error = String::new();
        let server = (0..20)
            .find_map(|_| match tiny_http::Server::http(("127.0.0.1", port)) {
                Ok(server) => Some(server),
                Err(error) => {
                    last_error = error.to_string();
                    std::thread::sleep(Duration::from_millis(100));
                    None
                }
            })
            .map(Arc::new)
            .ok_or_else(|| format!("Browser bridge unavailable: {last_error}"))?;
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
                let track = state
                    .policy
                    .as_ref()
                    .is_some_and(|p| p.track_activity && p.applies_to(&resolved_browser))
                    || state
                        .tracking_policy
                        .as_ref()
                        .is_some_and(|p| p.applies_to(&resolved_browser));
                let client = state.clients.entry(heartbeat.client).or_default();
                client.browser = heartbeat.browser;
                client.ext_v = heartbeat.v;
                client.last_seen = clock::continuous_ms();
                client.applied_id = heartbeat.applied_id;
                client.error = heartbeat.error;
                client.blocks = heartbeat.blocks;
                client.active_domain = if observe || track {
                    heartbeat.active_domain.to_lowercase()
                } else {
                    String::new()
                };
                client.browser_focused = (observe || track) && heartbeat.browser_focused;
                client.search_distracted = observe && heartbeat.search_distracted;
                client.channel_blocked = observe && heartbeat.channel_blocked;
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
                    .filter(|p| p.expires_at > clock::now() && p.applies_to(&resolved_browser))
                    .or_else(|| {
                        state.tracking_policy.clone().filter(|p| {
                            p.expires_at > clock::now() && p.applies_to(&resolved_browser)
                        })
                    });
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
            _ownership: ownership,
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
    /// Test helper: set the activity report (domain + focus) a heartbeat
    /// would have stored for this client.
    #[cfg(test)]
    pub fn set_activity(&self, client: &str, browser: &str, domain: &str, focused: bool) {
        let mut s = self.state.lock().unwrap();
        let c = s.clients.entry(client.into()).or_default();
        c.browser = browser.into();
        c.ext_v = EXTENSION_VERSION;
        c.last_seen = clock::continuous_ms();
        c.active_domain = domain.into();
        c.browser_focused = focused;
        c.error.clear();
    }
    pub fn publish(&self, policy: Option<Policy>) {
        if let Ok(mut state) = self.state.lock() {
            state.policy = policy;
            if let Some(tracking) = state.tracking_policy.as_mut() {
                tracking.expires_at = clock::now() + 65_000;
            }
        }
    }
    pub fn set_screen_time_tracking(&self, enabled: bool) {
        if let Ok(mut state) = self.state.lock() {
            if !enabled {
                state.tracking_blocked_rules.clear();
            }
            state.tracking_policy = enabled.then(|| Policy {
                browsers: Vec::new(),
                id: "screen-time-observation".into(),
                mode: "blocklist".into(),
                domains: Vec::new(),
                exceptions: Vec::new(),
                keywords: Vec::new(),
                extra_blocked: state.tracking_blocked_rules.clone(),
                youtube_blocked_channels: Vec::new(),
                youtube_allowed_channels: Vec::new(),
                youtube_study_mode: false,
                youtube_allow_watch_later: false,
                expires_at: clock::now() + 65_000,
                observe_activity: true,
                track_activity: true,
                session_cause: None,
                rule_causes: Vec::new(),
            });
        }
    }
    /// Replace the observe-only lease's exhausted website rules and
    /// their per-rule block causes (see `rule_causes`).
    pub fn set_screen_time_blocked_rules(&self, rules: &[String], causes: &[RuleCause]) {
        if let Ok(mut state) = self.state.lock() {
            state.tracking_blocked_rules = rules.to_vec();
            if let Some(policy) = state.tracking_policy.as_mut() {
                policy.extra_blocked = rules.to_vec();
                policy.rule_causes = causes.to_vec();
                policy.expires_at = clock::now() + 65_000;
            }
        }
    }
    /// Refresh the short-lived observe-only lease. This carries no block rules.
    pub fn refresh_screen_time_tracking(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(policy) = state.tracking_policy.as_mut() {
                policy.expires_at = clock::now() + 65_000;
            }
        }
    }
    /// Status of the most recently seen browser connection, for callers
    /// that don't care which browser it is.
    pub fn status(&self) -> BrowserStatus {
        self.status_for("")
    }
    /// Pick the best (freshest, current-version) client from the
    /// candidates the caller already filtered.
    fn best_client<'s>(
        s: &'s State,
        mut best: Option<&'s ClientState>,
        matches: impl Fn(&ClientState) -> bool,
    ) -> Option<&'s ClientState> {
        for c in s.clients.values() {
            if !matches(c) {
                continue;
            }
            let stale = c.ext_v != EXTENSION_VERSION;
            let better = match &best {
                None => true,
                Some(b) => {
                    let b_stale = b.ext_v != EXTENSION_VERSION;
                    (!stale && b_stale) || (stale == b_stale && c.last_seen > b.last_seen)
                }
            };
            if better {
                best = Some(c);
            }
        }
        best
    }
    /// Build the public status for the chosen client. `installed` is the
    /// bridge's installed-browser list used by `resolve_browser`;
    /// `policy_none` is whether no session policy is currently published
    /// (`cleanup_pending` needs it plus a non-empty applied id, exactly
    /// as `status_for` reported it).
    fn status_from_best(
        best: Option<&ClientState>,
        installed: &[String],
        policy_none: bool,
    ) -> BrowserStatus {
        match best {
            Some(c) => {
                let connected =
                    c.last_seen != 0 && clock::continuous_ms().saturating_sub(c.last_seen) < 45_000;
                BrowserStatus {
                    active_domain: c.active_domain.clone(),
                    browser_focused: c.browser_focused,
                    search_distracted: c.search_distracted,
                    channel_blocked: c.channel_blocked,
                    activity_fresh: c.last_seen != 0
                        && clock::continuous_ms().saturating_sub(c.last_seen) < 6_000,
                    available: true,
                    connected,
                    applied_id: c.applied_id.clone(),
                    client_browser: resolve_browser(&c.browser, installed),
                    extension_stale: connected && c.ext_v != EXTENSION_VERSION,
                    error: c.error.clone(),
                    blocks: c.blocks,
                    cleanup_pending: policy_none && !c.applied_id.is_empty(),
                    ext_v_debug: c.ext_v,
                    raw_browser_debug: c.browser.clone(),
                }
            }
            None => BrowserStatus {
                active_domain: String::new(),
                browser_focused: false,
                search_distracted: false,
                channel_blocked: false,
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
    /// Status of the connection in the given browser (platform browser
    /// id). An empty `browser_id` falls back to the most recently seen
    /// connection. When no paired extension reports that browser, the
    /// status comes back not-connected.
    pub fn status_for(&self, browser_id: &str) -> BrowserStatus {
        match self.state.lock() {
            Ok(s) => {
                let best = Self::best_client(&s, None, |c| {
                    let resolved = resolve_browser(&c.browser, &self.installed);
                    browser_id.is_empty() || same_browser(browser_id, &resolved)
                });
                Self::status_from_best(best, &self.installed, s.policy.is_none())
            }
            Err(_) => unavailable("Browser bridge failed."),
        }
    }
    /// Status of the connected browser whose platform id matches the
    /// given foreground app sample id (macOS bundle id or Windows
    /// executable path), if any. Returns a not-connected status when the
    /// foreground app is not a paired browser, so another paired
    /// browser's report can never be attributed to this app.
    pub fn status_for_app(&self, app_id: &str) -> BrowserStatus {
        if app_id.is_empty() {
            return unavailable("");
        }
        match self.state.lock() {
            Ok(s) => {
                let best = Self::best_client(&s, None, |c| {
                    let resolved = resolve_browser(&c.browser, &self.installed);
                    same_browser(app_id, &resolved)
                });
                Self::status_from_best(best, &self.installed, s.policy.is_none())
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
        search_distracted: false,
        channel_blocked: false,
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
    fn frozen_policy_is_an_allowlist_of_only_the_frozen_websites() {
        // Step 12: a frozen session's own blocklist, exceptions,
        // and keywords are all replaced by the frozen allowlist;
        // the adult overlay still applies on top of it.
        let c = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            block_adult_content: true,
            domains: vec!["youtube.com".into(), "search:chess".into()],
            exceptions: vec!["youtube.com".into()],
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec![],
                allowed_websites: vec!["docs.example.com".into()],
            },
            ..Default::default()
        };
        let p = Policy::new("s", &c, 60_000);
        assert_eq!(p.mode, "allowlist");
        assert_eq!(p.domains, vec!["docs.example.com"]);
        assert!(p.exceptions.is_empty());
        assert!(p.keywords.is_empty());
        assert!(p.extra_blocked.contains(&"pornhub.com".to_string()));
        // Default frozen: no websites allowed at all.
        let c = FocusConfig {
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec![],
                allowed_websites: vec![],
            },
            ..Default::default()
        };
        let p = Policy::new("s", &c, 60_000);
        assert_eq!(p.mode, "allowlist");
        assert!(p.domains.is_empty());
    }

    #[test]
    fn policy_splits_keywords_and_adult_overlay_from_host_rules() {
        let c = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            block_adult_content: true,
            domains: vec![
                "youtube.com/shorts".into(),
                "*.example.com".into(),
                "search:chess".into(),
            ],
            ..Default::default()
        };
        let p = Policy::new("s", &c, 60_000);
        assert_eq!(p.mode, "blocklist");
        assert_eq!(p.domains, vec!["youtube.com/shorts", "*.example.com"]);
        assert_eq!(p.keywords, vec!["chess"]);
        assert!(p.extra_blocked.contains(&"pornhub.com".to_string()));
        let off = FocusConfig {
            website_blocking: WebsiteBlocking::None,
            block_adult_content: true,
            domains: vec!["search:chess".into()],
            ..Default::default()
        };
        let p = Policy::new("s", &off, 60_000);
        assert!(p.domains.is_empty());
        assert!(p.keywords.is_empty());
        assert!(p.extra_blocked.is_empty());
    }
    #[test]
    fn policy_appends_exhausted_allowance_rules_to_the_overlay() {
        // No website blocking and no adult toggle: an exhausted site
        // still lands in extra_blocked, where the extension blocks
        // it regardless of mode — no extension change needed.
        let c = FocusConfig::default();
        let p = Policy::new("s", &c, 60_000).with_exhausted_rules(&["youtube.com".into()]);
        assert!(p.extra_blocked.contains(&"youtube.com".to_string()));
        // Adult entries are never duplicated by the append.
        let adult = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            block_adult_content: true,
            ..Default::default()
        };
        let p = Policy::new("s", &adult, 60_000)
            .with_exhausted_rules(&["pornhub.com".into(), "youtube.com".into()]);
        assert_eq!(
            p.extra_blocked
                .iter()
                .filter(|e| *e == "pornhub.com")
                .count(),
            1
        );
        assert!(p.extra_blocked.contains(&"youtube.com".to_string()));
    }
    #[test]
    fn policy_carries_exceptions_only_for_blocklist() {
        let c = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            domains: vec!["youtube.com".into()],
            exceptions: vec!["youtube.com".into()],
            ..Default::default()
        };
        let p = Policy::new("s", &c, 60_000);
        assert_eq!(p.exceptions, vec!["youtube.com"]);
        // Allowlist: exceptions are meaningless, so they are stripped.
        let allow = FocusConfig {
            website_blocking: WebsiteBlocking::Allowlist,
            ..c.clone()
        };
        assert!(Policy::new("s", &allow, 60_000).exceptions.is_empty());
        // Offline: everything is blocked, exceptions do NOT apply.
        let mut off = c.clone();
        off.general_internet = false;
        let p = Policy::new("s", &off, 60_000);
        assert_eq!(p.mode, "offline");
        assert!(p.exceptions.is_empty());
        // No website blocking: no exceptions either.
        let mut none = c;
        none.website_blocking = WebsiteBlocking::None;
        assert!(Policy::new("s", &none, 60_000).exceptions.is_empty());
    }
    #[test]
    fn policy_carries_normalized_youtube_channel_rules() {
        let c = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            youtube_blocked_channels: vec!["youtube.com/@Beta".into()],
            youtube_allowed_channels: vec!["UCabcdefghijklmnopqrstuv".into()],
            youtube_study_mode: true,
            youtube_allow_watch_later: true,
            ..Default::default()
        };
        let p = Policy::new("s", &c, 60_000);
        assert_eq!(p.youtube_blocked_channels, vec!["@beta"]);
        assert_eq!(p.youtube_allowed_channels, vec!["UCabcdefghijklmnopqrstuv"]);
        assert!(p.youtube_study_mode);
        assert!(p.youtube_allow_watch_later);
        // Invalid entries never break a policy; they are dropped.
        let dirty = FocusConfig {
            youtube_blocked_channels: vec!["youtube.com/watch?v=x".into()],
            ..c.clone()
        };
        assert!(Policy::new("s", &dirty, 60_000)
            .youtube_blocked_channels
            .is_empty());
        // YouTube rules are carried even without website blocking (fix 2026-11-03:
        // a channel-only block must reach the extension).
        let mut none = c.clone();
        none.website_blocking = WebsiteBlocking::None;
        let p = Policy::new("s", &none, 60_000);
        assert_eq!(p.youtube_blocked_channels, vec!["@beta"]);
        assert!(p.youtube_study_mode && p.youtube_allow_watch_later);
        // Offline with no YouTube rules: still offline, no channels.
        let off_no_yt = FocusConfig {
            website_blocking: WebsiteBlocking::Blocklist,
            general_internet: false,
            ..Default::default()
        };
        let p = Policy::new("s", &off_no_yt, 60_000);
        assert_eq!(p.mode, "offline");
        assert!(p.youtube_blocked_channels.is_empty());
        assert!(!p.youtube_study_mode);
        // Offline WITH YouTube rules: not offline mode, channels carried.
        let mut off = c;
        off.general_internet = false;
        let p = Policy::new("s", &off, 60_000);
        assert_ne!(p.mode, "offline");
        assert_eq!(p.youtube_blocked_channels, vec!["@beta"]);
        assert!(p.youtube_study_mode);
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
    #[test]
    fn status_for_app_scopes_the_report_to_the_foreground_browser() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::test_bridge(dir.path());
        let chrome = "11111111-1111-1111-1111-111111111111";
        let comet = "22222222-2222-2222-2222-222222222222";
        // The most recently seen client (Comet) reports its own domain;
        // the Chrome client reports instagram.com and is focused.
        bridge.set_activity(chrome, "com.google.chrome", "instagram.com", true);
        bridge.set_activity(comet, "ai.perplexity.comet", "comet.example.com", true);
        // status_for_app scopes by the foreground app instead.
        let s = bridge.status_for_app("com.google.Chrome");
        assert_eq!(s.client_browser, "com.google.chrome");
        assert_eq!(s.active_domain, "instagram.com");
        assert!(s.browser_focused);
        assert!(s.activity_fresh);
        assert!(s.connected);
        // A non-browser foreground app matches nothing.
        let s = bridge.status_for_app("com.microsoft.VSCode");
        assert!(!s.connected);
        assert!(s.active_domain.is_empty());
        assert!(s.client_browser.is_empty());
        // Windows exe-path foreground app matches the same client.
        let s = bridge.status_for_app(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        assert_eq!(s.client_browser, "com.google.chrome");
        assert_eq!(s.active_domain, "instagram.com");
        // The foreground Comet app does not leak Chrome's domain.
        let s = bridge.status_for_app("ai.perplexity.comet");
        assert_eq!(s.active_domain, "comet.example.com");
    }
}
