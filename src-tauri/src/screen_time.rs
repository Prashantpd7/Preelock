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
    /// Length of the interval in ms (must be <= 3000 to be credited).
    pub delta: u64,
}

/// The core mutual-exclusion accounting rule. Every interval is credited
/// to exactly one category:
///
/// 1. the active website, when the foreground app IS the connected
///    browser AND the extension reports a fresh, focused, identifiable
///    domain (never for browser UI pages such as New Tab, settings,
///    history or downloads — those report an empty domain);
/// 2. otherwise the foreground application (the browser itself included
///    when it is used without an active website).
///
/// Website usage replaces browser application usage for the interval; it
/// is never added to it, so an interval can never land in both buckets.
/// Returns `None` for intervals that must not be credited at all (idle
/// over 60 s, an empty foreground id, a zero/gap delta).
pub fn attribute_interval(input: &Attribution<'_>) -> Option<ScreenTimeBucket> {
    if input.delta == 0 || input.delta > 3000 || input.app_id.is_empty() || input.idle_ms >= 60_000
    {
        return None;
    }
    let website_active = input.activity_fresh
        && input.browser_focused
        && !input.active_domain.is_empty()
        && crate::browser::same_browser(input.app_id, input.browser_app);
    if website_active {
        Some(ScreenTimeBucket {
            day_ms: input.day_ms,
            kind: "site".into(),
            target: input.active_domain.into(),
            name: input.active_domain.into(),
            ms: input.delta,
        })
    } else {
        Some(ScreenTimeBucket {
            day_ms: input.day_ms,
            kind: "app".into(),
            target: input.app_id.into(),
            name: input.app_name.into(),
            ms: input.delta,
        })
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
