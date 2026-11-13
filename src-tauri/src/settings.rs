use crate::{
    clock, desktop_settings as native,
    ipc::IpcClient,
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

/// M6: marker file recording that the USER enabled the "Start
/// Preelock at login" preference themselves (the Settings toggle),
/// kept separate from the automatic login registration that
/// `sync_login_for_sessions` writes for an active session or an
/// enabled schedule. The marker is the only input that lets a
/// login launch show the main window when no session is live (see
/// `quit::login_window_action`); the automatic registration alone
/// never shows it. Written/removed only by the `Login` settings
/// action below, never by the automatic sync. A missing file means
/// the user never enabled the preference in the app (enabling it
/// directly in System Settings is not visible here — that launch
/// starts hidden once, and the tray reopens it).
pub const LOGIN_USER_PREF_FILE: &str = "login-user-enabled";

pub fn login_user_pref_enabled(data_dir: &std::path::Path) -> bool {
    data_dir.join(LOGIN_USER_PREF_FILE).exists()
}

pub fn set_login_user_pref(data_dir: &std::path::Path, enabled: bool) -> Result<(), String> {
    let path = data_dir.join(LOGIN_USER_PREF_FILE);
    if enabled {
        std::fs::write(&path, "1").map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

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
    // Step 14: warning points normalize on save (deduped, sorted);
    // an invalid list is a save error, never a silent default.
    // Grace is a plain range check (0 = off).
    let warnings = crate::notifications::parse_warning_minutes(&p.warning_minutes)?;
    p.warning_minutes = crate::notifications::format_warning_minutes(&warnings);
    if p.grace_seconds > 300 {
        return Err("Grace period must be from 0 to 300 seconds (0 turns it off).".into());
    }
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
    // Screen Time category corrections: keep only well-known
    // categories with sane keys. A bad value must never fail a save —
    // it is dropped, and the built-in mapping applies instead.
    p.category_overrides.retain(|key, value| {
        key.len() <= 500 && matches!(value.as_str(), "Distracting" | "Productive" | "Others")
    });
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionTransition {
    pub completed: bool,
    pub started: bool,
}

pub(crate) fn session_transition(
    previous: &HashMap<String, (Status, bool)>,
    id: &str,
    status: &Status,
    in_grace: bool,
) -> SessionTransition {
    let prev = previous.get(id);
    let completed = *status == Status::Completed
        && prev.is_some_and(|(old, _)| matches!(old, Status::Running | Status::Preparing));
    let started = *status == Status::Running
        && !in_grace
        && match prev {
            // Scheduled and one-shot sessions enter Running directly
            // without passing through Ready or Preparing.
            None => true,
            Some((old, old_in_grace)) => {
                matches!(old, Status::Ready | Status::Preparing)
                    || (*old == Status::Running && *old_in_grace)
            }
        };
    SessionTransition { completed, started }
}

#[derive(Default)]
pub struct Runtime {
    previous: Option<HashMap<String, (Status, bool)>>,
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
                let transition =
                    session_transition(previous, &s.id, &s.status, s.grace_until_ms.is_some());
                if transition.completed {
                    if p.notifications_enabled && p.session_alert {
                        native::notify("Focus complete", "You reached your target.");
                        if p.sound {
                            native::sound(p.volume);
                        }
                    }
                    if p.offer_break {
                        self.offered = s.id.clone();
                    }
                }
                if transition.started && p.notifications_enabled && p.session_start_alert {
                    native::notify("Focus started", "Your session timer is running.");
                    if p.session_start_sound {
                        native::sound(p.volume);
                    }
                }
            }
        }
        self.previous = Some(
            data.sessions
                .iter()
                .map(|s| (s.id.clone(), (s.status.clone(), s.grace_until_ms.is_some())))
                .collect(),
        );
        let active = data.sessions.iter().find(|s| {
            matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
        });
        let now = clock::continuous_ms();
        if self.advance_break(now, active.is_some()) && p.notifications_enabled && p.break_alert {
            native::notify("Break finished", "Ready for your next session?");
            if p.notifications_sound {
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
                let _ = tray.set_tooltip(Some(if p.menu_timer {
                    crate::tray::tooltip_text(crate::tray::live_session(data), now as i64)
                } else {
                    "Preelock".to_string()
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
                    if active.is_none() && self.break_end == 0 && p.notifications_enabled {
                        native::notify("Time to focus", "Choose a task when you’re ready.");
                        if p.notifications_sound {
                            native::sound(p.volume);
                        }
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
    ReconnectBrowser,
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
    let client = app.state::<IpcClient>();
    let runtime = app.state::<Mutex<Runtime>>();
    match action {
        SettingsAction::Info | SettingsAction::Status => {
            let mut info = if matches!(action, SettingsAction::Info) {
                native::info()?
            } else {
                json!({})
            };
            info["browser_test"] = json!(client.browser_test_result()?);
            let runtime = runtime.lock().map_err(|_| "Settings unavailable.")?;
            info["break_remaining"] =
                json!(runtime.break_end.saturating_sub(clock::continuous_ms()));
            info["break_offer"] = json!(runtime.offered);
            info["camera_status"] = json!(platform::camera_status());
            info["platform"] = json!(platform::name());
            info["version"] = json!(app.package_info().version.to_string());
            // The user's own "Open at login" preference (the Settings
            // toggle), recorded in the `login-user-enabled` marker
            // file — separate from the OS registration state in
            // `info["login"]`, which also reflects live sessions and
            // enabled schedules. The toggle shows this value.
            if let Ok(dir) = app.path().app_data_dir() {
                info["login_user_pref"] = json!(login_user_pref_enabled(&dir));
            }
            Ok(info)
        }
        SettingsAction::Login { enabled } => {
            if enabled && cfg!(debug_assertions) {
                return Err("Open at login is available in the installed app.".into());
            }
            // Record the user's own choice first (see the marker docs
            // above). A failure to write the marker is logged, not
            // fatal: the decision below then reads the marker that is
            // actually on disk.
            if let Ok(dir) = app.path().app_data_dir() {
                if let Err(e) = set_login_user_pref(&dir, enabled) {
                    log::warn!("Could not record open-at-login preference: {e}");
                }
            }
            // Apply the single login rule (user preference OR live
            // session OR enabled schedule) in one OS call, so turning
            // the toggle OFF while a strict session is live never
            // unregisters the login item: mandatory recovery after a
            // shutdown still works. Errors propagate so Settings can
            // show the real saved/error state.
            // In dev mode the decision is not applied (the debug
            // binary cannot be registered); turning the preference off
            // still cleans up a stale registration, as before.
            if cfg!(debug_assertions) {
                if !enabled {
                    native::login(false)?;
                }
            } else {
                native::login(crate::login_registration_decision(app))?;
            }
            Ok(json!(null))
        }
        SettingsAction::AllowNotifications => {
            if !native::permission() {
                return Err(
                    "Notifications are blocked. Open notification settings in the Permissions guide to enable Preelock."
                        .into(),
                );
            }
            let (sound, volume) = client
                .ping()
                .map(|p| {
                    let (_, s, v) = p.notification_prefs;
                    (s, v)
                })
                .unwrap_or((true, 50));
            native::notify("Notifications ready", "Preelock can now send focus alerts.");
            if sound {
                native::sound(volume);
            }
            Ok(json!(null))
        }
        SettingsAction::PreviewSound { volume } => {
            native::sound(volume);
            Ok(json!(null))
        }
        SettingsAction::TestBrowser => {
            client.test_browser_connection()?;
            Ok(json!(null))
        }
        SettingsAction::ReconnectBrowser => {
            client.refresh_browser_connection()?;
            Ok(json!({ "reconnected": true }))
        }
        SettingsAction::TestCamera => {
            client.test_camera()?;
            Ok(json!(null))
        }
        SettingsAction::StopCameraTest => {
            client.stop_test()?;
            Ok(json!(null))
        }
        SettingsAction::StartBreak => {
            let snap = client.snapshot()?;
            let has_active = snap.sessions.iter().any(|s| {
                matches!(
                    s.status,
                    Status::Running | Status::Paused | Status::Preparing
                )
            });
            if has_active {
                return Err("Finish your focus session first.".into());
            }
            let mut r = runtime.lock().map_err(|_| "Settings unavailable.")?;
            r.break_end =
                clock::continuous_ms() + u64::from(snap.preferences.break_minutes) * 60_000;
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
            let snap = client.snapshot()?;
            let preferences = Preferences {
                saved_setups: snap.preferences.saved_setups,
                ..Preferences::default()
            };
            client.execute(Action::UpdatePreferences { preferences })?;
            Ok(json!(null))
        }
        SettingsAction::DeleteHistory { confirmation } => {
            if confirmation != "DELETE HISTORY" {
                return Err("Type DELETE HISTORY to confirm.".into());
            }
            let mut data = client.snapshot()?;
            data.sessions.clear();
            client.replace_data(data)?;
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
            let data = client.backup()?;
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
            if client.ping().is_ok_and(|p| p.has_active) {
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
            if client.ping().is_ok_and(|p| p.has_active) {
                return Err("Finish your session before restoring a backup.".into());
            }
            let recovery = app
                .path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join(format!("before-restore-{}.json", clock::now()));
            std::fs::write(recovery, backup_json(client.backup()?)?).map_err(|e| e.to_string())?;
            client.replace_data(data)?;
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
        // Item 3: the menu-bar timer and both session sounds default
        // on for saves made before those fields existed.
        assert!(p.menu_timer);
        assert!(p.sound);
        assert!(p.session_start_sound);
        assert!(p.notifications_sound);
        assert!(p.session_start_alert);
        assert_eq!(p.warning_minutes, "2");
    }
    #[test]
    fn saved_off_preferences_stay_off() {
        // Item 3: a stored false is never overridden by the new
        // defaults — defaults only fill in missing fields.
        let p: Preferences = serde_json::from_str(
            r#"{"theme":"light","default_minutes":50,"menu_timer":false,"sound":false,"session_start_sound":false,"notifications_sound":false,"session_start_alert":false,"warning_minutes":"5"}"#,
        )
        .unwrap();
        assert!(!p.menu_timer);
        assert!(!p.sound);
        assert!(!p.session_start_sound);
        assert!(!p.notifications_sound);
        assert!(!p.session_start_alert);
        assert_eq!(p.warning_minutes, "5");
    }
    #[test]
    fn session_transition_detects_scheduled_and_grace_starts() {
        let previous = HashMap::new();
        assert!(session_transition(&previous, "sched-1", &Status::Running, false).started);

        // Inside grace: wait for grace end before firing "Focus started".
        assert!(!session_transition(&previous, "sched-1", &Status::Running, true).started);

        // Exiting grace triggers start notification; steady running does not.
        let mut after_grace_prev = HashMap::new();
        after_grace_prev.insert("sched-1".into(), (Status::Running, true));
        assert!(session_transition(&after_grace_prev, "sched-1", &Status::Running, false).started);

        after_grace_prev.insert("sched-1".into(), (Status::Running, false));
        assert!(!session_transition(&after_grace_prev, "sched-1", &Status::Running, false).started);
    }
    #[test]
    fn category_overrides_load_empty_for_old_saves() {
        let p: Preferences =
            serde_json::from_str(r#"{"theme":"dark","default_minutes":25}"#).unwrap();
        assert!(p.category_overrides.is_empty());
    }
    #[test]
    fn category_overrides_survive_validation_and_bad_values_drop() {
        let mut p = Preferences::default();
        p.category_overrides
            .insert("com.example.app".into(), "Distracting".into());
        p.category_overrides
            .insert("example.com".into(), "Productive".into());
        p.category_overrides
            .insert("bogus.example".into(), "Bogus".into());
        let p = validate_preferences(p).unwrap();
        assert_eq!(
            p.category_overrides
                .get("com.example.app")
                .map(String::as_str),
            Some("Distracting")
        );
        assert_eq!(
            p.category_overrides.get("example.com").map(String::as_str),
            Some("Productive")
        );
        assert!(!p.category_overrides.contains_key("bogus.example"));
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
                lock: Default::default(),
                lock_password: None,
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
    #[test]
    fn login_user_pref_marker_round_trips() {
        // M6 req 9: the marker records only the user's own toggle,
        // never the automatic registration.
        let dir = tempfile::tempdir().unwrap();
        assert!(!login_user_pref_enabled(dir.path()));
        set_login_user_pref(dir.path(), true).unwrap();
        assert!(login_user_pref_enabled(dir.path()));
        set_login_user_pref(dir.path(), true).unwrap();
        assert!(login_user_pref_enabled(dir.path()));
        set_login_user_pref(dir.path(), false).unwrap();
        assert!(!login_user_pref_enabled(dir.path()));
        // Removing twice is fine (no marker left behind).
        set_login_user_pref(dir.path(), false).unwrap();
        assert!(!login_user_pref_enabled(dir.path()));
    }
}
