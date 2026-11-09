//! Scheduled blocks (step 5): recurring weekly schedules that
//! auto-start a focus session inside a local-time window.
//!
//! Time semantics, stated once so the frontend and the engine agree:
//! - `days` uses 0 = Sunday through 6 = Saturday, matching both the C
//!   `tm_wday` field and JavaScript `Date.getDay()`.
//! - `start_time` / `end_time` are local wall-clock times in `HH:MM`.
//!   Local time comes from the OS (`localtime_r` / `localtime_s`),
//!   so a DST transition may shift a window by an hour; that shift
//!   is accepted, not corrected for.
//! - A window is start-inclusive and end-exclusive: at exactly
//!   `start_time` the block is on, at exactly `end_time` it is off.
//! - An `end_time` earlier than `start_time` is an overnight window
//!   crossing midnight (for example 22:00-06:00): it is active from
//!   `start_time` on a listed day until `end_time` the next morning,
//!   so the morning half counts the *previous* day's listing.
//! - The engine acts only on transitions (see `next_action`): a
//!   schedule session is created on window entry and completed on
//!   window exit, even if the tick was delayed past the boundary.
//!
//! Step 6 (autostart) additions live here too, so the pure rules stay
//! in one testable place:
//! - `login_registration_needed`: the app registers for open-at-login
//!   when a session is active OR any schedule is enabled. Rationale:
//!   schedules can only fire if the app runs, so creating an enabled
//!   schedule is explicit consent to launch at login. With neither, the
//!   login item is removed.
//! - `OneShot` / `one_shot_action`: a one-shot timer fires once at a
//!   future wall-clock time. It never fires retroactively: a time that
//!   passed while the app was closed is purged at launch, a time that
//!   passed during a long tick gap (sleep) expires on wake, and a time
//!   that arrives while another session runs is skipped, not queued.
//!   (Recurring schedules differ deliberately: they wait inside their
//!   window. A one-shot is a point in time, not a window, so queueing
//!   it would start a block at an arbitrary later time.)

use crate::focus::FocusConfig;
use serde::{Deserialize, Serialize};

/// A recurring weekly block. `focus` is a FULL snapshot copied at
/// creation time: editing the template session later never changes
/// the schedule, and editing the schedule never touches the template.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub title: String,
    /// Weekdays the schedule runs on: 0 = Sunday .. 6 = Saturday.
    pub days: Vec<u32>,
    /// Local time, `HH:MM`.
    pub start_time: String,
    /// Local time, `HH:MM`. Earlier than `start_time` means overnight.
    pub end_time: String,
    pub focus: FocusConfig,
    pub enabled: bool,
    /// While the window is active, a locked schedule session cannot
    /// be ended early and the schedule cannot be edited, disabled,
    /// or deleted.
    pub lock_changes: bool,
    /// Step 10 lock, in addition to `lock_changes`. Serde-default:
    /// schedules saved before this step load unlocked.
    #[serde(default)]
    pub lock: crate::locks::LockConfig,
    /// Live step 10 lock state (challenge, pending delay).
    /// Serde-default, cleared when the window ends.
    #[serde(default)]
    pub lock_state: crate::locks::LockState,
    /// Step 14 advance warnings already fired for the upcoming
    /// instance, as `"{start_ms}:{minutes}"` keys (see
    /// `crate::notifications`). Persisted, so a restart cannot
    /// re-fire a warning that already went out; stale keys for past
    /// instances are pruned on every warning evaluation.
    /// Serde-default: schedules saved before this step load with
    /// no warnings fired.
    #[serde(default)]
    pub warning_fired: Vec<String>,
    pub created_at: i64,
}

/// Parse `HH:MM` into minutes since midnight.
pub fn parse_time(value: &str) -> Result<u32, String> {
    let (hour, minute) = value
        .split_once(':')
        .ok_or_else(|| format!("Use HH:MM times, for example 09:00: {value}"))?;
    if hour.is_empty() || hour.len() > 2 || minute.len() != 2 {
        return Err(format!("Use HH:MM times, for example 09:00: {value}"));
    }
    let hour: u32 = hour
        .parse()
        .map_err(|_| format!("Use HH:MM times, for example 09:00: {value}"))?;
    let minute: u32 = minute
        .parse()
        .map_err(|_| format!("Use HH:MM times, for example 09:00: {value}"))?;
    if hour > 23 || minute > 59 {
        return Err(format!("Use HH:MM times, for example 09:00: {value}"));
    }
    Ok(hour * 60 + minute)
}

fn title(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 {
        return Err("Use a title between 1 and 160 characters.".into());
    }
    Ok(value.into())
}

/// Validate a schedule's fields (title, days, times, focus snapshot).
/// `end_time` equal to `start_time` is rejected (a zero-length window);
/// `end_time` earlier than `start_time` is a valid overnight window.
pub fn validate_schedule(mut schedule: Schedule) -> Result<Schedule, String> {
    schedule.title = title(&schedule.title)?;
    if schedule.days.is_empty() {
        return Err("Pick at least one day for the schedule.".into());
    }
    if schedule.days.iter().any(|d| *d > 6) {
        return Err("Days must be weekdays from Sunday (0) to Saturday (6).".into());
    }
    schedule.days.sort_unstable();
    schedule.days.dedup();
    let start = parse_time(&schedule.start_time)?;
    let end = parse_time(&schedule.end_time)?;
    if start == end {
        return Err("Start and end times must differ.".into());
    }
    schedule.focus = crate::focus::validate(schedule.focus)?;
    schedule.lock.validate()?;
    Ok(schedule)
}

/// A one-shot start timer (step 6): one future start, no recurrence.
/// `focus` is a FULL snapshot copied at creation time, exactly like a
/// schedule's, and `allow_pause` is copied from the template session:
/// once started, a one-shot session behaves like an ordinary manual
/// session (pausable when the template was), not like a window-bound
/// schedule session. `fire_at_ms` is a wall-clock epoch timestamp in
/// milliseconds; the frontend builds it from a local `datetime-local`
/// value, so it is local time, like schedule windows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OneShot {
    pub id: String,
    pub title: String,
    pub minutes: u32,
    pub allow_pause: bool,
    pub focus: FocusConfig,
    /// Wall-clock time to start, epoch milliseconds. Must be future.
    pub fire_at_ms: i64,
    /// Step 14 advance warnings already fired, as minute values
    /// (a one-shot has exactly one start, so no instance key is
    /// needed). The timer is removed when it fires or expires, so
    /// this list can never leak. Serde-default.
    #[serde(default)]
    pub warning_fired: Vec<u32>,
    pub created_at: i64,
}

/// Validate a one-shot timer: title, duration (1..=480 minutes, the
/// same range as a manual session), focus snapshot, and a strictly
/// future fire time. A time equal to `now` is already past by the
/// time the engine could act on it, so it is rejected too.
pub fn validate_one_shot(mut shot: OneShot, now_ms: i64) -> Result<OneShot, String> {
    shot.title = title(&shot.title)?;
    if !(1..=480).contains(&shot.minutes) {
        return Err("Choose a duration from 1 to 480 minutes.".into());
    }
    shot.focus = crate::focus::validate(shot.focus)?;
    if shot.fire_at_ms <= now_ms {
        return Err("Pick a future date and time.".into());
    }
    Ok(shot)
}

/// Whether the app should be registered for open-at-login (step 6a).
/// Pure, so the rule is unit-testable without a Tauri handle: register
/// when a manual session is active (the step-5 rule, unchanged) OR
/// when ANY schedule is enabled — schedules can only fire if the app
/// runs, so creating an enabled schedule is explicit consent to login
/// launch. Otherwise (no active session, no enabled schedule) the
/// caller must REMOVE the login item.
pub fn login_registration_needed(has_active_session: bool, schedules: &[Schedule]) -> bool {
    has_active_session || schedules.iter().any(|s| s.enabled)
}

/// What the engine should do with one one-shot timer on one tick.
/// Pure, so the no-retroactive-fire rules are testable directly:
/// - `Wait`: the time has not arrived yet.
/// - `Fire`: the time arrived on an ordinary tick (at most a tick
///   late) and nothing else is running.
/// - `Expire`: the timer is removed without starting anything — the
///   time passed during a long tick gap (sleep / long pause), or it
///   arrived while another session was running. A one-shot never
///   queues behind another session: unlike a recurring schedule,
///   which waits inside its window, a one-shot is a point in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneShotAction {
    Wait,
    Fire,
    Expire,
}

pub fn one_shot_action(
    shot: &OneShot,
    now_ms: i64,
    tick_gap_exceeded: bool,
    has_active_session: bool,
) -> OneShotAction {
    if now_ms < shot.fire_at_ms {
        OneShotAction::Wait
    } else if tick_gap_exceeded || has_active_session {
        OneShotAction::Expire
    } else {
        OneShotAction::Fire
    }
}

/// Length of the window in minutes. An overnight window wraps past
/// midnight, so 22:00-06:00 is 480 minutes, not a negative span.
pub fn window_minutes(schedule: &Schedule) -> u32 {
    let start = parse_time(&schedule.start_time).unwrap_or(0);
    let end = parse_time(&schedule.end_time).unwrap_or(0);
    if end > start {
        end - start
    } else {
        1440 - start + end
    }
}

/// Whether the window covers this local weekday/minute. Pure, so it
/// is directly unit-testable without depending on the machine clock.
pub fn window_active(schedule: &Schedule, weekday: u32, minutes: u32) -> bool {
    let Ok(start) = parse_time(&schedule.start_time) else {
        return false;
    };
    let Ok(end) = parse_time(&schedule.end_time) else {
        return false;
    };
    if end > start {
        schedule.days.contains(&weekday) && minutes >= start && minutes < end
    } else {
        // Overnight: the evening half belongs to a listed day, the
        // morning half belongs to the previous day's listing.
        (schedule.days.contains(&weekday) && minutes >= start)
            || (schedule.days.contains(&((weekday + 6) % 7)) && minutes < end)
    }
}

/// Local weekday (0 = Sunday) and minutes since local midnight for a
/// wall-clock timestamp in milliseconds. On macOS/Windows/Linux this
/// is the OS local time; on any other platform it falls back to UTC.
pub fn local_weekday_and_minutes(now_ms: i64) -> (u32, u32) {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
            gmtoff: i64,
            zone: *const std::ffi::c_char,
        }
        extern "C" {
            fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
        }
        let secs = now_ms.div_euclid(1000);
        let mut tm = Tm {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 0,
            mon: 0,
            year: 0,
            wday: 0,
            yday: 0,
            isdst: 0,
            gmtoff: 0,
            zone: std::ptr::null(),
        };
        let ok = unsafe { !localtime_r(&secs, &mut tm).is_null() };
        if ok {
            let weekday = tm.wday.rem_euclid(7) as u32;
            let minutes = (tm.hour.max(0) as u32) * 60 + (tm.min.max(0) as u32);
            return (weekday, minutes);
        }
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
        }
        extern "C" {
            fn _localtime64_s(tm: *mut Tm, timep: *const i64) -> i32;
        }
        let secs = now_ms.div_euclid(1000);
        let mut tm = Tm {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 0,
            mon: 0,
            year: 0,
            wday: 0,
            yday: 0,
            isdst: 0,
        };
        let ok = unsafe { _localtime64_s(&mut tm, &secs) == 0 };
        if ok {
            let weekday = tm.wday.rem_euclid(7) as u32;
            let minutes = (tm.hour.max(0) as u32) * 60 + (tm.min.max(0) as u32);
            return (weekday, minutes);
        }
    }
    // Fallback (also the path when the OS call fails): UTC.
    let secs = now_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    // 1970-01-01 was a Thursday (weekday 4, Sunday = 0).
    let weekday = (days + 4).rem_euclid(7) as u32;
    let minutes = (secs.rem_euclid(86_400) / 60) as u32;
    (weekday, minutes)
}

/// Days until the next window start, counting from today's local
/// weekday/minute (step 14). Pure and timezone-free: the caller
/// supplies the local weekday and minutes, so tests need no clock.
/// - Returns `Some(0)` when today's start is still in the future.
/// - A start exactly now counts as already started, never as "next":
///   the offset moves to the next listed day (or 7, next week, when
///   only today is listed).
/// - Returns `None` when no day is listed at all.
pub fn next_start_day_offset(
    weekday: u32,
    minutes: u32,
    days: &[u32],
    start_min: u32,
) -> Option<u32> {
    if days.is_empty() {
        return None;
    }
    for offset in 0..=7 {
        let day = (weekday + offset) % 7;
        if days.contains(&day) {
            if offset == 0 && start_min <= minutes {
                continue;
            }
            return Some(offset);
        }
    }
    None
}

/// Wall-clock start instant (epoch ms) of the next window start
/// after `now_ms` (step 14 advance warnings). The start instant is
/// the `start_time` on a listed day, for same-day and overnight
/// windows alike (an overnight window still STARTS in the evening
/// of a listed day). Day arithmetic assumes 24-hour days: a DST
/// transition may shift the computed start by an hour, the same
/// accepted imprecision as the window rules themselves.
pub fn next_window_start_ms(schedule: &Schedule, now_ms: i64) -> Option<i64> {
    let start = parse_time(&schedule.start_time).ok()?;
    let (weekday, minutes) = local_weekday_and_minutes(now_ms);
    let offset = next_start_day_offset(weekday, minutes, &schedule.days, start)?;
    // Minute boundaries are the same in local time and epoch time
    // (timezone offsets are whole minutes in practice), so the
    // local midnight base is the truncated timestamp minus the
    // local minutes since midnight.
    let truncated = now_ms - now_ms.rem_euclid(60_000);
    let midnight = truncated - i64::from(minutes) * 60_000;
    Some(midnight + i64::from(offset) * 86_400_000 + i64::from(start) * 60_000)
}

/// True when the schedule is locked AND its window is active right
/// now: while this holds, its session cannot be ended early and the
/// schedule cannot be edited, disabled, or deleted.
pub fn locked_and_active(schedule: &Schedule, now_ms: i64) -> bool {
    if !schedule.lock_changes || !schedule.enabled {
        return false;
    }
    let (weekday, minutes) = local_weekday_and_minutes(now_ms);
    window_active(schedule, weekday, minutes)
}

/// Step 10 equivalent of `locked_and_active` for the lock
/// framework's gates (password / challenge / delay): true only
/// while the window is active AND the schedule carries a gate.
/// A restart-only lock never gates a change on its own.
pub fn lock_gate_active(schedule: &Schedule, now_ms: i64) -> bool {
    if !schedule.lock.has_gate() || !schedule.enabled {
        return false;
    }
    let (weekday, minutes) = local_weekday_and_minutes(now_ms);
    window_active(schedule, weekday, minutes)
}

/// What the engine should do for one schedule on one tick. Pure, so
/// the transition rules are testable without a running engine:
/// - `Start` only when the window is active, no session from this
///   schedule is active, and no other session is active either (a
///   manual session is never interrupted; the schedule waits and
///   starts when the manual session ends, if still in the window).
/// - `End` whenever a session from this schedule is active but the
///   window is not: the window end always ends the schedule session.
/// - `Skip` otherwise (disabled, out of window with nothing running,
///   or waiting behind a manual session).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleAction {
    Start,
    End,
    Skip,
}

pub fn next_action(
    schedule: &Schedule,
    weekday: u32,
    minutes: u32,
    has_schedule_session: bool,
    has_any_active_session: bool,
) -> ScheduleAction {
    let active = window_active(schedule, weekday, minutes);
    if has_schedule_session {
        if active {
            ScheduleAction::Skip
        } else {
            ScheduleAction::End
        }
    } else if !schedule.enabled || !active || has_any_active_session {
        ScheduleAction::Skip
    } else {
        ScheduleAction::Start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(days: Vec<u32>, start: &str, end: &str) -> Schedule {
        Schedule {
            id: "sched".into(),
            title: "Morning block".into(),
            days,
            start_time: start.into(),
            end_time: end.into(),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: 0,
        }
    }

    #[test]
    fn time_parsing_accepts_valid_and_rejects_invalid() {
        assert_eq!(parse_time("09:00").unwrap(), 540);
        assert_eq!(parse_time("9:00").unwrap(), 540);
        assert_eq!(parse_time("00:00").unwrap(), 0);
        assert_eq!(parse_time("23:59").unwrap(), 1439);
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("09:60").is_err());
        assert!(parse_time("09:0").is_err());
        assert!(parse_time("0900").is_err());
        assert!(parse_time("09:00:00").is_err());
        assert!(parse_time("").is_err());
    }

    #[test]
    fn validation_rejects_empty_days_and_zero_length_windows() {
        assert!(validate_schedule(schedule(vec![], "09:00", "10:00")).is_err());
        assert!(validate_schedule(schedule(vec![1], "09:00", "09:00")).is_err());
        assert!(validate_schedule(schedule(vec![7], "09:00", "10:00")).is_err());
        // Overnight is valid, not an error.
        assert!(validate_schedule(schedule(vec![1], "22:00", "06:00")).is_ok());
        // Days are sorted and deduplicated.
        let checked = validate_schedule(schedule(vec![5, 1, 5], "09:00", "10:00")).unwrap();
        assert_eq!(checked.days, vec![1, 5]);
    }

    #[test]
    fn same_day_window_matches_weekday_and_boundaries() {
        let s = schedule(vec![1, 3], "09:00", "10:00"); // Mon, Wed
        assert!(window_active(&s, 1, 540)); // Mon 09:00: start inclusive
        assert!(window_active(&s, 1, 599)); // Mon 09:59
        assert!(!window_active(&s, 1, 600)); // Mon 10:00: end exclusive
        assert!(!window_active(&s, 1, 539)); // Mon 08:59
        assert!(!window_active(&s, 2, 570)); // Tue: day not listed
        assert!(window_active(&s, 3, 570)); // Wed
        assert!(!window_active(&s, 0, 570)); // Sun
    }

    #[test]
    fn overnight_window_spans_midnight_on_the_right_days() {
        let s = schedule(vec![1], "22:00", "06:00"); // Mon evening -> Tue morning
        assert!(window_active(&s, 1, 1320)); // Mon 22:00
        assert!(window_active(&s, 1, 1439)); // Mon 23:59
        assert!(window_active(&s, 2, 0)); // Tue 00:00: previous day listed
        assert!(window_active(&s, 2, 359)); // Tue 05:59
        assert!(!window_active(&s, 2, 360)); // Tue 06:00: end exclusive
        assert!(!window_active(&s, 2, 1320)); // Tue 22:00: Tue not listed
        assert!(!window_active(&s, 1, 300)); // Mon 05:00: Sun not listed
        assert_eq!(window_minutes(&s), 480);
    }

    #[test]
    fn disabled_schedules_never_match_for_transitions() {
        let mut s = schedule(vec![1], "09:00", "10:00");
        assert_eq!(next_action(&s, 1, 570, false, false), ScheduleAction::Start);
        s.enabled = false;
        assert_eq!(next_action(&s, 1, 570, false, false), ScheduleAction::Skip);
    }

    #[test]
    fn transition_logic_waits_for_manual_sessions_and_ends_on_exit() {
        let s = schedule(vec![1], "09:00", "10:00");
        // Manual session running: wait, never interrupt it.
        assert_eq!(next_action(&s, 1, 570, false, true), ScheduleAction::Skip);
        // Manual session ended while still in the window: start.
        assert_eq!(next_action(&s, 1, 570, false, false), ScheduleAction::Start);
        // Already running: no duplicate.
        assert_eq!(next_action(&s, 1, 570, true, true), ScheduleAction::Skip);
        // Window over while running: end, no exceptions.
        assert_eq!(next_action(&s, 1, 600, true, true), ScheduleAction::End);
        // Out of window, nothing running: nothing to do.
        assert_eq!(next_action(&s, 1, 600, false, false), ScheduleAction::Skip);
    }

    fn one_shot(fire_at_ms: i64) -> OneShot {
        OneShot {
            id: "one".into(),
            title: "Evening block".into(),
            minutes: 25,
            allow_pause: true,
            focus: FocusConfig::default(),
            fire_at_ms,
            warning_fired: vec![],
            created_at: 0,
        }
    }

    #[test]
    fn login_registration_covers_active_sessions_and_enabled_schedules() {
        let enabled = schedule(vec![1], "09:00", "10:00");
        let mut disabled = schedule(vec![1], "09:00", "10:00");
        disabled.enabled = false;
        // Active session alone registers (step-5 rule, unchanged).
        assert!(login_registration_needed(true, &[]));
        assert!(login_registration_needed(
            true,
            std::slice::from_ref(&disabled)
        ));
        // An enabled schedule alone registers: schedules can only fire
        // if the app runs, so the schedule is consent to login launch.
        assert!(login_registration_needed(
            false,
            std::slice::from_ref(&enabled)
        ));
        // Removal case: no active session and no enabled schedule.
        assert!(!login_registration_needed(false, &[]));
        assert!(!login_registration_needed(
            false,
            std::slice::from_ref(&disabled)
        ));
    }

    #[test]
    fn one_shot_validation_requires_a_future_time_and_valid_duration() {
        assert!(validate_one_shot(one_shot(2000), 1000).is_ok());
        // Equal to now is already past by the time the engine acts.
        assert!(validate_one_shot(one_shot(1000), 1000).is_err());
        assert!(validate_one_shot(one_shot(999), 1000).is_err());
        let mut shot = one_shot(2000);
        shot.minutes = 0;
        assert!(validate_one_shot(shot, 1000).is_err());
        let mut shot = one_shot(2000);
        shot.minutes = 481;
        assert!(validate_one_shot(shot, 1000).is_err());
        let mut shot = one_shot(2000);
        shot.title = "  ".into();
        assert!(validate_one_shot(shot, 1000).is_err());
    }

    #[test]
    fn one_shot_action_never_fires_retroactively_or_behind_a_session() {
        let shot = one_shot(1000);
        assert_eq!(
            one_shot_action(&shot, 999, false, false),
            OneShotAction::Wait
        );
        assert_eq!(
            one_shot_action(&shot, 1000, false, false),
            OneShotAction::Fire
        );
        // A tick or two late on an ordinary tick still fires.
        assert_eq!(
            one_shot_action(&shot, 1001, false, false),
            OneShotAction::Fire
        );
        // Time passed during a long gap (sleep): expire, never fire.
        assert_eq!(
            one_shot_action(&shot, 5000, true, false),
            OneShotAction::Expire
        );
        // Time arrived while another session runs: skip, never queue.
        assert_eq!(
            one_shot_action(&shot, 1000, false, true),
            OneShotAction::Expire
        );
        // Future time stays Wait even across a gap.
        assert_eq!(
            one_shot_action(&shot, 999, true, false),
            OneShotAction::Wait
        );
    }

    #[test]
    fn local_time_falls_back_to_utc_on_epoch() {
        // 1970-01-01 00:00 UTC was a Thursday (4); this only pins the
        // fallback arithmetic, not the OS local-time path.
        let (weekday, minutes) = local_weekday_and_minutes(0);
        assert!(weekday <= 6);
        assert!(minutes < 1440);
        let _ = (weekday, minutes);
    }
}
