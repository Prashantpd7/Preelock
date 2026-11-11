//! Step 11 anti-tamper layer: achievable, outcome-based anti-tamper only.
//!
//! HONEST LIMITS — read before claiming anything stronger:
//! - This module does NOT prevent uninstall. Windows lets users
//!   uninstall Microsoft Store apps freely, and a macOS bundle can be
//!   trashed. It does NOT prevent force-quit or a Task Manager kill:
//!   the OS owns process lifetime.
//! - What it guarantees is the outcome: killing the app, removing or
//!   never connecting the extension, restarting, or jumping the clock
//!   does NOT end a block early. The block resumes with its remaining
//!   time preserved (kill/restart), stays enforced by blocking the
//!   browser app itself (extension never applied), or keeps its
//!   remaining time and pending delays intact (clock jumps).
//! - A session is "genuinely unrecoverable" only when its state cannot
//!   be loaded or validated at all (legacy timer upgrade, storage
//!   failure). Every live session whose remaining/focused time is
//!   checkpointed in SQLite is recoverable, so it resumes — not ended —
//!   whether the previous process died in the same boot or the desktop
//!   app was closed, quit or crashed. None of those is recorded as a
//!   force quit: the block never stopped, so there is nothing to flag.
//!   The legacy `force_quit` field survives on historical records.
//!
//! All helpers here are pure and shared across macOS and Windows;
//! there is no platform `cfg` in this module.

use crate::models::Status;

/// A wall-clock discrepancy up to this size is ordinary tick jitter
/// (the backend ticks every second; `commit` also runs on user
/// actions, so the monotonic elapsed since the last commit can be
/// large without any tampering). Only a discrepancy beyond this
/// tolerance counts as a clock jump. This matches the 5-second
/// threshold the old `clock_change` event used.
pub const CLOCK_JUMP_TOLERANCE_MS: i64 = 5_000;

/// Signed wall-clock jump: how far the wall clock moved beyond what
/// the monotonic clock accounts for since the previous commit.
/// Positive = clock jumped forward (or the wall clock was set ahead);
/// negative = clock moved backward. A machine waking from sleep
/// shows ~0 here, because the suspend-aware monotonic clock
/// (`clock::continuous_ms`) advances through sleep just like the
/// wall clock, so sleep is NOT a jump.
pub fn wall_jump_ms(prev_wall_ms: i64, now_ms: i64, monotonic_elapsed_ms: u64) -> i64 {
    now_ms
        .saturating_sub(prev_wall_ms)
        .saturating_sub(monotonic_elapsed_ms.min(i64::MAX as u64) as i64)
}

/// True when the wall clock jumped forward beyond tolerance.
pub fn is_forward_jump(jump_ms: i64) -> bool {
    jump_ms > CLOCK_JUMP_TOLERANCE_MS
}

/// True when the wall clock moved backward beyond tolerance.
pub fn is_backward_jump(jump_ms: i64) -> bool {
    jump_ms < -CLOCK_JUMP_TOLERANCE_MS
}

/// Push a pending step-10 delay-lock deadline forward by a forward
/// clock jump, so the jump cannot shortcut the delay: the user must
/// still wait the full delay in real (monotonic) time. A backward
/// jump leaves the deadline untouched — it can only lengthen the
/// real wait, never shorten it — and a jump within tolerance
/// leaves it untouched as well. `None` (no pending delay) stays
/// `None`.
pub fn shift_delay_until(until_ms: Option<i64>, jump_ms: i64) -> Option<i64> {
    if is_forward_jump(jump_ms) {
        until_ms.map(|until| until.saturating_add(jump_ms))
    } else {
        until_ms
    }
}

/// Status a live session resumes as after kill/restart: a paused
/// session stays paused (the user's pause is preserved); anything
/// else live resumes Running, so blocking starts at once without
/// waiting for the browser — website rules follow when the
/// extension connects.
pub fn resume_status(was: &Status) -> Status {
    if matches!(was, Status::Paused) {
        Status::Paused
    } else {
        Status::Running
    }
}

/// How the engine must treat the session browser while a session
/// is Running and requires website blocking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserEnforcement {
    /// The extension is connected and has applied this session's
    /// policy: website blocking works normally.
    Connected,
    /// The extension previously applied this session's policy
    /// (`policy_ever_applied`) but is not currently applied: the
    /// 2026-10-22 transient-disconnect behaviour is preserved —
    /// the session stays Running, native app blocking continues,
    /// website blocking pauses, and the Dashboard shows the
    /// "Connect browser" button. The browser app itself is NOT
    /// blocked in this case.
    TransientDisconnected,
    /// The session requires website blocking but its policy was
    /// NEVER applied by any extension for this session: fail
    /// closed by blocking the session browser app itself while
    /// the session is Running. Automatically lifts the moment
    /// the extension connects and applies the policy.
    FailClosed,
    /// No fail-closed question arises: the session needs no
    /// website blocking, is not Running, or is on a break
    /// (breaks relax restrictions by design, steps 7/9).
    NotRequired,
}

/// Classify the browser state for one session. `applied` must
/// already mean "connected AND applied_id matches this session
/// AND no bridge error" — the caller computes that from the
/// session's own browser status, never from another browser.
/// `policy_ever_applied` is the session's persisted flag, set
/// whenever `applied` is ever observed for this session run.
pub fn classify_browser(
    requires_website_blocking: bool,
    running: bool,
    on_break: bool,
    applied: bool,
    policy_ever_applied: bool,
) -> BrowserEnforcement {
    if !requires_website_blocking || !running || on_break {
        return BrowserEnforcement::NotRequired;
    }
    if applied {
        return BrowserEnforcement::Connected;
    }
    if policy_ever_applied {
        return BrowserEnforcement::TransientDisconnected;
    }
    BrowserEnforcement::FailClosed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_jump_distinguishes_jumps_from_sleep_and_jitter() {
        // Ordinary tick: wall and monotonic advance together.
        assert_eq!(wall_jump_ms(1_000, 2_000, 1_000), 0);
        // Sleep: both clocks advance through it, so no jump.
        assert_eq!(wall_jump_ms(1_000, 361_000, 360_000), 0);
        // 4 s of jitter is within tolerance.
        assert!(!is_forward_jump(wall_jump_ms(1_000, 6_000, 1_000)));
        assert!(!is_backward_jump(wall_jump_ms(1_000, 6_000, 1_000)));
        // Forward jump: wall moved 10 minutes, monotonic 1 s.
        let jump = wall_jump_ms(1_000, 601_000, 1_000);
        assert_eq!(jump, 599_000);
        assert!(is_forward_jump(jump));
        assert!(!is_backward_jump(jump));
        // Backward jump: wall moved back 10 minutes.
        let jump = wall_jump_ms(601_000, 1_000, 1_000);
        assert_eq!(jump, -601_000);
        assert!(is_backward_jump(jump));
        assert!(!is_forward_jump(jump));
    }

    #[test]
    fn delay_deadline_is_not_shortcut_by_clock_jumps() {
        let until = Some(1_000_000);
        // Forward jump pushes the deadline by exactly the jump.
        assert_eq!(
            shift_delay_until(until, 600_000),
            Some(1_600_000),
            "a 10-minute forward jump must not let a delay expire early"
        );
        // Backward jump and jitter leave it untouched.
        assert_eq!(shift_delay_until(until, -600_000), until);
        assert_eq!(shift_delay_until(until, 4_000), until);
        assert_eq!(shift_delay_until(None, 600_000), None);
    }

    #[test]
    fn fail_closed_only_when_policy_was_never_applied() {
        use BrowserEnforcement::*;
        // Applied right now: normal.
        assert_eq!(classify_browser(true, true, false, true, false), Connected);
        // Was applied before, now gone: transient disconnect —
        // session keeps running, browser app NOT blocked.
        assert_eq!(
            classify_browser(true, true, false, false, true),
            TransientDisconnected
        );
        // Never applied: fail closed, browser app blocked.
        assert_eq!(
            classify_browser(true, true, false, false, false),
            FailClosed
        );
        // Connected but for a different session (applied=false)
        // with no history is still never-applied: fail closed.
        assert_eq!(
            classify_browser(true, true, false, false, false),
            FailClosed
        );
        // No website blocking, not running, or on break: nothing.
        assert_eq!(
            classify_browser(false, true, false, false, false),
            NotRequired
        );
        assert_eq!(
            classify_browser(true, false, false, false, false),
            NotRequired
        );
        assert_eq!(
            classify_browser(true, true, true, false, false),
            NotRequired
        );
    }

    #[test]
    fn resume_status_preserves_a_user_pause() {
        assert_eq!(resume_status(&Status::Paused), Status::Paused);
        assert_eq!(resume_status(&Status::Running), Status::Running);
        assert_eq!(resume_status(&Status::Preparing), Status::Running);
    }
}
