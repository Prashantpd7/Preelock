//! Pomodoro focus/break cycles inside a session (step 7).
//!
//! Model: a Pomodoro session alternates Focus phases with break
//! phases. The focus phase is measured in *productive* time, not wall
//! time: its remaining time decreases only by the productive time the
//! attention accounting (or, for sessions without attention, the plain
//! timer) actually credits. Break phases are measured in tick time
//! while the session is Running; pausing pauses the break too, because
//! a Paused session gets no ticks at all. Breaks always end on their
//! own when their time expires, so a session can never strand itself
//! in a break: every phase transition either consumes time or moves
//! to a phase with a positive length (validated at config time), and
//! the session still completes as soon as its productive target is
//! reached, whatever phase it is in.
//!
//! Blocking follows the phase: during Focus the existing app and
//! website blocking applies unchanged; during a break the engine
//! publishes no network policy (the extension already removes its
//! rules when the policy goes away, so no extension change is needed)
//! and the native sample is never treated as blocked. The policy is
//! republished on every tick, so the transition back to Focus restores
//! website blocking on the next tick without any special refresh path.
use serde::{Deserialize, Serialize};

use crate::{attention::State, models::Session};

/// Pomodoro configuration, stored on [`crate::focus::FocusConfig`].
/// Serde-default: sessions saved before this step load as disabled.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PomodoroConfig {
    pub enabled: bool,
    pub focus_minutes: u32,
    pub short_break_minutes: u32,
    pub long_break_minutes: u32,
    pub cycles_before_long_break: u32,
}
impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            cycles_before_long_break: 4,
        }
    }
}
impl PomodoroConfig {
    pub fn focus_ms(&self) -> u64 {
        u64::from(self.focus_minutes) * 60_000
    }
    pub fn short_break_ms(&self) -> u64 {
        u64::from(self.short_break_minutes) * 60_000
    }
    pub fn long_break_ms(&self) -> u64 {
        u64::from(self.long_break_minutes) * 60_000
    }
    /// Range check, applied by `focus::validate` (which covers manual
    /// sessions, schedule snapshots and one-shot snapshots alike).
    /// Disabled configs are never rejected: their values are ignored.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if !(1..=180).contains(&self.focus_minutes) {
            return Err("Focus length must be 1-180 minutes.".into());
        }
        if !(1..=60).contains(&self.short_break_minutes) {
            return Err("Short break must be 1-60 minutes.".into());
        }
        if !(1..=90).contains(&self.long_break_minutes) {
            return Err("Long break must be 1-90 minutes.".into());
        }
        if !(2..=8).contains(&self.cycles_before_long_break) {
            return Err("Focus cycles before a long break must be 2-8.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Focus,
    ShortBreak,
    LongBreak,
}

/// Live phase state, stored on the session and persisted with it, so
/// a restart resumes the phase it was in (with no offline credit).
/// Serde-default: sessions saved before this step load as Focus with
/// no time left, which `ensure_remaining` heals to a full phase.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PomodoroState {
    pub phase: Phase,
    pub phase_remaining_ms: u64,
    pub completed_focus_cycles: u32,
}

/// A fresh run always starts in Focus with a full focus phase.
pub fn initial_state(config: &PomodoroConfig) -> PomodoroState {
    if config.enabled {
        PomodoroState {
            phase: Phase::Focus,
            phase_remaining_ms: config.focus_ms(),
            completed_focus_cycles: 0,
        }
    } else {
        PomodoroState::default()
    }
}

/// True when this session is on a break right now (blocking relaxed).
pub fn in_break(s: &Session) -> bool {
    s.focus.pomodoro.enabled && s.pomodoro.phase != Phase::Focus
}

fn phase_length_ms(config: &PomodoroConfig, phase: &Phase) -> u64 {
    match phase {
        Phase::Focus => config.focus_ms(),
        Phase::ShortBreak => config.short_break_ms(),
        Phase::LongBreak => config.long_break_ms(),
    }
}

/// A state with no time left in its phase (loaded from an older save,
/// or a zeroed default) restarts that phase at full length instead of
/// spinning a transition. Phase lengths are validated positive, so
/// this always makes progress.
fn ensure_remaining(s: &mut Session) {
    if s.focus.pomodoro.enabled && s.pomodoro.phase_remaining_ms == 0 {
        s.pomodoro.phase_remaining_ms = phase_length_ms(&s.focus.pomodoro, &s.pomodoro.phase);
    }
}

/// A focus phase just ended: count the cycle and open the break it
/// earned — a long break every `cycles_before_long_break` cycles,
/// otherwise a short break.
pub fn complete_focus_phase(s: &mut Session, now: i64) {
    if !s.focus.pomodoro.enabled {
        return;
    }
    // Step 9: a Pomodoro break and a reward break never stack. A
    // delayed break pending when this break starts is cancelled
    // (no penalty, credit untouched) rather than queued behind it.
    crate::rewards::cancel_pending_for_pomodoro_break(s, now);
    s.pomodoro.completed_focus_cycles = s.pomodoro.completed_focus_cycles.saturating_add(1);
    let long = s
        .pomodoro
        .completed_focus_cycles
        .is_multiple_of(s.focus.pomodoro.cycles_before_long_break);
    s.pomodoro.phase = if long {
        Phase::LongBreak
    } else {
        Phase::ShortBreak
    };
    s.pomodoro.phase_remaining_ms = phase_length_ms(&s.focus.pomodoro, &s.pomodoro.phase);
    s.attention.state = State::Idle;
    s.attention.reason = "On break: productive time is paused.".into();
    s.attention.candidate_ms = 0;
    s.report.event(
        now,
        "pomodoro_break",
        &format!(
            "{} break started ({} min). Blocking is relaxed until the break ends.",
            if long { "Long" } else { "Short" },
            if long {
                s.focus.pomodoro.long_break_minutes
            } else {
                s.focus.pomodoro.short_break_minutes
            },
        ),
    );
}

/// A break just ended: back to Focus with a full focus phase, and the
/// attention classifier restarts from fresh observations.
pub fn start_focus_phase(s: &mut Session, now: i64) {
    if !s.focus.pomodoro.enabled {
        return;
    }
    s.pomodoro.phase = Phase::Focus;
    s.pomodoro.phase_remaining_ms = s.focus.pomodoro.focus_ms();
    s.attention.state = State::Unknown;
    s.attention.reason = "Waiting for fresh observations".into();
    s.attention.candidate = State::Unknown;
    s.attention.candidate_ms = 0;
    s.report
        .event(now, "pomodoro_focus", "Focus phase started.");
}

/// Productive time credited during a Focus phase (attention tick
/// path). Break time never reaches this function: the tick runs
/// `advance_break` instead, so break time can never count toward the
/// session target. Productive overflow past the phase end is dropped
/// from phase accounting only — it was already credited to the
/// session target by the caller — and the next focus phase starts
/// fresh.
pub fn advance_focus(s: &mut Session, productive_ms: u64, now: i64) {
    if !s.focus.pomodoro.enabled || s.pomodoro.phase != Phase::Focus {
        return;
    }
    ensure_remaining(s);
    let used = productive_ms.min(s.pomodoro.phase_remaining_ms);
    s.pomodoro.phase_remaining_ms -= used;
    if s.pomodoro.phase_remaining_ms == 0 {
        complete_focus_phase(s, now);
    }
}

/// Tick time passing during a break phase. The break ends on its own
/// when its time expires; any overshoot is dropped (the focus phase
/// starts now, it does not inherit break overflow as productive time).
pub fn advance_break(s: &mut Session, elapsed_ms: u64, now: i64) {
    if !in_break(s) {
        return;
    }
    ensure_remaining(s);
    if elapsed_ms >= s.pomodoro.phase_remaining_ms {
        start_focus_phase(s, now);
    } else {
        s.pomodoro.phase_remaining_ms -= elapsed_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::FocusConfig;
    use crate::models::Status;

    fn enabled_config() -> PomodoroConfig {
        PomodoroConfig {
            enabled: true,
            focus_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            cycles_before_long_break: 4,
        }
    }

    fn session_with(config: PomodoroConfig) -> Session {
        let focus = FocusConfig {
            pomodoro: config,
            ..Default::default()
        };
        Session {
            id: "test".into(),
            title: "Test".into(),
            duration_minutes: 100,
            remaining_ms: 100 * 60_000,
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
            pomodoro: PomodoroState::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
        }
    }

    #[test]
    fn disabled_config_never_changes_phase() {
        let mut s = session_with(PomodoroConfig::default());
        s.pomodoro = initial_state(&s.focus.pomodoro);
        advance_focus(&mut s, 25 * 60_000, 1);
        advance_break(&mut s, 5 * 60_000, 2);
        assert_eq!(s.pomodoro.phase, Phase::Focus);
        assert!(!in_break(&s));
    }

    #[test]
    fn focus_phase_ends_on_productive_time_only() {
        let mut s = session_with(enabled_config());
        s.pomodoro = initial_state(&s.focus.pomodoro);
        // Distracted or idle ticks credit no productive time, so the
        // focus phase does not move.
        advance_focus(&mut s, 0, 1);
        assert_eq!(s.pomodoro.phase, Phase::Focus);
        assert_eq!(s.pomodoro.phase_remaining_ms, 25 * 60_000);
        advance_focus(&mut s, 10 * 60_000, 2);
        assert_eq!(s.pomodoro.phase_remaining_ms, 15 * 60_000);
        advance_focus(&mut s, 15 * 60_000, 3);
        assert_eq!(s.pomodoro.phase, Phase::ShortBreak);
        assert_eq!(s.pomodoro.phase_remaining_ms, 5 * 60_000);
        assert_eq!(s.pomodoro.completed_focus_cycles, 1);
        assert!(in_break(&s));
    }

    #[test]
    fn long_break_follows_configured_cycle_count() {
        let mut s = session_with(enabled_config());
        s.pomodoro = initial_state(&s.focus.pomodoro);
        for cycle in 1..=4 {
            assert_eq!(s.pomodoro.phase, Phase::Focus);
            advance_focus(&mut s, 25 * 60_000, cycle);
            if cycle < 4 {
                assert_eq!(s.pomodoro.phase, Phase::ShortBreak);
                advance_break(&mut s, 5 * 60_000, cycle);
            } else {
                assert_eq!(s.pomodoro.phase, Phase::LongBreak);
                assert_eq!(s.pomodoro.phase_remaining_ms, 15 * 60_000);
            }
        }
        assert_eq!(s.pomodoro.completed_focus_cycles, 4);
        // After the long break the cycle count keeps going: the next
        // focus phase earns a short break again (5 % 4 != 0).
        advance_break(&mut s, 15 * 60_000, 5);
        assert_eq!(s.pomodoro.phase, Phase::Focus);
        advance_focus(&mut s, 25 * 60_000, 6);
        assert_eq!(s.pomodoro.phase, Phase::ShortBreak);
    }

    #[test]
    fn break_ends_automatically_and_never_counts_as_productive() {
        let mut s = session_with(enabled_config());
        s.pomodoro = initial_state(&s.focus.pomodoro);
        advance_focus(&mut s, 25 * 60_000, 1);
        assert!(in_break(&s));
        // A partial break stays a break.
        advance_break(&mut s, 2 * 60_000, 2);
        assert_eq!(s.pomodoro.phase, Phase::ShortBreak);
        assert_eq!(s.pomodoro.phase_remaining_ms, 3 * 60_000);
        // Productive-time calls during a break are ignored.
        advance_focus(&mut s, 60_000, 3);
        assert_eq!(s.pomodoro.phase, Phase::ShortBreak);
        // The break ends on its own, even with overshoot (a wake tick
        // with a huge delta ends it rather than extending it).
        advance_break(&mut s, 60 * 60_000, 4);
        assert_eq!(s.pomodoro.phase, Phase::Focus);
        assert_eq!(s.pomodoro.phase_remaining_ms, 25 * 60_000);
    }

    #[test]
    fn zeroed_state_heals_to_a_full_phase() {
        let mut s = session_with(enabled_config());
        // As loaded from a pre-step save: enabled config, Focus phase,
        // no time left. The next advance must not spin or complete.
        assert_eq!(s.pomodoro.phase_remaining_ms, 0);
        advance_focus(&mut s, 60_000, 1);
        assert_eq!(s.pomodoro.phase_remaining_ms, 25 * 60_000 - 60_000);
    }

    #[test]
    fn config_validation_bounds() {
        let mut c = enabled_config();
        assert!(c.validate().is_ok());
        c.focus_minutes = 0;
        assert!(c.validate().is_err());
        c.focus_minutes = 25;
        c.short_break_minutes = 61;
        assert!(c.validate().is_err());
        c.short_break_minutes = 5;
        c.long_break_minutes = 91;
        assert!(c.validate().is_err());
        c.long_break_minutes = 15;
        c.cycles_before_long_break = 1;
        assert!(c.validate().is_err());
        c.cycles_before_long_break = 9;
        assert!(c.validate().is_err());
        c.enabled = false;
        assert!(c.validate().is_ok());
    }
}
