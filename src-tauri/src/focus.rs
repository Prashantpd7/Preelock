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
    /// Per-block exceptions (step 2): host/path rules that stay
    /// accessible even when they match a block rule. Only meaningful in
    /// blocklist mode; in allowlist mode the allowed list already
    /// defines what is open. Never overrides the adult overlay or
    /// offline mode (see the precedence note on `rule_matches` and in
    /// browser-extension/rules.js). Serde-default: old saved sessions
    /// load as an empty list.
    pub exceptions: Vec<String>,
    /// App-blocking depth (step 4): folders whose apps are blocked.
    /// Stored as typed (trailing separators stripped by
    /// `blocked_folders` validation). Matching is a normalized,
    /// case-insensitive path-prefix check with a separator boundary
    /// (see `path_under_folder`). Serde-default: old saved sessions
    /// load as an empty list.
    pub blocked_folders: Vec<String>,
    /// App-blocking depth (step 4): window-title substrings. If the
    /// frontmost window's title contains one of these (case-insensitive
    /// substring), the app counts as blocked even if it is otherwise
    /// allowed. Serde-default: old saved sessions load as empty.
    pub blocked_titles: Vec<String>,
    /// YouTube channel rules (step 3), stored normalized (see
    /// `normalize_youtube_channel`): `@handle` (lowercase), a bare UC
    /// channel ID (case preserved), or `c:name` / `user:name`
    /// (lowercase) for legacy channel URLs. Block-list mode blocks the
    /// listed channels; study mode allows ONLY the allowed list (see
    /// `youtube_study_mode`). Serde-default: old saved sessions load
    /// as empty lists / false.
    pub youtube_blocked_channels: Vec<String>,
    pub youtube_allowed_channels: Vec<String>,
    /// Study mode: only `youtube_allowed_channels` play. The extension
    /// additionally blocks the YouTube homepage (/), Shorts
    /// (/shorts/*), feed pages (/feed/*) and search results (/results).
    /// With an empty allowed list, study mode blocks every channel
    /// (Watch Later excepted when `youtube_allow_watch_later` is on).
    pub youtube_study_mode: bool,
    /// Blocks entertainment categories/formats on YouTube (comedy, pranks,
    /// vlogs, funny/animal clips, gaming, music, Shorts) while keeping
    /// non-entertainment videos playable.
    pub youtube_block_entertainment: bool,
    /// Watch-Later exception: exactly the user's Watch Later playlist
    /// page (`youtube.com/playlist?list=WL`) stays open even when
    /// youtube.com is domain-blocked or absent from the allowlist. It
    /// never covers another route — videos (including ones carrying
    /// `list=WL`), Shorts, search, channel pages and other playlists
    /// stay blocked. This is extension-enforced only: query parameters
    /// are invisible to the engine, so the engine still counts
    /// youtube.com as not relevant on the allowed page (see the
    /// engine-limitation note in browser-extension/rules.js).
    pub youtube_allow_watch_later: bool,
    pub block_adult_content: bool,
    pub website_blocking: WebsiteBlocking,
    pub general_internet: bool,
    pub browser_app: String,
    pub activity_consent: bool,
    pub camera: bool,
    pub attention: crate::attention::Policy,
    /// Pomodoro focus/break cycles (step 7). Serde-default: old saved
    /// sessions load as disabled.
    pub pomodoro: crate::pomodoro::PomodoroConfig,
    /// Reward and delay breaks (step 9). Serde-default: old saved
    /// sessions load as disabled.
    pub reward: crate::rewards::RewardConfig,
    /// Frozen Turkey (step 12). Serde-default: old saved sessions
    /// load unfrozen. Rides the focus snapshot, so schedule and
    /// one-shot sessions inherit it exactly like Pomodoro/reward.
    pub frozen: crate::frozen::FrozenConfig,
}
impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Standard,
            apps: vec![],
            domains: vec![],
            exceptions: vec![],
            blocked_folders: vec![],
            blocked_titles: vec![],
            youtube_blocked_channels: vec![],
            youtube_allowed_channels: vec![],
            youtube_study_mode: false,
            youtube_block_entertainment: false,
            youtube_allow_watch_later: false,
            block_adult_content: false,
            website_blocking: WebsiteBlocking::Auto,
            general_internet: true,
            browser_app: String::new(),
            activity_consent: false,
            camera: false,
            attention: Default::default(),
            pomodoro: Default::default(),
            reward: Default::default(),
            frozen: Default::default(),
        }
    }
}
impl FocusConfig {
    /// Keyword rules (the `search:<keyword>` entries of `domains`),
    /// lowercased, for the extension policy.
    pub fn website_keywords(&self) -> Vec<String> {
        self.domains.iter().filter_map(|r| keyword_of(r)).collect()
    }
    /// Host rules only (everything in `domains` except keyword rules),
    /// for the extension policy's domain list.
    pub fn website_host_rules(&self) -> Vec<String> {
        self.domains
            .iter()
            .filter(|r| !is_keyword_rule(r))
            .cloned()
            .collect()
    }
    pub fn network_required(&self) -> bool {
        !self.general_internet
            || !self.domains.is_empty()
            || (self.mode != Mode::Standard && !self.browser_app.is_empty())
            || !self.attention.work_domains.is_empty()
            // Channel rules need the extension too. The Watch-Later
            // toggle alone is not a rule (it only carves out pages
            // that other rules block), so it does not require a browser.
            || !self.youtube_blocked_channels.is_empty()
            || !self.youtube_allowed_channels.is_empty()
            || self.youtube_study_mode
            || self.youtube_block_entertainment
            // Step 12: a frozen session always publishes its
            // allowlist policy, even when it has no other website
            // rules — otherwise a browser opened during a frozen
            // session would browse unfiltered.
            || self.frozen.enabled
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
    /// Safari selected where this session relies on the browser to enforce
    /// its website rules: as the session browser, as an approved work app
    /// (both modes), or in a Strict session's app allowlist. A Safari in a
    /// Standard session's blocklist is not approved usage — blocking it
    /// needs no extension — so it does not count.
    pub fn safari_approved(&self) -> bool {
        safari(&self.browser_app)
            || self.attention.work_apps.iter().any(|id| safari(id))
            || (self.mode != Mode::Standard && self.apps.iter().any(|id| safari(id)))
    }
    pub fn blocked(&self, id: &str) -> bool {
        self.blocked_sample(id, "", "")
    }
    /// Step 4: the engine's per-sample check. `path` is the foreground
    /// app's executable/bundle path (the platform Sample's `path`
    /// field); `title` is its frontmost window title. Folder and title
    /// rules are extra block triggers on top of the existing
    /// allowlist: they block even an otherwise allowed app, but never
    /// a protected system app, and never when activity consent is off.
    /// Everything else — hiding the window, counting distracted time,
    /// logging the interruption — is the engine's existing blocked
    /// handling, unchanged. Folder/title rules, like all rules here,
    /// apply only while a session is active (the engine only samples
    /// during a Running session).
    pub fn blocked_sample(&self, id: &str, path: &str, title: &str) -> bool {
        if protected(id) {
            return false;
        }
        // Step 12 Frozen Turkey: a frozen session is a strict
        // allowlist, checked before the consent gate and before
        // folder/title rules (both are redundant here — everything
        // not allowed is blocked anyway). The paired browser gets
        // NO automatic exemption: it is blocked unless it is
        // explicitly in the frozen allowed-apps list. Validate
        // guarantees activity consent for frozen sessions, so the
        // engine always samples them; this check runs first so a
        // direct `blocked()` caller gets the frozen answer even
        // without consent.
        if self.frozen.enabled {
            return !self.frozen.app_allowed(id);
        }
        if !self.activity_consent {
            // App blocking is off: other apps are left alone.
            return false;
        }
        // Folder check: prefer the sample path; fall back to the id
        // when it looks like a path (Windows exe paths, Mac executable
        // paths for bundle-less processes). Bundle ids never match a
        // folder rule — they contain no path separator.
        let candidate = if !path.is_empty() {
            path
        } else if id.contains(['/', '\\']) {
            id
        } else {
            ""
        };
        if folder_blocked(candidate, &self.blocked_folders) {
            return true;
        }
        if title_blocked(title, &self.blocked_titles) {
            return true;
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
/// Step 4: normalized path for folder matching. Reuses `normalize_id`
/// (strips the Win32 `\\?\` prefix, unifies separators to `\`, lowercases)
/// and strips trailing separators, so `C:\Games\` and `C:/Games` are the
/// same folder. Comparison is case-insensitive on BOTH platforms: NTFS
/// is case-insensitive, and the default macOS filesystem (APFS) is
/// case-insensitive too, so a case-sensitive match would miss apps whose
/// path spelling differs only by case. A case-sensitive APFS volume is
/// the known exception — there a folder rule could over-match a
/// same-spelling-different-case path, which fail-safe users can avoid
/// by listing exact paths.
fn normalize_path(path: &str) -> String {
    normalize_id(path).trim_end_matches('\\').to_string()
}
/// Step 4: true when `path` equals `folder` or lies strictly under it.
/// The separator boundary matters: folder `/Foo` must NOT match
/// `/Foobar` — the character after the folder prefix must be a path
/// separator (guaranteed here by comparing against `folder + '\'`).
pub fn path_under_folder(path: &str, folder: &str) -> bool {
    let path = normalize_path(path);
    let folder = normalize_path(folder);
    if path.is_empty() || folder.is_empty() {
        return false;
    }
    path == folder || path.starts_with(&format!("{folder}\\"))
}
/// Step 4: true when the app's path lies under any blocked folder.
pub fn folder_blocked(path: &str, folders: &[String]) -> bool {
    !path.is_empty() && folders.iter().any(|f| path_under_folder(path, f))
}
/// Step 4: true when the window title contains any title rule
/// (case-insensitive substring, trimmed rules; empty rules never match).
pub fn title_blocked(title: &str, titles: &[String]) -> bool {
    let title = title.to_ascii_lowercase();
    !title.is_empty()
        && titles.iter().any(|t| {
            let rule = t.trim().to_ascii_lowercase();
            !rule.is_empty() && title.contains(&rule)
        })
}
/// Step 4: true when a folder path is an operating-system folder that
/// must never be blocked: macOS `/System` (or anything under it), the
/// Windows directory itself (any drive, e.g. `C:\Windows`, or anything
/// under it — which covers System32 and SysWOW64), or any path with a
/// `System32` / `SysWOW64` component. Everything else, including
/// Program Files and /Applications, is the user's choice and allowed.
/// Limitation: a Windows install in a non-standard directory (not
/// named `Windows`) is not detected by this name-based check.
pub fn is_system_folder(path: &str) -> bool {
    let norm = normalize_path(path);
    if norm.is_empty() {
        return false;
    }
    let parts: Vec<&str> = norm.split('\\').collect();
    // macOS: /System -> "\system".
    if parts.first() == Some(&"") && parts.get(1) == Some(&"system") {
        return true;
    }
    // Windows: <drive>:\Windows, or any System32/SysWOW64 component.
    if parts.len() >= 2 && parts[1] == "windows" {
        return true;
    }
    parts.iter().any(|p| *p == "system32" || *p == "syswow64")
}
/// Step 4: true for a whole filesystem root or drive root
/// (`/`, `C:\`, `C:`), which would block essentially every app.
fn is_root_folder(path: &str) -> bool {
    let raw = path.trim();
    if raw == "/" || raw == "\\" {
        return true;
    }
    let norm = normalize_path(raw);
    // After trailing-separator stripping, "C:\" normalizes to "c:".
    norm.len() == 2 && norm.as_bytes()[1] == b':' && norm.as_bytes()[0].is_ascii_alphabetic()
}
/// Step 4: validate and normalize the wizard's blocked-folder list.
/// Empty lines are skipped; anything else is kept as typed (matching
/// normalizes later). At most 100 folders; empty paths, OS folders and
/// filesystem roots are rejected with a plain-language error.
pub fn blocked_folders(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 blocked folders.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.chars().any(char::is_control) || value.len() > 1024 {
            return Err(format!("Invalid blocked folder: {value}"));
        }
        if is_root_folder(value) {
            return Err(format!(
                "A whole drive or filesystem root cannot be blocked: {value}"
            ));
        }
        if is_system_folder(value) {
            return Err(format!("System folders cannot be blocked: {value}"));
        }
        result.push(value.trim_end_matches(['/', '\\']).to_string());
    }
    result.sort();
    result.dedup();
    Ok(result)
}
/// Step 4: validate the wizard's window-title rules. One rule per line,
/// trimmed, 1-100 characters, no control characters, at most 100 rules.
/// Deduplicated case-insensitively (matching is case-insensitive).
pub fn title_rules(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 window-title rules.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.chars().any(char::is_control) || value.chars().count() > 100 {
            return Err(format!("Invalid window-title rule: {value}"));
        }
        result.push(value.to_string());
    }
    result.sort_by_key(|a| a.to_ascii_lowercase());
    let mut deduped: Vec<String> = vec![];
    for value in result {
        if deduped
            .last()
            .is_none_or(|last| !last.eq_ignore_ascii_case(&value))
        {
            deduped.push(value);
        }
    }
    Ok(deduped)
}
/// Step 4: Microsoft Store app identity. A Store (UWP/MSIX) app is
/// identified by its package family name, stored as
/// `store:<package family name>` — e.g.
/// `store:Microsoft.WindowsCalculator_8wekyb3d8bbwe`. The `store:`
/// prefix keeps Store ids distinct from exe paths and bundle ids, so
/// `ids_match` compares them as opaque identities (its filename
/// fallback compares the whole string, since a store id contains no
/// path separator). A user can also type a store id by hand in the
/// wizard: it is just an entry in the app list.
pub fn store_id_from_pfn(pfn: &str) -> Option<String> {
    let pfn = pfn.trim();
    if pfn.is_empty()
        || pfn.len() > 255
        || pfn.chars().any(char::is_control)
        || pfn.chars().any(char::is_whitespace)
        || !pfn
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        // A package family name is always Name_PublisherHash: it must
        // contain the underscore separating name from publisher hash.
        || !pfn.contains('_')
    {
        return None;
    }
    Some(format!("store:{pfn}"))
}
pub fn is_store_id(id: &str) -> bool {
    id.len() > 6 && id[..6].eq_ignore_ascii_case("store:")
}
/// The package family name inside a `store:` id, or None.
pub fn pfn_from_store_id(id: &str) -> Option<&str> {
    if is_store_id(id) {
        Some(&id[6..])
    } else {
        None
    }
}
pub fn protected(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let leaf = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    [
        "com.preelock.desktop",
        // Legacy identifier: an old bundled build may still report this.
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
            // The macOS dev/release binary is a bare executable whose
            // sample id is its path (e.g. target/debug/preelock), so the
            // leaf — not the bundle id — identifies it. Without this,
            // Preelock hides its own window and the user can never open
            // it to stop a session.
            "preelock",
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
/// Safari is a known browser with no Preelock extension. It stays a normal
/// application everywhere (screen time, app blocking, normal use); this is
/// only used to refuse it where a session's website rules must be enforced
/// inside the browser.
pub fn safari(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id == "com.apple.safari" || id.ends_with("\\safari.exe")
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
pub const ADULT_DOMAINS: &[&str] = &[
    "pornhub.com",
    "xvideos.com",
    "xnxx.com",
    "xhamster.com",
    "redtube.com",
    "youporn.com",
    "brazzers.com",
    "chaturbate.com",
    "stripchat.com",
    "livejasmin.com",
    "cam4.com",
    "onlyfans.com",
];
/// Website-rule semantics (mirrored exactly in browser-extension/rules.js).
/// Rule forms, stored normalized in FocusConfig.domains:
/// - example.com: matches the host itself and any subdomain.
/// - *.example.com: matches any subdomain, but NOT the bare domain.
/// - example.com/path: host rule as above, plus the URL path must equal
///   the rule path or start with it followed by / (youtube.com/shorts
///   matches /shorts and /shorts/abc, not /shortsabc and not /).
/// - search:<keyword>: never matches a host; only the extension enforces
///   it, blocking search-engine result pages whose query contains the
///   keyword (case-insensitive substring).
///
/// Engine limit: the engine only sees active_domain (a host, never a
/// URL, since the bridge never transmits URLs), so it evaluates rules
/// with an empty path. Exact for plain/wildcard rules; for path rules,
/// blocklist mode counts an unblocked path on that domain as relevant
/// (the extension replaces blocked pages, so a blocked-path tab cannot
/// linger), while allowlist mode is conservative and counts a path-rule
/// domain as NOT relevant, since the path cannot be confirmed. Keyword
/// rules reach the engine only via the extension reporting
/// search_distracted. Rules apply only while a session is active.
pub fn keyword_of(rule: &str) -> Option<String> {
    let rest = rule.strip_prefix("search:")?;
    let keyword = rest.trim();
    if keyword.is_empty() {
        None
    } else {
        Some(keyword.to_ascii_lowercase())
    }
}
pub fn is_keyword_rule(rule: &str) -> bool {
    rule.trim_start()
        .to_ascii_lowercase()
        .starts_with("search:")
}
pub fn search_query_matches(rule: &str, query: &str) -> bool {
    match keyword_of(rule) {
        Some(keyword) => !keyword.is_empty() && query.to_ascii_lowercase().contains(&keyword),
        None => false,
    }
}
fn host_part_matches(wildcard: bool, base: &str, host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || base.is_empty() {
        return false;
    }
    if wildcard {
        host.ends_with(&format!(".{base}"))
    } else {
        host == base || host.ends_with(&format!(".{base}"))
    }
}
/// The shared matcher. host is a bare hostname; path is a URL path
/// (empty or starting with /, query and fragment already stripped).
/// Keyword rules never match here: they need a search query, not a host.
pub fn rule_matches(rule: &str, host: &str, path: &str) -> bool {
    if is_keyword_rule(rule) {
        return false;
    }
    let rule = rule.trim().to_ascii_lowercase();
    let (host_part, rule_path) = match rule.split_once('/') {
        Some((h, p)) => (h, format!("/{}", p.trim_end_matches('/'))),
        None => (rule.as_str(), String::new()),
    };
    let rule_path = if rule_path == "/" {
        String::new()
    } else {
        rule_path
    };
    let (wildcard, base) = match host_part.strip_prefix("*.") {
        Some(base) => (true, base),
        None => (false, host_part),
    };
    if !host_part_matches(wildcard, base, host) {
        return false;
    }
    if rule_path.is_empty() {
        return true;
    }
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_ascii_lowercase()
    };
    path == rule_path || path.starts_with(&format!("{rule_path}/"))
}
fn valid_public_host(host: &str) -> bool {
    if host.parse::<std::net::IpAddr>().is_ok()
        || !host.contains('.')
        || host.len() > 253
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return false;
    }
    !host.split('.').any(|l| {
        l.is_empty()
            || l.len() > 63
            || l.starts_with('-')
            || l.ends_with('-')
            || !l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}
/// Validate and normalize one website rule as typed. Returns the stored
/// form (lowercased, scheme/trailing dot/trailing slash stripped), or a
/// plain-language error for the wizard.
pub fn validate_rule(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Empty website rule.".into());
    }
    if value.chars().any(char::is_control) {
        return Err(format!("Invalid website rule: {value}"));
    }
    let lower = value.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("search:") {
        let keyword = rest.split_whitespace().collect::<Vec<_>>().join(" ");
        if keyword.len() < 2
            || keyword.len() > 100
            || !keyword
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " -_.'".contains(c))
        {
            return Err(format!(
                "Search keyword must be 2-100 letters, digits, spaces or - _ . ': {value}"
            ));
        }
        return Ok(format!("search:{keyword}"));
    }
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let (host_part, path_part) = match without_scheme.split_once('/') {
        Some((h, p)) => (h, Some(p)),
        None => (without_scheme, None),
    };
    if host_part.contains([':', '@', '?', '#', ' '])
        || (host_part.contains('*') && !host_part.starts_with("*."))
    {
        return Err(format!("Invalid website rule: {value}"));
    }
    let (wildcard, base) = match host_part.strip_prefix("*.") {
        Some(base) => (true, base.trim_end_matches('.')),
        None => (false, host_part.trim_end_matches('.')),
    };
    if base.is_empty() || !valid_public_host(base) {
        return Err(format!("Invalid website rule: {value}"));
    }
    let host = if wildcard {
        format!("*.{base}")
    } else {
        base.to_string()
    };
    let Some(path_part) = path_part else {
        return Ok(host);
    };
    if path_part.contains(['?', '#', ' ']) || path_part.chars().any(char::is_control) {
        return Err(format!("Invalid website rule: {value}"));
    }
    let path = format!("/{}", path_part.trim_end_matches('/'));
    if path == "/" {
        return Ok(host);
    }
    Ok(format!("{host}{path}"))
}
/// Validate and normalize the wizard's website rules. Bare-domain-only
/// lists such as attention.work_domains keep using domains(); this is
/// for FocusConfig.domains only.
pub fn website_rules(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 websites.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        result.push(validate_rule(value)?);
    }
    result.sort();
    result.dedup();
    Ok(result)
}
/// Per-block exception precedence, implemented identically in the Rust
/// engine (`browser_domain_relevant` in engine.rs) and the extension
/// (`urlBlockedByPolicy` / `compileRules` in browser-extension/rules.js):
/// 1. Offline mode (general_internet=false): everything is blocked;
///    exceptions do NOT apply.
/// 2. Adult overlay: if block_adult_content is on and the host matches
///    ADULT_DOMAINS, it is blocked; exceptions do NOT override it.
/// 3. Exceptions: a URL/domain matching an exception rule is ALLOWED,
///    overriding blocklist rules AND keyword rules (including keyword
///    blocks on a search engine whose host matches an exception).
/// 4. Block rules: blocklist mode blocks on rule match; allowlist mode
///    blocks anything not in the allowed list (exceptions are
///    meaningless there, since the allowed list already defines what
///    is open, so they are ignored in allowlist mode).
///
/// Exceptions use the same rule language as block rules (domains,
/// wildcards, paths) but never `search:` rules: an exception is a
/// host/path rule only, and `search:` entries are rejected here.
/// Exceptions apply only while a session/block is active; they never
/// permanently allow or block anything.
pub fn exception_rules(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 exceptions.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if is_keyword_rule(value) {
            return Err(format!(
                "Exceptions cannot use search: rules; use a domain, *.example.com, or a domain with a path instead: {value}"
            ));
        }
        result.push(validate_rule(value)?);
    }
    result.sort();
    result.dedup();
    Ok(result)
}
/// YouTube channel rules (step 3), mirrored exactly in
/// browser-extension/rules.js (`normalizeYouTubeChannel`) and
/// src/services/domains.ts (`cleanYouTubeChannel`).
///
/// Why a watch page needs a content script: a channel page URL
/// contains the channel (/@handle, /channel/UC..., /c/..., /user/...),
/// but a watch page URL (youtube.com/watch?v=...) does not — so
/// watch-page enforcement reads the channel from the page DOM in
/// browser-extension/youtube.js. Channel pages and study-mode surface
/// pages are enforced by DNR rules compiled from these same keys.
///
/// Stored key forms (the comparable key — handles compare
/// case-insensitively, UC IDs case-sensitively):
/// - `@handle`: a handle, lowercased (handles are 3-30 chars).
/// - `UC...`: a channel ID (UC + at least 10 ID chars), case preserved.
/// - `c:name` / `user:name`: legacy channel URLs, lowercased.
///
/// Accepted input forms: any of the stored forms, or a YouTube URL
/// (youtube.com, www.youtube.com, m.youtube.com, with or without a
/// scheme) whose path is a channel page — including a channel tab URL
/// such as youtube.com/@handle/videos, which normalizes to the channel
/// itself. Video, search, playlist, Shorts, feed and embed URLs are
/// rejected: they are not channels.
fn valid_handle(body: &str) -> bool {
    (3..=30).contains(&body.len())
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        && body
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && body
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}
fn valid_uc_id(id: &str) -> bool {
    id.len() >= 12
        && id.len() <= 64
        && id.starts_with("UC")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
fn valid_legacy_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}
/// Parse channel-page path segments (the part after youtube.com/) into
/// a stored key, or None when the path is not a channel page. Only the
/// first segment(s) matter: trailing tab segments (/videos, /streams)
/// are ignored, since they still identify the same channel.
fn channel_key_from_segments(segments: &[&str]) -> Option<String> {
    let first = segments.first()?;
    if let Some(handle) = first.strip_prefix('@') {
        let body = handle.to_ascii_lowercase();
        return valid_handle(&body).then_some(format!("@{body}"));
    }
    match *first {
        "channel" => {
            let id = segments.get(1)?;
            valid_uc_id(id).then_some((*id).to_string())
        }
        "c" | "user" => {
            let name = segments.get(1)?.to_ascii_lowercase();
            valid_legacy_name(&name).then_some(format!("{}:{name}", first))
        }
        _ => None,
    }
}
/// The channel key for a YouTube URL pathname such as `/@handle`,
/// `/@handle/videos`, `/channel/UC...`, `/c/name` or `/user/name`.
/// Returns None for any other path (watch, search, playlist, Shorts,
/// feed, homepage). Shared with the extension's channel-page DNR
/// decision; the extension re-implements it in rules.js.
pub fn channel_key_from_path(path: &str) -> Option<String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    channel_key_from_segments(&segments)
}
/// Validate and normalize one YouTube channel entry as typed. Returns
/// the stored key, or a plain-language error for the wizard.
pub fn normalize_youtube_channel(value: &str) -> Result<String, String> {
    let hint = "Use @handle, a YouTube channel URL (youtube.com/@handle, youtube.com/channel/UC..., youtube.com/c/name, youtube.com/user/name), or a UC channel ID.";
    let raw = value.trim();
    if raw.is_empty() || raw.chars().any(char::is_control) || raw.contains(char::is_whitespace) {
        return Err(format!("Invalid YouTube channel: {raw}. {hint}"));
    }
    // A stored key is already normalized input.
    if let Some(handle) = raw.strip_prefix('@') {
        let body = handle.to_ascii_lowercase();
        return valid_handle(&body)
            .then_some(format!("@{body}"))
            .ok_or_else(|| format!("Invalid YouTube channel: {raw}. {hint}"));
    }
    if valid_uc_id(raw) {
        return Ok(raw.to_string());
    }
    for prefix in ["c:", "user:"] {
        if let Some(name) = raw.strip_prefix(prefix) {
            let name = name.to_ascii_lowercase();
            return valid_legacy_name(&name)
                .then_some(format!("{prefix}{name}"))
                .ok_or_else(|| format!("Invalid YouTube channel: {raw}. {hint}"));
        }
    }
    let lower = raw.to_ascii_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    // Query and fragment are never part of a channel entry.
    let without_query = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let Some((host, _)) = without_query.split_once('/') else {
        return Err(format!("Invalid YouTube channel: {raw}. {hint}"));
    };
    if !matches!(host, "youtube.com" | "www.youtube.com" | "m.youtube.com") {
        return Err(format!("Invalid YouTube channel: {raw}. {hint}"));
    }
    // The path keeps the original case for UC IDs: re-derive it from
    // the scheme-stripped original rather than the lowercased copy.
    // The scheme length comes from the lowercased prefix check above,
    // so mixed-case schemes (Https://) strip correctly too.
    let scheme_len = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        0
    };
    let original_path = raw[scheme_len..]
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .split_once('/')
        .map(|(_, p)| p)
        .unwrap_or("");
    channel_key_from_path(&format!("/{original_path}"))
        .ok_or_else(|| format!("Invalid YouTube channel: {raw}. {hint}"))
}
/// Validate and normalize a wizard channel list: sorted, deduped,
/// blank lines skipped, at most 100 entries (like `website_rules`).
pub fn youtube_channels(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 100 {
        return Err("Use at most 100 YouTube channels.".into());
    }
    let mut result = vec![];
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        result.push(normalize_youtube_channel(value)?);
    }
    result.sort();
    result.dedup();
    Ok(result)
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
    config.domains = website_rules(config.domains)?;
    config.exceptions = exception_rules(config.exceptions)?;
    config.youtube_blocked_channels = youtube_channels(config.youtube_blocked_channels)?;
    config.youtube_allowed_channels = youtube_channels(config.youtube_allowed_channels)?;
    config.blocked_folders = blocked_folders(config.blocked_folders)?;
    config.blocked_titles = title_rules(config.blocked_titles)?;
    // A blocked folder containing the paired browser's executable would
    // hide the browser the session's website rules depend on (folder
    // rules override the browser exemption in `blocked_sample`). On
    // Windows the browser id IS its exe path; on macOS it is a bundle
    // id, which never matches a folder, so this only fires on Windows.
    if !config.browser_app.is_empty()
        && config
            .blocked_folders
            .iter()
            .any(|f| path_under_folder(&config.browser_app, f))
    {
        return Err(
            "A blocked folder contains your paired browser. Remove that folder, or pick a different browser."
                .into(),
        );
    }
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
    // Step 12: a frozen session needs neither a work app nor an
    // approved domain list of its own — its allowlists ARE the
    // restriction (empty allowed apps = only Preelock + system;
    // empty allowed websites = no website at all).
    if config.mode == Mode::Strict
        && config.apps.is_empty()
        && config.browser_app.is_empty()
        && !config.frozen.enabled
    {
        return Err("Choose a work app or browser for Strict mode.".into());
    }
    if config.mode == Mode::Strict
        && !config.browser_app.is_empty()
        && config.general_internet
        && !matches!(config.website_blocking, WebsiteBlocking::None)
        && config.domains.is_empty()
        && config.youtube_blocked_channels.is_empty()
        && config.youtube_allowed_channels.is_empty()
        && !config.youtube_study_mode
        && !config.youtube_block_entertainment
        && config.frozen.allowed_websites.is_empty()
    {
        return Err("Strict mode needs an approved domain list, or browser offline mode.".into());
    }
    // Step 12: a frozen session with no paired browser is valid —
    // its allowlist policy has no browser list, so it applies to
    // any browser the user opens (and the sampler blocks every
    // browser app not in the frozen allowed-apps list anyway).
    // A frozen session WITH a picked browser still needs that
    // browser to be supported, like any other session.
    // Session-specific Safari restriction (Safari has no Preelock
    // extension): a session whose website rules the extension must enforce
    // cannot approve Safari as a work app either, not just as its browser.
    // Only checked where browser rules exist — Safari as a plain app in a
    // session without website blocking is untouched, and Safari keeps
    // running and being tracked normally.
    if config.network_required() && config.safari_approved() {
        return Err(
            "Safari can't enforce this session's website rules. Choose a supported browser for the session, or remove Safari from your work apps."
                .into(),
        );
    }
    let needs_browser_pick =
        config.network_required() && !(config.frozen.enabled && config.browser_app.is_empty());
    if needs_browser_pick && !supported_browser(&config.browser_app) {
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
    // Pomodoro ranges (step 7). This one check covers manual
    // sessions, schedule snapshots and one-shot snapshots, since all
    // of them validate through here.
    config.pomodoro.validate()?;
    // Reward ranges (step 9). Same single check covers manual
    // sessions, schedule snapshots and one-shot snapshots.
    config.reward.validate()?;
    // Frozen Turkey (step 12). Same single check covers manual
    // sessions, schedule snapshots and one-shot snapshots.
    config.frozen = config.frozen.validate()?;
    if config.frozen.enabled {
        // Breaks would relax the full lock, so a frozen session
        // has none: the wizard disables both break options while
        // Frozen is checked, and the backend rejects the
        // combination rather than silently dropping a break the
        // user thought they had.
        if config.pomodoro.enabled || config.reward.enabled {
            return Err(
                "Frozen sessions have no breaks: turn off Pomodoro and earned breaks to freeze the computer."
                    .into(),
            );
        }
        // App blocking is the frozen enforcement path; without
        // sampling consent there is nothing to block with.
        if !config.activity_consent {
            return Err(
                "Frozen needs app blocking: turn on 'Block all other apps' for a frozen session."
                    .into(),
            );
        }
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
    fn protected_covers_preelock_in_every_form() {
        // The app must never hide its own window: in a bundled build
        // the id is the bundle identifier, in dev it is the bare
        // executable path (macos.m reports the path when there is no
        // bundle). The old identifier is covered for old builds.
        assert!(protected("com.preelock.desktop"));
        assert!(protected("com.vystriq.desktop"));
        assert!(protected(
            "/Users/someone/My_WorkSpace/Preelock/src-tauri/target/debug/preelock"
        ));
        assert!(protected(
            "C:\\work\\preelock\\target\\release\\preelock.exe"
        ));
        assert!(!protected("com.apple.Terminal"));
        // blocked_sample must never flag Preelock itself, so a strict
        // session can never strand the user with no way to stop it.
        let strict = FocusConfig {
            mode: Mode::Strict,
            apps: vec!["ai.perplexity.comet".into()],
            activity_consent: true,
            ..Default::default()
        };
        assert!(!strict.blocked_sample(
            "/Users/someone/My_WorkSpace/Preelock/src-tauri/target/debug/preelock",
            "",
            "Preelock"
        ));
        assert!(strict.blocked_sample("com.apple.Terminal", "", "Terminal"));
    }
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
    fn frozen_blocks_every_app_except_the_allowlist_and_protected() {
        // Step 12: frozen app blocking goes through the existing
        // blocked_sample path, as a strict allowlist with no
        // automatic browser exemption. Protected apps (Preelock
        // itself, system processes) are never blocked.
        let mut c = FocusConfig {
            activity_consent: true,
            attention: crate::attention::Policy {
                work_apps: vec!["com.microsoft.VSCode".into()],
                ..Default::default()
            },
            browser_app: "com.google.Chrome".into(),
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec!["com.microsoft.VSCode".into()],
                allowed_websites: vec![],
            },
            ..Default::default()
        };
        assert!(c.network_required());
        assert!(!c.blocked("com.microsoft.VSCode"));
        assert!(c.blocked("com.google.Chrome"));
        assert!(c.blocked("com.apple.Safari"));
        assert!(!c.blocked("com.preelock.desktop"));
        assert!(!c.blocked("com.apple.Finder"));
        // Default frozen (empty allowlist): everything except
        // protected apps is blocked, including the browser.
        c.frozen.allowed_apps = vec![];
        assert!(c.blocked("com.microsoft.VSCode"));
        assert!(c.blocked("com.google.Chrome"));
        assert!(!c.blocked("com.preelock.desktop"));
        // Unfrozen, the same config does not block the browser.
        c.frozen.enabled = false;
        assert!(!c.blocked("com.google.Chrome"));
    }

    #[test]
    fn frozen_rejects_breaks_and_missing_consent() {
        let c = FocusConfig {
            activity_consent: true,
            attention: crate::attention::Policy {
                work_apps: vec!["com.microsoft.VSCode".into()],
                ..Default::default()
            },
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec![],
                allowed_websites: vec![],
            },
            ..Default::default()
        };
        assert!(validate(c.clone()).is_ok());
        let mut with_break = c.clone();
        with_break.pomodoro.enabled = true;
        assert!(validate(with_break).is_err());
        let mut with_reward = c.clone();
        with_reward.reward.enabled = true;
        assert!(validate(with_reward).is_err());
        let mut no_consent = c;
        no_consent.activity_consent = false;
        assert!(validate(no_consent).is_err());
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
    #[test]
    fn rule_matches_plain_wildcard_and_bare_domain() {
        assert!(rule_matches("example.com", "example.com", ""));
        assert!(rule_matches("example.com", "www.example.com", ""));
        assert!(rule_matches("example.com", "a.b.example.com", "/anything"));
        assert!(!rule_matches("example.com", "notexample.com", ""));
        assert!(!rule_matches("example.com", "example.com.evil.com", ""));
        assert!(rule_matches("*.example.com", "www.example.com", ""));
        assert!(rule_matches("*.example.com", "a.b.example.com", ""));
        assert!(!rule_matches("*.example.com", "example.com", ""));
        assert!(!rule_matches("*.example.com", "notexample.com", ""));
    }
    #[test]
    fn rule_matches_path_prefix_only() {
        assert!(rule_matches("youtube.com/shorts", "youtube.com", "/shorts"));
        assert!(rule_matches(
            "youtube.com/shorts",
            "www.youtube.com",
            "/shorts/abc"
        ));
        assert!(!rule_matches("youtube.com/shorts", "youtube.com", "/"));
        assert!(!rule_matches(
            "youtube.com/shorts",
            "youtube.com",
            "/shortsabc"
        ));
        assert!(!rule_matches("youtube.com/shorts", "youtube.com", "/watch"));
        assert!(!rule_matches("youtube.com/shorts", "other.com", "/shorts"));
        assert!(rule_matches(
            "*.example.com/guide",
            "docs.example.com",
            "/guide/intro"
        ));
        assert!(!rule_matches(
            "*.example.com/guide",
            "example.com",
            "/guide"
        ));
    }
    #[test]
    fn keyword_rule_representation() {
        assert!(is_keyword_rule("search:chess"));
        assert!(!is_keyword_rule("example.com"));
        assert_eq!(
            keyword_of("search:Chess Videos"),
            Some("chess videos".into())
        );
        assert!(!rule_matches("search:chess", "google.com", "/search"));
        assert!(search_query_matches(
            "search:chess",
            "how to play chess well"
        ));
        assert!(search_query_matches("search:chess", "CHESS openings"));
        assert!(!search_query_matches("search:chess", "checkers rules"));
        assert!(!search_query_matches("example.com", "chess"));
    }
    #[test]
    fn website_rules_accept_new_forms_and_reject_bad_ones() {
        assert_eq!(
            website_rules(vec![
                " HTTPS://Example.COM/Shorts/ ".into(),
                "*.example.com".into(),
                "search:Chess".into(),
                "example.com".into(),
            ])
            .unwrap(),
            vec![
                "*.example.com",
                "example.com",
                "example.com/shorts",
                "search:chess"
            ]
        );
        for bad in [
            "*.com",
            "example.com:80",
            "example.com/pa th",
            "127.0.0.1",
            "localhost",
            "search:",
            "search:x",
            "a*b.example.com",
            "example.com?x=1",
        ] {
            assert!(website_rules(vec![bad.into()]).is_err(), "{bad}");
        }
        assert!(!ADULT_DOMAINS.is_empty());
        for d in ADULT_DOMAINS {
            assert!(valid_public_host(d), "{d}");
            assert!(rule_matches(d, d, ""));
            assert!(rule_matches(d, &format!("www.{d}"), ""));
        }
    }
    #[test]
    fn exception_rules_accept_host_forms_and_reject_search_rules() {
        assert_eq!(
            exception_rules(vec![
                " HTTPS://Example.COM/Shorts/ ".into(),
                "*.example.com".into(),
                "example.com".into(),
            ])
            .unwrap(),
            vec!["*.example.com", "example.com", "example.com/shorts"]
        );
        for bad in ["search:chess", "search:", "*.com", "127.0.0.1"] {
            assert!(exception_rules(vec![bad.into()]).is_err(), "{bad}");
        }
        // The validator rejects search: exceptions with a clear message.
        let err = exception_rules(vec!["search:chess".into()]).unwrap_err();
        assert!(err.contains("search:"), "{err}");
        // Old saved sessions (no exceptions field) load as empty.
        let old: FocusConfig = serde_json::from_str(r#"{"mode":"standard","domains":[]}"#).unwrap();
        assert!(old.exceptions.is_empty());
        assert!(validate(old).is_ok());
    }
    #[test]
    fn youtube_channel_normalization_accepts_every_entry_form() {
        assert_eq!(
            normalize_youtube_channel("@Some_Handle-1").unwrap(),
            "@some_handle-1"
        );
        assert_eq!(
            normalize_youtube_channel("https://www.youtube.com/@SomeHandle/").unwrap(),
            "@somehandle"
        );
        assert_eq!(
            normalize_youtube_channel("youtube.com/@handle/videos").unwrap(),
            "@handle"
        );
        assert_eq!(
            normalize_youtube_channel("youtube.com/channel/UCabcdefghijklmnopqrstuv").unwrap(),
            "UCabcdefghijklmnopqrstuv"
        );
        assert_eq!(
            normalize_youtube_channel("UCabcdefghijklmnopqrstuv").unwrap(),
            "UCabcdefghijklmnopqrstuv"
        );
        assert_eq!(
            normalize_youtube_channel("https://youtube.com/c/SomeName").unwrap(),
            "c:somename"
        );
        assert_eq!(
            normalize_youtube_channel("youtube.com/user/SomeName").unwrap(),
            "user:somename"
        );
        // Stored keys round-trip unchanged.
        assert_eq!(
            normalize_youtube_channel("c:somename").unwrap(),
            "c:somename"
        );
        assert_eq!(normalize_youtube_channel("@handle").unwrap(), "@handle");
        assert_eq!(
            channel_key_from_path("/@Handle/videos"),
            Some("@handle".into())
        );
        assert_eq!(
            channel_key_from_path("/channel/UCabcdefghijklmnopqrstuv"),
            Some("UCabcdefghijklmnopqrstuv".into())
        );
        assert_eq!(channel_key_from_path("/watch"), None);
        assert_eq!(channel_key_from_path("/"), None);
    }
    #[test]
    fn youtube_channel_normalization_rejects_non_channels() {
        for bad in [
            "",
            "youtube.com",
            "youtube.com/watch?v=abc",
            "youtube.com/results?search_query=cats",
            "youtube.com/playlist?list=WL",
            "youtube.com/shorts/abc",
            "youtube.com/feed/subscriptions",
            "https://example.com/@handle",
            "https://youtu.be/abc",
            "@",
            "@ab",
            "UCshort",
            "not a channel",
            "handle",
        ] {
            assert!(normalize_youtube_channel(bad).is_err(), "{bad}");
            assert!(
                normalize_youtube_channel(bad)
                    .unwrap_err()
                    .contains("@handle"),
                "{bad}"
            );
        }
    }
    #[test]
    fn youtube_channel_lists_sort_dedupe_and_validate() {
        assert_eq!(
            youtube_channels(vec![
                "youtube.com/@Beta".into(),
                "@alpha".into(),
                " @ALPHA ".into(),
                "".into(),
            ])
            .unwrap(),
            vec!["@alpha", "@beta"]
        );
        assert!(youtube_channels(vec!["youtube.com/watch?v=x".into()]).is_err());
        // Old saved sessions (no YouTube fields) load as empty / false.
        let old: FocusConfig = serde_json::from_str(r#"{"mode":"standard","domains":[]}"#).unwrap();
        assert!(old.youtube_blocked_channels.is_empty());
        assert!(old.youtube_allowed_channels.is_empty());
        assert!(!old.youtube_study_mode);
        assert!(!old.youtube_block_entertainment);
        assert!(!old.youtube_allow_watch_later);
        // validate() normalizes channel lists in place. Channel rules
        // require a browser, like domain rules, so the config pairs one.
        let c = validate(FocusConfig {
            browser_app: "ai.perplexity.comet".into(),
            youtube_blocked_channels: vec!["youtube.com/@Beta".into()],
            youtube_allowed_channels: vec!["UCabcdefghijklmnopqrstuv".into()],
            youtube_study_mode: true,
            youtube_allow_watch_later: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.youtube_blocked_channels, vec!["@beta"]);
        assert!(c.youtube_study_mode && c.youtube_allow_watch_later);
        // Channel rules alone require a browser, like domain rules.
        assert!(c.network_required());
        // …but the Watch-Later toggle alone is not a rule.
        let wl_only = FocusConfig {
            youtube_allow_watch_later: true,
            ..Default::default()
        };
        assert!(!wl_only.network_required());
        assert!(validate(wl_only).is_ok());
        // Entertainment blocking alone requires a browser like channel rules.
        let ent = validate(FocusConfig {
            browser_app: "ai.perplexity.comet".into(),
            website_blocking: WebsiteBlocking::None,
            youtube_block_entertainment: true,
            ..Default::default()
        })
        .unwrap();
        assert!(ent.youtube_block_entertainment);
        assert!(ent.network_required());
    }
    #[test]
    fn folder_prefix_needs_separator_boundary() {
        // The brief's example: /Foo must NOT match /Foobar.
        assert!(path_under_folder("/Foo/bar.exe", "/Foo"));
        assert!(!path_under_folder("/Foobar/bar.exe", "/Foo"));
        assert!(path_under_folder("/Foo", "/Foo"));
        // Windows spellings: separator style, case, trailing slash.
        assert!(path_under_folder(r"C:\Games\Steam\game.exe", r"C:\Games"));
        assert!(path_under_folder("C:/Games/Steam/game.exe", r"C:\Games\"));
        assert!(path_under_folder(r"c:\games\steam\game.exe", r"C:\GAMES"));
        assert!(!path_under_folder(r"C:\Games2\game.exe", r"C:\Games"));
        assert!(!path_under_folder("", r"C:\Games"));
        assert!(!path_under_folder(r"C:\Games\game.exe", ""));
        // Case-insensitive on Mac too (default APFS is case-insensitive).
        assert!(path_under_folder(
            "/applications/games/x.app/Contents/MacOS/x",
            "/Applications/Games"
        ));
        assert!(folder_blocked(
            r"C:\Games\game.exe",
            &["C:\\Other".to_string(), r"C:\Games".to_string()]
        ));
        assert!(!folder_blocked(
            r"C:\Games\game.exe",
            &[r"C:\Other".to_string()]
        ));
    }
    #[test]
    fn title_substring_is_case_insensitive() {
        assert!(title_blocked("YouTube - LoFi Beats", &["youtube".into()]));
        assert!(title_blocked("Chess - Game 3", &["CHESS".into()]));
        assert!(!title_blocked("Visual Studio Code", &["chess".into()]));
        assert!(!title_blocked("YouTube", &["".into()]));
        assert!(!title_blocked("", &["youtube".into()]));
        assert!(!title_blocked("YouTube", &[]));
    }
    #[test]
    fn store_id_form_round_trips() {
        let pfn = "Microsoft.WindowsCalculator_8wekyb3d8bbwe";
        assert_eq!(
            store_id_from_pfn(pfn).as_deref(),
            Some("store:Microsoft.WindowsCalculator_8wekyb3d8bbwe")
        );
        let id = store_id_from_pfn(pfn).unwrap();
        assert!(is_store_id(&id));
        assert_eq!(pfn_from_store_id(&id), Some(pfn));
        assert!(!is_store_id("C:\\Program Files\\app.exe"));
        assert_eq!(pfn_from_store_id("com.apple.Safari"), None);
        // A package family name always has the Name_PublisherHash form.
        assert_eq!(store_id_from_pfn(""), None);
        assert_eq!(store_id_from_pfn("Calculator"), None);
        assert_eq!(store_id_from_pfn("Bad Name_hash"), None);
        assert_eq!(store_id_from_pfn("Name_hash!"), None);
    }
    #[test]
    fn system_folders_are_rejected() {
        for folder in [
            r"C:\Windows",
            r"C:\Windows\System32",
            r"C:\Windows\SysWOW64\drivers",
            r"C:\Windows\System32\drivers",
            "/System",
            "/System/Library",
        ] {
            assert!(is_system_folder(folder), "{folder}");
            assert!(blocked_folders(vec![folder.into()]).is_err(), "{folder}");
        }
        for folder in [
            r"C:\Program Files\Games",
            r"C:\Users\Prashant\Games",
            "/Applications",
            "/Applications/Games",
            "/Users/prashant/Games",
        ] {
            assert!(!is_system_folder(folder), "{folder}");
        }
        // Roots are rejected separately from OS folders.
        assert!(blocked_folders(vec!["/".into()]).is_err());
        assert!(blocked_folders(vec![r"C:\".into()]).is_err());
        assert!(blocked_folders(vec!["".into()]).unwrap().is_empty());
        // Normalization: trailing separators stripped, sorted, deduped.
        assert_eq!(
            blocked_folders(vec![r"C:\Games\".into(), r"C:\Games".into()]).unwrap(),
            vec![r"C:\Games"]
        );
        // Old saved sessions (no step-4 fields) load as empty lists.
        let old: FocusConfig = serde_json::from_str(r#"{"mode":"standard","domains":[]}"#).unwrap();
        assert!(old.blocked_folders.is_empty());
        assert!(old.blocked_titles.is_empty());
        assert!(validate(old).is_ok());
    }
    #[test]
    fn title_rules_validate_and_dedupe() {
        assert_eq!(
            title_rules(vec![" YouTube ".into(), "youtube".into(), "".into()]).unwrap(),
            vec!["YouTube"]
        );
        assert!(title_rules(vec!["x".repeat(101)]).is_err());
        assert!(title_rules(vec!["a\u{1}b".into()]).is_err());
    }
    #[test]
    fn folder_and_title_rules_override_allowed_apps() {
        let mut config = FocusConfig {
            mode: Mode::Strict,
            apps: vec![r"C:\Work\Code.exe".into()],
            blocked_folders: vec![r"C:\Work".into()],
            activity_consent: true,
            ..Default::default()
        };
        // An allowed app inside a blocked folder is blocked anyway.
        assert!(config.blocked_sample(r"C:\Work\Code.exe", r"C:\Work\Code.exe", "Code"));
        config.blocked_folders = vec![];
        config.blocked_titles = vec!["chess".into()];
        // An allowed app with a matching window title is blocked too.
        assert!(config.blocked_sample(r"C:\Work\Code.exe", r"C:\Work\Code.exe", "Chess - Game 3"));
        assert!(!config.blocked_sample(r"C:\Work\Code.exe", r"C:\Work\Code.exe", "Code"));
        // Folder rules also work through the id fallback when the
        // sample path is empty (a Windows exe path as id).
        config.blocked_folders = vec![r"C:\Work".into()];
        config.blocked_titles = vec![];
        assert!(config.blocked(r"C:\Work\Code.exe"));
        // Protected apps are never blocked, even on a title match.
        config.blocked_titles = vec!["finder".into()];
        assert!(!config.blocked_sample(
            "com.apple.Finder",
            "/System/Library/CoreServices/Finder.app",
            "Finder"
        ));
        // Without activity consent, folder/title rules are inert.
        config.activity_consent = false;
        assert!(!config.blocked_sample(r"C:\Work\Code.exe", r"C:\Work\Code.exe", "Chess"));
        // A blocked folder containing the paired browser is rejected
        // (the config is otherwise valid, so this must be the error).
        let mut browser = FocusConfig {
            browser_app: r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
            blocked_folders: vec![r"C:\Program Files\Google".into()],
            activity_consent: true,
            ..Default::default()
        };
        browser.attention.work_apps = vec!["com.microsoft.VSCode".into()];
        assert!(validate(browser.clone()).is_err());
        browser.blocked_folders = vec![r"C:\Games".into()];
        assert!(validate(browser).is_ok());
    }
}
#[cfg(test)]
mod safari_session_rules {
    use super::*;
    fn config() -> FocusConfig {
        let mut c = FocusConfig {
            browser_app: "ai.perplexity.comet".into(),
            website_blocking: WebsiteBlocking::Blocklist,
            domains: vec!["youtube.com".into()],
            ..Default::default()
        };
        c.attention.work_apps = vec!["com.microsoft.VSCode".into()];
        c
    }
    #[test]
    fn safari_is_known_without_extension_support() {
        assert!(safari("com.apple.Safari"));
        assert!(safari("C:\\Program Files\\Safari\\Safari.exe"));
        assert!(known_browser("com.apple.Safari"));
        assert!(!supported_browser("com.apple.Safari"));
        assert!(!safari("com.google.Chrome"));
    }
    #[test]
    fn safari_work_app_is_rejected_when_the_session_blocks_websites() {
        let mut c = config();
        c.attention.work_apps.push("com.apple.Safari".into());
        let err = validate(c).unwrap_err();
        assert!(err.contains("Safari"), "{err}");
    }
    #[test]
    fn safari_as_the_session_browser_is_rejected() {
        let mut c = config();
        c.browser_app = "com.apple.Safari".into();
        let err = validate(c).unwrap_err();
        assert!(err.contains("Safari"), "{err}");
    }
    #[test]
    fn strict_sessions_cannot_allowlist_safari() {
        let mut c = config();
        c.mode = Mode::Strict;
        c.website_blocking = WebsiteBlocking::Allowlist;
        c.apps = vec!["com.apple.Safari".into()];
        let err = validate(c).unwrap_err();
        assert!(err.contains("Safari"), "{err}");
    }
    #[test]
    fn blocking_safari_or_using_it_without_website_rules_stays_valid() {
        // Standard mode: `apps` is the blocklist, so blocking Safari is fine.
        let mut blocked = config();
        blocked.apps = vec!["com.apple.Safari".into()];
        assert!(validate(blocked).is_ok());
        // No website rules at all: Safari is just an app you work in.
        let mut plain = config();
        plain.browser_app = String::new();
        plain.domains = Vec::new();
        plain.website_blocking = WebsiteBlocking::None;
        plain.attention.work_apps = vec!["com.apple.Safari".into()];
        assert!(validate(plain).is_ok());
    }
}
