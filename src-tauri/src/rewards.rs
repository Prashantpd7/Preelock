//! Reward and delay breaks (step 9).
//!
//! Focused work earns break credit, and a break the user asks for can
//! be delayed before it starts. Both ride the step-7 break path, so
//! they inherit its guarantees with no extension change:
//!
//! * Credit accrues ONLY from productive time actually credited
//!   during a Focus phase — the same productive-time source the
//!   Pomodoro phase machine uses (attention `observe` deltas on
//!   attention sessions, `sessions::advance` slices on plain-timer
//!   sessions). Distracted, idle, paused, and break time earn nothing.
//! * During any break (Pomodoro, reward, or delayed-once-started)
//!   blocking relaxes exactly as Pomodoro breaks do: the engine
//!   publishes no network policy and the native sample is never
//!   treated as blocked, the tick skips productive accounting, and
//!   break time never counts toward the productive target and never
//!   consumes an allowance budget (step 8: the tick passes one
//!   unified `in_break` flag to accrual and enforcement).
//! * Breaks always end on their own: a reward break ends when its
//!   credit runs out or its max length hits, and a session ending
//!   (or its schedule window closing, or its target completing) ends
//!   any break and discards unspent credit. Breaks never extend the
//!   session's productive target.
//!
//! State machine (one break at a time, never stacked):
//!
//! * Focus — the only state that earns credit. A Pomodoro break and a
//!   reward break never run together: taking a reward break during a
//!   Pomodoro Focus phase pauses the Pomodoro cadence (the phase and
//!   its remaining time are frozen while the reward break runs, and
//!   resume where they left off), it does not stack on it. Taking a
//!   reward break while a Pomodoro break is already running is
//!   rejected, and a Pomodoro break starting while a delayed break is
//!   pending cancels that pending break (no penalty, credit untouched
//!   — see `pomodoro::complete_focus_phase`).
//! * Pending — a requested break whose delay has not elapsed yet.
//!   Focus and blocking continue normally; the delay counts down on
//!   live ticks only, so pausing pauses the delay too. Cancelling
//!   returns to normal focus with no penalty and no credit spent:
//!   credit is spent minute-for-minute AS the break runs (see
//!   `advance_break`), never at request time.
//! * Reward break — blocking relaxed, break time counted nowhere.
//!   Its length is `min(credit, max_break)` at start; each minute
//!   (millisecond, in the accounting) of break consumes one minute
//!   of credit, so a break ends exactly when the credit that funded
//!   it runs out, or earlier at the max length with credit left over.
//!
//! Caps and bounds (validated in `RewardConfig::validate`, which
//! `focus::validate` calls, so manual sessions, schedule snapshots
//! and one-shot snapshots all get the same checks):
//!
//! * Earn rate: `earn_focus_minutes` of productive focus earns
//!   `earn_break_minutes` of credit (defaults 25 -> 5, mirroring the
//!   Pomodoro defaults). Fractional progress is kept in
//!   `earn_progress_ms`, so partial focus is never lost; credit
//!   earned beyond the cap is lost, not banked.
//! * Credit cap: `min(max_credit_minutes, the session's total
//!   productive target)` — a session can never bank more break time
//!   than its whole target, and never more than the configured max
//!   (default 30 minutes). The cap is fixed for the session, so
//!   credit banked early is never clawed back as the remaining
//!   target shrinks.
//! * Single break max: `max_break_minutes` (default 30, range 1-60).
//! * Delay: `delay_minutes` (default 0 = the break starts at once;
//!   any real delay is 1-60 minutes).
//!
//! Persistence: credit lives in the session state
//! (`Session.reward`, serde-default, same pattern as step 7's
//! `Session.pomodoro`), so pre-step saves load with no credit and no
//! pending or active break, and no database migration is needed.
use serde::{Deserialize, Serialize};

use crate::{attention::State, models::Session};

/// Reward-break configuration, stored on [`crate::focus::FocusConfig`].
/// Serde-default: sessions saved before this step load as disabled.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RewardConfig {
    pub enabled: bool,
    pub earn_focus_minutes: u32,
    pub earn_break_minutes: u32,
    pub max_credit_minutes: u32,
    pub max_break_minutes: u32,
    /// 0 = a requested break starts at once; otherwise the delay in
    /// minutes before it starts (bounded 1-60 by `validate`).
    pub delay_minutes: u32,
}
impl Default for RewardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            earn_focus_minutes: 25,
            earn_break_minutes: 5,
            max_credit_minutes: 30,
            max_break_minutes: 30,
            delay_minutes: 0,
        }
    }
}
impl RewardConfig {
    pub fn earn_focus_ms(&self) -> u64 {
        u64::from(self.earn_focus_minutes) * 60_000
    }
    pub fn earn_break_ms(&self) -> u64 {
        u64::from(self.earn_break_minutes) * 60_000
    }
    pub fn max_credit_ms(&self) -> u64 {
        u64::from(self.max_credit_minutes) * 60_000
    }
    pub fn max_break_ms(&self) -> u64 {
        u64::from(self.max_break_minutes) * 60_000
    }
    pub fn delay_ms(&self) -> u64 {
        u64::from(self.delay_minutes) * 60_000
    }
    /// Range check, applied by `focus::validate` (which covers manual
    /// sessions, schedule snapshots and one-shot snapshots alike).
    /// Disabled configs are never rejected: their values are ignored.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if !(1..=180).contains(&self.earn_focus_minutes) {
            return Err("Focus time to earn a break must be 1-180 minutes.".into());
        }
        if !(1..=60).contains(&self.earn_break_minutes) {
            return Err("Break time earned must be 1-60 minutes.".into());
        }
        if !(1..=120).contains(&self.max_credit_minutes) {
            return Err("Maximum banked break credit must be 1-120 minutes.".into());
        }
        if !(1..=60).contains(&self.max_break_minutes) {
            return Err("Maximum single break must be 1-60 minutes.".into());
        }
        if self.delay_minutes > 60 {
            return Err("Break delay must be 0-60 minutes (0 starts at once).".into());
        }
        Ok(())
    }
    /// Effective credit cap for this session: the configured max,
    /// never more than the session's total productive target.
    pub fn cap_ms(&self, session: &Session) -> u64 {
        self.max_credit_ms()
            .min(u64::from(session.duration_minutes) * 60_000)
    }
}

/// Live reward-break state, stored on the session and persisted with
/// it. Encoding invariant: `pending_delay_ms > 0` means a delayed
/// break is pending, `break_remaining_ms > 0` means a reward break is
/// running; both are never positive at once (a break starts only
/// when its delay has fully elapsed, and a new request is rejected
/// while either is positive).
/// Serde-default: sessions saved before this step load with no
/// credit, no progress, and no pending or active break.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RewardState {
    pub credit_ms: u64,
    /// Productive focus time banked toward the next earn unit; kept
    /// across ticks so partial focus is never lost.
    pub earn_progress_ms: u64,
    pub pending_delay_ms: u64,
    pub break_remaining_ms: u64,
}

/// True when a delayed break is waiting to start (focus continues).
pub fn pending(s: &Session) -> bool {
    s.reward.pending_delay_ms > 0
}

/// True when a reward break is running right now (blocking relaxed).
pub fn on_break(s: &Session) -> bool {
    s.reward.break_remaining_ms > 0
}

/// True when the session is on ANY break (Pomodoro or reward): the
/// single flag the engine passes to blocking, policy publishing,
/// and allowance accrual, so all break kinds relax identically.
pub fn on_any_break(s: &Session) -> bool {
    crate::pomodoro::in_break(s) || on_break(s)
}

/// Productive time credited during a Focus phase. Callers guarantee
/// the session is in Focus (the engine tick and `sessions::advance`
/// both route break time elsewhere), so this function never needs to
/// check the phase itself. Credit beyond the cap is lost, not banked.
pub fn accrue(s: &mut Session, productive_ms: u64) {
    if !s.focus.reward.enabled || productive_ms == 0 {
        return;
    }
    let focus_unit = s.focus.reward.earn_focus_ms();
    if focus_unit == 0 {
        return;
    }
    let earn_unit = s.focus.reward.earn_break_ms();
    let cap = s.focus.reward.cap_ms(s);
    let mut progress = s.reward.earn_progress_ms.saturating_add(productive_ms);
    while progress >= focus_unit {
        progress -= focus_unit;
        s.reward.credit_ms = s.reward.credit_ms.saturating_add(earn_unit).min(cap);
    }
    s.reward.earn_progress_ms = progress;
}

/// Start a reward break now, spending credit as it runs. Defensive
/// no-op unless the session has credit, is reward-enabled, and is on
/// no break of either kind — every caller checks first, this is the
/// second line of defence against stacking.
fn start_break(s: &mut Session, now: i64) {
    if !s.focus.reward.enabled || s.reward.credit_ms == 0 || on_any_break(s) {
        return;
    }
    let length = s.reward.credit_ms.min(s.focus.reward.max_break_ms());
    if length == 0 {
        return;
    }
    s.reward.pending_delay_ms = 0;
    s.reward.break_remaining_ms = length;
    s.attention.state = State::Idle;
    s.attention.reason = "On reward break: productive time is paused.".into();
    s.attention.candidate_ms = 0;
    s.report.event(
        now,
        "reward_break",
        &format!(
            "Reward break started ({} min). Blocking is relaxed until the break ends.",
            length / 60_000,
        ),
    );
}

/// The user asks for a break. With no delay it starts at once; with a
/// delay it becomes pending and starts automatically when the delay
/// elapses. Nothing is spent at request time, so cancelling a pending
/// break costs nothing. Only a Running session in Focus with credit
/// can take a break: Pomodoro breaks, reward breaks, and pending
/// breaks all reject a second request, and a Paused session gets no
/// ticks, so its delay could never count down.
pub fn request_break(s: &mut Session, now: i64) -> Result<(), String> {
    if !s.focus.reward.enabled {
        return Err("Reward breaks are not enabled for this session.".into());
    }
    if s.status != crate::models::Status::Running {
        return Err("Reward breaks can only be taken while a session is running.".into());
    }
    if on_break(s) || crate::pomodoro::in_break(s) {
        return Err("Already on a break.".into());
    }
    if pending(s) {
        return Err("A delayed break is already waiting to start.".into());
    }
    if s.reward.credit_ms == 0 {
        return Err("No break credit earned yet. Focus first, then take a break.".into());
    }
    let delay = s.focus.reward.delay_ms();
    if delay == 0 {
        start_break(s, now);
    } else {
        s.reward.pending_delay_ms = delay;
        s.report.event(
            now,
            "reward_break_delayed",
            &format!(
                "Break starts in {} minutes. Focus and blocking continue until then.",
                s.focus.reward.delay_minutes,
            ),
        );
    }
    Ok(())
}

/// Cancel a pending delayed break before it starts. Returns to
/// normal focus with no penalty: no credit was spent at request time.
pub fn cancel_pending(s: &mut Session, now: i64) -> Result<(), String> {
    if !pending(s) {
        return Err("No delayed break is waiting to start.".into());
    }
    s.reward.pending_delay_ms = 0;
    s.report.event(
        now,
        "reward_break_cancelled",
        "Delayed break cancelled. No credit spent.",
    );
    Ok(())
}

/// Silent version for the Pomodoro hook: a Pomodoro break starting
/// while a delayed break is pending cancels the pending break (the
/// two must never stack), with an event explaining why. Credit is
/// untouched.
pub fn cancel_pending_for_pomodoro_break(s: &mut Session, now: i64) {
    if pending(s) {
        s.reward.pending_delay_ms = 0;
        s.report.event(
            now,
            "reward_break_cancelled",
            "Delayed break cancelled because a Pomodoro break started. No credit spent.",
        );
    }
}

/// Tick time passing during a pending delay. Focus continues
/// normally during the delay (the caller still runs productive
/// accounting for this tick); when the delay elapses the break
/// begins automatically, at once, with no further user action.
pub fn advance_delay(s: &mut Session, elapsed_ms: u64, now: i64) {
    if !pending(s) {
        return;
    }
    if elapsed_ms >= s.reward.pending_delay_ms {
        s.reward.pending_delay_ms = 0;
        start_break(s, now);
    } else {
        s.reward.pending_delay_ms -= elapsed_ms;
    }
}

/// Tick time passing during a reward break, spent minute-for-minute
/// from credit: the break's remaining time and the credit balance
/// fall together, so the break ends exactly when the credit that
/// funded it runs out (or earlier, at the max length, with credit
/// left over). The break ends on its own; any overshoot is dropped.
pub fn advance_break(s: &mut Session, elapsed_ms: u64, now: i64) {
    if !on_break(s) {
        return;
    }
    let take = elapsed_ms.min(s.reward.break_remaining_ms);
    s.reward.break_remaining_ms -= take;
    s.reward.credit_ms = s.reward.credit_ms.saturating_sub(take);
    if s.reward.break_remaining_ms == 0 {
        s.attention.state = State::Unknown;
        s.attention.reason = "Waiting for fresh observations".into();
        s.attention.candidate = State::Unknown;
        s.attention.candidate_ms = 0;
        s.report.event(
            now,
            "reward_break_ended",
            &format!(
                "Reward break ended. {} min credit left.",
                s.reward.credit_ms / 60_000,
            ),
        );
    }
}

/// A session ending, completing, or being interrupted ends any break
/// and discards unspent credit (and partial earn progress). No break
/// may outlive the session that earned it.
pub fn clear(s: &mut Session) {
    s.reward = RewardState::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::FocusConfig;
    use crate::models::Status;

    fn enabled_config() -> RewardConfig {
        RewardConfig {
            enabled: true,
            earn_focus_minutes: 25,
            earn_break_minutes: 5,
            max_credit_minutes: 30,
            max_break_minutes: 30,
            delay_minutes: 0,
        }
    }

    fn session_with(config: RewardConfig, duration_minutes: u32) -> Session {
        let focus = FocusConfig {
            reward: config,
            ..Default::default()
        };
        Session {
            id: "test".into(),
            title: "Test".into(),
            duration_minutes,
            remaining_ms: u64::from(duration_minutes) * 60_000,
            focused_ms: 0,
            allow_pause: true,
            status: Status::Running,
            started_at: Some(0),
            ended_at: None,
            deadline_ms: None,
            focus,
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

    #[test]
    fn credit_earned_only_from_productive_focus_time() {
        let mut s = session_with(enabled_config(), 120);
        // Distracted or idle ticks never reach accrue; only the
        // productive ms passed in here count.
        accrue(&mut s, 0);
        assert_eq!(s.reward.credit_ms, 0);
        accrue(&mut s, 10 * 60_000);
        // 10 of 25 focus minutes: progress kept, no credit yet.
        assert_eq!(s.reward.credit_ms, 0);
        assert_eq!(s.reward.earn_progress_ms, 10 * 60_000);
        accrue(&mut s, 15 * 60_000);
        assert_eq!(s.reward.credit_ms, 5 * 60_000);
        assert_eq!(s.reward.earn_progress_ms, 0);
        // A second full earn unit banks another 5 minutes.
        accrue(&mut s, 25 * 60_000);
        assert_eq!(s.reward.credit_ms, 10 * 60_000);
    }

    #[test]
    fn disabled_config_earns_nothing() {
        let mut s = session_with(RewardConfig::default(), 120);
        accrue(&mut s, 100 * 60_000);
        assert_eq!(s.reward.credit_ms, 0);
    }

    #[test]
    fn credit_capped_at_configured_max_and_session_target() {
        // Configured max (30) binds before the 120-minute target.
        let mut s = session_with(enabled_config(), 120);
        accrue(&mut s, 25 * 60_000 * 10);
        assert_eq!(s.reward.credit_ms, 30 * 60_000);
        // A short session caps below the configured max: a 20-minute
        // session can never bank more than 20 minutes of break.
        let mut short = session_with(enabled_config(), 20);
        accrue(&mut short, 25 * 60_000 * 10);
        assert_eq!(short.reward.credit_ms, 20 * 60_000);
    }

    #[test]
    fn break_spends_credit_minute_for_minute_and_ends_when_credit_runs_out() {
        let mut s = session_with(enabled_config(), 120);
        s.reward.credit_ms = 5 * 60_000;
        request_break(&mut s, 1).unwrap();
        assert!(on_break(&s));
        assert_eq!(s.reward.break_remaining_ms, 5 * 60_000);
        advance_break(&mut s, 2 * 60_000, 2);
        assert!(on_break(&s));
        assert_eq!(s.reward.break_remaining_ms, 3 * 60_000);
        assert_eq!(s.reward.credit_ms, 3 * 60_000);
        // Overshoot ends the break rather than extending it, and
        // credit never goes negative.
        advance_break(&mut s, 60 * 60_000, 3);
        assert!(!on_break(&s));
        assert_eq!(s.reward.credit_ms, 0);
    }

    #[test]
    fn break_ends_at_max_length_with_credit_left_over() {
        let mut cfg = enabled_config();
        cfg.max_break_minutes = 3;
        let mut s = session_with(cfg, 120);
        s.reward.credit_ms = 10 * 60_000;
        request_break(&mut s, 1).unwrap();
        assert_eq!(s.reward.break_remaining_ms, 3 * 60_000);
        advance_break(&mut s, 3 * 60_000, 2);
        assert!(!on_break(&s));
        assert_eq!(s.reward.credit_ms, 7 * 60_000);
    }

    #[test]
    fn delay_counts_down_then_break_starts_automatically() {
        let mut cfg = enabled_config();
        cfg.delay_minutes = 5;
        let mut s = session_with(cfg, 120);
        s.reward.credit_ms = 5 * 60_000;
        request_break(&mut s, 1).unwrap();
        assert!(pending(&s));
        assert!(!on_break(&s));
        advance_delay(&mut s, 2 * 60_000, 2);
        assert!(pending(&s));
        assert_eq!(s.reward.pending_delay_ms, 3 * 60_000);
        // No credit spent while waiting.
        assert_eq!(s.reward.credit_ms, 5 * 60_000);
        advance_delay(&mut s, 3 * 60_000, 3);
        assert!(!pending(&s));
        assert!(on_break(&s));
    }

    #[test]
    fn delay_cancel_returns_to_focus_with_no_penalty() {
        let mut cfg = enabled_config();
        cfg.delay_minutes = 5;
        let mut s = session_with(cfg, 120);
        s.reward.credit_ms = 5 * 60_000;
        request_break(&mut s, 1).unwrap();
        advance_delay(&mut s, 60_000, 2);
        cancel_pending(&mut s, 3).unwrap();
        assert!(!pending(&s));
        assert!(!on_break(&s));
        assert_eq!(s.reward.credit_ms, 5 * 60_000);
        assert!(cancel_pending(&mut s, 4).is_err());
    }

    #[test]
    fn request_rejected_without_credit_or_while_on_a_break() {
        let mut s = session_with(enabled_config(), 120);
        assert!(request_break(&mut s, 1).is_err());
        s.reward.credit_ms = 5 * 60_000;
        request_break(&mut s, 1).unwrap();
        assert!(request_break(&mut s, 2).is_err());
        // A second request while one is pending is also rejected.
        let mut cfg = enabled_config();
        cfg.delay_minutes = 5;
        let mut d = session_with(cfg, 120);
        d.reward.credit_ms = 5 * 60_000;
        request_break(&mut d, 1).unwrap();
        assert!(request_break(&mut d, 2).is_err());
        // Paused sessions cannot take a break: no ticks would count
        // the delay down.
        let mut p = session_with(enabled_config(), 120);
        p.status = Status::Paused;
        p.reward.credit_ms = 5 * 60_000;
        assert!(request_break(&mut p, 1).is_err());
    }

    #[test]
    fn no_stacking_with_pomodoro_breaks() {
        let mut s = session_with(enabled_config(), 120);
        s.focus.pomodoro = crate::pomodoro::PomodoroConfig {
            enabled: true,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            cycles_before_long_break: 4,
        };
        s.pomodoro = crate::pomodoro::initial_state(&s.focus.pomodoro);
        s.reward.credit_ms = 5 * 60_000;
        // A reward break during Pomodoro Focus is allowed and freezes
        // the phase (the phase machine is never advanced while the
        // reward break runs — that is the engine's branch, asserted
        // here by the phase staying put through the break).
        request_break(&mut s, 1).unwrap();
        assert!(on_break(&s));
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::Focus);
        advance_break(&mut s, 5 * 60_000, 2);
        assert!(!on_break(&s));
        // But a reward break during a Pomodoro break is rejected.
        crate::pomodoro::advance_focus(&mut s, 25 * 60_000, 3);
        assert!(crate::pomodoro::in_break(&s));
        s.reward.credit_ms = 5 * 60_000;
        assert!(request_break(&mut s, 4).is_err());
        // And a Pomodoro break starting cancels a pending delayed
        // break instead of stacking on it.
        let mut cfg = enabled_config();
        cfg.delay_minutes = 5;
        let mut d = session_with(cfg, 120);
        d.focus.pomodoro = crate::pomodoro::PomodoroConfig {
            enabled: true,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            cycles_before_long_break: 4,
        };
        d.pomodoro = crate::pomodoro::initial_state(&d.focus.pomodoro);
        d.reward.credit_ms = 5 * 60_000;
        request_break(&mut d, 1).unwrap();
        assert!(pending(&d));
        crate::pomodoro::advance_focus(&mut d, 25 * 60_000, 2);
        assert!(crate::pomodoro::in_break(&d));
        assert!(!pending(&d));
        assert_eq!(d.reward.credit_ms, 5 * 60_000);
    }

    #[test]
    fn reward_break_time_never_consumes_allowances() {
        // The engine passes one unified break flag to allowance
        // accrual (step 8's `accrue_for_tick` already skips break
        // ticks); this proves a reward break actually raises it.
        let mut s = session_with(enabled_config(), 120);
        s.reward.credit_ms = 5 * 60_000;
        request_break(&mut s, 1).unwrap();
        assert!(on_any_break(&s));
        let mut list = vec![crate::allowances::Allowance {
            id: "a".into(),
            title: "Video".into(),
            kind: crate::allowances::TargetKind::Site,
            target: "youtube.com".into(),
            budget_minutes: 60,
            period: crate::allowances::Period::Day,
            mode: crate::allowances::ResetMode::Fixed,
            enabled: true,
            usage: vec![],
            created_at: 0,
        }];
        let usage = crate::allowances::TickUsage {
            sample_id: "com.google.Chrome",
            sample_idle_ms: 0,
            sample_available: true,
            browser_domain: "youtube.com",
            browser_focused: true,
            browser_activity_fresh: true,
            browser_client_id: "com.google.Chrome",
            session_browser_app: "com.google.Chrome",
        };
        assert!(!crate::allowances::accrue_for_tick(
            &mut list,
            1_800_000_000_000,
            1000,
            on_any_break(&s),
            &usage,
        ));
        assert_eq!(crate::allowances::used_ms(&list[0], 1_800_000_000_000), 0);
    }

    #[test]
    fn session_end_discards_unspent_credit() {
        let mut s = session_with(enabled_config(), 120);
        s.reward.credit_ms = 5 * 60_000;
        s.reward.earn_progress_ms = 10 * 60_000;
        clear(&mut s);
        assert_eq!(s.reward.credit_ms, 0);
        assert_eq!(s.reward.earn_progress_ms, 0);
    }

    #[test]
    fn config_validation_bounds() {
        let mut c = enabled_config();
        assert!(c.validate().is_ok());
        c.earn_focus_minutes = 0;
        assert!(c.validate().is_err());
        c.earn_focus_minutes = 25;
        c.earn_break_minutes = 61;
        assert!(c.validate().is_err());
        c.earn_break_minutes = 5;
        c.max_credit_minutes = 121;
        assert!(c.validate().is_err());
        c.max_credit_minutes = 30;
        c.max_break_minutes = 0;
        assert!(c.validate().is_err());
        c.max_break_minutes = 30;
        c.delay_minutes = 61;
        assert!(c.validate().is_err());
        c.delay_minutes = 0;
        assert!(c.validate().is_ok());
        c.enabled = false;
        c.delay_minutes = 61;
        assert!(c.validate().is_ok());
    }
}
