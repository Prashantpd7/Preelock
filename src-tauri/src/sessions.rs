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
/// Step 12: true when a frozen schedule's block is currently in
/// force, so its schedule cannot be changed or deleted out from
/// under it. Either a live session from this schedule is running,
/// or the schedule's window is active right now (the session
/// starts on the next tick, so the window alone already counts).
/// A frozen schedule outside its window, with no live session, is
/// freely editable — the lock only exists while the block does.
fn frozen_schedule_active(
    data: &Snapshot,
    schedule: &crate::schedules::Schedule,
    now: i64,
) -> bool {
    if !schedule.focus.frozen.enabled {
        return false;
    }
    if data.sessions.iter().any(|s| {
        s.schedule_id.as_deref() == Some(schedule.id.as_str())
            && matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
    }) {
        return true;
    }
    let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
    crate::schedules::window_active(schedule, weekday, minutes)
}
/// Productive time consumed in one `advance` slice: the session
/// target falls, focused time rises, and (step 9) reward credit
/// accrues from exactly this productive time — never from break,
/// distracted, or idle time, which never reaches this helper.
fn consume_focus(s: &mut Session, take: u64) {
    if take == 0 {
        return;
    }
    s.remaining_ms -= take;
    s.focused_ms += take;
    crate::rewards::accrue(s, take);
}

pub fn advance(s: &mut Session, elapsed: u64, now: i64) {
    if s.status != Status::Running || s.focus.attention.enabled {
        return;
    }
    // Plain-timer path (no Pomodoro phases) and Pomodoro path (step
    // 7) share one loop, extended by step 9 with two reward branches
    // that run before any Focus accounting:
    // - Reward break: break time never touches remaining, it only
    //   runs the break (and its funding credit) down.
    // - Pending delayed break: the delay and focus run CONCURRENTLY
    //   — the slice counts as focus time AND counts the delay down,
    //   so focus and blocking continue normally until the delay
    //   elapses and the break starts. If that focus slice completes
    //   a Pomodoro phase, the Pomodoro break cancels the pending
    //   break (see `pomodoro::complete_focus_phase`), so the delay
    //   step below is skipped when the pending break is gone.
    // A single elapsed window may span several phases, so loop until
    // it is consumed; every iteration either consumes time or moves
    // to a phase with a validated positive length, so the loop always
    // terminates. The session still completes the moment its
    // productive target is reached, whatever phase it is in — and
    // completing or ending a session clears any reward state, so no
    // break or credit outlives it.
    let mut left = elapsed;
    while left > 0 && s.status == Status::Running {
        if crate::rewards::on_break(s) {
            let take = left.min(s.reward.break_remaining_ms);
            crate::rewards::advance_break(s, take, now);
            left -= take;
            // Projected completion: productive time still owed, plus
            // this break.
            s.deadline_ms = Some(now + s.remaining_ms as i64 + s.reward.break_remaining_ms as i64);
            continue;
        }
        if crate::rewards::pending(s) {
            if crate::pomodoro::in_break(s) {
                // Defensive only: a Pomodoro break start cancels a
                // pending break, so this state should be unreachable.
                crate::rewards::cancel_pending_for_pomodoro_break(s, now);
                continue;
            }
            let slice = left.min(s.reward.pending_delay_ms);
            let take;
            if s.focus.pomodoro.enabled {
                if s.pomodoro.phase_remaining_ms == 0 {
                    s.pomodoro.phase_remaining_ms = s.focus.pomodoro.focus_ms();
                }
                take = slice.min(s.pomodoro.phase_remaining_ms).min(s.remaining_ms);
                if take == 0 {
                    break;
                }
                consume_focus(s, take);
                s.pomodoro.phase_remaining_ms -= take;
                if s.remaining_ms == 0 {
                    s.status = Status::Completed;
                    s.ended_at = Some(now);
                    s.deadline_ms = None;
                    crate::rewards::clear(s);
                    crate::locks::clear_state(&mut s.lock_state);
                    break;
                }
                if s.pomodoro.phase_remaining_ms == 0 {
                    crate::pomodoro::complete_focus_phase(s, now);
                } else {
                    s.deadline_ms = Some(now + s.remaining_ms as i64);
                }
            } else {
                take = slice.min(s.remaining_ms);
                if take == 0 {
                    break;
                }
                consume_focus(s, take);
                if s.remaining_ms == 0 {
                    s.status = Status::Completed;
                    s.ended_at = Some(now);
                    s.deadline_ms = None;
                    crate::rewards::clear(s);
                    crate::locks::clear_state(&mut s.lock_state);
                    break;
                }
                s.deadline_ms = Some(now + s.remaining_ms as i64);
            }
            if !crate::rewards::pending(s) {
                // The focus slice completed a Pomodoro phase, which
                // cancelled the pending break: only the focus part of
                // the slice is consumed, and the loop continues into
                // the Pomodoro break branch for the remainder.
                left -= take;
                continue;
            }
            left -= slice;
            // Still pending: count the delay down, starting the break
            // automatically at zero.
            crate::rewards::advance_delay(s, slice, now);
            continue;
        }
        if !s.focus.pomodoro.enabled {
            let take = left.min(s.remaining_ms);
            if take == 0 {
                break;
            }
            consume_focus(s, take);
            left -= take;
            if s.remaining_ms == 0 {
                s.status = Status::Completed;
                s.ended_at = Some(now.saturating_sub(left.min(i64::MAX as u64) as i64));
                s.deadline_ms = None;
                crate::rewards::clear(s);
                crate::locks::clear_state(&mut s.lock_state);
            } else {
                s.deadline_ms = Some(now + s.remaining_ms as i64);
            }
            continue;
        }
        match s.pomodoro.phase {
            crate::pomodoro::Phase::Focus => {
                if s.pomodoro.phase_remaining_ms == 0 {
                    s.pomodoro.phase_remaining_ms = s.focus.pomodoro.focus_ms();
                }
                let take = left.min(s.pomodoro.phase_remaining_ms).min(s.remaining_ms);
                if take == 0 {
                    break;
                }
                consume_focus(s, take);
                s.pomodoro.phase_remaining_ms -= take;
                left -= take;
                if s.remaining_ms == 0 {
                    s.status = Status::Completed;
                    s.ended_at = Some(now.saturating_sub(left.min(i64::MAX as u64) as i64));
                    s.deadline_ms = None;
                    crate::rewards::clear(s);
                    crate::locks::clear_state(&mut s.lock_state);
                    break;
                }
                if s.pomodoro.phase_remaining_ms == 0 {
                    crate::pomodoro::complete_focus_phase(s, now);
                } else {
                    s.deadline_ms = Some(now + s.remaining_ms as i64);
                }
            }
            crate::pomodoro::Phase::ShortBreak | crate::pomodoro::Phase::LongBreak => {
                if s.pomodoro.phase_remaining_ms == 0 {
                    crate::pomodoro::start_focus_phase(s, now);
                    continue;
                }
                let take = left.min(s.pomodoro.phase_remaining_ms);
                s.pomodoro.phase_remaining_ms -= take;
                left -= take;
                // Projected completion: productive time still owed,
                // plus this break. (The test-only restore helper reads
                // the deadline back: with the break included, its
                // computed advance saturates to zero, so a restored
                // break-phase session is never wrongly completed.)
                s.deadline_ms =
                    Some(now + s.remaining_ms as i64 + s.pomodoro.phase_remaining_ms as i64);
                if s.pomodoro.phase_remaining_ms == 0 {
                    crate::pomodoro::start_focus_phase(s, now);
                }
            }
        }
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
            lock,
            lock_password,
        } => {
            let duration = minutes(duration)?;
            let mut focus = crate::focus::validate(focus)?;
            // Step 12: a frozen session is never pausable, whatever
            // the caller asked for — the gate below would reject
            // every pause anyway, so the stored flag says so too.
            let allow_pause = if focus.frozen.enabled {
                false
            } else {
                allow_pause
            };
            focus.attention.enabled = true;
            let pomodoro = crate::pomodoro::initial_state(&focus.pomodoro);
            // Step 10: hash a supplied password immediately; the
            // plaintext never reaches the stored session.
            let lock = crate::locks::apply_password(lock, lock_password)?;
            let id = uuid::Uuid::new_v4().to_string();
            let lock_state = crate::locks::new_state(&lock, &format!("{id}:{now}"));
            data.sessions.push(Session {
                id,
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
                schedule_id: None,
                pomodoro,
                reward: Default::default(),
                lock,
                lock_state,
                policy_applied: false,
                browser_fail_closed: false,
                // Step 14: manual sessions never get grace.
                grace_until_ms: None,
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
        // Scheduled blocks (step 5). The focus snapshot is stored
        // as given (validated): later edits to the template session
        // never reach it, and edits here never touch the template.
        Action::CreateSchedule {
            title: name,
            days,
            start_time,
            end_time,
            focus,
            enabled,
            lock_changes,
            lock,
            lock_password,
        } => {
            let lock = crate::locks::apply_password(lock, lock_password)?;
            let id = uuid::Uuid::new_v4().to_string();
            let lock_state = crate::locks::new_state(&lock, &format!("{id}:{now}"));
            let schedule = crate::schedules::validate_schedule(crate::schedules::Schedule {
                id,
                title: name,
                days,
                start_time,
                end_time,
                focus,
                enabled,
                lock_changes,
                lock,
                lock_state,
                warning_fired: vec![],
                created_at: now,
            })?;
            data.schedules.push(schedule);
        }
        Action::UpdateSchedule {
            id,
            title: name,
            days,
            start_time,
            end_time,
            focus,
            enabled,
            lock_changes,
            lock: incoming_lock,
            lock_password,
            password,
            challenge_answer,
        } => {
            let existing = data
                .schedules
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or("Schedule not found.")?;
            // A locked schedule cannot be edited, disabled, or
            // deleted during its active window — including turning
            // the lock itself off, which would defeat the lock.
            if crate::schedules::locked_and_active(&existing, now) {
                return Err(
                    "This schedule is locked while its window is active. It cannot be changed until the window ends."
                        .into(),
                );
            }
            // Step 12: a frozen schedule cannot be changed while
            // its block is active — editing it (including turning
            // Frozen itself off) would be an override. No lock
            // proof or delay unlocks it; only the window ending
            // (or the session's timer) lifts it.
            if frozen_schedule_active(data, &existing, now) {
                return Err(
                    "This frozen schedule cannot be changed while its block is active. It cannot be changed until the window ends."
                        .into(),
                );
            }
            // Step 10 gate: while the window is active, a
            // password / challenge / delay lock also blocks the
            // change. The check runs on a clone: on failure
            // nothing is persisted, and on success the schedule
            // below gets fresh lock state anyway.
            if crate::schedules::lock_gate_active(&existing, now) {
                let mut state = existing.lock_state.clone();
                crate::locks::check_gate(
                    &existing.lock,
                    &mut state,
                    "schedule",
                    "change it",
                    "change",
                    password.as_deref(),
                    challenge_answer.as_deref(),
                    now,
                )?;
            }
            // `None` (the quick enabled-toggle path) keeps the
            // existing lock; `Some` replaces it wholesale (see
            // `locks::merge_schedule_lock`).
            let new_lock =
                crate::locks::merge_schedule_lock(&existing.lock, incoming_lock, lock_password)?;
            let lock_state = crate::locks::new_state(&new_lock, &format!("{}:{now}", existing.id));
            let created_at = existing.created_at;
            let updated = crate::schedules::validate_schedule(crate::schedules::Schedule {
                id: id.clone(),
                title: name,
                days,
                start_time,
                end_time,
                focus,
                enabled,
                lock_changes,
                lock: new_lock,
                lock_state,
                // Step 14: editing a schedule keeps its fired
                // warnings (pruning in the warning evaluation
                // drops keys for instances that no longer match).
                warning_fired: existing.warning_fired.clone(),
                created_at,
            })?;
            let slot = data
                .schedules
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Schedule not found.")?;
            *slot = updated;
        }
        Action::DeleteSchedule {
            id,
            password,
            challenge_answer,
        } => {
            let index = data
                .schedules
                .iter()
                .position(|s| s.id == id)
                .ok_or("Schedule not found.")?;
            if crate::schedules::locked_and_active(&data.schedules[index], now) {
                return Err(
                    "This schedule is locked while its window is active. It cannot be deleted until the window ends."
                        .into(),
                );
            }
            // Step 12: same frozen rule as for edits — deleting a
            // frozen schedule while its block is active would end
            // the block early, so it is rejected outright.
            if frozen_schedule_active(data, &data.schedules[index].clone(), now) {
                return Err(
                    "This frozen schedule cannot be deleted while its block is active. It cannot be deleted until the window ends."
                        .into(),
                );
            }
            if crate::schedules::lock_gate_active(&data.schedules[index], now) {
                let schedule = data.schedules[index].clone();
                let mut state = schedule.lock_state.clone();
                crate::locks::check_gate(
                    &schedule.lock,
                    &mut state,
                    "schedule",
                    "delete it",
                    "change",
                    password.as_deref(),
                    challenge_answer.as_deref(),
                    now,
                )?;
            }
            data.schedules.remove(index);
        }
        // Delay-lock requests for schedules (step 10): only
        // meaningful while the window is active and a delay lock
        // is set; otherwise the change is not delay-gated.
        Action::RequestScheduleLockDelay { id } => {
            let schedule = data
                .schedules
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Schedule not found.")?;
            if !crate::schedules::lock_gate_active(schedule, now)
                || schedule.lock.delay_minutes == 0
            {
                return Err("This schedule has no active delay lock.".into());
            }
            crate::locks::request_delay(&schedule.lock, &mut schedule.lock_state, "change", now)?;
        }
        Action::CancelScheduleLockDelay { id } => {
            let schedule = data
                .schedules
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Schedule not found.")?;
            crate::locks::cancel_delay(&mut schedule.lock_state)?;
        }
        // One-shot start timers (step 6). The focus snapshot is
        // stored as given (validated): later edits to the template
        // session never reach it.
        Action::CreateOneShot {
            title: name,
            minutes: duration,
            allow_pause,
            focus,
            fire_at_ms,
        } => {
            let shot = crate::schedules::validate_one_shot(
                crate::schedules::OneShot {
                    id: uuid::Uuid::new_v4().to_string(),
                    title: name,
                    minutes: duration,
                    allow_pause,
                    focus,
                    fire_at_ms,
                    warning_fired: vec![],
                    created_at: now,
                },
                now,
            )?;
            data.one_shots.push(shot);
        }
        Action::DeleteOneShot { id } => {
            let index = data
                .one_shots
                .iter()
                .position(|s| s.id == id)
                .ok_or("One-shot timer not found.")?;
            data.one_shots.remove(index);
        }
        // Time allowances (step 8). An edit keeps the accrued usage
        // and creation time of the existing allowance: changing a
        // budget never erases what was already used in the period,
        // and the frontend-sent usage log is never trusted over the
        // stored one. Deleting or disabling lifts enforcement at
        // once, because exhaustion is computed from the stored
        // allowances on every tick and policy publish.
        Action::CreateAllowance {
            title,
            kind,
            target,
            budget_minutes,
            period,
            mode,
            enabled,
        } => {
            let allowance = crate::allowances::validate_allowance(crate::allowances::Allowance {
                id: uuid::Uuid::new_v4().to_string(),
                title,
                kind,
                target,
                budget_minutes,
                period,
                mode,
                enabled,
                usage: vec![],
                created_at: now,
            })?;
            data.allowances.push(allowance);
        }
        Action::UpdateAllowance { allowance } => {
            let existing = data
                .allowances
                .iter()
                .find(|a| a.id == allowance.id)
                .ok_or("Allowance not found.")?;
            let usage = existing.usage.clone();
            let created_at = existing.created_at;
            let mut updated = allowance;
            updated.id = existing.id.clone();
            updated.usage = usage;
            updated.created_at = created_at;
            let mut updated = crate::allowances::validate_allowance(updated)?;
            crate::allowances::prune(&mut updated, now);
            let slot = data
                .allowances
                .iter_mut()
                .find(|a| a.id == updated.id)
                .ok_or("Allowance not found.")?;
            *slot = updated;
        }
        Action::DeleteAllowance { id } => {
            let index = data
                .allowances
                .iter()
                .position(|a| a.id == id)
                .ok_or("Allowance not found.")?;
            data.allowances.remove(index);
        }
        // App locks: independent timed / daily-limit blocks,
        // enforced whether or not a session runs. The whole-minute
        // input is range-checked here; `validate_lock` then checks
        // the computed rule as a backstop and normalizes the
        // target. Deleting lifts enforcement at once, because it is
        // computed from the stored locks on every tick and policy
        // publish, never stored.
        Action::CreateAppLock {
            title,
            kind,
            target,
            rule,
        } => {
            let rule = match rule {
                crate::applocks::AppLockRuleInput::Timed { minutes } => {
                    if !(1..=10080).contains(&minutes) {
                        return Err(
                            "Choose a timed lock duration from 1 to 10080 minutes (7 days).".into(),
                        );
                    }
                    crate::applocks::LockRule::Timed {
                        ends_at_ms: now + i64::from(minutes) * 60_000,
                    }
                }
                crate::applocks::AppLockRuleInput::DailyLimit { limit_minutes } => {
                    if !(1..=1440).contains(&limit_minutes) {
                        return Err("Choose a daily limit from 1 to 1440 minutes.".into());
                    }
                    crate::applocks::LockRule::DailyLimit {
                        limit_ms: u64::from(limit_minutes) * 60_000,
                    }
                }
            };
            let lock = crate::applocks::validate_lock(
                crate::applocks::AppLock {
                    id: uuid::Uuid::new_v4().to_string(),
                    title,
                    kind,
                    target,
                    rule,
                    enabled: true,
                    created_at: now,
                },
                now,
            )?;
            data.app_locks.push(lock);
        }
        Action::DeleteAppLock { id } => {
            let index = data
                .app_locks
                .iter()
                .position(|l| l.id == id)
                .ok_or("App lock not found.")?;
            data.app_locks.remove(index);
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
        Action::DeleteSession {
            id,
            password,
            challenge_answer,
        } => {
            let index = data
                .sessions
                .iter()
                .position(|s| s.id == id)
                .ok_or("Session not found.")?;
            // A locked schedule's live session cannot be deleted out
            // from under the block during its active window.
            if let Some(schedule_id) = &data.sessions[index].schedule_id {
                if let Some(schedule) = data.schedules.iter().find(|s| &s.id == schedule_id) {
                    if crate::schedules::locked_and_active(schedule, now) {
                        return Err(
                            "This scheduled block is locked while its window is active. It cannot be deleted until the window ends."
                                .into(),
                        );
                    }
                }
            }
            // Step 12: deleting a live frozen session would end
            // its full-computer lock early, so it is rejected
            // outright — no lock proof or delay unlocks it. A
            // not-yet-started or finished frozen session deletes
            // normally, like any other template.
            if matches!(
                data.sessions[index].status,
                Status::Running | Status::Paused | Status::Preparing
            ) && data.sessions[index].focus.frozen.enabled
            {
                return Err(
                    "This frozen session cannot be deleted while it is running. It ends when its timer ends."
                        .into(),
                );
            }
            // Step 10 gate: only while the session is live. A
            // not-yet-started session can always be deleted (its
            // lock has not started yet), and a finished session's
            // lock has already lifted with its timer.
            if matches!(
                data.sessions[index].status,
                Status::Running | Status::Paused | Status::Preparing
            ) {
                let session = data.sessions[index].clone();
                let mut state = session.lock_state.clone();
                crate::locks::check_gate(
                    &session.lock,
                    &mut state,
                    "session",
                    "delete it",
                    "delete",
                    password.as_deref(),
                    challenge_answer.as_deref(),
                    now,
                )?;
            }
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
            // Fresh run, same configuration. A restarted schedule
            // session becomes a manual session: it is no longer tied
            // to the schedule's window, so it is not ended at window
            // end and is not covered by the schedule's lock.
            s.schedule_id = None;
            s.force_quit = false;
            // Step 11: a fresh run has no extension history — its
            // policy must be applied anew before it can count as
            // a transient disconnect instead of fail-closed.
            s.policy_applied = false;
            s.browser_fail_closed = false;
            // Step 14: a restarted session is a fresh manual run —
            // it keeps no grace from its previous run.
            s.grace_until_ms = None;
            s.status = Status::Ready;
            s.started_at = None;
            s.ended_at = None;
            s.deadline_ms = None;
            s.remaining_ms = u64::from(s.duration_minutes) * 60_000;
            s.focused_ms = 0;
            s.report = Default::default();
            s.attention = Default::default();
            s.pomodoro = crate::pomodoro::initial_state(&s.focus.pomodoro);
            // A fresh run banks no credit and holds no break: the
            // previous run's reward state does not carry over.
            s.reward = Default::default();
            // Step 10: the lock configuration carries over (a
            // restarted session is the same block, re-armed), but
            // its live state is fresh — a new challenge, no
            // pending delay from the previous run.
            let lock = s.lock.clone();
            s.lock_state = crate::locks::new_state(&lock, &format!("{}:{now}", s.id));
            // A fresh run restores the camera the session was created with;
            // a mid-run "stop" was runtime state, not configuration.
            if s.focus.attention.verification == "camera_and_activity" {
                s.focus.camera = true;
            }
        }
        // Reward and delay breaks (step 9).
        Action::TakeRewardBreak { id } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            crate::rewards::request_break(s, now)?;
        }
        Action::CancelDelayedBreak { id } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            crate::rewards::cancel_pending(s, now)?;
        }
        // Delay-lock requests (step 10). Only a live session can
        // hold a pending delay, and only for a guarded action.
        Action::RequestLockDelay { id, for_action } => {
            if !["end", "pause", "delete"].contains(&for_action.as_str()) {
                return Err(
                    "A delay can only be requested for ending, pausing, or deleting.".into(),
                );
            }
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            if !matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            ) {
                return Err("Only a live session can have a delay request.".into());
            }
            // Step 12: a delay would be a false promise for a
            // frozen session — even an expired delay cannot unlock
            // it — so requesting one is rejected up front.
            if s.focus.frozen.enabled {
                return Err(
                    "Frozen sessions cannot be ended early, even after a delay. The lock ends when its timer ends."
                        .into(),
                );
            }
            crate::locks::request_delay(&s.lock, &mut s.lock_state, &for_action, now)?;
        }
        Action::CancelLockDelay { id } => {
            let s = data
                .sessions
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or("Session not found.")?;
            crate::locks::cancel_delay(&mut s.lock_state)?;
        }
        action => {
            let (id, target, expected, proofs) = match action {
                Action::StartSession { id } => (id, Status::Running, Some(Status::Ready), None),
                Action::PauseSession {
                    id,
                    password,
                    challenge_answer,
                } => (
                    id,
                    Status::Paused,
                    Some(Status::Running),
                    Some((password, challenge_answer, "pause", "pause it")),
                ),
                Action::ResumeSession { id } => (id, Status::Running, Some(Status::Paused), None),
                Action::EndSession {
                    id,
                    password,
                    challenge_answer,
                } => (
                    id,
                    Status::Ended,
                    None,
                    Some((password, challenge_answer, "end", "end it early")),
                ),
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
            // Step 14: ending during grace is free — the block has
            // not started enforcing yet, so the frozen gate, the
            // schedule time-range lock, the step-10 gate, and the
            // Strict-mode early-end rule below are all skipped for
            // an end in grace. Pausing keeps its existing gates,
            // and deleting a live session keeps its gates (end
            // first, then delete, during grace).
            let ending_in_grace = target == Status::Ended
                && crate::notifications::grace_active(s.grace_until_ms, now);
            // Time-range lock (step 5): while a locked schedule's
            // window is active, its session cannot be ended early
            // (or paused, which would suspend the block). Manual
            // sessions are unaffected: they have no schedule_id.
            if matches!(target, Status::Paused | Status::Ended) && !ending_in_grace {
                if let Some(schedule_id) = &s.schedule_id {
                    let locked = data.schedules.iter().any(|schedule| {
                        &schedule.id == schedule_id
                            && crate::schedules::locked_and_active(schedule, now)
                    });
                    if locked {
                        let end = data
                            .schedules
                            .iter()
                            .find(|schedule| &schedule.id == schedule_id)
                            .map(|schedule| schedule.end_time.clone())
                            .unwrap_or_default();
                        return Err(if target == Status::Paused {
                            format!(
                                "This scheduled block is locked until {end}. It cannot be paused."
                            )
                        } else {
                            format!(
                                "This scheduled block is locked until {end}. It cannot be ended early."
                            )
                        });
                    }
                }
            }
            // Step 12: a frozen session is implicitly maximally
            // locked. While live, pausing or ending it early is
            // rejected outright — before the step-10 gate, so no
            // password, challenge, or expired delay can unlock it.
            // There is no emergency unlock: only natural expiry
            // (timer, or schedule window end) lifts a frozen lock.
            if matches!(target, Status::Paused | Status::Ended)
                && !ending_in_grace
                && matches!(
                    s.status,
                    Status::Running | Status::Paused | Status::Preparing
                )
                && s.focus.frozen.enabled
            {
                return Err(if target == Status::Paused {
                    "This frozen session cannot be paused. It ends when its timer ends."
                } else {
                    "This frozen session cannot be ended early. It ends when its timer ends."
                }
                .into());
            }
            if expected.is_some_and(|expected| s.status != expected) {
                return Err("This session does not allow that action.".into());
            }
            // Step 10 gate: only while the session is live. A
            // Ready session's lock has not started yet (ending a
            // template is always allowed), and a finished session
            // has no lock left to check. The gate runs on the
            // live session itself, so an elapsed delay clears
            // its pending request as part of this successful
            // action and the commit persists it.
            if let Some((password, challenge_answer, for_action, verb)) = proofs {
                if !ending_in_grace
                    && matches!(
                        s.status,
                        Status::Running | Status::Paused | Status::Preparing
                    )
                {
                    crate::locks::check_gate(
                        &s.lock,
                        &mut s.lock_state,
                        "session",
                        verb,
                        for_action,
                        password.as_deref(),
                        challenge_answer.as_deref(),
                        now,
                    )?;
                }
            }
            if s.focus.mode != crate::focus::Mode::Standard
                && s.status != Status::Ready
                && !ending_in_grace
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
                    // Step 14: an ended session keeps no grace.
                    s.grace_until_ms = None;
                    // Step 9: ending a session ends any break and
                    // discards unspent credit.
                    crate::rewards::clear(s);
                    // Step 10: ending a session lifts its lock.
                    crate::locks::clear_state(&mut s.lock_state);
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
                lock: Default::default(),
                lock_password: None,
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
        assert!(apply(
            &mut d,
            Action::PauseSession {
                id,
                password: None,
                challenge_answer: None,
            },
            2000
        )
        .is_err());
    }
    #[test]
    fn paused_time_is_not_focus_time() {
        let mut d = data(true);
        advance(&mut d.sessions[0], 10_000, 11_000);
        let id = d.sessions[0].id.clone();
        apply(
            &mut d,
            Action::PauseSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            11_000,
        )
        .unwrap();
        restore(&mut d, 100_000);
        assert_eq!(d.sessions[0].remaining_ms, 50_000);
        apply(&mut d, Action::ResumeSession { id }, 100_000).unwrap();
        assert_eq!(d.sessions[0].deadline_ms, Some(150_000));
    }
    fn pomodoro_data() -> Snapshot {
        let mut d = Snapshot::default();
        let focus = crate::focus::FocusConfig {
            pomodoro: crate::pomodoro::PomodoroConfig {
                enabled: true,
                focus_minutes: 25,
                short_break_minutes: 5,
                long_break_minutes: 15,
                cycles_before_long_break: 2,
            },
            ..Default::default()
        };
        apply(
            &mut d,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 60,
                allow_pause: true,
                focus,
                lock: Default::default(),
                lock_password: None,
            },
            1000,
        )
        .unwrap();
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id }, 1000).unwrap();
        // Exercise the plain-timer path (attention off): the phase
        // loop, not the attention accounting, drives the phases here.
        d.sessions[0].focus.attention.enabled = false;
        d
    }
    #[test]
    fn pomodoro_break_time_never_counts_toward_target() {
        let mut d = pomodoro_data();
        let s = &mut d.sessions[0];
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::Focus);
        assert_eq!(s.pomodoro.phase_remaining_ms, 25 * 60_000);
        advance(s, 25 * 60_000, 25 * 60_000 + 1000);
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::ShortBreak);
        assert_eq!(s.remaining_ms, 35 * 60_000);
        assert_eq!(s.focused_ms, 25 * 60_000);
        // Five break minutes pass: the target does not move.
        advance(s, 5 * 60_000, 30 * 60_000 + 1000);
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::Focus);
        assert_eq!(s.remaining_ms, 35 * 60_000);
        assert_eq!(s.focused_ms, 25 * 60_000);
        // Ten more focus minutes count again.
        advance(s, 10 * 60_000, 40 * 60_000 + 1000);
        assert_eq!(s.remaining_ms, 25 * 60_000);
        assert_eq!(s.focused_ms, 35 * 60_000);
    }
    #[test]
    fn pomodoro_window_spanning_phases_completes_on_target() {
        let mut d = pomodoro_data();
        // One long window spans a focus phase and its break; the
        // leftover time then counts toward the next focus phase.
        advance(&mut d.sessions[0], 40 * 60_000, 40 * 60_000 + 1000);
        let s = &d.sessions[0];
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::Focus);
        assert_eq!(s.remaining_ms, 25 * 60_000);
        assert_eq!(s.focused_ms, 35 * 60_000);
        // The rest of the target finishes the session mid-focus: no
        // extra break is owed after completion.
        advance(&mut d.sessions[0], 60 * 60_000, 100 * 60_000 + 1000);
        let s = &d.sessions[0];
        assert_eq!(s.status, Status::Completed);
        assert_eq!(s.remaining_ms, 0);
        assert_eq!(s.focused_ms, 60 * 60_000);
    }
    #[test]
    fn pomodoro_restart_resets_phase_state() {
        let mut d = pomodoro_data();
        advance(&mut d.sessions[0], 25 * 60_000, 25 * 60_000 + 1000);
        assert_eq!(
            d.sessions[0].pomodoro.phase,
            crate::pomodoro::Phase::ShortBreak
        );
        assert_eq!(d.sessions[0].pomodoro.completed_focus_cycles, 1);
        // 5 min break + 25 min focus (2nd cycle, long break) + 15 min
        // long break + 10 min focus = the remaining 35 productive min.
        advance(&mut d.sessions[0], 55 * 60_000, 80 * 60_000 + 1000);
        assert_eq!(d.sessions[0].status, Status::Completed);
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::RestartSession { id }, 200_000).unwrap();
        let s = &d.sessions[0];
        assert_eq!(s.pomodoro.phase, crate::pomodoro::Phase::Focus);
        assert_eq!(s.pomodoro.phase_remaining_ms, 25 * 60_000);
        assert_eq!(s.pomodoro.completed_focus_cycles, 0);
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
                lock: Default::default(),
                lock_password: None,
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
        assert!(apply(
            &mut d,
            Action::PauseSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            2000
        )
        .is_err());
        assert!(apply(
            &mut d,
            Action::EndSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            2000
        )
        .is_err());
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
        assert!(apply(
            &mut d,
            Action::DeleteSession {
                id,
                password: None,
                challenge_answer: None,
            },
            2000
        )
        .is_err());
        assert_eq!(d.sessions[0].status, Status::Running);
    }
    /// A schedule whose window covers `now` (local time), built from
    /// the same clock the lock check uses, so the test is stable at
    /// any hour: the window starts up to 10 minutes ago and ends 50
    /// minutes from now (wrapping into an overnight window near
    /// midnight, which is still active in its evening half).
    fn active_window_schedule(now: i64, lock: bool) -> crate::schedules::Schedule {
        let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
        let start = minutes.saturating_sub(10);
        let end = (minutes + 50) % 1440;
        let end = if end == start { (end + 1) % 1440 } else { end };
        crate::schedules::Schedule {
            id: "sched".into(),
            title: "Block".into(),
            days: vec![weekday],
            start_time: format!("{:02}:{:02}", start / 60, start % 60),
            end_time: format!("{:02}:{:02}", end / 60, end % 60),
            focus: Default::default(),
            enabled: true,
            lock_changes: lock,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: now,
        }
    }
    fn schedule_with_session(now: i64, lock: bool) -> Snapshot {
        let mut d = Snapshot::default();
        let schedule = active_window_schedule(now, lock);
        apply(
            &mut d,
            Action::CreateSchedule {
                title: schedule.title.clone(),
                days: schedule.days.clone(),
                start_time: schedule.start_time.clone(),
                end_time: schedule.end_time.clone(),
                focus: schedule.focus.clone(),
                enabled: true,
                lock_changes: lock,
                lock: Default::default(),
                lock_password: None,
            },
            now,
        )
        .unwrap();
        // The engine would create this on window entry; the test
        // links it by hand to exercise the lock rules.
        apply(
            &mut d,
            Action::CreateSession {
                title: "Block".into(),
                minutes: 60,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            },
            now,
        )
        .unwrap();
        d.sessions[0].schedule_id = Some(d.schedules[0].id.clone());
        d.sessions[0].status = Status::Running;
        d
    }
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }
    #[test]
    fn schedules_validate_and_persist_in_snapshot() {
        let mut d = Snapshot::default();
        assert!(apply(
            &mut d,
            Action::CreateSchedule {
                title: "Block".into(),
                days: vec![],
                start_time: "09:00".into(),
                end_time: "10:00".into(),
                focus: Default::default(),
                enabled: true,
                lock_changes: false,
                lock: Default::default(),
                lock_password: None,
            },
            1,
        )
        .is_err());
        assert!(apply(
            &mut d,
            Action::CreateSchedule {
                title: "Block".into(),
                days: vec![1],
                start_time: "09:00".into(),
                end_time: "09:00".into(),
                focus: Default::default(),
                enabled: true,
                lock_changes: false,
                lock: Default::default(),
                lock_password: None,
            },
            1,
        )
        .is_err());
        apply(
            &mut d,
            Action::CreateSchedule {
                title: "Block".into(),
                days: vec![1, 3],
                start_time: "22:00".into(),
                end_time: "06:00".into(),
                focus: Default::default(),
                enabled: true,
                lock_changes: true,
                lock: Default::default(),
                lock_password: None,
            },
            1,
        )
        .unwrap();
        assert_eq!(d.schedules.len(), 1);
        assert_eq!(d.schedules[0].start_time, "22:00");
        assert!(d.schedules[0].lock_changes);
    }
    #[test]
    fn locked_schedule_rejects_end_edit_delete_and_disable() {
        let now = now_ms();
        let mut d = schedule_with_session(now, true);
        let schedule_id = d.schedules[0].id.clone();
        let session_id = d.sessions[0].id.clone();
        // Ending the locked schedule session early is rejected.
        assert!(apply(
            &mut d,
            Action::EndSession {
                id: session_id,
                password: None,
                challenge_answer: None,
            },
            now
        )
        .is_err());
        assert_eq!(d.sessions[0].status, Status::Running);
        // Editing, disabling (via update) and deleting are rejected.
        let update = Action::UpdateSchedule {
            id: schedule_id.clone(),
            title: "Renamed".into(),
            days: vec![1],
            start_time: "09:00".into(),
            end_time: "10:00".into(),
            focus: Default::default(),
            enabled: false,
            lock_changes: true,
            lock: None,
            lock_password: None,
            password: None,
            challenge_answer: None,
        };
        assert!(apply(&mut d, update, now).is_err());
        assert!(apply(
            &mut d,
            Action::DeleteSchedule {
                id: schedule_id.clone(),
                password: None,
                challenge_answer: None,
            },
            now
        )
        .is_err());
        assert_eq!(d.schedules.len(), 1);
        assert_eq!(d.schedules[0].title, "Block");
        // The same session ends fine once the lock is off.
        let mut unlocked = schedule_with_session(now, false);
        let unlocked_session = unlocked.sessions[0].id.clone();
        apply(
            &mut unlocked,
            Action::EndSession {
                id: unlocked_session,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(unlocked.sessions[0].status, Status::Ended);
    }
    fn frozen_focus() -> crate::focus::FocusConfig {
        crate::focus::FocusConfig {
            activity_consent: true,
            attention: crate::attention::Policy {
                work_apps: vec!["com.microsoft.VSCode".into()],
                ..Default::default()
            },
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec!["com.microsoft.VSCode".into()],
                allowed_websites: vec![],
            },
            ..Default::default()
        }
    }

    #[test]
    fn frozen_session_rejects_pause_end_delete_and_delay_while_live() {
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateSession {
                title: "Frozen".into(),
                minutes: 25,
                allow_pause: true,
                focus: frozen_focus(),
                lock: Default::default(),
                lock_password: None,
            },
            1000,
        )
        .unwrap();
        // Step 12: creation forces allow_pause off — a frozen
        // session is never pausable, whatever was asked for.
        assert!(!d.sessions[0].allow_pause);
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id: id.clone() }, 1000).unwrap();
        assert_eq!(d.sessions[0].status, Status::Running);
        for action in [
            Action::PauseSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            Action::EndSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            Action::DeleteSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            Action::RequestLockDelay {
                id: id.clone(),
                for_action: "end".into(),
            },
        ] {
            assert!(apply(&mut d, action, 2000).is_err());
        }
        assert_eq!(d.sessions[0].status, Status::Running);
        // A finished frozen session is no longer gated: it can be
        // deleted like any other template.
        d.sessions[0].status = Status::Completed;
        apply(
            &mut d,
            Action::DeleteSession {
                id,
                password: None,
                challenge_answer: None,
            },
            2000,
        )
        .unwrap();
        assert!(d.sessions.is_empty());
    }

    #[test]
    fn frozen_schedule_rejects_edit_and_delete_while_its_session_is_live() {
        let now = now_ms();
        let mut d = Snapshot::default();
        let schedule = active_window_schedule(now, false);
        apply(
            &mut d,
            Action::CreateSchedule {
                title: schedule.title.clone(),
                days: schedule.days.clone(),
                start_time: schedule.start_time.clone(),
                end_time: schedule.end_time.clone(),
                focus: frozen_focus(),
                enabled: true,
                lock_changes: false,
                lock: Default::default(),
                lock_password: None,
            },
            now,
        )
        .unwrap();
        apply(
            &mut d,
            Action::CreateSession {
                title: "Block".into(),
                minutes: 60,
                allow_pause: false,
                focus: frozen_focus(),
                lock: Default::default(),
                lock_password: None,
            },
            now,
        )
        .unwrap();
        d.sessions[0].schedule_id = Some(d.schedules[0].id.clone());
        d.sessions[0].status = Status::Running;
        let sched_id = d.schedules[0].id.clone();
        let focus = d.schedules[0].focus.clone();
        let err = apply(
            &mut d,
            Action::UpdateSchedule {
                id: sched_id.clone(),
                title: "Renamed".into(),
                days: vec![0],
                start_time: "09:00".into(),
                end_time: "10:00".into(),
                focus,
                enabled: true,
                lock_changes: false,
                lock: None,
                lock_password: None,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap_err();
        assert!(err.contains("frozen"), "unexpected error: {err}");
        assert_eq!(d.schedules[0].title, "Block");
        let err = apply(
            &mut d,
            Action::DeleteSchedule {
                id: sched_id,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap_err();
        assert!(err.contains("frozen"), "unexpected error: {err}");
        assert_eq!(d.schedules.len(), 1);
    }

    #[test]
    fn manual_sessions_are_unaffected_by_schedule_locks() {
        let now = now_ms();
        let mut d = schedule_with_session(now, true);
        // A manual session (no schedule_id) ends normally even while
        // a locked schedule window is active.
        apply(
            &mut d,
            Action::CreateSession {
                title: "Manual".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            },
            now,
        )
        .unwrap();
        let manual_id = d.sessions[1].id.clone();
        d.sessions[1].status = Status::Running;
        apply(
            &mut d,
            Action::EndSession {
                id: manual_id,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(d.sessions[1].status, Status::Ended);
        // An unlocked schedule can be edited, disabled and deleted.
        let schedule_id = d.schedules[0].id.clone();
        let mut unlocked = schedule_with_session(now, false);
        let unlocked_id = unlocked.schedules[0].id.clone();
        apply(
            &mut unlocked,
            Action::UpdateSchedule {
                id: unlocked_id.clone(),
                title: "Renamed".into(),
                days: vec![1],
                start_time: "09:00".into(),
                end_time: "10:00".into(),
                focus: Default::default(),
                enabled: false,
                lock_changes: false,
                lock: None,
                lock_password: None,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap();
        assert_eq!(unlocked.schedules[0].title, "Renamed");
        assert!(!unlocked.schedules[0].enabled);
        apply(
            &mut unlocked,
            Action::DeleteSchedule {
                id: unlocked_id,
                password: None,
                challenge_answer: None,
            },
            now,
        )
        .unwrap();
        assert!(unlocked.schedules.is_empty());
        let _ = schedule_id;
    }
    #[test]
    fn one_shots_validate_future_only_and_delete_by_id() {
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateOneShot {
                title: "Later".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                fire_at_ms: 5000,
            },
            1000,
        )
        .unwrap();
        assert_eq!(d.one_shots.len(), 1);
        assert_eq!(d.one_shots[0].minutes, 25);
        assert!(d.one_shots[0].allow_pause);
        // Past and present times are rejected, never stored.
        assert!(apply(
            &mut d,
            Action::CreateOneShot {
                title: "Past".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                fire_at_ms: 1000,
            },
            1000,
        )
        .is_err());
        assert_eq!(d.one_shots.len(), 1);
        let id = d.one_shots[0].id.clone();
        assert!(apply(
            &mut d,
            Action::DeleteOneShot {
                id: "missing".into()
            },
            1000
        )
        .is_err());
        apply(&mut d, Action::DeleteOneShot { id }, 1000).unwrap();
        assert!(d.one_shots.is_empty());
    }

    #[test]
    fn allowances_validate_edit_preserves_usage_and_delete_by_id() {
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateAllowance {
                title: "YouTube".into(),
                kind: crate::allowances::TargetKind::Site,
                target: "youtube.com".into(),
                budget_minutes: 30,
                period: crate::allowances::Period::Day,
                mode: crate::allowances::ResetMode::Fixed,
                enabled: true,
            },
            1000,
        )
        .unwrap();
        assert_eq!(d.allowances.len(), 1);
        // A path target is rejected, never stored.
        assert!(apply(
            &mut d,
            Action::CreateAllowance {
                title: "Shorts".into(),
                kind: crate::allowances::TargetKind::Site,
                target: "youtube.com/shorts".into(),
                budget_minutes: 30,
                period: crate::allowances::Period::Day,
                mode: crate::allowances::ResetMode::Fixed,
                enabled: true,
            },
            1000,
        )
        .is_err());
        assert_eq!(d.allowances.len(), 1);
        // Accrued usage survives an edit that changes the budget,
        // and the frontend-sent usage log is ignored in favour of
        // the stored one.
        crate::allowances::accrue(&mut d.allowances[0], 60_000, 10 * 60_000);
        let mut edited = d.allowances[0].clone();
        edited.budget_minutes = 60;
        edited.enabled = false;
        edited.usage.clear();
        // The edit time must not precede the accrued bucket, or the
        // edit-time prune (which drops buckets ahead of the clock)
        // would drop it — in production the edit time is wall-clock
        // time, always after past accrual.
        apply(
            &mut d,
            Action::UpdateAllowance { allowance: edited },
            120_000,
        )
        .unwrap();
        assert_eq!(d.allowances[0].budget_minutes, 60);
        assert!(!d.allowances[0].enabled);
        assert_eq!(d.allowances[0].usage.len(), 1);
        assert_eq!(
            crate::allowances::used_ms(&d.allowances[0], 120_000),
            10 * 60_000
        );
        let id = d.allowances[0].id.clone();
        assert!(apply(
            &mut d,
            Action::DeleteAllowance {
                id: "missing".into()
            },
            120_000
        )
        .is_err());
        apply(&mut d, Action::DeleteAllowance { id }, 120_000).unwrap();
        assert!(d.allowances.is_empty());
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
    fn reward_data(delay_minutes: u32) -> Snapshot {
        let mut d = Snapshot::default();
        let focus = crate::focus::FocusConfig {
            reward: crate::rewards::RewardConfig {
                enabled: true,
                earn_focus_minutes: 25,
                earn_break_minutes: 5,
                max_credit_minutes: 30,
                max_break_minutes: 30,
                delay_minutes,
            },
            ..Default::default()
        };
        apply(
            &mut d,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 120,
                allow_pause: true,
                focus,
                lock: Default::default(),
                lock_password: None,
            },
            1000,
        )
        .unwrap();
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id }, 1000).unwrap();
        // Exercise the plain-timer path (attention off): the
        // advance loop, not the attention accounting, drives the
        // reward state here; the engine tick covers attention
        // sessions with the same helpers.
        d.sessions[0].focus.attention.enabled = false;
        d
    }
    #[test]
    fn reward_advance_earns_spends_and_never_moves_target_on_break() {
        let mut d = reward_data(0);
        // 25 productive minutes earn 5 minutes of credit.
        advance(&mut d.sessions[0], 25 * 60_000, 25 * 60_000 + 1000);
        assert_eq!(d.sessions[0].reward.credit_ms, 5 * 60_000);
        assert_eq!(d.sessions[0].remaining_ms, 95 * 60_000);
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::TakeRewardBreak { id }, 2000).unwrap();
        assert!(crate::rewards::on_break(&d.sessions[0]));
        // Five break minutes: the target does not move, and the
        // credit is spent minute-for-minute.
        advance(&mut d.sessions[0], 5 * 60_000, 30 * 60_000 + 1000);
        assert!(!crate::rewards::on_break(&d.sessions[0]));
        assert_eq!(d.sessions[0].reward.credit_ms, 0);
        assert_eq!(d.sessions[0].remaining_ms, 95 * 60_000);
        // Focus counts again after the break.
        advance(&mut d.sessions[0], 10 * 60_000, 40 * 60_000 + 1000);
        assert_eq!(d.sessions[0].remaining_ms, 85 * 60_000);
    }
    #[test]
    fn reward_delay_counts_down_with_focus_then_starts_and_cancels() {
        let mut d = reward_data(5);
        advance(&mut d.sessions[0], 25 * 60_000, 25 * 60_000 + 1000);
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::TakeRewardBreak { id: id.clone() }, 2000).unwrap();
        assert!(crate::rewards::pending(&d.sessions[0]));
        // Three minutes pass: focus continues (target falls) and the
        // delay counts down, with no credit spent.
        advance(&mut d.sessions[0], 3 * 60_000, 28 * 60_000 + 1000);
        assert!(crate::rewards::pending(&d.sessions[0]));
        assert_eq!(d.sessions[0].reward.pending_delay_ms, 2 * 60_000);
        assert_eq!(d.sessions[0].remaining_ms, 92 * 60_000);
        assert_eq!(d.sessions[0].reward.credit_ms, 5 * 60_000);
        // The delay elapses: the break starts automatically.
        advance(&mut d.sessions[0], 2 * 60_000, 30 * 60_000 + 1000);
        assert!(!crate::rewards::pending(&d.sessions[0]));
        assert!(crate::rewards::on_break(&d.sessions[0]));
        // A second session: cancelling a pending break costs nothing.
        let mut e = reward_data(5);
        advance(&mut e.sessions[0], 25 * 60_000, 25 * 60_000 + 1000);
        let eid = e.sessions[0].id.clone();
        apply(&mut e, Action::TakeRewardBreak { id: eid.clone() }, 2000).unwrap();
        apply(&mut e, Action::CancelDelayedBreak { id: eid }, 3000).unwrap();
        assert!(!crate::rewards::pending(&e.sessions[0]));
        assert!(!crate::rewards::on_break(&e.sessions[0]));
        assert_eq!(e.sessions[0].reward.credit_ms, 5 * 60_000);
    }
    #[test]
    fn reward_end_session_discards_credit_and_restart_resets() {
        let mut d = reward_data(0);
        advance(&mut d.sessions[0], 25 * 60_000, 25 * 60_000 + 1000);
        assert_eq!(d.sessions[0].reward.credit_ms, 5 * 60_000);
        let id = d.sessions[0].id.clone();
        apply(
            &mut d,
            Action::EndSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            },
            2000,
        )
        .unwrap();
        assert_eq!(d.sessions[0].reward.credit_ms, 0);
        assert!(!crate::rewards::pending(&d.sessions[0]));
        apply(&mut d, Action::RestartSession { id }, 3000).unwrap();
        assert_eq!(d.sessions[0].reward.credit_ms, 0);
        assert_eq!(d.sessions[0].reward.earn_progress_ms, 0);
    }

    // Step 10 lock framework.
    fn locked_data() -> Snapshot {
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateSession {
                title: "Locked".into(),
                minutes: 60,
                allow_pause: true,
                focus: Default::default(),
                lock: crate::locks::LockConfig {
                    password_hash: None,
                    challenge_length: 16,
                    delay_minutes: 5,
                    restart_lock: true,
                },
                lock_password: Some("secret".into()),
            },
            1000,
        )
        .unwrap();
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id }, 1000).unwrap();
        d
    }
    fn end_with(
        d: &mut Snapshot,
        id: &str,
        password: Option<&str>,
        challenge: Option<&str>,
        now: i64,
    ) -> Result<(), String> {
        apply(
            d,
            Action::EndSession {
                id: id.into(),
                password: password.map(str::to_string),
                challenge_answer: challenge.map(str::to_string),
            },
            now,
        )
    }
    #[test]
    fn lock_blocks_end_until_every_gate_passes() {
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        let challenge = d.sessions[0].lock_state.challenge.clone().unwrap();
        assert_eq!(challenge.len(), 16);
        assert!(d.sessions[0].lock.password_hash.is_some());
        // No proofs: blocked.
        assert!(end_with(&mut d, &id, None, None, 2000).is_err());
        // Password alone: the challenge still blocks.
        assert!(end_with(&mut d, &id, Some("secret"), None, 2000).is_err());
        // Both proofs, but no delay request yet: blocked.
        assert!(end_with(&mut d, &id, Some("secret"), Some(&challenge), 2000).is_err());
        // Request the delay, then try too early: still blocked.
        apply(
            &mut d,
            Action::RequestLockDelay {
                id: id.clone(),
                for_action: "end".into(),
            },
            2000,
        )
        .unwrap();
        assert!(end_with(
            &mut d,
            &id,
            Some("secret"),
            Some(&challenge),
            2000 + 4 * 60_000
        )
        .is_err());
        // A delay for another action does not unlock ending: the
        // pending request is for "end", so a pause attempt fails
        // on its own missing request (and the session is Running,
        // so pause is otherwise valid).
        assert!(apply(
            &mut d,
            Action::PauseSession {
                id: id.clone(),
                password: Some("secret".into()),
                challenge_answer: Some(challenge.clone()),
            },
            2000 + 4 * 60_000,
        )
        .is_err());
        // At the delay's end, both proofs end the session, and
        // the lock state lifts with it.
        end_with(
            &mut d,
            &id,
            Some("secret"),
            Some(&challenge),
            2000 + 5 * 60_000,
        )
        .unwrap();
        assert_eq!(d.sessions[0].status, Status::Ended);
        assert_eq!(d.sessions[0].lock_state.challenge, None);
        assert_eq!(d.sessions[0].lock_state.delay_until_ms, None);
    }
    #[test]
    fn ending_during_grace_bypasses_lock_and_frozen_gates() {
        // Step 14: a block that has not started enforcing yet can
        // always be ended freely — no lock proof, no frozen
        // no-override, no delay wait. The grace flag clears with
        // the end, so the finished session keeps no grace.
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        assert!(end_with(&mut d, &id, None, None, 2000).is_err());
        d.sessions[0].grace_until_ms = Some(2000 + 60_000);
        end_with(&mut d, &id, None, None, 2000).unwrap();
        assert_eq!(d.sessions[0].status, Status::Ended);
        assert_eq!(d.sessions[0].grace_until_ms, None);
    }

    #[test]
    fn ending_after_grace_expiry_keeps_all_gates() {
        // The bypass is the grace window only: once it has passed,
        // the same locked session needs its proofs again.
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        d.sessions[0].grace_until_ms = Some(2000);
        assert!(end_with(&mut d, &id, None, None, 3000).is_err());
        assert_eq!(d.sessions[0].status, Status::Running);
    }

    #[test]
    fn frozen_session_in_grace_can_be_ended_freely() {
        // Step 14: a frozen session in grace is not yet frozen —
        // ending it is free, while the same session out of grace
        // keeps the step-12 no-override rule.
        let mut d = Snapshot::default();
        apply(
            &mut d,
            Action::CreateSession {
                title: "Frozen".into(),
                minutes: 25,
                allow_pause: true,
                focus: frozen_focus(),
                lock: Default::default(),
                lock_password: None,
            },
            1000,
        )
        .unwrap();
        let id = d.sessions[0].id.clone();
        apply(&mut d, Action::StartSession { id: id.clone() }, 1000).unwrap();
        assert!(end_with(&mut d, &id, None, None, 2000).is_err());
        d.sessions[0].grace_until_ms = Some(2000 + 60_000);
        end_with(&mut d, &id, None, None, 2000).unwrap();
        assert_eq!(d.sessions[0].status, Status::Ended);
    }

    #[test]
    fn lock_delay_can_be_cancelled_and_ready_sessions_are_ungated() {
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        let challenge = d.sessions[0].lock_state.challenge.clone().unwrap();
        apply(
            &mut d,
            Action::RequestLockDelay {
                id: id.clone(),
                for_action: "delete".into(),
            },
            2000,
        )
        .unwrap();
        apply(&mut d, Action::CancelLockDelay { id: id.clone() }, 3000).unwrap();
        // After cancelling, deleting needs a fresh delay request.
        assert!(apply(
            &mut d,
            Action::DeleteSession {
                id: id.clone(),
                password: Some("secret".into()),
                challenge_answer: Some(challenge.clone()),
            },
            4000,
        )
        .is_err());
        // A not-yet-started locked session can still be deleted:
        // its lock has not started yet. Create a second one and
        // delete it while Ready, with no proofs at all.
        apply(
            &mut d,
            Action::CreateSession {
                title: "Ready locked".into(),
                minutes: 10,
                allow_pause: true,
                focus: Default::default(),
                lock: crate::locks::LockConfig {
                    password_hash: None,
                    challenge_length: 16,
                    delay_minutes: 5,
                    restart_lock: true,
                },
                lock_password: Some("secret".into()),
            },
            5000,
        )
        .unwrap();
        let ready_id = d.sessions[1].id.clone();
        assert_eq!(d.sessions[1].status, Status::Ready);
        apply(
            &mut d,
            Action::DeleteSession {
                id: ready_id,
                password: None,
                challenge_answer: None,
            },
            6000,
        )
        .unwrap();
        assert_eq!(d.sessions.len(), 1);
    }
    #[test]
    fn lock_restart_keeps_config_but_refreshes_challenge() {
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        let challenge = d.sessions[0].lock_state.challenge.clone().unwrap();
        let hash = d.sessions[0].lock.password_hash.clone().unwrap();
        apply(
            &mut d,
            Action::RequestLockDelay {
                id: id.clone(),
                for_action: "end".into(),
            },
            2000,
        )
        .unwrap();
        end_with(
            &mut d,
            &id,
            Some("secret"),
            Some(&challenge),
            2000 + 5 * 60_000,
        )
        .unwrap();
        apply(&mut d, Action::RestartSession { id: id.clone() }, 400_000).unwrap();
        assert_eq!(d.sessions[0].status, Status::Ready);
        // Same lock configuration, fresh challenge, no pending delay.
        assert_eq!(d.sessions[0].lock.password_hash, Some(hash));
        assert!(d.sessions[0].lock.restart_lock);
        assert_ne!(d.sessions[0].lock_state.challenge, Some(challenge));
        assert_eq!(d.sessions[0].lock_state.delay_until_ms, None);
    }
    #[test]
    fn lock_persists_through_json_roundtrip_like_a_restart() {
        let mut d = locked_data();
        let id = d.sessions[0].id.clone();
        let challenge = d.sessions[0].lock_state.challenge.clone().unwrap();
        // Simulate a restart: the session JSON is saved and loaded
        // again, with no lock fields lost.
        let json = serde_json::to_string(&d.sessions[0]).unwrap();
        assert!(
            !json.contains("secret"),
            "plaintext password must never be stored"
        );
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.lock, d.sessions[0].lock);
        assert_eq!(restored.lock_state.challenge, Some(challenge.clone()));
        d.sessions[0] = restored;
        // The restored lock still gates early exit.
        assert!(end_with(&mut d, &id, None, None, 2000).is_err());
        assert!(end_with(&mut d, &id, Some("secret"), Some("wrong"), 2000).is_err());
    }
    #[test]
    fn schedule_lock_blocks_change_until_delay_and_proofs() {
        let now = now_ms();
        let mut d = Snapshot::default();
        let window = active_window_schedule(now, false);
        let (days, start_time, end_time) = (
            window.days.clone(),
            window.start_time.clone(),
            window.end_time.clone(),
        );
        apply(
            &mut d,
            Action::CreateSchedule {
                title: "Block".into(),
                days,
                start_time,
                end_time,
                focus: Default::default(),
                enabled: true,
                lock_changes: false,
                lock: crate::locks::LockConfig {
                    password_hash: None,
                    challenge_length: 0,
                    delay_minutes: 5,
                    restart_lock: false,
                },
                lock_password: Some("sched-secret".into()),
            },
            now,
        )
        .unwrap();
        let schedule_id = d.schedules[0].id.clone();
        assert!(crate::schedules::lock_gate_active(&d.schedules[0], now));
        let update = |d: &mut Snapshot, password: Option<&str>| {
            apply(
                d,
                Action::UpdateSchedule {
                    id: schedule_id.clone(),
                    title: "Renamed".into(),
                    days: vec![1],
                    start_time: "09:00".into(),
                    end_time: "10:00".into(),
                    focus: Default::default(),
                    enabled: true,
                    lock_changes: false,
                    lock: None,
                    lock_password: None,
                    password: password.map(str::to_string),
                    challenge_answer: None,
                },
                now,
            )
        };
        // No proofs and no delay: blocked. Password alone, still
        // no delay: blocked.
        assert!(update(&mut d, None).is_err());
        assert!(update(&mut d, Some("sched-secret")).is_err());
        // Request the delay, try too early with the password: blocked.
        apply(
            &mut d,
            Action::RequestScheduleLockDelay {
                id: schedule_id.clone(),
            },
            now,
        )
        .unwrap();
        assert!(update(&mut d, Some("sched-secret")).is_err());
        // After the delay, the password lets the change through,
        // and the password lock survives the edit (lock: None
        // preserves it).
        let _ = now;
        assert!(apply(
            &mut d,
            Action::UpdateSchedule {
                id: schedule_id.clone(),
                title: "Renamed".into(),
                days: vec![1],
                start_time: "09:00".into(),
                end_time: "10:00".into(),
                focus: Default::default(),
                enabled: true,
                lock_changes: false,
                lock: None,
                lock_password: None,
                password: Some("sched-secret".into()),
                challenge_answer: None,
            },
            now + 5 * 60_000,
        )
        .is_ok());
        assert!(d.schedules[0].lock.password_hash.is_some());
    }
}
