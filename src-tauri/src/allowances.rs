//! Time allowances (step 8): daily/weekly time budgets for one site
//! or one app, enforced only while a session is running.
//!
//! Accounting scope, stated once (the task requires exactly one
//! coherent scope, and this is it):
//! - Budgets deplete ONLY from live engine ticks during a Running
//!   session in a Focus phase. Break time never consumes a budget
//!   (step 7's "break time never counts" rule), Paused/Preparing
//!   sessions accrue nothing, and there is NO always-on background
//!   tracker: when no session runs, usage does not accrue and
//!   enforcement is off, even if a budget is already exhausted.
//!   Allowances are independent of any single session — usage
//!   accumulates across sessions within the period — but they are
//!   only enforced while a session is running, because Preelock only
//!   enforces during sessions.
//! - One tick accrues at most the tick delta (capped at 3000 ms,
//!   the same cap the attention accounting uses for a single sample,
//!   so a sleep/wake gap can never dump minutes into a budget).
//!   Usage is never reconstructed from wall-clock elapsed time, so
//!   a restart cannot double-count: the first tick after a restart
//!   has a ~0 delta, and ticks already flushed to SQLite are never
//!   replayed. A crash can lose at most the engine's 5-second
//!   allowance flush interval (see `Engine::tick`).
//! - App usage: the frontmost sample's app id, matched with
//!   `focus::ids_match` (the step-4 identity rules, including the
//!   Windows exe-filename fallback). The user must not be idle
//!   (idle >= 60 s counts as away, not use). Requires a session with
//!   activity consent on a native platform, because only then does
//!   the engine sample at all.
//! - Site usage: the extension-reported active domain, matched with
//!   `focus::rule_matches` using an empty path (the engine only ever
//!   sees a host, exactly as in the relevance check). The domain
//!   only flows when the published policy has `observe_activity`
//!   (activity consent + attention on), and the report must be
//!   fresh and browser-focused. When a frontmost sample is
//!   available, the browser must also be the frontmost app — a
//!   background tab does not count. When no sample is available
//!   (no activity consent), the extension's own focus flag is the
//!   only foreground signal, so it is used alone.
//! - A tick can accrue into several allowances at once when their
//!   targets overlap (e.g. `youtube.com` and `*.youtube.com`):
//!   budgets are per-target, not a single global clock.
//!
//! Storage: usage is kept as per-minute buckets
//! (`UsageBucket { minute_ms, ms }`), merged when several ticks fall
//! in the same wall-clock minute. Buckets make both period kinds
//! computable from one log: fixed periods sum the buckets inside the
//! current calendar period, rolling periods sum the buckets inside
//! the trailing window. A bucket straddling a rolling boundary
//! counts in full (at most one minute of overcount at the boundary);
//! tests use minute-aligned times so the boundary is exact.
//!
//! Reset semantics:
//! - Fixed day: local midnight to local midnight. Fixed week: local
//!   Monday 00:00 to the next Monday 00:00.
//! - Fixed boundaries are computed from the OS local weekday/minutes
//!   (the same `schedules::local_weekday_and_minutes` step 5 uses),
//!   so a DST transition day may shift a boundary by up to one hour;
//!   that shift is accepted, not corrected for (step-5 convention).
//! - Rolling day/week: a trailing 24 h / 7 d window of true elapsed
//!   time from now; DST is irrelevant there.
//! - A budget ALWAYS resets at its boundary: exhaustion is a
//!   computed property of current usage, never stored state, so a
//!   period roll, a disable, or a delete lifts the block on the next
//!   tick / policy publish with no cleanup step.
//!
//! Enforcement:
//! - Apps: the engine's existing per-sample blocked path (hide +
//!   distracted time), extended with an allowance-exhausted check.
//!   The paired browser of the running session and protected system
//!   apps (see `focus::protected`, which covers Preelock itself) can
//!   never be allowance-blocked, whatever the allowance says.
//! - Sites: exhausted site rules are appended to the published
//!   extension policy's `extra_blocked` list — the step-1 adult
//!   overlay mechanism, which outranks allowlist rules and per-block
//!   exceptions in the extension, with no extension change (version
//!   stays 6). A session that would otherwise publish no policy
//!   publishes one when any site allowance is enabled, so site
//!   allowances work without a website-blocking session config.
//!
//! Validation limits (site targets): a site allowance target must be
//! a host-only rule — no `search:` keyword (keywords have no host
//! to budget, and the extension's DNR compiler cannot express a
//! keyword in `extra_blocked`) and no path (the engine only sees a
//! host, so a path-rule budget could never accrue yet would block
//! when exhausted — an incoherent pair). Use a plain or wildcard
//! domain instead.

use crate::{
    focus::{ids_match, protected, rule_matches, validate_rule},
    schedules::local_weekday_and_minutes,
};
use serde::{Deserialize, Serialize};

/// What the budget applies to: a website rule or an app id.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Site,
    App,
}

/// Length of one budget period.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    Day,
    Week,
}

/// How the period window is anchored.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    /// Trailing 24 h / 7 d from now.
    Rolling,
    /// Calendar day / calendar week (Monday start) in local time.
    Fixed,
}

/// Usage accrued inside one wall-clock minute. `minute_ms` is the
/// minute's start (epoch ms, truncated); `ms` is the total accrued
/// usage in that minute across all ticks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageBucket {
    pub minute_ms: i64,
    pub ms: u64,
}

/// One time budget. `usage` is the raw minute-bucket log; `used_ms`
/// and exhaustion are always computed from it for a given `now`,
/// never stored, so a period roll can never leave a stale block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Allowance {
    pub id: String,
    pub title: String,
    pub kind: TargetKind,
    /// Site: a host-only website rule (plain or wildcard domain).
    /// App: a platform app id (macOS bundle id, Windows exe path,
    /// or `store:<package family name>`), matched with
    /// `focus::ids_match`.
    pub target: String,
    pub budget_minutes: u32,
    pub period: Period,
    pub mode: ResetMode,
    pub enabled: bool,
    #[serde(default)]
    pub usage: Vec<UsageBucket>,
    pub created_at: i64,
}

fn title(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 {
        return Err("Use a title between 1 and 160 characters.".into());
    }
    Ok(value.into())
}

/// Validate an allowance's fields (title, target, budget). Usage
/// buckets are carried through untouched: editing a budget never
/// erases the usage already accrued in the current period.
pub fn validate_allowance(mut allowance: Allowance) -> Result<Allowance, String> {
    allowance.title = title(&allowance.title)?;
    match allowance.kind {
        TargetKind::Site => {
            let rule = validate_rule(&allowance.target)?;
            if crate::focus::is_keyword_rule(&rule) {
                return Err(
                    "A site allowance needs a domain, not a search: keyword — keywords have no host to budget."
                        .into(),
                );
            }
            if rule.contains('/') {
                return Err(
                    "A site allowance needs a plain or wildcard domain, not a domain with a path — the engine only sees hosts, so a path budget could never accrue."
                        .into(),
                );
            }
            allowance.target = rule;
        }
        TargetKind::App => {
            let target = allowance.target.trim();
            if target.is_empty() || target.len() > 1024 || target.chars().any(char::is_control) {
                return Err("Use an app id up to 1024 characters.".into());
            }
            if protected(target) {
                return Err("Preelock itself and system apps cannot have a time allowance.".into());
            }
            allowance.target = target.into();
        }
    }
    // A budget longer than its period could never exhaust; reject
    // it rather than silently creating a budget that never blocks.
    let max = match allowance.period {
        Period::Day => 1440,
        Period::Week => 10080,
    };
    if allowance.budget_minutes == 0 || allowance.budget_minutes > max {
        return Err(format!(
            "Choose a budget from 1 to {max} minutes for a {} allowance.",
            match allowance.period {
                Period::Day => "daily",
                Period::Week => "weekly",
            }
        ));
    }
    allowance.usage.sort_by_key(|b| b.minute_ms);
    Ok(allowance)
}

pub fn budget_ms(allowance: &Allowance) -> u64 {
    u64::from(allowance.budget_minutes) * 60_000
}

/// Start of the current fixed period in local time, from the OS
/// local weekday/minutes (step-5 convention; a DST transition day
/// may shift this by up to one hour — accepted, not corrected).
fn fixed_period_start(allowance: &Allowance, now_ms: i64) -> i64 {
    let (weekday, minutes) = local_weekday_and_minutes(now_ms);
    let day_start = now_ms
        .saturating_sub(i64::from(minutes) * 60_000)
        .saturating_sub(now_ms.rem_euclid(60_000));
    match allowance.period {
        Period::Day => day_start,
        Period::Week => {
            let days_since_monday = i64::from((weekday + 6) % 7);
            day_start.saturating_sub(days_since_monday * 86_400_000)
        }
    }
}

fn rolling_window_start(allowance: &Allowance, now_ms: i64) -> i64 {
    let window = match allowance.period {
        Period::Day => 86_400_000,
        Period::Week => 604_800_000,
    };
    now_ms.saturating_sub(window)
}

/// Usage in the current period, from the minute buckets. Buckets
/// recorded ahead of the current clock (clock moved backwards) are
/// not counted until the clock catches up with them.
pub fn used_ms(allowance: &Allowance, now_ms: i64) -> u64 {
    match allowance.mode {
        ResetMode::Fixed => {
            let start = fixed_period_start(allowance, now_ms);
            allowance
                .usage
                .iter()
                .filter(|b| b.minute_ms >= start && b.minute_ms <= now_ms)
                .map(|b| b.ms)
                .sum()
        }
        ResetMode::Rolling => {
            let start = rolling_window_start(allowance, now_ms);
            allowance
                .usage
                .iter()
                .filter(|b| b.minute_ms + 60_000 > start && b.minute_ms <= now_ms)
                .map(|b| b.ms)
                .sum()
        }
    }
}

/// Exhaustion is computed, never stored: disabling the allowance,
/// deleting it, or the period rolling over lifts the block without
/// any cleanup step. Disabled allowances never exhaust.
pub fn exhausted(allowance: &Allowance, now_ms: i64) -> bool {
    allowance.enabled && used_ms(allowance, now_ms) >= budget_ms(allowance)
}

/// Drop buckets that can never count again (older than the current
/// fixed period, or fully outside the rolling window). Called on
/// accrual and on allowance edits so the log stays bounded; it is
/// not needed for correctness, since `used_ms` filters anyway.
pub fn prune(allowance: &mut Allowance, now_ms: i64) {
    match allowance.mode {
        ResetMode::Fixed => {
            let start = fixed_period_start(allowance, now_ms);
            allowance
                .usage
                .retain(|b| b.minute_ms >= start && b.minute_ms <= now_ms);
        }
        ResetMode::Rolling => {
            let start = rolling_window_start(allowance, now_ms);
            allowance
                .usage
                .retain(|b| b.minute_ms + 60_000 > start && b.minute_ms <= now_ms);
        }
    }
}

/// Add `ms` of usage at wall-clock time `at_ms`, merging into the
/// minute bucket for that time. Callers pass only live tick deltas,
/// never wall-clock elapsed time (see the module note).
pub fn accrue(allowance: &mut Allowance, at_ms: i64, ms: u64) {
    if ms == 0 {
        return;
    }
    let minute_ms = at_ms.div_euclid(60_000) * 60_000;
    match allowance
        .usage
        .iter_mut()
        .find(|b| b.minute_ms == minute_ms)
    {
        Some(bucket) => bucket.ms = bucket.ms.saturating_add(ms),
        None => allowance.usage.push(UsageBucket { minute_ms, ms }),
    }
    allowance.usage.sort_by_key(|b| b.minute_ms);
    prune(allowance, at_ms);
}

pub fn matches_site(allowance: &Allowance, host: &str) -> bool {
    allowance.kind == TargetKind::Site && rule_matches(&allowance.target, host, "")
}

pub fn matches_app(allowance: &Allowance, app_id: &str) -> bool {
    allowance.kind == TargetKind::App && ids_match(&allowance.target, app_id)
}

/// Whether this app is allowance-blocked right now in a session
/// whose paired browser is `session_browser_app`. The browser and
/// protected apps are never blocked, whatever the allowance says —
/// this is the enforcement-time guard (creation-time validation
/// rejects protected ids, but the paired browser varies per
/// session, so only this check can protect it).
pub fn app_exhausted(
    allowances: &[Allowance],
    now_ms: i64,
    app_id: &str,
    session_browser_app: &str,
) -> bool {
    if app_id.is_empty() || protected(app_id) || ids_match(app_id, session_browser_app) {
        return false;
    }
    allowances
        .iter()
        .any(|a| a.enabled && matches_app(a, app_id) && exhausted(a, now_ms))
}

/// Enabled, exhausted site targets, sorted and deduped, for the
/// published policy's `extra_blocked` list.
pub fn exhausted_site_rules(allowances: &[Allowance], now_ms: i64) -> Vec<String> {
    let mut rules: Vec<String> = allowances
        .iter()
        .filter(|a| a.enabled && a.kind == TargetKind::Site && exhausted(a, now_ms))
        .map(|a| a.target.clone())
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

/// Enabled, exhausted site targets with their titles, for per-rule
/// block reasons on the published policy. Sorted by rule; a rule
/// exhausted by several allowances keeps the first title.
pub fn exhausted_site_causes(allowances: &[Allowance], now_ms: i64) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = allowances
        .iter()
        .filter(|a| a.enabled && a.kind == TargetKind::Site && exhausted(a, now_ms))
        .map(|a| (a.target.clone(), a.title.clone()))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// The exhausted allowance matching this app, if any — for "why am
/// I blocked" reasons. Mirrors `app_exhausted`'s guards exactly.
pub fn exhausted_allowance_for_app<'a>(
    allowances: &'a [Allowance],
    now_ms: i64,
    app_id: &str,
    session_browser_app: &str,
) -> Option<&'a Allowance> {
    if app_id.is_empty() || protected(app_id) || ids_match(app_id, session_browser_app) {
        return None;
    }
    allowances
        .iter()
        .find(|a| a.enabled && matches_app(a, app_id) && exhausted(a, now_ms))
}

/// Foreground signals one tick needs to decide allowance accrual.
/// Pure inputs so the rule is unit-testable without a running engine:
/// `sample_available` is false when the session has no activity
/// sampling (no consent, or a non-native build), in which case the
/// extension's focus flag is the only foreground signal for sites.
pub struct TickUsage<'a> {
    pub sample_id: &'a str,
    pub sample_idle_ms: u64,
    pub sample_available: bool,
    pub browser_domain: &'a str,
    pub browser_focused: bool,
    pub browser_activity_fresh: bool,
    /// Resolved platform id of the browser that reported the domain
    /// (empty when no browser is connected).
    pub browser_client_id: &'a str,
    pub session_browser_app: &'a str,
}

/// Accrue one tick's usage into every matching enabled allowance.
/// Returns whether anything changed. No-op on a break phase, on a
/// zero delta, and on a delta over the 3000 ms single-sample cap
/// (a sleep/wake gap must never dump minutes into a budget).
/// App usage needs a non-idle frontmost sample; site usage needs a
/// fresh, focused browser report for a non-empty domain, plus the
/// frontmost check when a sample is available.
pub fn accrue_for_tick(
    allowances: &mut [Allowance],
    now_ms: i64,
    delta_ms: u64,
    in_break: bool,
    usage: &TickUsage<'_>,
) -> bool {
    if in_break || delta_ms == 0 || delta_ms > 5000 {
        return false;
    }
    let mut changed = false;
    if usage.sample_available && !usage.sample_id.is_empty() && usage.sample_idle_ms < 180_000 {
        for allowance in allowances.iter_mut() {
            if allowance.enabled && matches_app(allowance, usage.sample_id) {
                accrue(allowance, now_ms, delta_ms);
                changed = true;
            }
        }
    }
    if !usage.browser_domain.is_empty() && usage.browser_activity_fresh && usage.browser_focused {
        let browser_frontmost = if usage.sample_available {
            !usage.sample_id.is_empty()
                && usage.sample_idle_ms < 180_000
                && (ids_match(usage.sample_id, usage.browser_client_id)
                    || ids_match(usage.sample_id, usage.session_browser_app))
        } else {
            // No sample to check against: the extension's focus flag
            // is the foreground signal.
            true
        };
        if browser_frontmost {
            for allowance in allowances.iter_mut() {
                if allowance.enabled && matches_site(allowance, usage.browser_domain) {
                    accrue(allowance, now_ms, delta_ms);
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowance(
        kind: TargetKind,
        target: &str,
        budget: u32,
        period: Period,
        mode: ResetMode,
    ) -> Allowance {
        validate_allowance(Allowance {
            id: "a1".into(),
            title: "Test".into(),
            kind,
            target: target.into(),
            budget_minutes: budget,
            period,
            mode,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .unwrap()
    }

    fn site(budget: u32, period: Period, mode: ResetMode) -> Allowance {
        allowance(TargetKind::Site, "youtube.com", budget, period, mode)
    }

    #[test]
    fn budget_depletes_and_exhausts_at_exactly_the_budget() {
        let mut a = site(30, Period::Day, ResetMode::Fixed);
        let now = 1_800_000_000_000;
        accrue(&mut a, now, 25 * 60_000);
        assert_eq!(used_ms(&a, now), 25 * 60_000);
        assert_eq!(budget_ms(&a) - used_ms(&a, now), 5 * 60_000);
        assert!(!exhausted(&a, now));
        accrue(&mut a, now + 60_000, 5 * 60_000);
        assert_eq!(used_ms(&a, now + 60_000), 30 * 60_000);
        assert_eq!(budget_ms(&a) - used_ms(&a, now + 60_000), 0);
        assert!(exhausted(&a, now + 60_000));
    }

    #[test]
    fn fixed_day_resets_at_the_local_day_boundary() {
        let mut a = site(60, Period::Day, ResetMode::Fixed);
        let now = 1_800_000_000_000;
        // 25 hours ago is always a previous calendar day, whatever
        // the local timezone of the test machine is.
        accrue(&mut a, now - 25 * 3_600_000, 60 * 60_000);
        assert_eq!(used_ms(&a, now), 0);
        assert!(!exhausted(&a, now));
        accrue(&mut a, now, 60 * 60_000);
        assert!(exhausted(&a, now));
    }

    #[test]
    fn fixed_week_resets_after_seven_days() {
        let mut a = site(60, Period::Week, ResetMode::Fixed);
        let now = 1_800_000_000_000;
        accrue(&mut a, now - 8 * 86_400_000, 60 * 60_000);
        assert_eq!(used_ms(&a, now), 0);
        // Three days ago is inside the current Monday-start week for
        // any weekday (the week is 7 days, so a 3-day-old bucket is
        // never before this Monday).
        accrue(&mut a, now - 3 * 86_400_000, 30 * 60_000);
        assert_eq!(used_ms(&a, now), 30 * 60_000);
        assert!(!exhausted(&a, now));
    }

    #[test]
    fn rolling_windows_expire_old_usage() {
        let mut day = site(60, Period::Day, ResetMode::Rolling);
        let now = 1_800_000_000_000;
        accrue(&mut day, now - 25 * 3_600_000, 60 * 60_000);
        assert_eq!(used_ms(&day, now), 0);
        accrue(&mut day, now - 23 * 3_600_000, 60 * 60_000);
        assert!(exhausted(&day, now));
        let mut week = site(60, Period::Week, ResetMode::Rolling);
        accrue(&mut week, now - 8 * 86_400_000, 60 * 60_000);
        assert_eq!(used_ms(&week, now), 0);
        accrue(&mut week, now - 6 * 86_400_000, 60 * 60_000);
        assert!(exhausted(&week, now));
    }

    #[test]
    fn exhausted_app_blocks_but_browser_and_protected_never_do() {
        let mut a = allowance(
            TargetKind::App,
            "com.example.game",
            10,
            Period::Day,
            ResetMode::Fixed,
        );
        let now = 1_800_000_000_000;
        accrue(&mut a, now, 10 * 60_000);
        let list = vec![a.clone()];
        assert!(app_exhausted(&list, now, "com.example.game", ""));
        // The paired browser of the running session is exempt.
        assert!(!app_exhausted(
            &list,
            now,
            "com.example.game",
            "com.example.game"
        ));
        // A protected id is never blocked, even if an allowance
        // somehow names it (creation validation rejects it too).
        let mut p = allowance(
            TargetKind::App,
            "com.example.game",
            10,
            Period::Day,
            ResetMode::Fixed,
        );
        p.target = "com.preelock.desktop".into();
        accrue(&mut p, now, 10 * 60_000);
        assert!(!app_exhausted(
            std::slice::from_ref(&p),
            now,
            "com.preelock.desktop",
            ""
        ));
        // Disabled: the block lifts immediately.
        let mut d = a.clone();
        d.enabled = false;
        assert!(!app_exhausted(
            std::slice::from_ref(&d),
            now,
            "com.example.game",
            ""
        ));
        assert!(exhausted_site_rules(std::slice::from_ref(&d), now).is_empty());
    }

    #[test]
    fn exhausted_site_rules_come_from_enabled_site_allowances_only() {
        let mut s = site(10, Period::Day, ResetMode::Fixed);
        let now = 1_800_000_000_000;
        accrue(&mut s, now, 10 * 60_000);
        let app = allowance(
            TargetKind::App,
            "com.example.game",
            10,
            Period::Day,
            ResetMode::Fixed,
        );
        let list = vec![s, app];
        assert_eq!(
            exhausted_site_rules(&list, now),
            vec!["youtube.com".to_string()]
        );
    }

    #[test]
    fn validation_rejects_keyword_path_and_protected_targets_and_oversized_budgets() {
        assert!(validate_allowance(Allowance {
            id: "x".into(),
            title: "T".into(),
            kind: TargetKind::Site,
            target: "search:chess".into(),
            budget_minutes: 30,
            period: Period::Day,
            mode: ResetMode::Fixed,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .is_err());
        assert!(validate_allowance(Allowance {
            id: "x".into(),
            title: "T".into(),
            kind: TargetKind::Site,
            target: "youtube.com/shorts".into(),
            budget_minutes: 30,
            period: Period::Day,
            mode: ResetMode::Fixed,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .is_err());
        assert!(validate_allowance(Allowance {
            id: "x".into(),
            title: "T".into(),
            kind: TargetKind::App,
            target: "preelock.exe".into(),
            budget_minutes: 30,
            period: Period::Day,
            mode: ResetMode::Fixed,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .is_err());
        assert!(validate_allowance(Allowance {
            id: "x".into(),
            title: "T".into(),
            kind: TargetKind::Site,
            target: "youtube.com".into(),
            budget_minutes: 1441,
            period: Period::Day,
            mode: ResetMode::Fixed,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .is_err());
        // A wildcard domain and a weekly budget up to 10080 are fine.
        assert!(validate_allowance(Allowance {
            id: "x".into(),
            title: "T".into(),
            kind: TargetKind::Site,
            target: "*.example.com".into(),
            budget_minutes: 10080,
            period: Period::Week,
            mode: ResetMode::Rolling,
            enabled: true,
            usage: vec![],
            created_at: 0,
        })
        .is_ok());
    }

    #[test]
    fn tick_accrual_skips_breaks_idle_and_oversized_deltas() {
        let mut list = vec![site(60, Period::Day, ResetMode::Fixed)];
        let now = 1_800_000_000_000;
        let usage = TickUsage {
            sample_id: "com.google.Chrome",
            sample_idle_ms: 0,
            sample_available: true,
            browser_domain: "youtube.com",
            browser_focused: true,
            browser_activity_fresh: true,
            browser_client_id: "com.google.Chrome",
            session_browser_app: "com.google.Chrome",
        };
        // Break time never consumes a budget.
        assert!(!accrue_for_tick(&mut list, now, 1000, true, &usage));
        assert_eq!(used_ms(&list[0], now), 0);
        // A normal focus tick accrues.
        assert!(accrue_for_tick(&mut list, now, 1000, false, &usage));
        assert_eq!(used_ms(&list[0], now), 1000);
        // Idle ticks do not (the user is away, not using the site).
        let idle = TickUsage {
            sample_idle_ms: 180_000,
            ..usage
        };
        assert!(!accrue_for_tick(
            &mut list,
            now + 60_000,
            1000,
            false,
            &idle
        ));
        // A wake-gap delta over the cap does not dump minutes in.
        assert!(!accrue_for_tick(
            &mut list,
            now + 120_000,
            120_000,
            false,
            &usage
        ));
        assert_eq!(used_ms(&list[0], now + 120_000), 1000);
        // A background tab (sample is another app) does not count.
        let background = TickUsage {
            sample_id: "com.example.editor",
            ..usage
        };
        assert!(!accrue_for_tick(
            &mut list,
            now + 180_000,
            1000,
            false,
            &background
        ));
        assert_eq!(used_ms(&list[0], now + 180_000), 1000);
    }

    #[test]
    fn tick_accrual_counts_apps_by_id_match_and_sites_without_a_sample() {
        let mut list = vec![
            allowance(
                TargetKind::App,
                "Code.exe",
                60,
                Period::Day,
                ResetMode::Fixed,
            ),
            site(60, Period::Day, ResetMode::Fixed),
        ];
        let now = 1_800_000_000_000;
        // Windows path-spelling variant of the app id still matches
        // (step-4 ids_match, including the filename fallback).
        let app_usage = TickUsage {
            sample_id: r"C:\Work\Code.exe",
            sample_idle_ms: 0,
            sample_available: true,
            browser_domain: "",
            browser_focused: false,
            browser_activity_fresh: false,
            browser_client_id: "",
            session_browser_app: "",
        };
        assert!(accrue_for_tick(&mut list, now, 1000, false, &app_usage));
        assert_eq!(used_ms(&list[0], now), 1000);
        assert_eq!(used_ms(&list[1], now), 0);
        // No sample at all: the extension focus flag alone counts
        // the site.
        let site_usage = TickUsage {
            sample_id: "",
            sample_idle_ms: 0,
            sample_available: false,
            browser_domain: "www.youtube.com",
            browser_focused: true,
            browser_activity_fresh: true,
            browser_client_id: "com.google.Chrome",
            session_browser_app: "",
        };
        assert!(accrue_for_tick(
            &mut list,
            now + 60_000,
            1000,
            false,
            &site_usage
        ));
        assert_eq!(used_ms(&list[1], now + 60_000), 1000);
    }
}
