use crate::{
    clock, desktop_settings as native,
    engine::Engine,
    models::{Action, Preferences, Snapshot, Status},
    platform,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};
use tauri::Manager;

pub fn validate_preferences(mut p: Preferences) -> Result<Preferences, String> {
    if !["dark", "light", "system"].contains(&p.theme.as_str())
        || !(1..=480).contains(&p.default_minutes)
        || p.daily_goal > 1440
        || p.volume > 100
        || !(1..=60).contains(&p.break_minutes)
    {
        return Err("Check your time, appearance and sound choices.".into());
    }
    let time: Vec<_> = p.reminder_time.split(':').collect();
    if time.len() != 2
        || time[0].len() != 2
        || time[1].len() != 2
        || time[0].parse::<u32>().map_or(true, |h| h > 23)
        || time[1].parse::<u32>().map_or(true, |m| m > 59)
        || p.reminder_days.iter().any(|d| *d > 6)
        || (p.reminder && p.reminder_days.is_empty())
    {
        return Err("Choose a reminder time and at least one day.".into());
    }
    p.reminder_days.sort();
    p.reminder_days.dedup();
    if p.saved_setups.len() > 30 {
        return Err("Keep up to 30 saved setups.".into());
    }
    let mut ids = HashSet::new();
    for preset in &mut p.saved_setups {
        preset.name = preset.name.trim().to_string();
        if preset.name.is_empty()
            || preset.name.chars().count() > 80
            || preset.id.is_empty()
            || preset.id.len() > 100
            || !ids.insert(preset.id.clone())
            || !(1..=480).contains(&preset.minutes)
        {
            return Err("Give each setup a name and a valid focus time.".into());
        }
        // A saved setup carries choices, never permission or an active session.
        let mut config = preset.focus.clone();
        config.activity_consent = true;
        preset.focus = crate::focus::validate(config)?;
        preset.focus.activity_consent = false;
        preset.focus.camera = false;
    }
    Ok(p)
}
#[derive(Serialize, Deserialize)]
struct Backup {
    format: String,
    version: u32,
    data: Snapshot,
}
fn restore_backup(content: &str) -> Result<Snapshot, String> {
    let backup: Backup =
        serde_json::from_str(content).map_err(|_| "This is not a valid Preelock backup.")?;
    if !["preelock-backup", "vystriq-backup"].contains(&backup.format.as_str())
        || backup.version != 1
    {
        return Err("This backup version is not supported.".into());
    }
    let mut data = backup.data;
    if data.sessions.len() > 10_000 || data.tasks.len() > 10_000 {
        return Err("This backup is too large.".into());
    }
    data.preferences = validate_preferences(data.preferences)?;
    let mut ids = HashSet::new();
    let mut intervals = HashSet::new();
    for s in &mut data.sessions {
        if s.id.is_empty()
            || s.id.len() > 100
            || !ids.insert(s.id.clone())
            || s.title.is_empty()
            || s.title.chars().count() > 160
            || !(1..=480).contains(&s.duration_minutes)
            || s.focused_ms > 28_800_000
            || s.remaining_ms > 28_800_000
            || s.attention.intervals.iter().any(|i| {
                i.id.is_empty() || !intervals.insert(i.id.clone()) || i.duration_ms > 28_800_000
            })
        {
            return Err("The backup contains invalid session records.".into());
        }
        let mut focus = s.focus.clone();
        focus.activity_consent = true;
        crate::focus::validate(focus)?;
        if matches!(
            s.status,
            Status::Running | Status::Paused | Status::Preparing
        ) {
            s.status = Status::Interrupted;
            s.ended_at = Some(clock::now());
        }
        s.deadline_ms = None;
        s.focus.camera = false;
        s.focus.activity_consent = false;
        s.report.active_app.clear();
        s.report.camera_status = "off".into();
        s.report.enforcement = "Not active".into();
    }
    ids.clear();
    for t in &data.tasks {
        if t.id.is_empty()
            || t.id.len() > 100
            || !ids.insert(t.id.clone())
            || t.title.is_empty()
            || t.title.chars().count() > 160
        {
            return Err("The backup contains invalid tasks.".into());
        }
    }
    data.browser = None;
    Ok(data)
}
fn backup_json(data: Snapshot) -> Result<String, String> {
    serde_json::to_string_pretty(&Backup {
        format: "preelock-backup".into(),
        version: 1,
        data,
    })
    .map_err(|e| e.to_string())
}
fn csv_cell(s: &str) -> String {
    // Spreadsheet formula injection protection for user-written session titles.
    let value = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.into()
    };
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn csv(data: &Snapshot) -> String {
    let mut out="Session,Status,Started (Unix ms),Ended (Unix ms),Focus seconds,Target minutes,Timing method\r\n".to_string();
    for s in &data.sessions {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\r\n",
            csv_cell(&s.title),
            serde_json::to_string(&s.status).unwrap_or_default(),
            s.started_at.map(|n| n.to_string()).unwrap_or_default(),
            s.ended_at.map(|n| n.to_string()).unwrap_or_default(),
            s.focused_ms / 1000,
            s.duration_minutes,
            if s.focus.attention.enabled {
                "Focus checks"
            } else {
                "Legacy timer"
            }
        ));
    }
    out
}
#[derive(Default)]
pub struct Runtime {
    previous: Option<HashMap<String, Status>>,
    pub break_end: u64,
    pub offered: String,
    reminder_key: String,
    next_reminder: u64,
    tray_title: String,
}
impl Runtime {
    fn advance_break(&mut self, now: u64, active: bool) -> bool {
        if active {
            self.break_end = 0;
            self.offered.clear();
            return false;
        }
        if self.break_end > 0 && now >= self.break_end {
            self.break_end = 0;
            return true;
        }
        false
    }
    pub fn update(&mut self, app: &tauri::AppHandle, data: &Snapshot) {
        let p = &data.preferences;
        if let Some(previous) = &self.previous {
            for s in &data.sessions {
                if s.status == Status::Completed
                    && previous
                        .get(&s.id)
                        .is_some_and(|old| matches!(old, Status::Running | Status::Preparing))
                {
                    if p.session_alert {
                        native::notify("Focus complete", "You reached your target.");
                    }
                    if p.sound {
                        native::sound(p.volume);
                    }
                    if p.offer_break {
                        self.offered = s.id.clone();
                    }
                }
                if s.status == Status::Running
                    && previous
                        .get(&s.id)
                        .is_some_and(|old| matches!(old, Status::Ready | Status::Preparing))
                {
                    if p.session_start_alert {
                        native::notify("Focus started", "Your session timer is running.");
                    }
                    if p.session_start_sound {
                        native::sound(p.volume);
                    }
                }
            }
        }
        self.previous = Some(
            data.sessions
                .iter()
                .map(|s| (s.id.clone(), s.status.clone()))
                .collect(),
        );
        let active = data.sessions.iter().find(|s| {
            matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
        });
        let now = clock::continuous_ms();
        if self.advance_break(now, active.is_some()) {
            if p.break_alert {
                native::notify("Break finished", "Ready for your next session?");
            }
            if p.sound {
                native::sound(p.volume);
            }
        }
        if !p.offer_break {
            self.offered.clear();
        }
        let title = if p.menu_timer {
            active
                .map(|s| {
                    format!(
                        "{}:{:02}",
                        s.remaining_ms.div_ceil(1000) / 60,
                        s.remaining_ms.div_ceil(1000) % 60
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        if title != self.tray_title {
            if let Some(tray) = app.tray_by_id("focus-controller") {
                #[cfg(target_os = "windows")]
                let _ = tray.set_tooltip(Some(if title.is_empty() {
                    "Preelock".to_string()
                } else {
                    format!("Preelock · {title} left")
                }));
                #[cfg(not(target_os = "windows"))]
                let _ = tray.set_title(if title.is_empty() {
                    None
                } else {
                    Some(title.as_str())
                });
            }
            self.tray_title = title;
        }
        if p.reminder && now >= self.next_reminder {
            self.next_reminder = now + 15_000;
            if let Ok(info) = native::info() {
                let key = format!("{}:{}", info["date"], p.reminder_time);
                if info["time"].as_str() == Some(&p.reminder_time)
                    && info["day"]
                        .as_u64()
                        .is_some_and(|d| p.reminder_days.contains(&(d as u32)))
                    && self.reminder_key != key
                {
                    self.reminder_key = key;
                    if active.is_none() && self.break_end == 0 {
                        native::notify("Time to focus", "Choose a task when you’re ready.");
                    }
                }
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingsAction {
    Info,
    Status,
    Login { enabled: bool },
    AllowNotifications,
    PreviewSound { volume: u32 },
    TestBrowser,
    TestCamera,
    StopCameraTest,
    StartBreak,
    EndBreak,
    DismissBreak,
    ExportHistory,
    Backup,
    Restore { confirmation: String },
    DeleteHistory { confirmation: String },
    ResetPreferences,
}
pub fn run(app: &tauri::AppHandle, action: SettingsAction) -> Result<Value, String> {
    let engine = app.state::<Mutex<Engine>>();
    let runtime = app.state::<Mutex<Runtime>>();
    match action {
        SettingsAction::Info | SettingsAction::Status => {
            let mut info = if matches!(action, SettingsAction::Info) {
                native::info()?
            } else {
                json!({})
            };
            info["browser_test"] = json!(engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .browser_test_result());
            let runtime = runtime.lock().map_err(|_| "Settings unavailable.")?;
            info["break_remaining"] =
                json!(runtime.break_end.saturating_sub(clock::continuous_ms()));
            info["break_offer"] = json!(runtime.offered);
            info["camera_status"] = json!(platform::camera_status());
            info["platform"] = json!(platform::name());
            info["version"] = json!(app.package_info().version.to_string());
            Ok(info)
        }
        SettingsAction::Login { enabled } => {
            native::login(enabled)?;
            Ok(json!(null))
        }
        SettingsAction::AllowNotifications => {
            if !native::permission() {
                return Err(
                    "Notifications are blocked. Open notification settings in the Permissions guide to enable Preelock."
                        .into(),
                );
            }
            native::notify("Notifications ready", "Preelock can now send focus alerts.");
            Ok(json!(null))
        }
        SettingsAction::PreviewSound { volume } => {
            native::sound(volume);
            Ok(json!(null))
        }
        SettingsAction::TestBrowser => {
            engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .test_browser()?;
            Ok(json!(null))
        }
        SettingsAction::TestCamera => {
            engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .test_camera()?;
            Ok(json!(null))
        }
        SettingsAction::StopCameraTest => {
            engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .stop_test();
            Ok(json!(null))
        }
        SettingsAction::StartBreak => {
            let e = engine.lock().map_err(|_| "Session engine unavailable.")?;
            if e.has_active() {
                return Err("Finish your focus session first.".into());
            }
            let mut r = runtime.lock().map_err(|_| "Settings unavailable.")?;
            r.break_end =
                clock::continuous_ms() + u64::from(e.current().preferences.break_minutes) * 60_000;
            r.offered.clear();
            Ok(json!(null))
        }
        SettingsAction::EndBreak | SettingsAction::DismissBreak => {
            let mut r = runtime.lock().map_err(|_| "Settings unavailable.")?;
            r.break_end = 0;
            r.offered.clear();
            Ok(json!(null))
        }
        SettingsAction::ResetPreferences => {
            let mut e = engine.lock().map_err(|_| "Session engine unavailable.")?;
            let preferences = Preferences {
                saved_setups: e.current().preferences.saved_setups,
                ..Preferences::default()
            };
            e.execute(Action::UpdatePreferences { preferences })?;
            Ok(json!(null))
        }
        SettingsAction::DeleteHistory { confirmation } => {
            if confirmation != "DELETE HISTORY" {
                return Err("Type DELETE HISTORY to confirm.".into());
            }
            let mut e = engine.lock().map_err(|_| "Session engine unavailable.")?;
            let mut data = e.current();
            data.sessions.clear();
            e.replace_data(data)?;
            Ok(json!(null))
        }
        SettingsAction::ExportHistory | SettingsAction::Backup => {
            let export = matches!(action, SettingsAction::ExportHistory);
            let path = native::file(
                true,
                if export {
                    "Preelock-history.csv"
                } else {
                    "Preelock-backup.json"
                },
            )?;
            if path.is_empty() {
                return Ok(json!({"cancelled":true}));
            }
            let data = engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .backup()?;
            let content = if export {
                csv(&data)
            } else {
                backup_json(data)?
            };
            std::fs::write(&path, content).map_err(|e| e.to_string())?;
            Ok(json!({"saved":true}))
        }
        SettingsAction::Restore { confirmation } => {
            if confirmation != "RESTORE" {
                return Err("Type RESTORE to replace your workspace.".into());
            }
            if engine
                .lock()
                .map_err(|_| "Session engine unavailable.")?
                .has_active()
            {
                return Err("Finish your session before restoring a backup.".into());
            }
            let path = native::file(false, "")?;
            if path.is_empty() {
                return Ok(json!({"cancelled":true}));
            }
            if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 20_000_000 {
                return Err("Choose a backup smaller than 20 MB.".into());
            }
            let data = restore_backup(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?;
            let mut e = engine.lock().map_err(|_| "Session engine unavailable.")?;
            if e.has_active() {
                return Err("Finish your session before restoring a backup.".into());
            }
            let recovery = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join(format!("before-restore-{}.json", clock::now()));
            std::fs::write(recovery, backup_json(e.backup()?)?).map_err(|e| e.to_string())?;
            e.replace_data(data)?;
            let mut r = runtime.lock().map_err(|_| "Settings unavailable.")?;
            *r = Runtime::default();
            Ok(json!({"restored":true}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_preferences_load_with_defaults() {
        let p: Preferences =
            serde_json::from_str(r#"{"theme":"light","default_minutes":50}"#).unwrap();
        assert_eq!(p.break_minutes, 5);
        assert!(!p.reminder);
        assert!(p.session_alert);
    }
    #[test]
    fn rejects_invalid_preferences() {
        let p = Preferences {
            volume: 101,
            ..Preferences::default()
        };
        assert!(validate_preferences(p).is_err());
        let p = Preferences {
            reminder: true,
            reminder_days: vec![],
            ..Preferences::default()
        };
        assert!(validate_preferences(p).is_err());
        let p = Preferences {
            reminder_time: "25:00".into(),
            ..Preferences::default()
        };
        assert!(validate_preferences(p).is_err());
    }
    #[test]
    fn backup_roundtrip_and_invalid_version() {
        let data = Snapshot::default();
        let json = backup_json(data).unwrap();
        assert_eq!(
            restore_backup(&json).unwrap().preferences.default_minutes,
            25
        );
        assert!(restore_backup(&json.replace("\"version\": 1", "\"version\": 9")).is_err());
        assert!(restore_backup("{}").is_err());
    }
    #[test]
    fn spreadsheet_titles_are_escaped() {
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
    }
    #[test]
    fn break_finishes_once_and_new_focus_cancels_it() {
        let mut runtime = Runtime {
            break_end: 1000,
            ..Runtime::default()
        };
        assert!(!runtime.advance_break(999, false));
        assert!(runtime.advance_break(1000, false));
        assert!(!runtime.advance_break(2000, false));
        runtime.break_end = 3000;
        runtime.offered = "session".into();
        assert!(!runtime.advance_break(2000, true));
        assert_eq!(runtime.break_end, 0);
        assert!(runtime.offered.is_empty());
    }
    #[test]
    fn restored_sessions_never_start_camera_or_blocks() {
        let mut data = Snapshot::default();
        crate::sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Work".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
            },
            1,
        )
        .unwrap();
        data.sessions[0].status = Status::Running;
        data.sessions[0].focus.camera = true;
        data.sessions[0].focus.activity_consent = true;
        let restored = restore_backup(&backup_json(data).unwrap()).unwrap();
        assert_eq!(restored.sessions[0].status, Status::Interrupted);
        assert!(!restored.sessions[0].focus.camera);
        assert!(!restored.sessions[0].focus.activity_consent);
    }
}
