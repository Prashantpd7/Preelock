//! App locks: independent timed and daily-limit blocks for one site
//! or one app, enforced whether or not a focus session is running.
//!
//! Scope, stated once:
//! - A timed lock blocks its target from creation until a
//!   wall-clock `ends_at` instant (up to 7 days out). Expiry is
//!   deletion, not a flag: expired timed locks are pruned on the
//!   next tick and the deletion is saved, so an expiry that passes
//!   while the app is closed is cleaned up by the first tick after
//!   reopen — and a killed/restarted app can never enforce a stale
//!   timed lock.
//! - A daily-limit lock watches the target's usage in the always-on
//!   Screen Time buckets (`screen_time_usage`, keyed by local
//!   calendar day). The moment today's usage reaches or exceeds the
//!   limit, the target is blocked for the rest of the day. Midnight
//!   resets automatically because usage is keyed by local calendar
//!   day. A limit set when today's usage already exceeds it blocks
//!   immediately — enforcement is computed, never stored.
//! - Both kinds work for apps and websites, and both enforce
//!   independently of sessions: apps are hidden by the no-session
//!   Screen Time tick (and by the session per-sample blocked path
//!   when a session runs, including on breaks), sites ride the
//!   extension's `extra_blocked` overlay via the observe-only
//!   Screen Time policy and the session policy publish — the same
//!   step-1 adult mechanism step 8 uses, so no extension change is
//!   needed (the protocol stays at version 6).
//! - Unlike allowances, there is deliberately NO session-browser
//!   exemption on the app path: the user explicitly targeted this
//!   app, and hiding (not killing) keeps the browser process and
//!   its extension alive. `focus::protected` still guards Preelock
//!   itself and system apps, at validation time and at enforcement
//!   time.
//!
//! Validation limits (site targets): the same host-only rule as
//! allowances — no `search:` keywords (keywords have no host to
//! watch) and no paths (the engine only ever sees a host, so a
//! path lock could never accrue usage yet would block when its
//! limit hit — an incoherent pair).

use crate::{
    allowances::TargetKind,
    focus::{ids_match, protected, rule_matches, validate_rule},
    models::ScreenTimeBucket,
};
use serde::{Deserialize, Serialize};

/// How the lock blocks its target.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LockRule {
    /// Block until this wall-clock instant (epoch ms). Computed
    /// from the requested minutes at creation; expiry is pruning,
    /// never a stored flag.
    Timed { ends_at_ms: i64 },
    /// Block for the rest of the local day once today's Screen
    /// Time usage reaches this many milliseconds.
    DailyLimit { limit_ms: u64 },
}

/// One app lock. Enforcement (`is_enforced`) is always computed
/// from the rule plus current Screen Time usage, never stored, so
/// a disable, a delete, an expiry, or a midnight roll lifts the
/// block with no cleanup step beyond pruning expired timed locks.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppLock {
    pub id: String,
    pub title: String,
    /// Site | App, reused from allowances (there is exactly one
    /// target-kind enum in the codebase).
    pub kind: TargetKind,
    /// Site: a host-only website rule (plain or wildcard domain).
    /// App: a platform app id (macOS bundle id, Windows exe path,
    /// or `store:<package family name>`), matched with
    /// `focus::ids_match`.
    pub target: String,
    pub rule: LockRule,
    pub enabled: bool,
    pub created_at: i64,
}

/// Frontend input for `CreateAppLock`: whole minutes, so the
/// action JSON stays small and human (`{"kind":"timed",
/// "minutes":30}`). The engine computes the stored rule from it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppLockRuleInput {
    Timed { minutes: u32 },
    DailyLimit { limit_minutes: u32 },
}

fn title(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 {
        return Err("Use a title between 1 and 160 characters.".into());
    }
    Ok(value.into())
}

/// Validate an app lock's fields (title, target, rule). The stored
/// rule already carries the computed `ends_at_ms` / `limit_ms`
/// (the `CreateAppLock` arm derives them from whole minutes and
/// range-checks the input first); the bounds here are the backstop
/// for hand-built locks.
pub fn validate_lock(mut lock: AppLock, now_ms: i64) -> Result<AppLock, String> {
    lock.title = title(&lock.title)?;
    match lock.kind {
        TargetKind::Site => {
            let rule = validate_rule(&lock.target)?;
            if crate::focus::is_keyword_rule(&rule) {
                return Err(
                    "An app lock needs a domain, not a search: keyword — keywords have no host to watch."
                        .into(),
                );
            }
            if rule.contains('/') {
                return Err(
                    "An app lock needs a plain or wildcard domain, not a domain with a path — the engine only sees hosts, so a path lock could never accrue usage."
                        .into(),
                );
            }
            lock.target = rule;
        }
        TargetKind::App => {
            let target = lock.target.trim();
            if target.is_empty() || target.len() > 1024 || target.chars().any(char::is_control) {
                return Err("Use an app id up to 1024 characters.".into());
            }
            if protected(target) {
                return Err("Preelock itself and system apps cannot be app-locked.".into());
            }
            lock.target = target.into();
        }
    }
    match lock.rule {
        LockRule::Timed { ends_at_ms } => {
            let duration_ms = ends_at_ms.saturating_sub(now_ms);
            if duration_ms <= 0 || duration_ms > 10080 * 60_000 {
                return Err(
                    "Choose a timed lock duration from 1 to 10080 minutes (7 days).".into(),
                );
            }
        }
        LockRule::DailyLimit { limit_ms } => {
            if limit_ms == 0 || limit_ms > 1440 * 60_000 {
                return Err("Choose a daily limit from 1 to 1440 minutes.".into());
            }
        }
    }
    Ok(lock)
}

/// A timed lock enforces while its wall-clock end is still in the
/// future. Disabled locks never enforce.
pub fn timed_active(lock: &AppLock, now_ms: i64) -> bool {
    lock.enabled && matches!(lock.rule, LockRule::Timed { ends_at_ms } if now_ms < ends_at_ms)
}

/// Drop timed locks whose end has passed. Returns whether anything
/// was removed; the caller persists the deletion. Expiry is
/// deletion, so an expired lock can never enforce — not even
/// across a kill/restart, since the first tick prunes it.
pub fn prune_expired(locks: &mut Vec<AppLock>, now_ms: i64) -> bool {
    let before = locks.len();
    locks.retain(|l| !matches!(l.rule, LockRule::Timed { ends_at_ms } if ends_at_ms <= now_ms));
    locks.len() != before
}

/// Today's usage of one lock target from the always-on Screen Time
/// buckets: `ms` summed over buckets on this local calendar day
/// whose kind and target match. Apps match with
/// `focus::ids_match` (the step-4 identity rules); sites match
/// with `focus::rule_matches` on the host (the engine only ever
/// sees a host).
pub fn today_usage_ms(
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    kind: TargetKind,
    target: &str,
) -> u64 {
    let want_kind = match kind {
        TargetKind::App => "app",
        TargetKind::Site => "site",
    };
    buckets
        .iter()
        .filter(|b| b.day_ms == day_ms && b.kind == want_kind)
        .filter(|b| match kind {
            TargetKind::App => ids_match(target, &b.target),
            TargetKind::Site => rule_matches(target, &b.target, ""),
        })
        .map(|b| b.ms)
        .sum()
}

/// A daily-limit lock enforces once today's usage reaches the
/// limit — "at the limit" triggers, and a limit set when today's
/// usage already exceeds it blocks immediately (computed, not
/// stored). Disabled locks never enforce.
pub fn limit_exceeded(
    lock: &AppLock,
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    _now_ms: i64,
) -> bool {
    let LockRule::DailyLimit { limit_ms } = lock.rule else {
        return false;
    };
    lock.enabled && today_usage_ms(buckets, day_ms, lock.kind, &lock.target) >= limit_ms
}

/// Whether this lock blocks its target right now. Enforcement is
/// computed, never stored: disabling or deleting the lock, a timed
/// lock expiring, or a midnight roll lifts the block on the next
/// tick / policy publish.
pub fn is_enforced(lock: &AppLock, buckets: &[ScreenTimeBucket], day_ms: i64, now_ms: i64) -> bool {
    lock.enabled && (timed_active(lock, now_ms) || limit_exceeded(lock, buckets, day_ms, now_ms))
}

/// Whether this app is app-locked right now. Unlike
/// `allowances::app_exhausted` there is deliberately NO
/// session-browser exemption: the user explicitly targeted this
/// app, and hiding (not killing) keeps the browser process and its
/// extension alive. `focus::protected` still guards Preelock
/// itself and system apps.
pub fn app_locked(
    locks: &[AppLock],
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
    app_id: &str,
) -> bool {
    if app_id.is_empty() || protected(app_id) {
        return false;
    }
    locks.iter().any(|l| {
        l.kind == TargetKind::App
            && ids_match(&l.target, app_id)
            && is_enforced(l, buckets, day_ms, now_ms)
    })
}

/// The enforced lock matching this app, if any — for "why am I
/// blocked" reasons. Mirrors `app_locked`'s guards; the first
/// enforced match wins (locks are user-set and specific, so any
/// enforced one is a true cause).
pub fn enforced_lock_for_app<'a>(
    locks: &'a [AppLock],
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
    app_id: &str,
) -> Option<&'a AppLock> {
    if app_id.is_empty() || protected(app_id) {
        return None;
    }
    locks.iter().find(|l| {
        l.kind == TargetKind::App
            && ids_match(&l.target, app_id)
            && is_enforced(l, buckets, day_ms, now_ms)
    })
}

/// Whether this site (domain/URL) is app-locked right now.
pub fn site_locked(
    locks: &[AppLock],
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
    domain: &str,
) -> bool {
    if domain.is_empty() {
        return false;
    }
    locks.iter().any(|l| {
        l.kind == TargetKind::Site
            && rule_matches(&l.target, domain, "")
            && is_enforced(l, buckets, day_ms, now_ms)
    })
}

/// The enforced lock matching this site, if any.
pub fn enforced_lock_for_site<'a>(
    locks: &'a [AppLock],
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
    domain: &str,
) -> Option<&'a AppLock> {
    if domain.is_empty() {
        return None;
    }
    locks.iter().find(|l| {
        l.kind == TargetKind::Site
            && rule_matches(&l.target, domain, "")
            && is_enforced(l, buckets, day_ms, now_ms)
    })
}

/// Targets of enforced site-kind locks, sorted and deduped, for
/// the published policy's `extra_blocked` list (the same overlay
/// step 8 uses — no extension change).
pub fn locked_site_rules(
    locks: &[AppLock],
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
) -> Vec<String> {
    let mut rules: Vec<String> = locks
        .iter()
        .filter(|l| l.kind == TargetKind::Site && is_enforced(l, buckets, day_ms, now_ms))
        .map(|l| l.target.clone())
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

/// Live status for the UI: `(enforced, used_or_elapsed_ms,
/// total_ms)`. Timed: elapsed since creation vs the full lock
/// length (remaining = total - elapsed). Daily limit: today's
/// usage vs the limit.
///
/// The current UI computes status from the snapshot
/// client-side, so this is exercised by unit tests only;
/// the allow keeps `clippy -D warnings` green until a backend
/// status command needs it.
#[allow(dead_code)]
pub fn lock_status(
    lock: &AppLock,
    buckets: &[ScreenTimeBucket],
    day_ms: i64,
    now_ms: i64,
) -> (bool, u64, u64) {
    let enforced = is_enforced(lock, buckets, day_ms, now_ms);
    match lock.rule {
        LockRule::Timed { ends_at_ms } => {
            let total = ends_at_ms.saturating_sub(lock.created_at).max(0) as u64;
            let elapsed = now_ms.saturating_sub(lock.created_at).max(0) as u64;
            (enforced, elapsed.min(total), total)
        }
        LockRule::DailyLimit { limit_ms } => (
            enforced,
            today_usage_ms(buckets, day_ms, lock.kind, &lock.target),
            limit_ms,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(kind: TargetKind, target: &str, rule: LockRule, now: i64) -> AppLock {
        validate_lock(
            AppLock {
                id: "l1".into(),
                title: "Test".into(),
                kind,
                target: target.into(),
                rule,
                enabled: true,
                created_at: now,
            },
            now,
        )
        .unwrap()
    }

    fn timed_lock(target: &str, ends_at_ms: i64, now: i64) -> AppLock {
        lock(TargetKind::App, target, LockRule::Timed { ends_at_ms }, now)
    }

    fn daily_lock(target: &str, limit_minutes: u32, now: i64) -> AppLock {
        lock(
            TargetKind::Site,
            target,
            LockRule::DailyLimit {
                limit_ms: u64::from(limit_minutes) * 60_000,
            },
            now,
        )
    }

    fn bucket(day_ms: i64, kind: &str, target: &str, ms: u64) -> ScreenTimeBucket {
        ScreenTimeBucket {
            day_ms,
            kind: kind.into(),
            target: target.into(),
            name: target.into(),
            ms,
        }
    }

    #[test]
    fn timed_lock_enforces_before_ends_at_and_not_at_or_after() {
        let now = 1_800_000_000_000;
        let l = timed_lock("com.example.game", now + 30 * 60_000, now);
        let buckets: Vec<ScreenTimeBucket> = vec![];
        assert!(timed_active(&l, now));
        assert!(is_enforced(&l, &buckets, now, now));
        assert!(timed_active(&l, now + 30 * 60_000 - 1));
        // At the end instant the lock no longer enforces.
        assert!(!timed_active(&l, now + 30 * 60_000));
        assert!(!is_enforced(&l, &buckets, now, now + 30 * 60_000));
        assert!(!is_enforced(&l, &buckets, now, now + 31 * 60_000));
        // Disabled: the block lifts immediately.
        let mut d = l.clone();
        d.enabled = false;
        assert!(!is_enforced(&d, &buckets, now, now));
    }

    #[test]
    fn expired_timed_lock_prunes_and_never_enforces_across_restart() {
        let now = 1_800_000_000_000;
        // Valid when created (validated 2 s before its end), expired
        // since — the state a restart would load from disk.
        let expired = validate_lock(
            AppLock {
                id: "old".into(),
                title: "Old".into(),
                kind: TargetKind::Site,
                target: "example.com".into(),
                rule: LockRule::Timed {
                    ends_at_ms: now - 1000,
                },
                enabled: true,
                created_at: now - 60_000,
            },
            now - 2000,
        )
        .unwrap();
        let buckets: Vec<ScreenTimeBucket> = vec![];
        assert!(!is_enforced(&expired, &buckets, now, now));
        let mut locks = vec![expired];
        assert!(prune_expired(&mut locks, now));
        assert!(locks.is_empty());
        // Nothing left to prune: no-op.
        assert!(!prune_expired(&mut locks, now));
        // A live timed lock and a daily-limit lock survive pruning.
        let mut locks = vec![
            timed_lock("com.example.game", now + 60_000, now),
            daily_lock("youtube.com", 30, now),
        ];
        assert!(!prune_expired(&mut locks, now));
        assert_eq!(locks.len(), 2);
    }

    #[test]
    fn daily_limit_triggers_exactly_at_and_above_the_limit() {
        let now = 1_800_000_000_000;
        let day = 1_799_990_400_000;
        let l = daily_lock("youtube.com", 30, now);
        let under = vec![bucket(day, "site", "www.youtube.com", 29 * 60_000)];
        assert!(!limit_exceeded(&l, &under, day, now));
        assert!(!is_enforced(&l, &under, day, now));
        // Exactly at the limit triggers.
        let exact = vec![bucket(day, "site", "www.youtube.com", 30 * 60_000)];
        assert!(limit_exceeded(&l, &exact, day, now));
        assert!(is_enforced(&l, &exact, day, now));
        // Above the limit triggers too.
        let over = vec![bucket(day, "site", "www.youtube.com", 45 * 60_000)];
        assert!(limit_exceeded(&l, &over, day, now));
        // A wildcard lock target matches subdomains; an unrelated
        // domain does not count.
        let wild = daily_lock("*.example.com", 30, now);
        let sub = vec![bucket(day, "site", "app.example.com", 30 * 60_000)];
        assert!(is_enforced(&wild, &sub, day, now));
        let other = vec![bucket(day, "site", "other.com", 30 * 60_000)];
        assert!(!is_enforced(&wild, &other, day, now));
    }

    #[test]
    fn midnight_resets_the_daily_limit() {
        let now = 1_800_000_000_000;
        let today = 1_799_990_400_000;
        let yesterday = today - 86_400_000;
        let l = daily_lock("youtube.com", 30, now);
        // Yesterday's usage alone never triggers today's lock.
        let old = vec![bucket(yesterday, "site", "youtube.com", 120 * 60_000)];
        assert!(!limit_exceeded(&l, &old, today, now));
        assert_eq!(
            today_usage_ms(&old, today, TargetKind::Site, "youtube.com"),
            0
        );
        // Today's usage counts even with old buckets present.
        let mixed = vec![
            bucket(yesterday, "site", "youtube.com", 120 * 60_000),
            bucket(today, "site", "youtube.com", 30 * 60_000),
        ];
        assert!(is_enforced(&l, &mixed, today, now));
    }

    #[test]
    fn limit_set_when_usage_already_exceeded_blocks_immediately() {
        let now = 1_800_000_000_000;
        let day = 1_799_990_400_000;
        // The user already spent 45 minutes on the app today, then
        // creates a 30-minute daily limit: it enforces at once.
        let buckets = vec![bucket(day, "app", "com.example.game", 45 * 60_000)];
        let l = lock(
            TargetKind::App,
            "com.example.game",
            LockRule::DailyLimit {
                limit_ms: 30 * 60_000,
            },
            now,
        );
        assert!(is_enforced(&l, &buckets, day, now));
        assert!(app_locked(&[l], &buckets, day, now, "com.example.game"));
    }

    #[test]
    fn app_locked_guards_empty_and_protected_ids_and_sorts_site_rules() {
        let now = 1_800_000_000_000;
        let day = 1_799_990_400_000;
        let buckets: Vec<ScreenTimeBucket> = vec![];
        let locks = vec![timed_lock("com.example.game", now + 60_000, now)];
        // Empty and protected ids are never locked, whatever the locks say.
        assert!(!app_locked(&locks, &buckets, day, now, ""));
        assert!(!app_locked(
            &locks,
            &buckets,
            day,
            now,
            "com.preelock.desktop"
        ));
        assert!(!app_locked(&locks, &buckets, day, now, "preelock.exe"));
        // The locked app matches by id (case-insensitive).
        assert!(app_locked(&locks, &buckets, day, now, "com.example.game"));
        assert!(app_locked(&locks, &buckets, day, now, "COM.EXAMPLE.GAME"));
        assert!(!app_locked(&locks, &buckets, day, now, "com.example.other"));
        // Site rules come from enforced site locks only, sorted and deduped.
        let mut site_locks = vec![
            lock(
                TargetKind::Site,
                "youtube.com",
                LockRule::Timed {
                    ends_at_ms: now + 60_000,
                },
                now,
            ),
            lock(
                TargetKind::Site,
                "*.example.com",
                LockRule::Timed {
                    ends_at_ms: now + 60_000,
                },
                now,
            ),
            lock(
                TargetKind::Site,
                "youtube.com",
                LockRule::Timed {
                    ends_at_ms: now + 120_000,
                },
                now,
            ),
        ];
        // A disabled lock contributes nothing.
        let mut disabled = site_locks[0].clone();
        disabled.enabled = false;
        site_locks.push(disabled);
        // An app lock contributes nothing to the site list.
        site_locks.push(timed_lock("com.example.game", now + 60_000, now));
        assert_eq!(
            locked_site_rules(&site_locks, &buckets, day, now),
            vec!["*.example.com".to_string(), "youtube.com".to_string()]
        );
    }

    #[test]
    fn validation_rejects_bad_targets_titles_and_ranges() {
        let now = 1_800_000_000_000;
        let base = |kind: TargetKind, target: &str, rule: LockRule| AppLock {
            id: "x".into(),
            title: "T".into(),
            kind,
            target: target.into(),
            rule,
            enabled: true,
            created_at: now,
        };
        let timed = || LockRule::Timed {
            ends_at_ms: now + 30 * 60_000,
        };
        // Protected app.
        assert!(validate_lock(base(TargetKind::App, "preelock.exe", timed()), now).is_err());
        assert!(
            validate_lock(base(TargetKind::App, "com.preelock.desktop", timed()), now).is_err()
        );
        // Keyword site rule.
        assert!(validate_lock(base(TargetKind::Site, "search:chess", timed()), now).is_err());
        // Path site rule.
        assert!(validate_lock(base(TargetKind::Site, "youtube.com/shorts", timed()), now).is_err());
        // Empty title.
        assert!(validate_lock(
            AppLock {
                title: "  ".into(),
                ..base(TargetKind::Site, "youtube.com", timed())
            },
            now
        )
        .is_err());
        // Out-of-range durations.
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::Timed { ends_at_ms: now }
            ),
            now
        )
        .is_err());
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::Timed {
                    ends_at_ms: now + 10081 * 60_000
                }
            ),
            now
        )
        .is_err());
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::DailyLimit { limit_ms: 0 }
            ),
            now
        )
        .is_err());
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::DailyLimit {
                    limit_ms: 1441 * 60_000
                }
            ),
            now
        )
        .is_err());
        // Boundary values are fine, and targets normalize like allowances'.
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::Timed {
                    ends_at_ms: now + 10080 * 60_000
                }
            ),
            now
        )
        .is_ok());
        assert!(validate_lock(
            base(
                TargetKind::Site,
                "youtube.com",
                LockRule::DailyLimit {
                    limit_ms: 1440 * 60_000
                }
            ),
            now
        )
        .is_ok());
        let upper = validate_lock(base(TargetKind::Site, "YouTube.COM", timed()), now).unwrap();
        assert_eq!(upper.target, "youtube.com");
        let app =
            validate_lock(base(TargetKind::App, "  com.example.game  ", timed()), now).unwrap();
        assert_eq!(app.target, "com.example.game");
    }

    #[test]
    fn lock_status_reports_remaining_for_timed_and_used_for_daily() {
        let now = 1_800_000_000_000;
        let day = 1_799_990_400_000;
        let timed = AppLock {
            id: "t".into(),
            title: "T".into(),
            kind: TargetKind::App,
            target: "com.example.game".into(),
            rule: LockRule::Timed {
                ends_at_ms: now + 20 * 60_000,
            },
            enabled: true,
            created_at: now - 10 * 60_000,
        };
        let buckets: Vec<ScreenTimeBucket> = vec![];
        // 10 of 30 minutes elapsed; remaining = total - elapsed.
        assert_eq!(
            lock_status(&timed, &buckets, day, now),
            (true, 10 * 60_000, 30 * 60_000)
        );
        let daily = daily_lock("youtube.com", 30, now);
        let used = vec![bucket(day, "site", "youtube.com", 12 * 60_000)];
        assert_eq!(
            lock_status(&daily, &used, day, now),
            (false, 12 * 60_000, 30 * 60_000)
        );
    }

    #[test]
    fn rule_input_json_shapes_match_the_frontend_contract() {
        // create_app_lock rule payloads.
        let timed: AppLockRuleInput =
            serde_json::from_str(r#"{"kind":"timed","minutes":30}"#).unwrap();
        assert!(matches!(timed, AppLockRuleInput::Timed { minutes: 30 }));
        let daily: AppLockRuleInput =
            serde_json::from_str(r#"{"kind":"daily_limit","limit_minutes":45}"#).unwrap();
        assert!(matches!(
            daily,
            AppLockRuleInput::DailyLimit { limit_minutes: 45 }
        ));
        // Stored rule shapes.
        let rule: LockRule = serde_json::from_str(r#"{"type":"timed","ends_at_ms":123}"#).unwrap();
        assert!(matches!(rule, LockRule::Timed { ends_at_ms: 123 }));
        let rule: LockRule =
            serde_json::from_str(r#"{"type":"daily_limit","limit_ms":456}"#).unwrap();
        assert!(matches!(rule, LockRule::DailyLimit { limit_ms: 456 }));
        // A full snapshot object round-trips.
        let lock = lock(
            TargetKind::Site,
            "youtube.com",
            LockRule::DailyLimit {
                limit_ms: 1_800_000,
            },
            1_800_000_000_000,
        );
        let json = serde_json::to_string(&lock).unwrap();
        let back: AppLock = serde_json::from_str(&json).unwrap();
        assert_eq!(back.target, "youtube.com");
        assert!(matches!(back.kind, TargetKind::Site));
    }

    #[test]
    fn site_locked_and_enforced_lock_for_site_matches_domains_and_wildcards() {
        let now = 1_800_000_000_000;
        let day = 1_799_990_400_000;
        let timed = lock(
            TargetKind::Site,
            "youtube.com",
            LockRule::Timed {
                ends_at_ms: now + 30 * 60_000,
            },
            now,
        );
        let wildcard = lock(
            TargetKind::Site,
            "*.reddit.com",
            LockRule::Timed {
                ends_at_ms: now + 30 * 60_000,
            },
            now,
        );
        let locks = vec![timed, wildcard];
        let buckets = vec![];

        // Exact match
        assert!(site_locked(&locks, &buckets, day, now, "youtube.com"));
        // Subdomain of plain host matches rule_matches
        assert!(site_locked(&locks, &buckets, day, now, "www.youtube.com"));
        assert!(site_locked(&locks, &buckets, day, now, "m.youtube.com"));
        // Wildcard match
        assert!(site_locked(&locks, &buckets, day, now, "old.reddit.com"));
        assert!(site_locked(&locks, &buckets, day, now, "www.reddit.com"));
        // Non-matching sites
        assert!(!site_locked(&locks, &buckets, day, now, "github.com"));
        assert!(!site_locked(&locks, &buckets, day, now, "notyoutube.com"));
        assert!(!site_locked(&locks, &buckets, day, now, ""));

        // Enforced lock retrieval
        let found = enforced_lock_for_site(&locks, &buckets, day, now, "youtube.com");
        assert!(found.is_some());
        assert_eq!(found.unwrap().target, "youtube.com");
    }
}
