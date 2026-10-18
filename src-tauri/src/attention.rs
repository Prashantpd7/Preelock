//! Conservative, deterministic productive-time accounting. No input-rate scoring or comprehension claims.
use crate::{
    focus::FocusConfig,
    models::{Session, Status},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    #[default]
    Idle,
    Preparing,
    Focused,
    Distracted,
    Absent,
    Unknown,
    Recovering,
    Completed,
    EmergencyExit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub enabled: bool,
    pub task: String,
    pub verification: String,
    pub grace_seconds: u32,
    pub camera_fallback: String,
    pub work_apps: Vec<String>,
    pub work_domains: Vec<String>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: false,
            task: "other".into(),
            verification: "manual_checkpoints".into(),
            grace_seconds: 10,
            camera_fallback: "unknown".into(),
            work_apps: vec![],
            work_domains: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Interval {
    pub id: String,
    pub at: i64,
    pub duration_ms: u64,
    pub state: State,
    pub reason: String,
    pub corrected_ms: u64,
    pub correction_note: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tracking {
    pub state: State,
    pub reason: String,
    pub elapsed_ms: u64,
    pub distracted_ms: u64,
    pub absent_ms: u64,
    pub unknown_ms: u64,
    pub corrected_ms: u64,
    pub checkpoint_at: Option<i64>,
    pub intervals: Vec<Interval>,
    pub candidate: State,
    pub candidate_ms: u64,
}
#[derive(Default)]
pub struct Signals {
    pub activity_available: bool,
    pub approved_app: bool,
    pub idle_ms: u64,
    pub browser_relevant: Option<bool>,
    pub browser_required: bool,
    pub camera_status: String,
    pub manual_confirmed: bool,
}
pub fn classify(config: &FocusConfig, signal: &Signals) -> (State, String) {
    let p = &config.attention;
    if p.verification == "manual_checkpoints" {
        return if signal.manual_confirmed {
            (State::Focused, "Work confirmed by you.".into())
        } else {
            (
                State::Unknown,
                "Click ‘I’m working’ to count the next minute.".into(),
            )
        };
    }
    if !config.activity_consent || !signal.activity_available {
        return (
            State::Unknown,
            "App activity is unavailable. Check your session permission.".into(),
        );
    }
    if !signal.approved_app {
        return (
            State::Distracted,
            "Switch back to one of your work apps.".into(),
        );
    }
    if let Some(relevant) = signal.browser_relevant {
        if !relevant {
            return (State::Distracted, "Open one of your work websites.".into());
        }
    } else if signal.browser_required {
        return (
            State::Unknown,
            "Waiting for your browser. Check the extension connection.".into(),
        );
    }
    if p.verification == "camera_and_activity" {
        match signal.camera_status.as_str() {
            "person_detected" => {}
            "extended_absence" if signal.idle_ms >= u64::from(p.grace_seconds) * 1000 => {
                return (
                    State::Absent,
                    "You seem to be away. Time will resume when you return.".into(),
                )
            }
            "person_not_detected" | "extended_absence" => {
                return (
                    State::Unknown,
                    "Camera view is unclear. Timer paused.".into(),
                )
            }
            _ if p.camera_fallback == "activity_only" => {}
            _ => return (State::Unknown, "Camera unavailable. Timer paused.".into()),
        }
    }
    let idle_limit = match p.task.as_str() {
        "reading" | "lecture" => 20 * 60_000,
        _ => 5 * 60_000,
    };
    if signal.idle_ms > idle_limit && signal.camera_status != "person_detected" {
        return (
            State::Unknown,
            "No recent activity. Waiting for a fresh focus check.".into(),
        );
    }
    (
        State::Focused,
        if p.verification == "camera_and_activity" && signal.camera_status != "person_detected" {
            "Camera off. Counting your work app activity."
        } else {
            "Working in a chosen app. Focus is estimated."
        }
        .into(),
    )
}
fn record(s: &mut Session, delta: u64, state: State, reason: String, now: i64) {
    let t = &mut s.attention;
    if state != t.state {
        s.report
            .event(now, "attention_transition", &format!("{state:?}: {reason}"));
    }
    t.state = state.clone();
    t.reason = reason.clone();
    t.elapsed_ms = t.elapsed_ms.saturating_add(delta);
    t.checkpoint_at = Some(now);
    match state {
        State::Distracted => t.distracted_ms += delta,
        State::Absent => t.absent_ms += delta,
        State::Focused => {}
        _ => t.unknown_ms += delta,
    }
    // Coalesce bounded intervals; committed intervals also live in their own indexed SQLite table.
    if let Some(last) = t.intervals.last_mut() {
        if last.state == state
            && last.reason == reason
            && last.corrected_ms == 0
            && last.duration_ms + delta <= 60_000
        {
            last.duration_ms += delta;
            return;
        }
    }
    t.intervals.push(Interval {
        id: uuid::Uuid::new_v4().to_string(),
        at: now.saturating_sub(delta as i64),
        duration_ms: delta,
        state,
        reason,
        corrected_ms: 0,
        correction_note: String::new(),
    });
    if t.intervals.len() > 300 {
        t.intervals.remove(0);
    }
}
pub fn observe(s: &mut Session, signal: &Signals, delta: u64, now: i64) {
    if s.status != Status::Running || !s.focus.attention.enabled {
        return;
    }
    if s.remaining_ms == 0 {
        complete(s, now);
        return;
    }
    if delta > 3000 {
        s.attention.candidate = State::Recovering;
        s.attention.candidate_ms = 0;
        record(
            s,
            delta,
            State::Recovering,
            "Monitoring stopped or the Mac slept. This time was not counted.".into(),
            now,
        );
        return;
    }
    let (candidate, reason) = classify(&s.focus, signal);
    if s.attention.candidate == candidate {
        s.attention.candidate_ms = s.attention.candidate_ms.saturating_add(delta);
    } else {
        s.attention.candidate = candidate.clone();
        s.attention.candidate_ms = delta;
    }
    let threshold = if candidate == State::Focused {
        2000
    } else {
        u64::from(s.focus.attention.grace_seconds) * 1000
    };
    let settled = s.attention.candidate_ms >= threshold || candidate == State::Unknown;
    let state = if settled { candidate } else { State::Unknown };
    let accepted = if state == State::Focused {
        delta.min(s.remaining_ms)
    } else {
        delta
    };
    record(
        s,
        accepted,
        state.clone(),
        if settled {
            reason
        } else {
            "Checking focus… Timer paused briefly.".into()
        },
        now,
    );
    if state == State::Focused {
        s.focused_ms += accepted;
        s.remaining_ms -= accepted;
    }
    s.deadline_ms = None;
    if s.remaining_ms == 0 {
        complete(s, now);
    }
}
pub fn complete(s: &mut Session, now: i64) {
    s.status = Status::Completed;
    s.ended_at = Some(now);
    s.deadline_ms = None;
    s.attention.state = State::Completed;
    s.attention.reason = "Productive target reached".into();
    s.report.event(
        now,
        "productive_target_reached",
        "Accepted productive target reached; releasing restrictions.",
    );
}
pub fn recover(s: &mut Session, now: i64) {
    if !s.focus.attention.enabled || !matches!(s.status, Status::Running | Status::Preparing) {
        return;
    }
    if let Some(last) = s.attention.checkpoint_at {
        let gap = now.saturating_sub(last).max(0) as u64;
        if gap > 0 {
            record(
                s,
                gap,
                State::Recovering,
                "Process/restart gap: no productive time awarded".into(),
                now,
            );
        }
    }
    s.attention.state = State::Recovering;
    s.attention.reason = "Rechecking fresh signals after restart".into();
    s.attention.candidate_ms = 0;
    s.deadline_ms = None;
}
pub fn correct(s: &mut Session, interval_id: &str, note: &str, now: i64) -> Result<(), String> {
    if note.trim().len() < 3 || note.len() > 240 {
        return Err("Explain the correction in 3–240 characters.".into());
    }
    if !s.focus.attention.enabled || s.status == Status::Completed {
        return Err("This session cannot accept corrections.".into());
    }
    let i = s
        .attention
        .intervals
        .iter_mut()
        .find(|i| i.id == interval_id)
        .ok_or("Interval is no longer in the recent review window.")?;
    if i.state == State::Focused || i.corrected_ms > 0 {
        return Err("This interval was already accepted.".into());
    }
    // Offline gaps are deliberately not inferred even through a one-click correction.
    if i.state == State::Recovering {
        return Err("Monitoring gaps cannot be credited; resume with fresh observations.".into());
    }
    let credit = i.duration_ms.min(s.remaining_ms);
    if credit == 0 {
        return Err("No remaining target time to correct.".into());
    }
    i.corrected_ms = credit;
    i.correction_note = note.trim().into();
    s.attention.corrected_ms += credit;
    s.focused_ms += credit;
    s.remaining_ms -= credit;
    s.report.event(
        now,
        "manual_correction",
        &format!("Accepted {credit} ms by user review: {}", note.trim()),
    );
    if s.remaining_ms == 0 && s.status == Status::Running {
        complete(s, now);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        serde_json::from_value(serde_json::json!({"id":"test","title":"Study","duration_minutes":1,"remaining_ms":60000,"focused_ms":0,"allow_pause":true,"status":"running","started_at":0,"ended_at":null,"deadline_ms":null})).unwrap()
    }
    fn configured() -> Session {
        let mut s = session();
        s.focus.activity_consent = true;
        s.focus.attention = Policy {
            enabled: true,
            verification: "activity_only".into(),
            grace_seconds: 5,
            ..Default::default()
        };
        s
    }
    #[test]
    fn productive_target_does_not_count_distraction_or_sleep() {
        let mut s = configured();
        let good = Signals {
            activity_available: true,
            approved_app: true,
            ..Default::default()
        };
        observe(&mut s, &good, 1000, 1000);
        observe(&mut s, &good, 1000, 2000);
        assert_eq!(s.focused_ms, 1000);
        let bad = Signals {
            activity_available: true,
            ..Default::default()
        };
        for t in 3..12 {
            observe(&mut s, &bad, 1000, t * 1000);
        }
        assert_eq!(s.focused_ms, 1000);
        assert_eq!(s.attention.state, State::Distracted);
        observe(&mut s, &good, 120_000, 132000);
        assert_eq!(s.focused_ms, 1000);
        assert_eq!(s.attention.state, State::Recovering);
        assert!(s.attention.elapsed_ms > 120000);
    }
    #[test]
    fn quiet_reading_and_camera_conflict_are_not_distraction() {
        let mut s = configured();
        s.focus.attention.task = "reading".into();
        let mut signal = Signals {
            activity_available: true,
            approved_app: true,
            idle_ms: 600000,
            ..Default::default()
        };
        assert_eq!(classify(&s.focus, &signal).0, State::Focused);
        s.focus.attention.verification = "camera_and_activity".into();
        signal.camera_status = "extended_absence".into();
        signal.idle_ms = 0;
        assert_eq!(classify(&s.focus, &signal).0, State::Unknown);
        signal.idle_ms = 600000;
        assert_eq!(classify(&s.focus, &signal).0, State::Absent);
    }
    #[test]
    fn missing_camera_requires_explicit_fallback() {
        let mut s = configured();
        s.focus.attention.verification = "camera_and_activity".into();
        let signal = Signals {
            activity_available: true,
            approved_app: true,
            ..Default::default()
        };
        assert_eq!(classify(&s.focus, &signal).0, State::Unknown);
        s.focus.attention.camera_fallback = "activity_only".into();
        assert_eq!(classify(&s.focus, &signal).0, State::Focused);
    }
    #[test]
    fn recovery_never_awards_offline_time() {
        let mut s = configured();
        s.attention.checkpoint_at = Some(1000);
        recover(&mut s, 61000);
        assert_eq!(s.focused_ms, 0);
        assert_eq!(s.remaining_ms, 60000);
        assert_eq!(s.attention.unknown_ms, 60000);
    }
    #[test]
    fn review_is_audited_and_cannot_double_credit_or_credit_gaps() {
        let mut s = configured();
        record(&mut s, 5000, State::Unknown, "Uncertain".into(), 5000);
        let id = s.attention.intervals[0].id.clone();
        correct(&mut s, &id, "I was reading notes", 5001).unwrap();
        assert_eq!(s.focused_ms, 5000);
        assert!(correct(&mut s, &id, "Again", 5002).is_err());
        record(&mut s, 5000, State::Recovering, "Gap".into(), 10000);
        let id = s.attention.intervals.last().unwrap().id.clone();
        assert!(correct(&mut s, &id, "Reading", 10001).is_err());
    }
    #[test]
    fn target_completion_caps_credit_and_manual_policy_is_explicit() {
        let mut s = configured();
        s.remaining_ms = 500;
        s.focus.attention.verification = "manual_checkpoints".into();
        assert_eq!(classify(&s.focus, &Signals::default()).0, State::Unknown);
        s.attention.candidate = State::Focused;
        s.attention.candidate_ms = 2000;
        observe(
            &mut s,
            &Signals {
                manual_confirmed: true,
                ..Default::default()
            },
            1000,
            1000,
        );
        assert_eq!(s.focused_ms, 500);
        assert_eq!(s.status, Status::Completed);
    }
}
