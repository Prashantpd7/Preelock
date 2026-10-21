//! UI-independent timing and transition rules. All durations are milliseconds.
use crate::models::{Action, Preferences, Session, Snapshot, Status, Task};

fn title(value: String) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 {
        return Err("Use a title between 1 and 160 characters.".into());
    }
    Ok(value.into())
}
fn minutes(value: u32) -> Result<u32, String> {
    if !(1..=480).contains(&value) {
        return Err("Choose a duration from 1 to 480 minutes.".into());
    }
    Ok(value)
}
pub fn advance(s: &mut Session, elapsed: u64, now: i64) {
    if s.status != Status::Running || s.focus.attention.enabled {
        return;
    }
    let used = elapsed.min(s.remaining_ms);
    s.remaining_ms -= used;
    s.focused_ms += used;
    if s.remaining_ms == 0 {
        s.status = Status::Completed;
        s.ended_at =
            Some(now.saturating_sub(elapsed.saturating_sub(used).min(i64::MAX as u64) as i64));
        s.deadline_ms = None;
    } else {
        s.deadline_ms = Some(now + s.remaining_ms as i64);
    }
}
#[cfg(test)]
pub fn restore(data: &mut Snapshot, now: i64) {
    for s in &mut data.sessions {
        if s.status == Status::Running && !s.focus.attention.enabled {
            let deadline = s.deadline_ms.unwrap_or(now);
            let remaining = deadline.saturating_sub(now).max(0) as u64;
            let overdue = now.saturating_sub(deadline).max(0) as u64;
            advance(
                s,
                s.remaining_ms
                    .saturating_sub(remaining)
                    .saturating_add(overdue),
                now,
            );
        }
    }
}
pub fn apply(data: &mut Snapshot, action: Action, now: i64) -> Result<(), String> {
    match action {
        Action::CreateSession {
            title: name,
            minutes: duration,
            allow_pause,
            focus,
        } => {
            let duration = minutes(duration)?;
            let mut focus = crate::focus::validate(focus)?;
            focus.attention.enabled = true;
            data.sessions.push(Session {
                id: uuid::Uuid::new_v4().to_string(),
                title: title(name)?,
                duration_minutes: duration,
                remaining_ms: u64::from(duration) * 60_000,
                focused_ms: 0,
                allow_pause,
                status: Status::Ready,
                started_at: None,
                ended_at: None,
                deadline_ms: None,
                focus,
                report: Default::default(),
                attention: Default::default(),
                force_quit: false,
            });
        }
        // Reject the old IPC command as well as removing its UI. Keep the
        // variant readable for compatibility with an older running frontend.
        Action::EmergencyExit { .. } => {
            return Err("Early exit is unavailable.".into());
        }
        Action::CorrectInterval {
            id,
            interval_id,
            note,
        } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            crate::attention::correct(s, &interval_id, &note, now)?;
        }
        Action::DisableCamera { id } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            s.focus.camera = false;
            s.report.camera_status = "off".into();
            s.report
                .event(now, "camera_disabled", "Camera consent withdrawn.");
        }
        Action::EnableCamera { id } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            if !matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            ) {
                return Err("The camera can only be restarted during an active session.".into());
            }
            s.focus.camera = true;
            s.report
                .event(now, "camera_enabled", "Camera turned back on by you.");
        }
        Action::CreateTask { title: name } => data.tasks.push(Task {
            id: uuid::Uuid::new_v4().to_string(),
            title: title(name)?,
            completed: false,
            created_at: now,
        }),
        Action::UpdateTask {
            id,
            title: name,
            completed,
        } => {
            let task = data
                .tasks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Task not found.")?;
            task.title = title(name)?;
            task.completed = completed;
        }
        Action::DeleteTask { id } => {
            let before = data.tasks.len();
            data.tasks.retain(|t| t.id != id);
            if data.tasks.len() == before {
                return Err("Task not found.".into());
            }
        }
        Action::UpdatePreferences { preferences } => {
            data.preferences = crate::settings::validate_preferences(preferences)?;
        }
        Action::SetPreferences {
            theme,
            default_minutes,
        } => {
            if !["dark", "light", "system"].contains(&theme.as_str()) {
                return Err("Unknown theme.".into());
            }
            data.preferences = Preferences {
                theme,
                default_minutes: minutes(default_minutes)?,
                ..data.preferences.clone()
            };
        }
        Action::DeleteSession { id } => {
            let index = data
                .sessions
                .iter()
                .position(|s| s.id == id)
                .ok_or("Session not found.")?;
            if matches!(
                data.sessions[index].status,
                Status::Running | Status::Paused | Status::Preparing
            ) && data.sessions[index].focus.mode != crate::focus::Mode::Standard
            {
                return Err(
                    "Committed sessions cannot be paused, ended early, or deleted while active."
                        .into(),
                );
            }
            data.sessions.remove(index);
        }
        Action::RestartSession { id } => {
            if data.sessions.iter().any(|s| {
                s.id != id
                    && matches!(
                        s.status,
                        Status::Running | Status::Paused | Status::Preparing
                    )
            }) {
                return Err("End the current session before starting another.".into());
            }
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            if !matches!(
                s.status,
                Status::Completed | Status::Ended | Status::Interrupted
            ) {
                return Err("Only finished sessions can be restarted.".into());
            }
            // Fresh run, same configuration.
            s.force_quit = false;
            s.status = Status::Ready;
            s.started_at = None;
            s.ended_at = None;
            s.deadline_ms = None;
            s.remaining_ms = u64::from(s.duration_minutes) * 60_000;
            s.focused_ms = 0;
            s.report = Default::default();
            s.attention = Default::default();
            // A fresh run restores the camera the session was created with;
            // a mid-run "stop" was runtime state, not configuration.
            if s.focus.attention.verification == "camera_and_activity" {
                s.focus.camera = true;
            }
        }
        action => {
            let (id, target, expected) = match action {
                Action::StartSession { id } => (id, Status::Running, Some(Status::Ready)),
                Action::PauseSession { id } => (id, Status::Paused, Some(Status::Running)),
                Action::ResumeSession { id } => (id, Status::Running, Some(Status::Paused)),
                Action::EndSession { id } => (id, Status::Ended, None),
                _ => unreachable!(),
            };
            if target == Status::Running
                && data.sessions.iter().any(|s| {
                    s.id != id
                        && matches!(
                            s.status,
                            Status::Running | Status::Paused | Status::Preparing
                        )
                })
            {
                return Err("End the current session before starting another.".into());
            }
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            if expected.is_some_and(|expected| s.status != expected) {
                return Err("This session does not allow that action.".into());
            }
            if s.focus.mode != crate::focus::Mode::Standard
                && s.status != Status::Ready
                && matches!(target, Status::Paused | Status::Ended)
            {
                return Err(
                    "Committed sessions cannot be paused, ended early, or deleted while active."
                        .into(),
                );
            }
            let valid = match target {
                Status::Running => matches!(s.status, Status::Ready | Status::Paused),
                Status::Paused => s.status == Status::Running && s.allow_pause,
                Status::Ended => {
                    matches!(
                        s.status,
                        Status::Running | Status::Paused | Status::Ready | Status::Preparing
                    )
                }
                _ => false,
            };
            if !valid {
                return Err("This session does not allow that action.".into());
            }
            s.status = target;
            match s.status {
                Status::Running => {
                    s.started_at.get_or_insert(now);
                    s.deadline_ms = Some(now + s.remaining_ms as i64);
                }
                Status::Ended => {
                    s.ended_at = Some(now);
                    s.deadline_ms = None;
                }
                _ => s.deadline_ms = None,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn data(allow_pause: bool) -> Snapshot {
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 1,
                allow_pause,
                focus: Default::default(),
            },
            1000,
        )
        .unwrap();
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id }, 1000).unwrap();
        d.sessions[0].focus.attention.enabled = false;
        d
    }
    #[test]
    fn completion_caps_focus_at_planned_duration() {
        let mut d = data(true);
        advance(&mut d.sessions[0], 90_000, 91_000);
        assert_eq!(d.sessions[0].focused_ms, 60_000);
        assert_eq!(d.sessions[0].ended_at, Some(61_000));
        assert_eq!(d.sessions[0].status, Status::Completed);
    }
    #[test]
    fn finished_sessions_restart_as_fresh_runs() {
        let mut d = data(true);
        let id = d.sessions[0].id.clone();
        // A running session cannot be "restarted".
        assert!(apply(&mut d, Action::RestartSession { id: id.clone() }, 1000).is_err());
        advance(&mut d.sessions[0], 90_000, 91_000);
        assert_eq!(d.sessions[0].status, Status::Completed);
        d.sessions[0].force_quit = true;
        apply(&mut d, Action::RestartSession { id: id.clone() }, 200_000).unwrap();
        assert!(!d.sessions[0].force_quit);
        let s = &d.sessions[0];
        assert_eq!(s.status, Status::Ready);
        assert_eq!(s.started_at, None);
        assert_eq!(s.ended_at, None);
        assert_eq!(s.remaining_ms, 60_000);
        assert_eq!(s.focused_ms, 0);
        // A restarted camera session gets its camera back.
        d.sessions[0].focus.attention.verification = "camera_and_activity".into();
        d.sessions[0].focus.camera = false;
        d.sessions[0].status = Status::Completed;
        apply(&mut d, Action::RestartSession { id: id.clone() }, 300_000).unwrap();
        assert!(d.sessions[0].focus.camera);
        // The same card can start again.
        apply(&mut d, Action::StartSession { id }, 200_000).unwrap();
        assert_eq!(d.sessions[0].status, Status::Running);
    }
    #[test]
    fn camera_can_be_restarted_after_stop() {
        let mut d = data(true);
        let id = d.sessions[0].id.clone();
        d.sessions[0].focus.camera = true;
        apply(&mut d, Action::DisableCamera { id: id.clone() }, 1000).unwrap();
        assert!(!d.sessions[0].focus.camera);
        apply(&mut d, Action::EnableCamera { id: id.clone() }, 2000).unwrap();
        assert!(d.sessions[0].focus.camera);
        // Only during an active session.
        d.sessions[0].status = Status::Completed;
        assert!(apply(&mut d, Action::EnableCamera { id }, 3000).is_err());
    }
    #[test]
    fn pause_policy_is_enforced() {
        let mut d = data(false);
        let id = d.sessions[0].id.clone();
        assert!(apply(&mut d, Action::PauseSession { id }, 2000).is_err());
    }
    #[test]
    fn paused_time_is_not_focus_time() {
        let mut d = data(true);
        advance(&mut d.sessions[0], 10_000, 11_000);
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::PauseSession { id: id.clone() }, 11_000).unwrap();
        restore(&mut d, 100_000);
        assert_eq!(d.sessions[0].remaining_ms, 50_000);
        apply(&mut d, Action::ResumeSession { id }, 100_000).unwrap();
        assert_eq!(d.sessions[0].deadline_ms, Some(150_000));
    }
    #[test]
    fn restores_and_completes_overdue_sessions() {
        let mut d = data(true);
        restore(&mut d, 31_000);
        assert_eq!(d.sessions[0].remaining_ms, 30_000);
        restore(&mut d, 100_000);
        assert_eq!(d.sessions[0].status, Status::Completed);
        assert_eq!(d.sessions[0].focused_ms, 60_000);
        assert_eq!(d.sessions[0].ended_at, Some(61_000));
    }
    #[test]
    fn prevents_concurrent_sessions_and_invalid_input() {
        let mut d = data(true);
        assert!(apply(&mut d, Action::CreateTask { title: " ".into() }, 1).is_err());
        apply(
            &mut d,
            Action::CreateSession {
                title: "Second".into(),
                minutes: 2,
                allow_pause: true,
                focus: Default::default(),
            },
            1,
        )
        .unwrap();
        let id = d.sessions[1].id.clone();
        assert!(apply(&mut d, Action::StartSession { id }, 2).is_err());
    }
    #[test]
    fn monotonic_elapsed_is_independent_of_wall_clock() {
        let mut d = data(true);
        advance(&mut d.sessions[0], 10_000, -100_000);
        assert_eq!(d.sessions[0].remaining_ms, 50_000);
        assert_eq!(d.sessions[0].focused_ms, 10_000);
    }
    #[test]
    fn strict_commitment_rejects_all_early_exit_commands() {
        let mut d = data(true);
        d.sessions[0].focus.mode = crate::focus::Mode::Strict;
        let id = d.sessions[0].id.clone();
        assert!(apply(&mut d, Action::PauseSession { id: id.clone() }, 2000).is_err());
        assert!(apply(&mut d, Action::EndSession { id: id.clone() }, 2000).is_err());
        assert!(apply(
            &mut d,
            Action::EmergencyExit {
                id: id.clone(),
                confirmation: "yes".into()
            },
            2000
        )
        .is_err());
        assert!(apply(
            &mut d,
            Action::EmergencyExit {
                id: id.clone(),
                confirmation: "END FOCUS".into()
            },
            2000,
        )
        .is_err());
        assert!(apply(&mut d, Action::DeleteSession { id }, 2000).is_err());
        assert_eq!(d.sessions[0].status, Status::Running);
    }
    #[test]
    fn legacy_records_get_safe_defaults() {
        let d = data(true);
        let mut json = serde_json::to_value(&d.sessions[0]).unwrap();
        json.as_object_mut().unwrap().remove("focus");
        json.as_object_mut().unwrap().remove("report");
        let old: Session = serde_json::from_value(json).unwrap();
        assert_eq!(old.focus.mode, crate::focus::Mode::Standard);
        assert!(!old.focus.activity_consent);
        assert!(!old.focus.camera);
    }
}
