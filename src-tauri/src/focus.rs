use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Standard,
    Strict,
    Deep,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusConfig {
    pub mode: Mode,
    pub apps: Vec<String>,
    pub domains: Vec<String>,
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
            || self.mode != Mode::Standard
            || !self.attention.work_domains.is_empty()
    }
    pub fn apps_required(&self) -> bool {
        !self.apps.is_empty() || self.mode != Mode::Standard
    }
    pub fn blocked(&self, id: &str) -> bool {
        if protected(id) {
            return false;
        }
        let selected = self.apps.iter().any(|a| a.eq_ignore_ascii_case(id));
        if self.mode == Mode::Standard {
            selected
        } else {
            !selected && !id.eq_ignore_ascii_case(&self.browser_app)
        }
    }
}
pub fn protected(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let leaf = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    [
        "com.vystriq.desktop",
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
            "vystriq.exe",
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
    ]
    .contains(&id.as_str())
        || [
            "chrome.exe",
            "msedge.exe",
            "brave.exe",
            "vivaldi.exe",
            "chromium.exe",
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
    if config.apps_required() && !config.activity_consent {
        return Err("Application restrictions require visible activity-monitoring consent.".into());
    }
    if config.mode == Mode::Strict && config.general_internet && config.domains.is_empty() {
        return Err("Strict mode needs an approved domain list, or browser offline mode.".into());
    }
    if config.network_required() && !supported_browser(&config.browser_app) {
        return Err("Select a supported Chromium browser and pair its extension.".into());
    }
    if config.mode == Mode::Standard && config.apps.iter().any(|a| a == &config.browser_app) {
        return Err("The paired browser cannot also be in the app blocklist.".into());
    }
    if config.mode == Mode::Strict
        && config
            .apps
            .iter()
            .any(|id| known_browser(id) && !id.eq_ignore_ascii_case(&config.browser_app))
    {
        return Err("Strict mode cannot approve another known browser outside the paired profile. Remove it from approved apps.".into());
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
}
