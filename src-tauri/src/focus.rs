use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Standard,
    Strict,
    Deep,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WebsiteBlocking {
    /// Sessions saved before this choice existed: follow `mode`.
    #[default]
    Auto,
    /// No website blocking.
    None,
    /// `domains` lists the sites to block.
    Blocklist,
    /// `domains` lists the sites to keep; everything else is blocked.
    Allowlist,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusConfig {
    pub mode: Mode,
    pub apps: Vec<String>,
    pub domains: Vec<String>,
    pub website_blocking: WebsiteBlocking,
    pub general_internet: bool,
    pub browser_app: String,
    pub activity_consent: bool,
    pub camera: bool,
    pub attention: crate::attention::Policy,
}
impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Standard,
            apps: vec![],
            domains: vec![],
            website_blocking: WebsiteBlocking::Auto,
            general_internet: true,
            browser_app: String::new(),
            activity_consent: false,
            camera: false,
            attention: Default::default(),
        }
    }
}
impl FocusConfig {
    pub fn network_required(&self) -> bool {
        !self.general_internet
            || !self.domains.is_empty()
            || (self.mode != Mode::Standard && !self.browser_app.is_empty())
            || !self.attention.work_domains.is_empty()
    }
    /// The single browser this session's website rules cover. One browser
    /// per session: the engine requires it to be paired before the
    /// session starts.
    pub fn session_browsers(&self) -> Vec<String> {
        if self.browser_app.is_empty() {
            Vec::new()
        } else {
            vec![self.browser_app.clone()]
        }
    }
    pub fn apps_required(&self) -> bool {
        self.activity_consent
    }
    pub fn blocked(&self, id: &str) -> bool {
        if protected(id) {
            return false;
        }
        if !self.activity_consent {
            // App blocking is off: other apps are left alone.
            return false;
        }
        // "Block all other apps" is on: strict allowlist — everything except
        // the chosen work apps and the paired browser is blocked.
        let work = if self.mode == Mode::Standard {
            self.attention.work_apps.iter().any(|a| ids_match(a, id))
        } else {
            self.apps.iter().any(|a| ids_match(a, id))
        };
        !work && !ids_match(id, &self.browser_app)
    }
}
/// Robust app-identity comparison. On Windows the id is a full exe path, and
/// Windows can return the same exe with a different path spelling across
/// launches (\\?\ prefix, 8.3 short names, separator style). Normalize first,
/// then fall back to exe-filename comparison so a reopened work app is never
/// mistaken for a blocked app. On macOS the id is a bundle identifier, where
/// normalization is a no-op and the filename fallback never triggers.
fn normalize_id(id: &str) -> String {
    let mut s = id.trim().to_string();
    // Strip Win32 extended-path prefix: \\?\C:\... -> C:\...
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        s = rest.to_string();
    } else if let Some(rest) = s.strip_prefix("//?/") {
        s = rest.to_string();
    }
    s.replace('/', "\\").to_ascii_lowercase()
}
fn id_filename(id: &str) -> &str {
    id.rsplit(['/', '\\']).next().unwrap_or(id)
}
pub fn ids_match(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let na = normalize_id(a);
    let nb = normalize_id(b);
    if na == nb {
        return true;
    }
    // Fallback: same exe filename (handles reinstalls to a different folder
    // or short-name vs long-name path variants).
    let fa = id_filename(&na);
    let fb = id_filename(&nb);
    !fa.is_empty() && fa == fb
}
pub fn protected(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let leaf = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    [
        "com.preelock.desktop",
        "com.apple.finder",
        "com.apple.systempreferences",
        "com.apple.activitymonitor",
        "com.apple.loginwindow",
        "com.apple.securityagent",
        "com.apple.systemuiserver",
        "com.apple.controlcenter",
    ]
    .contains(&id.as_str())
        || [
            "preelock.exe",
            "explorer.exe",
            "taskmgr.exe",
            "systemsettings.exe",
            "applicationframehost.exe",
            "consent.exe",
            "logonui.exe",
            "securityhealthsystray.exe",
        ]
        .contains(&leaf)
}
pub fn supported_browser(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    [
        "com.google.chrome",
        "com.microsoft.edgemac",
        "com.brave.browser",
        "com.vivaldi.vivaldi",
        "org.chromium.chromium",
        "ai.perplexity.comet",
    ]
    .contains(&id.as_str())
        || [
            "chrome.exe",
            "msedge.exe",
            "brave.exe",
            "vivaldi.exe",
            "chromium.exe",
            "comet.exe",
        ]
        .iter()
        .any(|name| id.ends_with(&format!("\\{name}")))
}
pub fn known_browser(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    supported_browser(&id)
        || [
            "com.apple.safari",
            "org.mozilla.firefox",
            "com.operasoftware.opera",
        ]
        .contains(&id.as_str())
        || ["firefox.exe", "opera.exe"]
            .iter()
            .any(|name| id.ends_with(&format!("\\{name}")))
}
pub fn domains(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 domains.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim().trim_end_matches('.');
        if value.is_empty() {
            continue;
        }
        if value.chars().any(char::is_control)
            || value.contains(['/', ':', '@', '*', '?', '#', ' '])
        {
            return Err(format!(
                "Use a bare domain, without paths or wildcards: {value}"
            ));
        }
        let url = url::Url::parse(&format!("https://{value}")).map_err(|_| "Invalid domain.")?;
        let host = url
            .host_str()
            .ok_or("Invalid domain.")?
            .to_ascii_lowercase();
        if host.parse::<std::net::IpAddr>().is_ok()
            || !host.contains('.')
            || host.len() > 253
            || host.ends_with(".localhost")
            || host.ends_with(".local")
            || host.split('.').any(|l| {
                l.is_empty()
                    || l.len() > 63
                    || l.starts_with('-')
                    || l.ends_with('-')
                    || !l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(
                "Use public DNS domains; IP addresses and local network names are excluded.".into(),
            );
        }
        result.push(host);
    }
    result.sort();
    result.dedup();
    Ok(result)
}
pub fn validate(mut config: FocusConfig) -> Result<FocusConfig, String> {
    config.domains = domains(config.domains)?;
    config.attention.work_domains = domains(config.attention.work_domains)?;
    if !["coding", "lecture", "reading", "writing", "other"]
        .contains(&config.attention.task.as_str())
        || !["manual_checkpoints", "activity_only", "camera_and_activity"]
            .contains(&config.attention.verification.as_str())
        || !["unknown", "activity_only"].contains(&config.attention.camera_fallback.as_str())
        || !(3..=30).contains(&config.attention.grace_seconds)
    {
        return Err("Invalid attention policy.".into());
    }
    if config.attention.work_apps.len() > 100
        || config
            .attention
            .work_apps
            .iter()
            .any(|id| id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control))
    {
        return Err("Invalid productive application list.".into());
    }
    if config.apps.len() > 100
        || config
            .apps
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > 1024 || id.chars().any(char::is_control))
    {
        return Err("Invalid application selection.".into());
    }
    if config.attention.verification != "manual_checkpoints" {
        if !config.activity_consent {
            return Err(
                "Activity verification requires foreground-app and hostname sampling consent."
                    .into(),
            );
        }
        if config.mode == Mode::Standard
            && config.attention.work_apps.is_empty()
            && config.attention.work_domains.is_empty()
        {
            return Err("Choose productive applications or browser domains, or use self-reported checkpoints.".into());
        }
        if config
            .attention
            .work_apps
            .iter()
            .any(|id| config.mode == Mode::Standard && config.apps.contains(id))
        {
            return Err("A productive app cannot also be blocked.".into());
        }
    }
    config.apps.sort();
    config.apps.dedup();
    if config.apps.iter().any(|id| protected(id)) && config.mode == Mode::Standard {
        return Err("System recovery applications cannot be blocked.".into());
    }
    if config.mode == Mode::Strict && config.apps.is_empty() && config.browser_app.is_empty() {
        return Err("Choose a work app or browser for Strict mode.".into());
    }
    if config.mode == Mode::Strict
        && !config.browser_app.is_empty()
        && config.general_internet
        && config.domains.is_empty()
    {
        return Err("Strict mode needs an approved domain list, or browser offline mode.".into());
    }
    if config.network_required() && !supported_browser(&config.browser_app) {
        return Err("Choose your browser and connect it in Settings → Connections.".into());
    }
    if config.mode == Mode::Standard && config.apps.iter().any(|a| a == &config.browser_app) {
        return Err("The paired browser cannot also be in the app blocklist.".into());
    }
    // One browser per session: a second browser approved as a work app
    // would need its own pairing and its own rules. Browsers the
    // extension cannot run in stay forbidden, since they would browse
    // unfiltered.
    if config.mode == Mode::Strict
        && config
            .apps
            .iter()
            .any(|id| supported_browser(id) && !ids_match(id, &config.browser_app))
    {
        return Err("Only one browser per session. Remove the other browser from your work apps, or pick it as your browser.".into());
    }
    if config.mode == Mode::Strict
        && config
            .apps
            .iter()
            .any(|id| known_browser(id) && !supported_browser(id))
    {
        return Err("Strict mode cannot approve a browser the Preelock extension doesn't support. Remove it from approved apps.".into());
    }
    Ok(config)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub at: i64,
    pub kind: String,
    pub detail: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusReport {
    pub events: Vec<Event>,
    pub approved_ms: u64,
    pub distracting_ms: u64,
    pub idle_ms: u64,
    pub interruptions: u32,
    pub active_app: String,
    pub enforcement: String,
    pub camera_status: String,
    pub browser_blocks: u64,
}
impl FocusReport {
    pub fn event(&mut self, at: i64, kind: &str, detail: &str) {
        self.events.push(Event {
            at,
            kind: kind.into(),
            detail: detail.chars().take(240).collect(),
        });
        if self.events.len() > 200 {
            self.events.remove(0);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comet_pairing_and_app_only_strict_policy() {
        assert!(supported_browser("ai.perplexity.comet"));
        let app_only = FocusConfig {
            mode: Mode::Strict,
            apps: vec!["com.microsoft.VSCode".into()],
            activity_consent: true,
            ..Default::default()
        };
        assert!(!app_only.network_required());
        assert!(validate(app_only.clone()).is_ok());
        assert!(app_only.blocked("ai.perplexity.comet"));
        let browser = FocusConfig {
            browser_app: "ai.perplexity.comet".into(),
            domains: vec!["docs.rs".into()],
            ..app_only
        };
        assert!(browser.network_required());
        assert!(validate(browser.clone()).is_ok());
        assert!(!browser.blocked("ai.perplexity.comet"));
    }
    #[test]
    fn strict_rejects_second_browser_but_not_unsupported_ones() {
        let one = FocusConfig {
            mode: Mode::Strict,
            browser_app: "ai.perplexity.comet".into(),
            apps: vec!["ai.perplexity.comet".into(), "com.microsoft.VSCode".into()],
            activity_consent: true,
            domains: vec!["github.com".into()],
            ..Default::default()
        };
        assert!(validate(one.clone()).is_ok());
        assert_eq!(one.session_browsers(), vec!["ai.perplexity.comet"]);
        let two = FocusConfig {
            apps: vec![
                "ai.perplexity.comet".into(),
                "com.brave.Browser".into(),
                "com.microsoft.VSCode".into(),
            ],
            ..one.clone()
        };
        assert!(validate(two).is_err());
        let bad = FocusConfig {
            apps: vec!["com.apple.Safari".into()],
            ..one
        };
        assert!(validate(bad).is_err());
    }
    #[test]
    fn normalizes_domains_and_rejects_injection() {
        assert_eq!(
            domains(vec![" Example.COM. ".into(), "example.com".into()]).unwrap(),
            vec!["example.com"]
        );
        for bad in [
            "*.com",
            "a.com/path",
            "127.0.0.1",
            "localhost",
            "a.com:80",
            "a.com\nInjected",
            "-a.com",
        ] {
            assert!(domains(vec![bad.into()]).is_err(), "{bad}");
        }
    }
    #[test]
    fn strict_defaults_require_consent_and_domains() {
        assert!(validate(FocusConfig {
            mode: Mode::Strict,
            ..Default::default()
        })
        .is_err());
        let c = FocusConfig {
            mode: Mode::Strict,
            activity_consent: true,
            domains: vec!["docs.rs".into()],
            browser_app: "com.google.Chrome".into(),
            ..Default::default()
        };
        assert!(validate(c.clone()).is_ok());
        assert!(c.blocked("com.apple.Safari"));
        assert!(!c.blocked("com.apple.ActivityMonitor"));
        assert!(!c.blocked("com.google.Chrome"));
    }
    #[test]
    fn block_other_apps_switch() {
        // Unchecked: no app is blocked, other apps are left alone.
        let off = FocusConfig {
            mode: Mode::Strict,
            apps: vec!["com.microsoft.VSCode".into()],
            activity_consent: false,
            ..Default::default()
        };
        assert!(!off.blocked("com.apple.Safari"));
        assert!(!off.blocked("com.microsoft.VSCode"));
        // Checked in Standard mode: strict allowlist against the work apps.
        let mut flex = FocusConfig {
            mode: Mode::Standard,
            activity_consent: true,
            ..Default::default()
        };
        flex.attention.work_apps = vec!["com.microsoft.VSCode".into()];
        assert!(flex.blocked("com.apple.Safari"));
        assert!(!flex.blocked("com.microsoft.VSCode"));
        // Protected system apps are never blocked, even when checked.
        assert!(!flex.blocked("com.apple.Finder"));
    }
}
