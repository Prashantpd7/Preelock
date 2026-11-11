//! Native website identification that does not need the browser
//! extension.
//!
//! The OS adapters (macOS Accessibility, Windows UI Automation) answer one
//! question per engine tick: what page is the frontmost window of the
//! foreground browser showing? This module decides what may be kept from
//! that answer and how it combines with the extension's report.
//!
//! Privacy boundary: only a hostname leaves this module. Paths, queries,
//! page titles, form values and page contents are never stored, and the
//! adapters never read them. No DNS, proxy or IP information is involved,
//! so a VPN changes nothing here.
//!
//! Accounting boundary: [`credited`] returns at most one domain and its
//! source. Screen Time credits exactly one bucket per interval (a website
//! replaces the browser application, never adds to it), so the native and
//! extension paths can never both be credited for the same interval.
use crate::browser::same_browser;
use crate::platform;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Longest hostname accepted, matching the extension heartbeat's limit.
pub const MAX_HOST: usize = 253;

/// Which integration produced the domain credited for an interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The Preelock browser extension reported the focused tab.
    Extension,
    /// An OS adapter read the foreground browser's active page.
    Native,
}

/// Native tracking capability for one browser (requirement 8). Every
/// state is reachable in practice; nothing here claims more than the OS
/// adapter actually delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// The adapter read a verified hostname for this browser.
    Supported,
    /// The browser is known but exposes no supported native tab
    /// interface: its usage is still recorded at application level.
    BrowserUsageOnly,
    /// The adapter needs an OS permission that is not granted.
    PermissionRequired,
    /// This platform has no adapter at all.
    Unavailable,
    /// The adapter exists and is permitted, but had no verified page for
    /// this tick (private window, browser UI page, no window, elevated
    /// process). Intervals fall back to browser application usage.
    TemporarilyUnavailable,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Supported => "supported",
            Capability::BrowserUsageOnly => "browser_usage_only",
            Capability::PermissionRequired => "permission_required",
            Capability::Unavailable => "unavailable",
            Capability::TemporarilyUnavailable => "temporarily_unavailable",
        }
    }
}

/// A validated native observation of the foreground browser's page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSite {
    /// Browser the observation belongs to (the foreground app id it was
    /// sampled for, macOS bundle id or Windows executable path).
    pub app: String,
    /// Lower-case hostname only.
    pub domain: String,
}

/// Borrowed form used by the accounting rule, so an interval can be
/// attributed without allocating.
#[derive(Clone, Copy, Debug)]
pub struct NativeSiteRef<'a> {
    pub app: &'a str,
    pub domain: &'a str,
}

/// The extension's report for the foreground browser this interval.
#[derive(Clone, Copy, Debug)]
pub struct ExtensionSite<'a> {
    pub browser: &'a str,
    pub domain: &'a str,
    pub focused: bool,
    pub fresh: bool,
}

/// Browsers whose active page this platform's adapter can read.
///
/// macOS: every Chromium-family browser and Safari expose the page URL
/// through the Accessibility API (`AXWebArea`'s `AXURL`).
/// Windows: Chromium-family browsers expose their address bar value
/// through UI Automation. Safari does not exist there.
///
/// A browser not listed here is honestly reported as
/// [`Capability::BrowserUsageOnly`]; it is still tracked at application
/// level, exactly as before.
#[cfg(target_os = "macos")]
const NATIVE_ADAPTERS: &[&str] = &[
    "com.google.chrome",
    "com.microsoft.edgemac",
    "com.brave.browser",
    "com.vivaldi.vivaldi",
    "org.chromium.chromium",
    "ai.perplexity.comet",
    "com.operasoftware.opera",
    "com.apple.safari",
];
#[cfg(target_os = "windows")]
const NATIVE_ADAPTERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "brave.exe",
    "vivaldi.exe",
    "chromium.exe",
    "comet.exe",
    "opera.exe",
];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const NATIVE_ADAPTERS: &[&str] = &[];

/// Whether a native adapter exists for this platform app id. Accepts the
/// bundle id as-is and also a full executable path's file name (Windows
/// reports executable paths).
pub fn has_native_adapter(app_id: &str) -> bool {
    let id = app_id.to_ascii_lowercase();
    if id.is_empty() {
        return false;
    }
    if NATIVE_ADAPTERS.contains(&id.as_str()) {
        return true;
    }
    let leaf = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    NATIVE_ADAPTERS.iter().any(|a| a.eq_ignore_ascii_case(leaf))
}

/// Capability for a browser given the adapter's last answer.
pub fn capability_for(app_id: &str, status: &str, identified: bool) -> Capability {
    if !platform::supported() {
        return Capability::Unavailable;
    }
    if !has_native_adapter(app_id) {
        return Capability::BrowserUsageOnly;
    }
    match status {
        "permission_required" => Capability::PermissionRequired,
        "unsupported" | "unavailable" => Capability::Unavailable,
        "ok" if identified => Capability::Supported,
        _ => Capability::TemporarilyUnavailable,
    }
}

/// The single reconciliation rule: which domain (if any) an interval is
/// credited to, and where it came from.
///
/// 1. The extension wins when it reports a fresh, focused domain for the
///    foreground browser — it knows the focused tab exactly.
/// 2. Otherwise a native observation is used, but only when it belongs to
///    the foreground app (never another browser's page).
/// 3. Otherwise `None`: the interval is browser application usage, so no
///    website is invented and no time is lost.
///
/// At most one domain is ever returned, so the two sources cannot both be
/// credited for the same interval.
pub fn credited<'a>(
    app_id: &str,
    extension: ExtensionSite<'a>,
    native: Option<NativeSiteRef<'a>>,
) -> Option<(&'a str, Source)> {
    if extension.fresh
        && extension.focused
        && !extension.domain.is_empty()
        && same_browser(app_id, extension.browser)
    {
        return Some((extension.domain, Source::Extension));
    }
    let native = native?;
    if native.domain.is_empty() || !same_browser(app_id, native.app) {
        return None;
    }
    Some((native.domain, Source::Native))
}

/// Validate a hostname for storage. Lower-cased, no trailing dot, at most
/// [`MAX_HOST`] bytes, alphanumeric/`-`/`.` labels only (the same
/// alphabet the extension heartbeat accepts, so both sources produce the
/// same bucket target). The last label must contain a letter, which keeps
/// version numbers and other dotted numbers from being stored as sites.
fn valid_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > MAX_HOST {
        return None;
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return None;
    }
    if host
        .split('.')
        .any(|label| label.is_empty() || label.len() > 63)
    {
        return None;
    }
    if !host
        .rsplit('.')
        .next()
        .is_some_and(|last| last.bytes().any(|b| b.is_ascii_alphabetic()))
    {
        return None;
    }
    Some(host)
}

/// The hostname of a page URL, or `None` when the text is not an
/// `http(s)` page. Accepts a full URL or the scheme-less form browsers
/// show in their address bar (`youtube.com/watch?v=x`); everything else —
/// `chrome://`/`about:`/`file:`/`javascript:` pages, search queries typed
/// into the bar, text with spaces — is rejected rather than guessed.
pub fn domain_from_url(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.len() > 4096 || text.contains(char::is_whitespace) {
        return None;
    }
    let candidate = if text.contains("://") {
        text.to_string()
    } else {
        // A bare address: require a dotted host before the first path
        // segment, so "cats" (a search query) is never a website.
        let head = text.split(['/', '?', '#']).next().unwrap_or("");
        if !head.contains('.') {
            return None;
        }
        format!("https://{text}")
    };
    let url = url::Url::parse(&candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    valid_host(url.host_str()?)
}

/// Permission requests are issued at most once per process: a background
/// engine must never re-prompt on every tick.
static PERMISSION_ASKED: AtomicBool = AtomicBool::new(false);
/// Last logged capability, so a transition is logged once (255 = none).
static LAST_CAPABILITY: AtomicU8 = AtomicU8::new(255);

fn report(app_id: &str, capability: Capability) {
    let code = match capability {
        Capability::Supported => 1,
        Capability::BrowserUsageOnly => 2,
        Capability::PermissionRequired => 3,
        Capability::Unavailable => 4,
        Capability::TemporarilyUnavailable => 5,
    };
    if LAST_CAPABILITY.swap(code, Ordering::Relaxed) != code {
        log::info!(
            "Native website tracking for {app_id}: {}",
            capability.as_str()
        );
    }
}

/// Sample the foreground app's active browser page through the platform
/// adapter. Returns a validated hostname only — never a URL, path, title
/// or page content. Cheap for non-browsers: no OS query is made unless a
/// native adapter exists for the foreground app.
pub fn native_site(app_id: &str) -> Option<NativeSite> {
    if app_id.is_empty() {
        return None;
    }
    if !has_native_adapter(app_id) {
        report(app_id, Capability::BrowserUsageOnly);
        return None;
    }
    let sample = platform::web_sample();
    let domain = domain_from_url(&sample.url);
    let capability = capability_for(app_id, &sample.status, domain.is_some());
    if capability == Capability::PermissionRequired
        && !PERMISSION_ASKED.swap(true, Ordering::Relaxed)
    {
        // One system prompt per process; a denial is handled by falling
        // back to browser application usage, never by retrying in a loop.
        platform::request_web_permission();
    }
    report(app_id, capability);
    domain.map(|domain| NativeSite {
        app: app_id.to_string(),
        domain,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_hosts_only() {
        assert_eq!(
            domain_from_url("https://www.youtube.com/watch?v=x&list=WL").unwrap(),
            "www.youtube.com"
        );
        assert_eq!(
            domain_from_url("http://github.com:443/pulls").unwrap(),
            "github.com"
        );
        // The scheme-less address-bar form browsers display.
        assert_eq!(
            domain_from_url("youtube.com/watch?v=x").unwrap(),
            "youtube.com"
        );
        assert_eq!(domain_from_url("m.youtube.com").unwrap(), "m.youtube.com");
        // Host case and a trailing root dot are normalized, so the native
        // and extension paths produce the same bucket target.
        assert_eq!(
            domain_from_url("HTTPS://WWW.Example.COM./x").unwrap(),
            "www.example.com"
        );
        // Nothing that is not a web page becomes a website.
        for text in [
            "chrome://settings",
            "about:blank",
            "file:///Users/someone/notes.txt",
            "javascript:alert(1)",
            "edge://newtab",
            "view-source:https://example.com",
            "",
            "   ",
            "cats",
            "how to cook rice",
            "https://",
            "https://example.com:notaport/",
            "3.14",
            "1.2.3.4",
            "not a url",
        ] {
            assert_eq!(domain_from_url(text), None, "{text:?} must not be a site");
        }
    }

    #[test]
    fn rejects_malformed_and_oversized_hosts() {
        assert_eq!(domain_from_url("https://exa mple.com/"), None);
        assert_eq!(domain_from_url("https://.com/"), None);
        assert_eq!(domain_from_url("https://example..com/"), None);
        assert_eq!(
            domain_from_url(&format!("https://{}.com/", "a".repeat(300))),
            None
        );
        // A number-only last label is not a site (never a guessed domain).
        assert_eq!(domain_from_url("https://example.123/"), None);
        // A real scheme-less single-label address (a dev server) is not
        // guessed from the address bar, but an explicit http(s) URL is.
        assert_eq!(domain_from_url("localhost:3000"), None);
        assert_eq!(
            domain_from_url("https://localhost:3000/").unwrap(),
            "localhost"
        );
    }

    #[test]
    fn never_leaks_a_path_query_or_fragment() {
        let domain = domain_from_url("https://news.example.com/a/b?c=d#e").unwrap();
        assert_eq!(domain, "news.example.com");
        assert!(!domain.contains('/'));
        assert!(!domain.contains('?'));
        assert!(!domain.contains('#'));
    }

    #[test]
    fn extension_wins_over_native_for_the_same_interval() {
        let native = NativeSiteRef {
            app: "com.google.Chrome",
            domain: "github.com",
        };
        let credited = credited(
            "com.google.Chrome",
            ExtensionSite {
                browser: "com.google.chrome",
                domain: "youtube.com",
                focused: true,
                fresh: true,
            },
            Some(native),
        );
        assert_eq!(credited, Some(("youtube.com", Source::Extension)));
    }

    #[test]
    fn native_covers_every_extension_gap() {
        let native = NativeSiteRef {
            app: "com.google.Chrome",
            domain: "github.com",
        };
        // Disconnected (no client), stale heartbeat, unfocused window and
        // a browser UI page (empty domain) all fall through to native.
        for extension in [
            ExtensionSite {
                browser: "",
                domain: "",
                focused: false,
                fresh: false,
            },
            ExtensionSite {
                browser: "com.google.chrome",
                domain: "youtube.com",
                focused: false,
                fresh: true,
            },
            ExtensionSite {
                browser: "com.google.chrome",
                domain: "youtube.com",
                focused: true,
                fresh: false,
            },
            ExtensionSite {
                browser: "com.google.chrome",
                domain: "",
                focused: true,
                fresh: true,
            },
        ] {
            assert_eq!(
                credited("com.google.Chrome", extension, Some(native)),
                Some(("github.com", Source::Native))
            );
        }
    }

    #[test]
    fn another_browsers_observation_is_never_credited() {
        // The native sample always belongs to the frontmost window, but a
        // mismatched id must still be refused rather than misattributed.
        let native = NativeSiteRef {
            app: "com.microsoft.edgemac",
            domain: "edge-news.microsoft.com",
        };
        assert_eq!(
            credited(
                "com.google.Chrome",
                ExtensionSite {
                    browser: "",
                    domain: "",
                    focused: false,
                    fresh: false
                },
                Some(native)
            ),
            None
        );
        // Same for an extension report from another browser.
        assert_eq!(
            credited(
                "com.google.Chrome",
                ExtensionSite {
                    browser: "com.microsoft.edgemac",
                    domain: "edge-news.microsoft.com",
                    focused: true,
                    fresh: true,
                },
                None
            ),
            None
        );
    }

    #[test]
    fn unavailable_identification_credits_nothing() {
        assert_eq!(
            credited(
                "com.google.Chrome",
                ExtensionSite {
                    browser: "",
                    domain: "",
                    focused: false,
                    fresh: false
                },
                Some(NativeSiteRef {
                    app: "com.google.Chrome",
                    domain: ""
                })
            ),
            None
        );
        assert_eq!(
            credited(
                "com.apple.Safari",
                ExtensionSite {
                    browser: "",
                    domain: "",
                    focused: false,
                    fresh: false
                },
                None
            ),
            None
        );
    }

    #[test]
    fn capability_reports_only_what_was_delivered() {
        let browser = if cfg!(target_os = "windows") {
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        } else {
            "com.google.Chrome"
        };
        assert_eq!(capability_for(browser, "ok", true), Capability::Supported);
        assert_eq!(
            capability_for(browser, "ok", false),
            Capability::TemporarilyUnavailable
        );
        assert_eq!(
            capability_for(browser, "permission_required", false),
            Capability::PermissionRequired
        );
        assert_eq!(
            capability_for(browser, "unsupported", false),
            Capability::Unavailable
        );
        // A browser with no adapter is tracked at application level only.
        assert_eq!(
            capability_for("org.mozilla.firefox", "ok", false),
            Capability::BrowserUsageOnly
        );
        assert_eq!(
            capability_for("", "ok", false),
            Capability::BrowserUsageOnly
        );
    }

    /// Manual on-device check (not part of the suite): brings the OS
    /// adapter's answer into the open so a human can confirm it against a
    /// real browser window. Run with a browser in the foreground:
    /// `cargo test --lib -- --ignored --nocapture native_sample_probe`.
    #[test]
    #[ignore = "manual: needs a foreground browser window and OS permission"]
    fn native_sample_probe() {
        let sample = platform::web_sample();
        println!(
            "web_sample status={} url={} permission={}",
            sample.status,
            sample.url,
            platform::web_permission()
        );
        println!("domain={:?}", domain_from_url(&sample.url));
    }

    #[test]
    fn adapters_are_platform_specific() {
        if cfg!(target_os = "windows") {
            assert!(has_native_adapter(
                r"C:\Program Files\Google\Chrome\Application\chrome.exe"
            ));
            assert!(has_native_adapter(
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
            ));
            assert!(!has_native_adapter("com.apple.Safari"));
        } else if cfg!(target_os = "macos") {
            assert!(has_native_adapter("com.google.Chrome"));
            assert!(has_native_adapter("com.apple.Safari"));
            assert!(has_native_adapter("ai.perplexity.comet"));
            assert!(!has_native_adapter("chrome.exe"));
            assert!(!has_native_adapter("org.mozilla.firefox"));
        }
    }
}
