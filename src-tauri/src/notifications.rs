//! Step 14 Notifications: advance warnings before an auto-starting
//! block, plus a grace period after one starts. Both are local-only
//! and reuse the existing notification path
//! (`crate::desktop_settings::notify`, macOS native / Windows plugin
//! / no-op elsewhere) — there is no second notification system.
//!
//! WARNINGS
//! - Only scheduled blocks (step 5) and one-shot timers (step 6)
//!   get warnings: they start on their own. Manual sessions never
//!   get one — the user just started them.
//! - The warning points come from one global preference,
//!   `Preferences::warning_minutes`: a comma-separated list such as
//!   `"5, 15"` (default `"5"`), parsed by
//!   `parse_warning_minutes` into deduped, sorted minutes in 1..=60.
//!   An empty list means no warnings. There is deliberately no
//!   per-schedule override: the schedule wizard has no clean slot
//!   for one, so warnings are global-only.
//! - A warning fires when its warning time has arrived but the
//!   block has not started yet (`warning_time <= now < start`).
//!   The crossing form (not exact-equality) is what makes warnings
//!   restart-safe: a tick that runs late — after sleep, or on the
//!   first tick after relaunch — still fires a warning whose time
//!   passed while the app was down, as long as the block itself has
//!   not started. A warning NEVER fires for a block that already
//!   started, already ended, or was disabled/deleted before its
//!   time: started blocks are gone from the evaluation (their
//!   schedule session exists, their one-shot is removed), disabled
//!   schedules are skipped, and deleted items simply do not exist.
//! - Once-per-instance tracking is persisted, not in-memory, so a
//!   restart cannot re-fire a warning that already went out:
//!   schedules carry `warning_fired` keys `"{start_ms}:{minutes}"`
//!   for the upcoming instance only (stale keys for past instances
//!   are pruned on every evaluation), and one-shots carry the fired
//!   minute values directly (a one-shot has exactly one start, so
//!   no instance key is needed; the timer is removed when it fires
//!   or expires, so its fired list can never leak).
//! - Warnings respect the master `notifications_enabled` toggle:
//!   with it OFF, `Engine::evaluate_warnings` skips evaluation
//!   entirely — it does not even mark points fired, so turning
//!   notifications back on before the block starts still warns.
//! - Warning text states the block title and the TRUE minutes
//!   remaining (`ceil((start - now) / 60_000)`), never the nominal
//!   point: a 15-minute warning that fires late (restart 3 minutes
//!   before the block) says "starts in 3 minutes", not 15. If
//!   several points are due on the same late tick, each fires once
//!   with that same true remaining time.
//! - One honest edge, kept, not hidden: a one-shot warning may
//!   precede a one-shot that then expires because another session
//!   was still running at its fire time (the step-6 never-queues
//!   rule). The warning fired while the timer was live; the timer
//!   itself, not the warning, decides whether the block starts.
//!
//! GRACE
//! - After a scheduled or one-shot block starts, a global grace
//!   period (`Preferences::grace_seconds`, default 60, range 0..=300,
//!   0 = off) runs before enforcement kicks in, so the user can
//!   finish what they were doing. Grace is stored on the session
//!   itself as `grace_until_ms` (wall-clock epoch ms, `None` when
//!   off or expired) — no parallel state machine, and no new
//!   session status: a grace session is `Running` with a future
//!   `grace_until_ms`, and the UI shows it as starting with a
//!   countdown computed from that field.
//! - During grace, NOTHING is enforced and NOTHING accrues: the
//!   engine tick returns before sampling (so no app hiding, no
//!   distracted time, no productive time, no Pomodoro/reward/
//!   allowance accrual, no block-attempt rows), `publish()` sends
//!   no website policy (so the extension removes its rules), and
//!   the frozen overlay is not created (a frozen session in grace
//!   is not yet frozen — see `frozen.rs::is_enforced` combined
//!   with `grace_active` in `lib.rs`). Grace never extends the
//!   productive target: `remaining_ms` is untouched during grace.
//! - When grace ends, normal Running enforcement begins
//!   automatically on the same tick: the tick clears
//!   `grace_until_ms`, records one `grace_ended` report event, and
//!   falls through to the normal sample/policy path.
//! - If the schedule window ends during grace, the session ends
//!   normally (schedule window-end evaluation runs before the
//!   grace branch and matches grace sessions too). If the user
//!   ends the session during grace, that is allowed freely:
//!   `sessions::apply` skips the frozen gate, the schedule
//!   time-range lock, the step-10 lock gate, and the Strict-mode
//!   early-end rule for an end in grace, because the block has not
//!   started enforcing yet. (Pausing keeps its existing gates, and
//!   deleting a live session keeps its gates — ending first, then
//!   deleting, is the path during grace.)
//! - Grace survives restart on wall-clock time, like schedule
//!   windows: `grace_until_ms` is persisted in the session JSON.
//!   If it already passed while the app was down, the first tick
//!   clears it and the resumed session starts as Running; if time
//!   remains, it resumes with the remaining grace time. It can
//!   never strand a session: expiry is checked on every tick, and
//!   even a pathological clock game cannot lock the user out,
//!   because ending during grace is always free (and a frozen
//!   session's wall-clock timer still expires during grace — the
//!   timer, not grace, is the bound).
//! - MANUAL sessions get NO grace, by decision: the user started
//!   them deliberately, so there is nothing to finish up for, and
//!   keeping grace auto-only leaves the manual start path
//!   (including the Preparing browser-acknowledgement gate)
//!   completely untouched. The Settings copy says so plainly.
//! - A backward wall-clock jump delays grace expiry (grace is a
//!   wall-clock window, like schedule windows, whose DST/clock
//!   semantics step 5 already accepts); a forward jump shortens
//!   it. Either way the jump is visible as a step-11 `clock_jump`
//!   report event, the productive target is untouched, and a
//!   schedule session still ends at its window end.

use crate::focus::FocusConfig;

/// Maximum number of warning points in the global list. Ten is far
/// more than anyone needs (the UI hint shows two); the cap keeps a
/// mistyped paste from producing a notification storm.
pub const MAX_WARNING_POINTS: usize = 10;

/// Parse the global warning-minutes preference: comma-separated
/// integers in 1..=60, deduped and sorted. An empty (or all-blank)
/// string is valid and means no warnings; empty segments (a
/// trailing comma) are forgiven, but any non-integer or
/// out-of-range segment is an error, as is a list over the cap.
pub fn parse_warning_minutes(text: &str) -> Result<Vec<u32>, String> {
    let mut minutes = Vec::new();
    for segment in text.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let value: u32 = segment.parse().map_err(|_| {
            format!("Warning times must be whole minutes from 1 to 60, separated by commas: {text}")
        })?;
        if !(1..=60).contains(&value) {
            return Err(format!(
                "Warning times must be whole minutes from 1 to 60, separated by commas: {text}"
            ));
        }
        minutes.push(value);
    }
    minutes.sort_unstable();
    minutes.dedup();
    if minutes.len() > MAX_WARNING_POINTS {
        return Err(format!(
            "Use at most {MAX_WARNING_POINTS} warning times, separated by commas."
        ));
    }
    Ok(minutes)
}

/// Canonical form of a parsed list, for normalizing the preference
/// on save: `"5, 15"`. An empty list normalizes to `""`.
pub fn format_warning_minutes(minutes: &[u32]) -> String {
    minutes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// True when a session is inside its grace period right now.
/// Timestamp-only on purpose: the end-during-grace bypass in
/// `sessions::apply` must also cover a grace session the user
/// paused (a one-shot can be paused during grace), and a paused
/// grace session is still a block that has not enforced yet.
/// Callers that only care about Running sessions (policy publish,
/// frozen overlay, tray text) combine this with a status check.
pub fn grace_active(grace_until_ms: Option<i64>, now_ms: i64) -> bool {
    grace_until_ms.is_some_and(|until| now_ms < until)
}

/// Grace time left, rounded UP to whole seconds for countdown
/// display (a grace ending in 400 ms still shows as 1 second, never
/// as 0 while it is still active). Zero when not in grace.
pub fn grace_remaining_secs(grace_until_ms: Option<i64>, now_ms: i64) -> u64 {
    match grace_until_ms {
        Some(until) if until > now_ms => ((until - now_ms) as u64).div_ceil(1000),
        _ => 0,
    }
}

/// The `grace_until_ms` value for a session starting now: `None`
/// when grace is off (0 seconds), otherwise `now + seconds`.
pub fn grace_until_for(grace_seconds: u32, now_ms: i64) -> Option<i64> {
    if grace_seconds == 0 {
        None
    } else {
        Some(now_ms + i64::from(grace_seconds) * 1000)
    }
}

/// Body for the grace-start notification: the block title plus the
/// TRUE seconds remaining. The notification fires when the session
/// enters grace (blocking begins when the countdown ends), and a
/// deferred one — Windows background engine with no channel yet —
/// says the remaining time, never the original grace length, the
/// same true-remaining rule the advance warnings already use.
pub fn grace_body(title: &str, secs: u64) -> String {
    let second_word = if secs == 1 { "second" } else { "seconds" };
    format!(
        "{title} starts in {secs} {second_word}. Blocking begins when the countdown ends — finish up, nothing is blocked yet."
    )
}

/// Minutes until a block starts, rounded UP, for warning text.
/// Only called when `now < start`, so the result is at least 1.
pub fn minutes_until(start_ms: i64, now_ms: i64) -> u64 {
    ((start_ms - now_ms).max(1) as u64).div_ceil(60_000)
}

/// What a scheduled / one-shot session still needs before it can
/// start cleanly: a connected browser extension, camera permission,
/// or nothing. Pure, so the warning text and the tests can use it
/// without an engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissingPrerequisites {
    /// The session needs website blocking but its browser is not
    /// properly connected.
    pub browser: bool,
    /// The session needs camera checks but access is not granted.
    pub camera: bool,
}

/// Which fixable prerequisites one auto-starting block is still
/// missing, from the session's own focus snapshot:
/// - Browser: the focus needs website blocking
///   (`network_required`, the same rule the manual start preflight
///   enforces) and the caller says its browser(s) are not ready.
///   `browser_ready` is computed by the engine from the live bridge
///   state, so a connection from the wrong browser does not count.
/// - Camera: the focus needs camera checks (`camera` on, or
///   `camera_and_activity` verification) and access is not granted.
///   Camera checks exist on macOS only (Windows camera work is
///   still pending), so `camera_checks_available` is false
///   elsewhere and a camera requirement never warns there.
pub fn missing_prerequisites(
    focus: &FocusConfig,
    browser_ready: bool,
    camera_granted: bool,
    camera_checks_available: bool,
) -> MissingPrerequisites {
    MissingPrerequisites {
        browser: focus.network_required() && !browser_ready,
        camera: camera_checks_available
            && (focus.camera || focus.attention.verification == "camera_and_activity")
            && !camera_granted,
    }
}

/// Warning body with readiness: the plain warning text plus one
/// short line per missing prerequisite, so the user can fix it
/// before the block starts instead of hitting the fail-closed
/// browser block (or a paused camera timer) at start time.
pub fn warning_body_with_readiness(
    title: &str,
    start_ms: i64,
    now_ms: i64,
    missing: &MissingPrerequisites,
) -> String {
    let mut body = warning_body(title, start_ms, now_ms);
    if missing.browser {
        body.push_str(" Browser is not connected — connect it before the session starts.");
    }
    if missing.camera {
        body.push_str(
            " Camera access is not granted — allow camera access before the session starts.",
        );
    }
    body
}

/// Warning body text: the block title plus the true minutes
/// remaining. The notification title is fixed ("Block starting
/// soon"); the body carries the per-block facts.
pub fn warning_body(title: &str, start_ms: i64, now_ms: i64) -> String {
    let remaining = minutes_until(start_ms, now_ms);
    let minute_word = if remaining == 1 { "minute" } else { "minutes" };
    format!("{title} starts in {remaining} {minute_word}. Finish up — blocking begins then.")
}

/// Persisted fired-warning key for one schedule instance: the
/// instance's start instant plus the warning point. Start instants
/// are unique per occurrence (a weekly schedule's starts are a
/// week apart), so the key identifies exactly one instance.
pub fn schedule_warning_key(start_ms: i64, minutes: u32) -> String {
    format!("{start_ms}:{minutes}")
}

/// Warning points due for one schedule instance right now: every
/// point whose warning time has arrived, whose block has not
/// started, and whose key is not already fired. Pure, so the
/// once-per-instance rule (including restart, where `fired` is
/// reloaded from storage) is testable without an engine.
pub fn due_schedule_warnings(
    start_ms: i64,
    fired: &[String],
    points: &[u32],
    now_ms: i64,
) -> Vec<u32> {
    if now_ms >= start_ms {
        return vec![];
    }
    points
        .iter()
        .copied()
        .filter(|minutes| {
            start_ms - i64::from(*minutes) * 60_000 <= now_ms
                && !fired
                    .iter()
                    .any(|key| key == &schedule_warning_key(start_ms, *minutes))
        })
        .collect()
}

/// Drop fired keys that do not belong to this instance (past
/// instances, malformed keys, keys from before a schedule edit
/// changed the start time). Only the upcoming instance's keys are
/// ever meaningful, so pruning keeps the stored list to at most
/// one instance's points.
pub fn prune_schedule_warnings(fired: &mut Vec<String>, start_ms: i64) {
    fired.retain(|key| {
        key.split_once(':')
            .is_some_and(|(start, _)| start.parse::<i64>().is_ok_and(|parsed| parsed == start_ms))
    });
}

/// Warning points due for one one-shot timer right now: the same
/// crossing rule as schedules, with the fired minute values
/// directly (a one-shot has exactly one start).
pub fn due_one_shot_warnings(
    fire_at_ms: i64,
    fired: &[u32],
    points: &[u32],
    now_ms: i64,
) -> Vec<u32> {
    if now_ms >= fire_at_ms {
        return vec![];
    }
    points
        .iter()
        .copied()
        .filter(|minutes| {
            fire_at_ms - i64::from(*minutes) * 60_000 <= now_ms && !fired.contains(minutes)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::FocusConfig;
    use crate::schedules::Schedule;
    use crate::schedules::{
        local_weekday_and_minutes, next_start_day_offset, next_window_start_ms,
    };

    fn schedule_for(days: Vec<u32>, start: &str, end: &str) -> Schedule {
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
    fn warning_list_parsing_sorts_dedups_and_rejects_bad_values() {
        assert_eq!(parse_warning_minutes("5").unwrap(), vec![5]);
        assert_eq!(parse_warning_minutes("5, 15").unwrap(), vec![5, 15]);
        assert_eq!(parse_warning_minutes("15,5,15").unwrap(), vec![5, 15]);
        assert_eq!(parse_warning_minutes("").unwrap(), Vec::<u32>::new());
        assert_eq!(parse_warning_minutes("   ").unwrap(), Vec::<u32>::new());
        // A trailing comma is forgiven; a bad segment is not.
        assert_eq!(parse_warning_minutes("5,").unwrap(), vec![5]);
        assert!(parse_warning_minutes("0").is_err());
        assert!(parse_warning_minutes("61").is_err());
        assert!(parse_warning_minutes("abc").is_err());
        assert!(parse_warning_minutes("5, 61").is_err());
        assert!(parse_warning_minutes("5, -1").is_err());
        let many: Vec<String> = (1..=11).map(|n| n.to_string()).collect();
        assert!(parse_warning_minutes(&many.join(", ")).is_err());
        assert_eq!(
            format_warning_minutes(&parse_warning_minutes("15,5").unwrap()),
            "5, 15"
        );
        assert_eq!(format_warning_minutes(&[]), "");
    }

    #[test]
    fn next_start_day_offset_picks_the_next_listed_day() {
        // Monday (1), 08:00, start 09:00 today: today.
        assert_eq!(next_start_day_offset(1, 480, &[1, 3], 540), Some(0));
        // Start time already passed today: next listed day (Wed).
        assert_eq!(next_start_day_offset(1, 600, &[1, 3], 540), Some(2));
        // Exactly at the start time: the block is starting, so the
        // next start is the next listed day, never "now".
        assert_eq!(next_start_day_offset(1, 540, &[1, 3], 540), Some(2));
        // Only today listed and the start passed: next week.
        assert_eq!(next_start_day_offset(1, 600, &[1], 540), Some(7));
        // No listed days: no next start.
        assert_eq!(next_start_day_offset(1, 480, &[], 540), None);
    }

    #[test]
    fn next_window_start_lands_on_the_listed_day_and_time() {
        // Any real timestamp: the computed start must be in the
        // future, on a listed day, at exactly the start time, in
        // local time (checked through the same local-time helper
        // the engine uses, so this holds whatever the timezone).
        let now = crate::clock::now();
        let (weekday, minutes) = local_weekday_and_minutes(now);
        let start = (minutes + 120) % 1440;
        let day = if minutes + 120 < 1440 {
            weekday
        } else {
            (weekday + 1) % 7
        };
        let fmt = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        let end = (start + 60) % 1440;
        let schedule = schedule_for(vec![day], &fmt(start), &fmt(end));
        let next = next_window_start_ms(&schedule, now).unwrap();
        assert!(next > now);
        let (start_weekday, start_minutes) = local_weekday_and_minutes(next);
        assert_eq!(start_weekday, day);
        assert_eq!(start_minutes, start);
    }

    #[test]
    fn schedule_warnings_fire_once_per_instance_including_restart() {
        let start = 10_000_000;
        let points = vec![5, 15];
        // Before any warning time: nothing due.
        assert!(due_schedule_warnings(start, &[], &points, start - 16 * 60_000).is_empty());
        // At the 15-minute warning time: only that point.
        assert_eq!(
            due_schedule_warnings(start, &[], &points, start - 15 * 60_000),
            vec![15]
        );
        // Between the two points: the 15 is due, the 5 is not.
        assert_eq!(
            due_schedule_warnings(start, &[], &points, start - 10 * 60_000),
            vec![15]
        );
        // After firing the 15 (restart reloads the same fired list),
        // it never fires again, but the 5 still fires at its time.
        let fired = vec![schedule_warning_key(start, 15)];
        assert!(due_schedule_warnings(start, &fired, &points, start - 10 * 60_000).is_empty());
        assert_eq!(
            due_schedule_warnings(start, &fired, &points, start - 5 * 60_000),
            vec![5]
        );
        // At and after the start: nothing fires, ever.
        let fired = vec![
            schedule_warning_key(start, 15),
            schedule_warning_key(start, 5),
        ];
        assert!(due_schedule_warnings(start, &fired, &points, start).is_empty());
        assert!(due_schedule_warnings(start, &[], &points, start + 60_000).is_empty());
        // Pruning keeps only this instance's keys.
        let mut fired = vec![
            schedule_warning_key(start - 7 * 86_400_000, 5),
            schedule_warning_key(start, 15),
            "garbage".into(),
        ];
        prune_schedule_warnings(&mut fired, start);
        assert_eq!(fired, vec![schedule_warning_key(start, 15)]);
    }

    #[test]
    fn one_shot_warnings_use_the_same_crossing_rule() {
        let fire_at = 10_000_000;
        let points = vec![5, 15];
        assert_eq!(
            due_one_shot_warnings(fire_at, &[], &points, fire_at - 15 * 60_000),
            vec![15]
        );
        assert!(due_one_shot_warnings(fire_at, &[15], &points, fire_at - 10 * 60_000).is_empty());
        assert_eq!(
            due_one_shot_warnings(fire_at, &[15], &points, fire_at - 5 * 60_000),
            vec![5]
        );
        assert!(due_one_shot_warnings(fire_at, &[], &points, fire_at).is_empty());
    }

    #[test]
    fn warning_text_states_the_true_minutes_remaining() {
        // A 15-minute point firing late (3 minutes before the block)
        // says 3 minutes, not 15.
        assert_eq!(
            warning_body("Deep work", 10_000_000, 10_000_000 - 3 * 60_000),
            "Deep work starts in 3 minutes. Finish up — blocking begins then."
        );
        assert_eq!(
            warning_body("Deep work", 10_000_000, 10_000_000 - 60_000),
            "Deep work starts in 1 minute. Finish up — blocking begins then."
        );
        // 30 seconds before the block still reads as 1 minute.
        assert_eq!(
            warning_body("Deep work", 10_000_000, 10_000_000 - 30_000),
            "Deep work starts in 1 minute. Finish up — blocking begins then."
        );
    }

    fn focus_needing_browser() -> FocusConfig {
        FocusConfig {
            domains: vec!["example.com".into()],
            browser_app: "com.google.chrome".into(),
            ..FocusConfig::default()
        }
    }

    fn focus_needing_camera() -> FocusConfig {
        FocusConfig {
            camera: true,
            ..FocusConfig::default()
        }
    }

    #[test]
    fn missing_prerequisites_covers_browser_and_camera_independently() {
        // A plain timer session needs neither, whatever the state.
        let plain = FocusConfig::default();
        assert_eq!(
            missing_prerequisites(&plain, false, false, true),
            MissingPrerequisites {
                browser: false,
                camera: false,
            }
        );
        // Website blocking without a connected browser: browser only.
        let browser_focus = focus_needing_browser();
        assert_eq!(
            missing_prerequisites(&browser_focus, false, true, true),
            MissingPrerequisites {
                browser: true,
                camera: false,
            }
        );
        assert_eq!(
            missing_prerequisites(&browser_focus, true, true, true),
            MissingPrerequisites {
                browser: false,
                camera: false,
            }
        );
        // Camera on without permission: camera only.
        let camera_focus = focus_needing_camera();
        assert_eq!(
            missing_prerequisites(&camera_focus, true, false, true),
            MissingPrerequisites {
                browser: false,
                camera: true,
            }
        );
        assert_eq!(
            missing_prerequisites(&camera_focus, true, true, true),
            MissingPrerequisites {
                browser: false,
                camera: false,
            }
        );
        // Camera required by verification alone (camera flag off).
        let mut verification_focus = FocusConfig::default();
        verification_focus.attention.verification = "camera_and_activity".into();
        assert_eq!(
            missing_prerequisites(&verification_focus, true, false, true),
            MissingPrerequisites {
                browser: false,
                camera: true,
            }
        );
        // Camera checks do not exist on this platform: a camera
        // requirement never warns, even with access ungranted.
        assert_eq!(
            missing_prerequisites(&camera_focus, true, false, false),
            MissingPrerequisites {
                browser: false,
                camera: false,
            }
        );
        // Both missing at once.
        let mut both = focus_needing_browser();
        both.camera = true;
        assert_eq!(
            missing_prerequisites(&both, false, false, true),
            MissingPrerequisites {
                browser: true,
                camera: true,
            }
        );
        // A frozen session needs its browser too (network_required
        // covers frozen), so a disconnected browser warns for it.
        let mut frozen = FocusConfig::default();
        frozen.frozen.enabled = true;
        frozen.browser_app = "com.google.chrome".into();
        assert_eq!(
            missing_prerequisites(&frozen, false, true, true),
            MissingPrerequisites {
                browser: true,
                camera: false,
            }
        );
    }

    #[test]
    fn warning_body_with_readiness_appends_one_line_per_missing_item() {
        let none = MissingPrerequisites {
            browser: false,
            camera: false,
        };
        assert_eq!(
            warning_body_with_readiness("Deep work", 10_000_000, 10_000_000 - 5 * 60_000, &none),
            "Deep work starts in 5 minutes. Finish up — blocking begins then."
        );
        let browser_only = MissingPrerequisites {
            browser: true,
            camera: false,
        };
        assert_eq!(
            warning_body_with_readiness(
                "Deep work",
                10_000_000,
                10_000_000 - 5 * 60_000,
                &browser_only
            ),
            "Deep work starts in 5 minutes. Finish up — blocking begins then. Browser is not connected — connect it before the session starts."
        );
        let camera_only = MissingPrerequisites {
            browser: false,
            camera: true,
        };
        assert_eq!(
            warning_body_with_readiness(
                "Deep work",
                10_000_000,
                10_000_000 - 5 * 60_000,
                &camera_only
            ),
            "Deep work starts in 5 minutes. Finish up — blocking begins then. Camera access is not granted — allow camera access before the session starts."
        );
        let both = MissingPrerequisites {
            browser: true,
            camera: true,
        };
        assert_eq!(
            warning_body_with_readiness("Deep work", 10_000_000, 10_000_000 - 5 * 60_000, &both),
            "Deep work starts in 5 minutes. Finish up — blocking begins then. Browser is not connected — connect it before the session starts. Camera access is not granted — allow camera access before the session starts."
        );
    }

    #[test]
    fn grace_helpers_cover_off_active_and_expired() {
        assert_eq!(grace_until_for(0, 1_000_000), None);
        assert_eq!(grace_until_for(60, 1_000_000), Some(1_060_000));
        assert!(!grace_active(None, 1_000_000));
        assert!(grace_active(Some(1_060_000), 1_000_000));
        assert!(!grace_active(Some(1_060_000), 1_060_000));
        assert!(!grace_active(Some(1_060_000), 2_000_000));
        // Remaining time rounds up: 400 ms left still shows 1 s.
        assert_eq!(grace_remaining_secs(Some(1_000_400), 1_000_000), 1);
        assert_eq!(grace_remaining_secs(Some(1_060_000), 1_000_000), 60);
        assert_eq!(grace_remaining_secs(Some(1_060_000), 1_060_000), 0);
        assert_eq!(grace_remaining_secs(None, 1_000_000), 0);
    }

    #[test]
    fn grace_body_states_true_remaining_seconds() {
        assert_eq!(
            grace_body("Morning block", 60),
            "Morning block starts in 60 seconds. Blocking begins when the countdown ends — finish up, nothing is blocked yet."
        );
        // A deferred notification says what is left, never the
        // original grace length; one second is singular.
        assert_eq!(
            grace_body("Evening block", 1),
            "Evening block starts in 1 second. Blocking begins when the countdown ends — finish up, nothing is blocked yet."
        );
    }
}
