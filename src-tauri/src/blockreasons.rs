//! Block reasons: WHY something is blocked, in plain words.
//!
//! The engine knows the true cause at every block point (which
//! check fired), so reasons are threaded through from the decision
//! — never guessed from state after the fact.
//!
//! Cause priority (deterministic; most specific first):
//! 1. fail_closed — the session browser is blocked until its
//!    extension connects and applies the policy (the immediate
//!    technical state; checked first, matching the block-attempt
//!    log's existing order).
//! 2. TimedLock / DailyLimitReached — a user-set App lock on this
//!    exact target.
//! 3. AllowanceExhausted — a user-set budget that ran out.
//! 4. Frozen — a Frozen session blocks everything.
//! 5. SessionActive — a focus session's own rules.
//! 6. ScheduleActive — a scheduled block window.
//! Anything unrecognized renders as "Blocked by Preelock" and
//! should be rare: every engine block point supplies a real cause.

use serde::{Deserialize, Serialize};

/// Machine-readable block cause. Serialized into the published
/// policy (all extra fields are optional, so v6 extension clients
/// ignore them — the protocol stays at version 6) and used for
/// app-block notifications.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockCause {
    /// "session" | "schedule" | "frozen" | "timed_lock" |
    /// "daily_limit" | "allowance" | "adult" | "fail_closed" |
    /// "unknown"
    pub kind: String,
    /// Session / schedule / lock / allowance title, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Timed locks: wall-clock end (epoch ms), so the UI can show
    /// a live "X left" countdown instead of a stale snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at_ms: Option<i64>,
}

/// One `extra_blocked` rule paired with its cause, for the
/// extension's blocked page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuleCause {
    pub rule: String,
    pub cause: BlockCause,
}

impl BlockCause {
    fn of(kind: &str, title: &str) -> Self {
        Self {
            kind: kind.into(),
            title: title.into(),
            ends_at_ms: None,
        }
    }
    pub fn session(title: &str) -> Self {
        Self::of("session", title)
    }
    pub fn schedule(title: &str) -> Self {
        Self::of("schedule", title)
    }
    pub fn frozen() -> Self {
        Self::of("frozen", "")
    }
    pub fn timed_lock(title: &str, ends_at_ms: i64) -> Self {
        Self {
            kind: "timed_lock".into(),
            title: title.into(),
            ends_at_ms: Some(ends_at_ms),
        }
    }
    pub fn daily_limit(title: &str) -> Self {
        Self::of("daily_limit", title)
    }
    pub fn allowance(title: &str) -> Self {
        Self::of("allowance", title)
    }
    pub fn adult() -> Self {
        Self::of("adult", "")
    }
    pub fn fail_closed() -> Self {
        Self::of("fail_closed", "")
    }
    pub fn unknown() -> Self {
        Self::of("unknown", "")
    }
}

/// Specificity rank for per-rule causes: when one rule is blocked
/// by several causes, the lowest rank wins.
pub fn cause_rank(cause: &BlockCause) -> u8 {
    match cause.kind.as_str() {
        "timed_lock" | "daily_limit" => 0,
        "allowance" => 1,
        "adult" => 2,
        _ => 3,
    }
}

/// "2h 13m" style remaining time, for timed-lock reasons.
pub fn fmt_remaining(ms: i64) -> String {
    let ms = ms.max(0);
    let mins = ms / 60_000;
    if mins >= 60 {
        format!("{}h {}m", mins / 60, mins % 60)
    } else if mins >= 1 {
        format!("{}m", mins)
    } else {
        format!("{}s", (ms / 1000).max(1))
    }
}

/// Plain-words reason line for an app-block notification. `target`
/// is the app's display name; `now_ms` anchors timed-lock
/// countdowns.
pub fn cause_text(cause: &BlockCause, target: &str, now_ms: i64) -> String {
    let quoted = |t: &str| {
        if t.is_empty() {
            String::new()
        } else {
            format!(" “{t}”")
        }
    };
    match cause.kind.as_str() {
        "timed_lock" => {
            let ends = cause.ends_at_ms.unwrap_or(now_ms);
            format!(
                "{} is blocked for {} more",
                target,
                fmt_remaining(ends.saturating_sub(now_ms))
            )
        }
        "daily_limit" => format!(
            "{} reached its daily limit — blocked until midnight",
            target
        ),
        "allowance" => format!("{}: your{} budget is used up", target, quoted(&cause.title)),
        "session" => format!(
            "{} is blocked — your{} session is on",
            target,
            quoted(&cause.title)
        ),
        "schedule" => format!(
            "{} is blocked by your scheduled block{}",
            target,
            quoted(&cause.title)
        ),
        "frozen" => format!("{} is blocked — your Frozen session is on", target),
        "fail_closed" => format!("{} is blocked until the browser connects", target),
        _ => format!("{} is blocked by Preelock", target),
    }
}

/// The session-level cause: why this session's own rules block.
/// Frozen wins over schedule wins over an ordinary session.
pub fn session_cause(
    session: &crate::models::Session,
    schedules: &[crate::schedules::Schedule],
) -> BlockCause {
    if session.focus.frozen.enabled {
        BlockCause::frozen()
    } else if let Some(sid) = session.schedule_id.as_deref() {
        let title = schedules
            .iter()
            .find(|x| x.id == sid)
            .map(|x| x.title.as_str())
            .unwrap_or("Scheduled block");
        BlockCause::schedule(title)
    } else {
        BlockCause::session(&session.title)
    }
}

/// Resolve the display cause for a blocked app from the checks the
/// engine already ran — nothing is re-decided here. `lock` is the
/// enforced AppLock matching the app (if any); `allowance_title`
/// the exhausted allowance's title (if any); `session` the live
/// session whose own rules blocked the app (if any). Returns None
/// when nothing blocked.
pub fn app_cause(
    fail_closed: bool,
    lock: Option<&crate::applocks::AppLock>,
    allowance_title: Option<&str>,
    session: Option<&crate::models::Session>,
    schedules: &[crate::schedules::Schedule],
) -> Option<BlockCause> {
    if fail_closed {
        return Some(BlockCause::fail_closed());
    }
    if let Some(l) = lock {
        return Some(match l.rule {
            crate::applocks::LockRule::Timed { ends_at_ms } => {
                BlockCause::timed_lock(&l.title, ends_at_ms)
            }
            crate::applocks::LockRule::DailyLimit { .. } => BlockCause::daily_limit(&l.title),
        });
    }
    if let Some(title) = allowance_title {
        return Some(BlockCause::allowance(title));
    }
    session.map(|s| session_cause(s, schedules))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowances::TargetKind;
    use crate::applocks::{AppLock, LockRule};
    use crate::focus::FocusConfig;
    use crate::models::{Session, Status};

    fn session(title: &str) -> Session {
        Session {
            id: "s1".into(),
            title: title.into(),
            duration_minutes: 25,
            remaining_ms: 25 * 60_000,
            focused_ms: 0,
            allow_pause: true,
            status: Status::Running,
            started_at: Some(1000),
            ended_at: None,
            deadline_ms: None,
            focus: FocusConfig::default(),
            report: Default::default(),
            attention: Default::default(),
            force_quit: false,
            schedule_id: None,
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
        }
    }

    fn lock(rule: LockRule) -> AppLock {
        AppLock {
            id: "l1".into(),
            title: "Game block".into(),
            kind: TargetKind::App,
            target: "com.example.game".into(),
            rule,
            enabled: true,
            created_at: 1_000,
        }
    }

    #[test]
    fn session_block_names_the_session() {
        let s = session("Deep work");
        assert_eq!(session_cause(&s, &[]), BlockCause::session("Deep work"));
        assert_eq!(
            app_cause(false, None, None, Some(&s), &[]),
            Some(BlockCause::session("Deep work"))
        );
    }

    #[test]
    fn schedule_block_names_the_schedule() {
        let mut s = session("Auto");
        s.schedule_id = Some("sched-1".into());
        let schedules = vec![crate::schedules::Schedule {
            id: "sched-1".into(),
            title: "Morning block".into(),
            days: vec![1],
            start_time: "09:00".into(),
            end_time: "12:00".into(),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: 0,
        }];
        assert_eq!(
            session_cause(&s, &schedules),
            BlockCause::schedule("Morning block")
        );
        // A schedule id with no matching row still reads as scheduled.
        let mut s2 = session("Auto");
        s2.schedule_id = Some("gone".into());
        assert_eq!(
            session_cause(&s2, &[]),
            BlockCause::schedule("Scheduled block")
        );
    }

    #[test]
    fn frozen_beats_session_and_schedule() {
        let mut s = session("Deep work");
        s.schedule_id = Some("sched-1".into());
        s.focus.frozen.enabled = true;
        assert_eq!(session_cause(&s, &[]), BlockCause::frozen());
    }

    #[test]
    fn timed_lock_carries_title_and_end() {
        let l = lock(LockRule::Timed {
            ends_at_ms: 2_000_000,
        });
        let cause = app_cause(false, Some(&l), None, None, &[]).unwrap();
        assert_eq!(cause, BlockCause::timed_lock("Game block", 2_000_000));
        assert_eq!(
            cause_text(&cause, "Game", 1_000_000),
            "Game is blocked for 16m more"
        );
    }

    #[test]
    fn daily_limit_names_the_limit() {
        let l = lock(LockRule::DailyLimit {
            limit_ms: 3_600_000,
        });
        let cause = app_cause(false, Some(&l), None, None, &[]).unwrap();
        assert_eq!(cause, BlockCause::daily_limit("Game block"));
        assert_eq!(
            cause_text(&cause, "Game", 1_000_000),
            "Game reached its daily limit — blocked until midnight"
        );
    }

    #[test]
    fn allowance_exhausted_names_the_budget() {
        let cause = app_cause(false, None, Some("YouTube"), None, &[]).unwrap();
        assert_eq!(cause, BlockCause::allowance("YouTube"));
        assert_eq!(
            cause_text(&cause, "Safari", 1_000_000),
            "Safari: your “YouTube” budget is used up"
        );
    }

    #[test]
    fn priority_lock_beats_allowance_beats_session() {
        let s = session("Deep work");
        let l = lock(LockRule::Timed {
            ends_at_ms: 2_000_000,
        });
        // Lock wins over allowance and session.
        assert_eq!(
            app_cause(false, Some(&l), Some("YouTube"), Some(&s), &[]),
            Some(BlockCause::timed_lock("Game block", 2_000_000))
        );
        // Allowance wins over session.
        assert_eq!(
            app_cause(false, None, Some("YouTube"), Some(&s), &[]),
            Some(BlockCause::allowance("YouTube"))
        );
        // Session alone.
        assert_eq!(
            app_cause(false, None, None, Some(&s), &[]),
            Some(BlockCause::session("Deep work"))
        );
        // Nothing blocked: no cause.
        assert_eq!(app_cause(false, None, None, None, &[]), None);
    }

    #[test]
    fn fail_closed_is_the_immediate_cause() {
        let s = session("Deep work");
        let l = lock(LockRule::Timed {
            ends_at_ms: 2_000_000,
        });
        assert_eq!(
            app_cause(true, Some(&l), None, Some(&s), &[]),
            Some(BlockCause::fail_closed())
        );
        assert_eq!(
            cause_text(&BlockCause::fail_closed(), "Chrome", 1_000_000),
            "Chrome is blocked until the browser connects"
        );
    }

    #[test]
    fn fallback_reads_plainly() {
        assert_eq!(
            cause_text(&BlockCause::unknown(), "Game", 1_000_000),
            "Game is blocked by Preelock"
        );
        assert_eq!(fmt_remaining(2 * 3_600_000 + 13 * 60_000), "2h 13m");
        assert_eq!(fmt_remaining(45 * 60_000), "45m");
        assert_eq!(fmt_remaining(30_000), "30s");
    }

    #[test]
    fn cause_rank_orders_specificity() {
        assert!(
            cause_rank(&BlockCause::timed_lock("t", 1)) < cause_rank(&BlockCause::allowance("t"))
        );
        assert!(
            cause_rank(&BlockCause::daily_limit("t")) < cause_rank(&BlockCause::allowance("t"))
        );
        assert!(cause_rank(&BlockCause::allowance("t")) < cause_rank(&BlockCause::adult()));
        assert!(cause_rank(&BlockCause::adult()) < cause_rank(&BlockCause::unknown()));
    }

    #[test]
    fn causes_serialize_with_optional_fields_skipped() {
        let c = BlockCause::timed_lock("Game block", 2_000_000);
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"kind\":\"timed_lock\""));
        assert!(json.contains("\"ends_at_ms\":2000000"));
        let plain = BlockCause::frozen();
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("ends_at_ms"));
        assert!(!json.contains("title"));
        // A v6 client simply ignores the new fields; the policy still parses.
        let back: BlockCause = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plain);
    }
}
