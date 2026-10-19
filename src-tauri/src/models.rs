use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Preparing,
    Running,
    Paused,
    Completed,
    Ended,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub duration_minutes: u32,
    pub remaining_ms: u64,
    pub focused_ms: u64,
    pub allow_pause: bool,
    pub status: Status,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub deadline_ms: Option<i64>,
    #[serde(default)]
    pub focus: crate::focus::FocusConfig,
    #[serde(default)]
    pub report: crate::focus::FocusReport,
    #[serde(default)]
    pub attention: crate::attention::Tracking,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSetup {
    pub id: String,
    pub name: String,
    pub minutes: u32,
    pub focus: crate::focus::FocusConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: String,
    pub default_minutes: u32,
    pub large_text: bool,
    pub menu_timer: bool,
    pub daily_goal: u32,
    pub session_alert: bool,
    pub sound: bool,
    pub volume: u32,
    pub reminder: bool,
    pub reminder_time: String,
    pub reminder_days: Vec<u32>,
    pub offer_break: bool,
    pub break_minutes: u32,
    pub break_alert: bool,
    pub saved_setups: Vec<SavedSetup>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            default_minutes: 25,
            large_text: false,
            menu_timer: false,
            daily_goal: 0,
            session_alert: true,
            sound: false,
            volume: 50,
            reminder: false,
            reminder_time: "09:00".into(),
            reminder_days: vec![1, 2, 3, 4, 5],
            offer_break: true,
            break_minutes: 5,
            break_alert: true,
            saved_setups: vec![],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub tasks: Vec<Task>,
    pub preferences: Preferences,
    #[serde(skip_deserializing, default)]
    pub browser: Option<crate::browser::BrowserStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    CreateSession {
        title: String,
        minutes: u32,
        allow_pause: bool,
        #[serde(default)]
        focus: crate::focus::FocusConfig,
    },
    StartSession {
        id: String,
    },
    PauseSession {
        id: String,
    },
    ResumeSession {
        id: String,
    },
    EndSession {
        id: String,
    },
    DeleteSession {
        id: String,
    },
    EmergencyExit {
        id: String,
        confirmation: String,
    },
    ConfirmWork {
        id: String,
    },
    CorrectInterval {
        id: String,
        interval_id: String,
        note: String,
    },
    DisableCamera {
        id: String,
    },
    CreateTask {
        title: String,
    },
    UpdateTask {
        id: String,
        title: String,
        completed: bool,
    },
    UpdatePreferences {
        preferences: Preferences,
    },
    SetPreferences {
        theme: String,
        default_minutes: u32,
    },
}
