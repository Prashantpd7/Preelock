use crate::{browser::BrowserStatus, models::ScreenTimeBucket, platform::Sample};

pub fn local_day_start_ms(now_ms: i64) -> i64 {
    let (_, minutes) = crate::schedules::local_weekday_and_minutes(now_ms);
    now_ms
        .saturating_sub(i64::from(minutes) * 60_000)
        .saturating_sub(now_ms.rem_euclid(60_000))
}

/// One sample interval's foreground facts, needed to decide where the
/// interval is credited. Pure inputs so the rule is unit-testable
/// without a running engine or a native sample.
pub struct Attribution<'a> {
    pub day_ms: i64,
    /// Foreground application id (macOS bundle id or Windows
    /// executable path), from `platform::sample()`.
    pub app_id: &'a str,
    /// Foreground application display name.
    pub app_name: &'a str,
    /// OS idle time in ms for this interval.
    pub idle_ms: u64,
    /// Resolved platform id of the connected browser that reported
    /// `active_domain` (empty when no browser is connected).
    pub browser_app: &'a str,
    /// The active tab's hostname as reported by the extension (empty on
    /// New Tab, settings, history, downloads and other browser UI pages).
    pub active_domain: &'a str,
    /// The extension reports its browser window is focused.
    pub browser_focused: bool,
    /// The extension report is newer than the freshness window.
    pub activity_fresh: bool,
    /// Native (extension-free) observation of the foreground app's active
    /// page: the browser id the OS adapter sampled for, and the validated
    /// hostname. Both empty when the adapter had nothing verifiable
    /// (permission denied, private window with no exposed URL, browser UI
    /// page, non-browser foreground app).
    pub native_app: &'a str,
    pub native_domain: &'a str,
    /// Length of the interval in ms (must be <= 3000 to be credited).
    pub delta: u64,
}

/// The core mutual-exclusion accounting rule. Every interval is credited
/// to exactly one category:
///
/// 1. the active website, when the foreground app IS the browser and
///    either the extension reports a fresh, focused, identifiable domain
///    (never for browser UI pages such as New Tab, settings, history or
///    downloads — those report an empty domain) or the native OS adapter
///    validated a hostname for that same foreground app;
/// 2. otherwise the foreground application (the browser itself included
///    when it is used without an active website — the unidentified /
///    private fallback).
///
/// Website usage replaces browser application usage for the interval; it
/// is never added to it, so an interval can never land in both buckets,
/// and the extension and native sources can never both be credited.
/// Returns `None` for intervals that must not be credited at all (idle
/// over 60 s, an empty foreground id, a zero/gap delta).
/// The lock screen is never user activity: on macOS the frontmost
/// app while locked is the login window, on Windows it is LogonUI.
/// The idle rule below already drops locked intervals once idle
/// passes 60 s, but the first minute after locking would otherwise
/// be credited to the login window as application usage. Only
/// these two identities are excluded — every other protected app
/// (Finder, System Preferences, Preelock itself) is real foreground
/// usage and keeps its credit; hiding protection is a separate
/// concern (`focus::protected`) and is untouched.
fn is_lock_screen_app(app_id: &str) -> bool {
    if app_id.eq_ignore_ascii_case("com.apple.loginwindow") {
        return true;
    }
    let leaf = app_id.rsplit(['/', '\\']).next().unwrap_or(app_id);
    leaf.eq_ignore_ascii_case("logonui.exe")
}

pub fn attribute_interval(input: &Attribution<'_>) -> Option<ScreenTimeBucket> {
    if input.delta == 0
        || input.delta > 3000
        || input.app_id.is_empty()
        || input.idle_ms >= 60_000
        || is_lock_screen_app(input.app_id)
    {
        return None;
    }
    // One decision for both sources (see websites::credited): the
    // extension when it has a fresh focused domain for this browser,
    // otherwise the native observation, otherwise nothing — which is the
    // browser application's own interval below, never a lost one.
    let extension = crate::websites::ExtensionSite {
        browser: input.browser_app,
        domain: input.active_domain,
        focused: input.browser_focused,
        fresh: input.activity_fresh,
    };
    let native = (!input.native_domain.is_empty()).then_some(crate::websites::NativeSiteRef {
        app: input.native_app,
        domain: input.native_domain,
    });
    match crate::websites::credited(input.app_id, extension, native) {
        Some((domain, _source)) => Some(ScreenTimeBucket {
            day_ms: input.day_ms,
            kind: "site".into(),
            target: domain.into(),
            name: domain.into(),
            ms: input.delta,
        }),
        None => Some(ScreenTimeBucket {
            day_ms: input.day_ms,
            kind: "app".into(),
            target: input.app_id.into(),
            name: input.app_name.into(),
            ms: input.delta,
        }),
    }
}

/// Attribution for one native sample + browser status, shared by the
/// always-on sampler and the session sampler so both enforce exactly the
/// same rule. At most one bucket per interval (mutual exclusion).
pub fn sample_buckets(
    now: i64,
    delta: u64,
    sample: &Sample,
    browser: &BrowserStatus,
) -> Vec<ScreenTimeBucket> {
    sample_buckets_with_native(now, delta, sample, browser, None)
}

/// The same rule plus an optional native (extension-free) observation of
/// the foreground browser's active page. The native domain is used only
/// when the extension has nothing fresh and focused for this app; either
/// way one interval yields at most one bucket, so native and extension
/// observations can never add up to double time.
pub fn sample_buckets_with_native(
    now: i64,
    delta: u64,
    sample: &Sample,
    browser: &BrowserStatus,
    native: Option<&crate::websites::NativeSite>,
) -> Vec<ScreenTimeBucket> {
    let mut buckets = Vec::new();
    if let Some(bucket) = attribute_interval(&Attribution {
        day_ms: local_day_start_ms(now),
        app_id: &sample.id,
        app_name: &sample.name,
        idle_ms: sample.idle_ms,
        browser_app: &browser.client_browser,
        active_domain: &browser.active_domain,
        browser_focused: browser.browser_focused,
        activity_fresh: browser.activity_fresh,
        native_app: native.map(|n| n.app.as_str()).unwrap_or(""),
        native_domain: native.map(|n| n.domain.as_str()).unwrap_or(""),
        delta,
    }) {
        buckets.push(bucket);
    }
    buckets
}

pub fn add_usage(
    buckets: &mut Vec<ScreenTimeBucket>,
    day_ms: i64,
    kind: &str,
    target: &str,
    name: &str,
    ms: u64,
) {
    if target.is_empty() || ms == 0 {
        return;
    }
    if let Some(bucket) = buckets
        .iter_mut()
        .find(|b| b.day_ms == day_ms && b.kind == kind && b.target == target)
    {
        bucket.ms = bucket.ms.saturating_add(ms);
        if !name.is_empty() {
            bucket.name = name.into();
        }
    } else {
        buckets.push(ScreenTimeBucket {
            day_ms,
            kind: kind.into(),
            target: target.into(),
            name: name.into(),
            ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{browser::BrowserStatus, platform::Sample};

    fn status(client: &str, domain: &str, focused: bool, fresh: bool) -> BrowserStatus {
        BrowserStatus {
            available: true,
            connected: fresh,
            applied_id: String::new(),
            client_browser: client.into(),
            error: String::new(),
            blocks: 0,
            cleanup_pending: false,
            active_domain: domain.into(),
            browser_focused: focused,
            search_distracted: false,
            channel_blocked: false,
            media_playing: false,
            activity_fresh: fresh,
            extension_stale: false,
            ext_v_debug: 0,
            raw_browser_debug: String::new(),
        }
    }

    fn sample(id: &str, name: &str, idle_ms: u64) -> Sample {
        Sample {
            id: id.into(),
            name: name.into(),
            pid: 1,
            idle_ms,
            path: String::new(),
            title: String::new(),
        }
    }

    fn assert_one(buckets: &[ScreenTimeBucket], kind: &str, target: &str, ms: u64) {
        assert_eq!(buckets.len(), 1, "expected exactly one credited interval");
        assert_eq!(buckets[0].kind, kind);
        assert_eq!(buckets[0].target, target);
        assert_eq!(buckets[0].ms, ms);
    }

    fn credit(totals: &mut Vec<ScreenTimeBucket>, buckets: Vec<ScreenTimeBucket>) {
        for bucket in buckets {
            add_usage(
                totals,
                bucket.day_ms,
                &bucket.kind,
                &bucket.target,
                &bucket.name,
                bucket.ms,
            );
        }
    }

    fn native(app: &str, domain: &str) -> crate::websites::NativeSite {
        crate::websites::NativeSite {
            app: app.into(),
            domain: domain.into(),
        }
    }

    fn total(totals: &[ScreenTimeBucket]) -> u64 {
        totals.iter().map(|b| b.ms).sum()
    }

    #[test]
    fn native_site_is_credited_without_the_extension() {
        // Requirement 2: no extension connection at all. The OS adapter's
        // hostname is credited to the site and the browser app gets
        // nothing for that interval.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        assert_one(
            &sample_buckets_with_native(
                1,
                1000,
                &s,
                &disconnected,
                Some(&native("com.google.Chrome", "youtube.com")),
            ),
            "site",
            "youtube.com",
            1000,
        );
    }

    #[test]
    fn native_website_time_accumulates_per_domain() {
        // 10 min GitHub + 10 min WhatsApp, all through the native adapter.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        let mut totals = Vec::new();
        for domain in ["github.com", "whatsapp.com"] {
            let observation = native("com.google.Chrome", domain);
            for _ in 0..600 {
                credit(
                    &mut totals,
                    sample_buckets_with_native(1, 1000, &s, &disconnected, Some(&observation)),
                );
            }
        }
        assert_eq!(totals.len(), 2);
        assert!(totals.iter().all(|b| b.kind == "site"));
        assert_eq!(total(&totals), 20 * 60_000);
        assert_eq!(
            totals.iter().find(|b| b.target == "github.com").unwrap().ms,
            10 * 60_000
        );
    }

    #[test]
    fn browser_foreground_time_is_kept_when_no_site_is_identified() {
        // Incognito/private window, a browser UI page, or an adapter that
        // could not read the page: the interval is browser application
        // usage — never an invented site, never a dropped interval.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        assert_one(
            &sample_buckets_with_native(1, 1000, &s, &disconnected, None),
            "app",
            "com.google.Chrome",
            1000,
        );
        // An empty native domain means exactly the same thing.
        assert_one(
            &sample_buckets_with_native(
                1,
                1000,
                &s,
                &disconnected,
                Some(&native("com.google.Chrome", "")),
            ),
            "app",
            "com.google.Chrome",
            1000,
        );
    }

    #[test]
    fn extension_and_native_never_double_count_one_interval() {
        // Both sources describe the same second: exactly one bucket, one
        // second credited, and the extension's more accurate tab wins.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let extension = status("com.google.chrome", "youtube.com", true, true);
        let observation = native("com.google.Chrome", "github.com");
        let buckets = sample_buckets_with_native(1, 1000, &s, &extension, Some(&observation));
        assert_one(&buckets, "site", "youtube.com", 1000);
        let mut totals = Vec::new();
        credit(&mut totals, buckets);
        assert_eq!(total(&totals), 1000);
    }

    #[test]
    fn extension_disconnect_and_reconnect_hand_over_without_losing_time() {
        // 10 min extension-connected YouTube, 10 min native GitHub after
        // the extension disconnects, 10 min YouTube again after it
        // reconnects: 30 minutes total, never 40.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let mut totals = Vec::new();
        let connected = status("com.google.chrome", "youtube.com", true, true);
        for _ in 0..600 {
            credit(&mut totals, sample_buckets(1, 1000, &s, &connected));
        }
        let disconnected = status("", "", false, false);
        let observation = native("com.google.Chrome", "github.com");
        for _ in 0..600 {
            credit(
                &mut totals,
                sample_buckets_with_native(1, 1000, &s, &disconnected, Some(&observation)),
            );
        }
        for _ in 0..600 {
            credit(
                &mut totals,
                sample_buckets_with_native(1, 1000, &s, &connected, Some(&observation)),
            );
        }
        assert_eq!(total(&totals), 30 * 60_000);
        assert_eq!(
            totals
                .iter()
                .find(|b| b.target == "youtube.com")
                .unwrap()
                .ms,
            20 * 60_000
        );
        assert_eq!(
            totals.iter().find(|b| b.target == "github.com").unwrap().ms,
            10 * 60_000
        );
    }

    #[test]
    fn rapid_tab_switching_credits_each_domain_its_own_intervals() {
        // Alternating tabs every sample for 10 minutes: each host gets
        // exactly its own seconds and the total is unchanged.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        let youtube = native("com.google.Chrome", "youtube.com");
        let github = native("com.google.Chrome", "github.com");
        let mut totals = Vec::new();
        for interval in 0..600 {
            let observation = if interval % 2 == 0 { &youtube } else { &github };
            credit(
                &mut totals,
                sample_buckets_with_native(1, 1000, &s, &disconnected, Some(observation)),
            );
        }
        assert_eq!(total(&totals), 10 * 60_000);
        assert_eq!(totals.len(), 2);
        assert!(totals.iter().all(|b| b.ms == 5 * 60_000));
    }

    #[test]
    fn browser_switching_credits_only_the_foreground_browser() {
        // A native observation always belongs to the frontmost window. If
        // it were ever labelled with another browser, it must be refused
        // rather than misattributed; Chrome keeps the interval instead.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        assert_one(
            &sample_buckets_with_native(
                1,
                1000,
                &s,
                &disconnected,
                Some(&native("com.microsoft.edgemac", "edge-news.microsoft.com")),
            ),
            "app",
            "com.google.Chrome",
            1000,
        );
        // Edge in the foreground with its own observation is credited.
        let edge = sample("com.microsoft.edgemac", "Microsoft Edge", 0);
        assert_one(
            &sample_buckets_with_native(
                1,
                1000,
                &edge,
                &disconnected,
                Some(&native("com.microsoft.edgemac", "edge-news.microsoft.com")),
            ),
            "site",
            "edge-news.microsoft.com",
            1000,
        );
    }

    #[test]
    fn leaving_or_minimizing_the_browser_credits_the_foreground_app() {
        // The user switched to VS Code while the browser report (and the
        // last native observation) are still around: the foreground app
        // takes the interval, the browser/site get nothing.
        let s = sample("com.microsoft.VSCode", "Visual Studio Code", 0);
        let browser = status("com.google.chrome", "youtube.com", true, true);
        assert_one(
            &sample_buckets_with_native(
                1,
                1000,
                &s,
                &browser,
                Some(&native("com.google.Chrome", "youtube.com")),
            ),
            "app",
            "com.microsoft.VSCode",
            1000,
        );
    }

    #[test]
    fn wake_from_sleep_gap_is_never_credited_and_tracking_resumes() {
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        let observation = native("com.google.Chrome", "github.com");
        // The first tick after sleep carries the whole suspended span; it
        // is discarded (existing rule: delta over 3 s is a gap).
        assert!(
            sample_buckets_with_native(1, 600_000, &s, &disconnected, Some(&observation))
                .is_empty()
        );
        // The next ticks credit normally again.
        assert_one(
            &sample_buckets_with_native(1, 1000, &s, &disconnected, Some(&observation)),
            "site",
            "github.com",
            1000,
        );
    }

    #[test]
    fn identified_and_unidentified_time_add_up_to_browser_foreground_time() {
        // The requirement's 120-minute example: 50 YouTube + 30 WhatsApp
        // + 20 GitHub + 10 other site + 10 unidentified = 120 minutes of
        // Chrome, never 130 and never 110.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let disconnected = status("", "", false, false);
        let mut totals = Vec::new();
        let plan: [(&str, u32); 5] = [
            ("youtube.com", 3000),
            ("whatsapp.com", 1800),
            ("github.com", 1200),
            ("docs.rs", 600),
            ("", 600),
        ];
        for (domain, seconds) in plan {
            let observation = native("com.google.Chrome", domain);
            for _ in 0..seconds {
                credit(
                    &mut totals,
                    sample_buckets_with_native(1, 1000, &s, &disconnected, Some(&observation)),
                );
            }
        }
        assert_eq!(total(&totals), 120 * 60_000);
        let browser_ms: u64 = totals
            .iter()
            .filter(|b| b.target == "com.google.Chrome")
            .map(|b| b.ms)
            .sum();
        assert_eq!(browser_ms, 10 * 60_000, "only the unidentified time");
        let site_ms: u64 = totals
            .iter()
            .filter(|b| b.kind == "site")
            .map(|b| b.ms)
            .sum();
        assert_eq!(site_ms, 110 * 60_000);
    }

    #[test]
    fn merges_per_target_and_separates_days() {
        let mut buckets = Vec::new();
        add_usage(&mut buckets, 1, "app", "com.example", "Example", 1000);
        add_usage(&mut buckets, 1, "app", "com.example", "Example App", 2000);
        add_usage(&mut buckets, 2, "app", "com.example", "Example", 3000);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].ms, 3000);
        assert_eq!(buckets[0].name, "Example App");
    }

    #[test]
    fn ignores_empty_and_zero_usage() {
        let mut buckets = Vec::new();
        add_usage(&mut buckets, 1, "site", "", "", 1000);
        add_usage(&mut buckets, 1, "site", "example.com", "example.com", 0);
        assert!(buckets.is_empty());
    }

    #[test]
    fn site_replaces_browser_app_for_the_same_interval() {
        // 30 minutes on Instagram in Chrome must be exactly 30 minutes:
        // one site bucket, no Chrome app bucket for the same intervals.
        let mut totals = Vec::new();
        for _ in 0..1800 {
            let s = sample("com.google.Chrome", "Google Chrome", 0);
            let b = status("com.google.chrome", "instagram.com", true, true);
            for bucket in sample_buckets(1, 1000, &s, &b) {
                add_usage(
                    &mut totals,
                    bucket.day_ms,
                    &bucket.kind,
                    &bucket.target,
                    &bucket.name,
                    bucket.ms,
                );
            }
        }
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].kind, "site");
        assert_eq!(totals[0].target, "instagram.com");
        assert_eq!(totals[0].ms, 30 * 60_000);
    }

    #[test]
    fn browser_new_tab_counts_as_browser_app() {
        // Chrome New Tab / settings / history: empty active domain, so the
        // browser application itself receives the time.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let b = status("com.google.chrome", "", true, true);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.google.Chrome", 1000);
    }

    #[test]
    fn switching_from_site_to_another_app_transfers_attribution() {
        // The extension may still hold a fresh focused domain report while
        // the user switched to VS Code; the foreground check must move the
        // credit to VS Code and never keep crediting the stale site.
        let s = sample("com.microsoft.VSCode", "Visual Studio Code", 0);
        let b = status("com.google.chrome", "instagram.com", true, true);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.microsoft.VSCode", 1000);
    }

    #[test]
    fn site_switches_cleanly_between_domains() {
        // 10 min instagram + 10 min youtube + 10 min Chrome New Tab = 30
        // minutes total, never 60.
        let mut totals = Vec::new();
        for _ in 0..600 {
            let s = sample("com.google.Chrome", "Google Chrome", 0);
            let buckets = sample_buckets(
                1,
                1000,
                &s,
                &status("com.google.chrome", "instagram.com", true, true),
            );
            for bucket in buckets {
                add_usage(
                    &mut totals,
                    bucket.day_ms,
                    &bucket.kind,
                    &bucket.target,
                    &bucket.name,
                    bucket.ms,
                );
            }
        }
        for _ in 0..600 {
            let s = sample("com.google.Chrome", "Google Chrome", 0);
            let buckets = sample_buckets(
                1,
                1000,
                &s,
                &status("com.google.chrome", "youtube.com", true, true),
            );
            for bucket in buckets {
                add_usage(
                    &mut totals,
                    bucket.day_ms,
                    &bucket.kind,
                    &bucket.target,
                    &bucket.name,
                    bucket.ms,
                );
            }
        }
        for _ in 0..600 {
            let s = sample("com.google.Chrome", "Google Chrome", 0);
            let buckets = sample_buckets(1, 1000, &s, &status("com.google.chrome", "", true, true));
            for bucket in buckets {
                add_usage(
                    &mut totals,
                    bucket.day_ms,
                    &bucket.kind,
                    &bucket.target,
                    &bucket.name,
                    bucket.ms,
                );
            }
        }
        let total: u64 = totals.iter().map(|b| b.ms).sum();
        assert_eq!(total, 30 * 60_000);
        assert_eq!(
            totals
                .iter()
                .find(|b| b.target == "instagram.com")
                .unwrap()
                .ms,
            10 * 60_000
        );
        assert_eq!(
            totals
                .iter()
                .find(|b| b.target == "youtube.com")
                .unwrap()
                .ms,
            10 * 60_000
        );
        assert_eq!(
            totals
                .iter()
                .find(|b| b.target == "com.google.Chrome")
                .unwrap()
                .ms,
            10 * 60_000
        );
    }

    #[test]
    fn background_tab_receives_no_site_credit() {
        // The extension reports its window is not focused (a background
        // tab or an unfocused browser window): no site credit, the
        // browser application gets the interval instead.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let b = status("com.google.chrome", "instagram.com", false, true);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.google.Chrome", 1000);
    }

    #[test]
    fn delayed_heartbeat_falls_back_to_browser_app() {
        // A delayed extension heartbeat (activity not fresh) must never
        // credit a stale site; the browser application receives the
        // interval so nothing is double-counted.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let b = status("com.google.chrome", "instagram.com", true, false);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.google.Chrome", 1000);
    }

    #[test]
    fn disconnected_extension_never_invents_website_usage() {
        // No connected browser: the foreground browser is still an app and
        // receives normal application credit, with no site invented.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let b = status("", "", false, false);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.google.Chrome", 1000);
    }

    #[test]
    fn other_paired_browser_report_never_leaks_into_foreground_app() {
        // Edge is paired and reported last, but Chrome is foreground:
        // Edge's domain must not be credited to Chrome's intervals.
        let s = sample("com.google.Chrome", "Google Chrome", 0);
        let b = status(
            "com.microsoft.edgemac",
            "edge-news.microsoft.com",
            true,
            true,
        );
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "app", "com.google.Chrome", 1000);
    }

    #[test]
    fn lock_screen_samples_are_never_credited() {
        // M5: the lock screen is never user activity. macOS reports
        // the login window as foreground; Windows reports LogonUI.
        // Either would otherwise collect up to a minute of credit
        // before the 60 s idle rule kicks in.
        let b = status("com.google.chrome", "instagram.com", true, true);
        let loginwindow = sample("com.apple.loginwindow", "Login Window", 0);
        assert!(sample_buckets(1, 1000, &loginwindow, &b).is_empty());
        // Idle, long after locking: still nothing (both rules agree).
        let locked_idle = sample("com.apple.loginwindow", "Login Window", 300_000);
        assert!(sample_buckets(1, 1000, &locked_idle, &b).is_empty());
        let logonui = sample(r"C:\Windows\System32\LogonUI.exe", "Windows Logon UI", 0);
        assert!(sample_buckets(1, 1000, &logonui, &b).is_empty());
        // Contrast: a real foreground app in the same interval is
        // still credited, and other protected apps keep their credit.
        let chrome = sample("com.google.Chrome", "Google Chrome", 0);
        assert_one(
            &sample_buckets(1, 1000, &chrome, &b),
            "site",
            "instagram.com",
            1000,
        );
        let finder = sample("com.apple.finder", "Finder", 0);
        let no_browser = status("", "", false, false);
        assert_one(
            &sample_buckets(1, 1000, &finder, &no_browser),
            "app",
            "com.apple.finder",
            1000,
        );
    }

    #[test]
    fn idle_interval_is_never_credited() {
        let b = status("com.google.chrome", "instagram.com", true, true);
        let idle = sample("com.google.Chrome", "Google Chrome", 60_000);
        assert!(sample_buckets(1, 1000, &idle, &b).is_empty());
        let active = sample("com.google.Chrome", "Google Chrome", 0);
        assert!(sample_buckets(1, 0, &active, &b).is_empty());
        assert!(sample_buckets(1, 3001, &active, &b).is_empty());
    }

    #[test]
    fn empty_foreground_app_is_not_credited() {
        let s = sample("", "", 0);
        let b = status("com.google.chrome", "instagram.com", true, true);
        assert!(sample_buckets(1, 1000, &s, &b).is_empty());
    }

    #[test]
    fn windows_exe_path_matches_reported_browser() {
        let s = sample(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "Google Chrome",
            0,
        );
        let b = status("com.google.chrome", "github.com", true, true);
        let buckets = sample_buckets(1, 1000, &s, &b);
        assert_one(&buckets, "site", "github.com", 1000);
        // New Tab on Windows still credits the browser app.
        let new_tab = status("com.google.chrome", "", true, true);
        let buckets = sample_buckets(1, 1000, &s, &new_tab);
        assert_one(
            &buckets,
            "app",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            1000,
        );
    }

    #[test]
    fn keeps_display_name_and_day_for_app_buckets() {
        let s = sample("com.microsoft.VSCode", "Visual Studio Code", 0);
        let b = status("", "", false, false);
        let buckets = sample_buckets(1_800_000_000_000, 1000, &s, &b);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].kind, "app");
        assert_eq!(buckets[0].target, "com.microsoft.VSCode");
        assert_eq!(buckets[0].name, "Visual Studio Code");
        assert_eq!(buckets[0].ms, 1000);
        assert_eq!(
            buckets[0].day_ms,
            crate::screen_time::local_day_start_ms(1_800_000_000_000)
        );
    }
}
