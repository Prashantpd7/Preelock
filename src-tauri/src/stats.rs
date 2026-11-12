//! Statistics upgrade (step 13): block-attempt logs, JSON export
//! shape, and statistics deletion scope.
//!
//! Everything here is local-only: attempts are stored in the app's
//! own SQLite database (migration 008), export is a JSON string the
//! frontend downloads as a file, and deletion removes attempt rows
//! only. Sessions, schedules, settings and lock configurations are
//! never touched by statistics deletion — see
//! `Engine::delete_statistics` and the Settings copy, which names
//! exactly what is deleted ("block-attempt history").
//!
//! Website attempts are deliberately NOT per-domain rows: the
//! extension reports only one aggregate `blocks` counter (see
//! `browser.rs` / `worker.js`), incremented only by the
//! `onRuleMatchedDebug` listener, which exists for unpacked
//! installations. There is no per-domain signal to log, so website
//! blocking is reported as the existing per-session aggregate
//! (`FocusReport::browser_blocks`, summed on the Analytics page)
//! and as the live aggregate in the export, never as invented
//! per-site rows. The extension is untouched by this step and
//! stays at version 6.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Attempts older than this are pruned automatically (90 days).
pub const RETENTION_MS: i64 = 90 * 86_400_000;

/// At most one attempt row per app per window, even if the app
/// flickers in and out of the blocked state.
pub const THROTTLE_MS: i64 = 60_000;

/// Maximum rows the Analytics page asks for in one list call.
pub const RECENT_LIMIT: i64 = 100;

/// What was blocked: a native app, or (aggregate only, see the
/// module note) a website.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    App,
    Site,
}

/// One recorded block attempt. `target` is an app id (macOS bundle
/// id, Windows exe path, or `store:<package family name>`) for app
/// attempts; site attempts are not stored as rows (see module note),
/// so a stored row is always an app attempt in practice, but the
/// kind is kept so a future per-domain signal can use the same table
/// without a migration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockAttempt {
    pub at_ms: i64,
    pub kind: AttemptKind,
    pub target: String,
    pub session_id: Option<String>,
    pub reason: String,
}

impl BlockAttempt {
    pub fn app(at_ms: i64, target: &str, session_id: &str, reason: &str) -> Self {
        Self {
            at_ms,
            kind: AttemptKind::App,
            target: target.into(),
            session_id: Some(session_id.into()),
            reason: reason.into(),
        }
    }
}

/// Transition + throttle state for app-block logging, kept live in
/// the engine (never persisted: a restart starts with no app
/// "currently blocked", which is correct — the sampler re-detects
/// the blocked app on the first tick and logs it again, subject to
/// the throttle).
#[derive(Default)]
pub struct AttemptTracker {
    /// App ids currently in the blocked state, from the sampler.
    pub blocked_now: std::collections::HashSet<String>,
    /// Last logged time per app id, for the 60-second throttle.
    last_logged: HashMap<String, i64>,
}

impl AttemptTracker {
    /// True when a transition into the blocked state for `app_id`
    /// should produce a row now. A re-entry within the throttle
    /// window after the app left the blocked state is suppressed,
    /// so a flickering app cannot flood the log; the first entry
    /// after the window is logged again.
    pub fn should_log(&mut self, app_id: &str, now_ms: i64) -> bool {
        let last = self.last_logged.get(app_id).copied().unwrap_or(i64::MIN);
        if now_ms.saturating_sub(last) < THROTTLE_MS {
            return false;
        }
        self.last_logged.insert(app_id.into(), now_ms);
        true
    }

    /// Forget the live blocked set (session ended, or no live
    /// session on a tick): the next blocked sample is a fresh
    /// transition, still subject to the throttle.
    pub fn clear_blocked(&mut self) {
        self.blocked_now.clear();
    }
}

/// Cutoff for pruning: rows strictly older than this go.
pub fn prune_cutoff_ms(now_ms: i64) -> i64 {
    now_ms.saturating_sub(RETENTION_MS)
}

/// Per-target counts from a list of attempts, most-blocked first,
/// ties broken by target name so the list is stable.
pub fn counts_by_target(attempts: &[BlockAttempt]) -> Vec<(String, AttemptKind, u64)> {
    let mut counts: HashMap<(&str, AttemptKind), u64> = HashMap::new();
    for attempt in attempts {
        *counts
            .entry((attempt.target.as_str(), attempt.kind))
            .or_insert(0) += 1;
    }
    let mut out: Vec<(String, AttemptKind, u64)> = counts
        .into_iter()
        .map(|((target, kind), count)| (target.to_string(), kind, count))
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Sessions summary for the JSON export: productive time is the
/// counted focus time (`focused_ms`), distracted time is the
/// attention-tracked distracted total. Finished sessions only
/// contribute to the completed count; live sessions still
/// contribute their time so far.
pub fn sessions_summary(sessions: &[crate::models::Session]) -> (u64, u64, usize, usize) {
    let productive = sessions.iter().map(|s| s.focused_ms).sum();
    let distracted = sessions.iter().map(|s| s.report.distracting_ms).sum();
    let completed = sessions
        .iter()
        .filter(|s| s.status == crate::models::Status::Completed)
        .count();
    (productive, distracted, sessions.len(), completed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_attempt(at: i64, target: &str) -> BlockAttempt {
        BlockAttempt::app(at, target, "session-1", "Blocked app")
    }

    #[test]
    fn throttle_suppresses_re_entry_within_sixty_seconds() {
        let mut tracker = AttemptTracker::default();
        assert!(tracker.should_log("com.example.Game", 1_000));
        // Same app 30 s later: suppressed.
        assert!(!tracker.should_log("com.example.Game", 31_000));
        // Exactly at the boundary: logged again.
        assert!(tracker.should_log("com.example.Game", 61_000));
        // A different app is independent.
        assert!(tracker.should_log("com.example.Other", 61_000));
    }

    #[test]
    fn counts_rank_most_blocked_first_with_stable_ties() {
        let attempts = vec![
            app_attempt(1, "b.app"),
            app_attempt(2, "a.app"),
            app_attempt(3, "b.app"),
            app_attempt(4, "a.app"),
        ];
        let counts = counts_by_target(&attempts);
        assert_eq!(counts.len(), 2);
        // Tie at 2 each: alphabetical target order.
        assert_eq!(counts[0].0, "a.app");
        assert_eq!(counts[0].2, 2);
        assert_eq!(counts[1].0, "b.app");
    }

    #[test]
    fn prune_cutoff_is_ninety_days() {
        assert_eq!(prune_cutoff_ms(90 * 86_400_000 + 5_000), 5_000);
        assert_eq!(RETENTION_MS, 90 * 86_400_000);
    }

    #[test]
    fn sessions_summary_totals_focus_and_distraction() {
        let mut one = crate::models::Session {
            id: "s1".into(),
            title: "Work".into(),
            duration_minutes: 25,
            remaining_ms: 0,
            focused_ms: 1_200_000,
            allow_pause: true,
            status: crate::models::Status::Completed,
            started_at: None,
            ended_at: None,
            deadline_ms: None,
            focus: Default::default(),
            report: Default::default(),
            attention: Default::default(),
            force_quit: false,
            schedule_id: None,
            kind: crate::models::SessionKind::Focus,
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
            grace_notified: false,
            durability: Default::default(),
        };
        one.report.distracting_ms = 60_000;
        let (productive, distracted, total, completed) = sessions_summary(&[one]);
        assert_eq!(productive, 1_200_000);
        assert_eq!(distracted, 60_000);
        assert_eq!(total, 1);
        assert_eq!(completed, 1);
    }
}
