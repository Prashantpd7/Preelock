use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    Running,
    Paused,
    Completed,
    Ended,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preferences {
    pub theme: String,
    pub default_minutes: u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            default_minutes: 25,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub tasks: Vec<Task>,
    pub preferences: Preferences,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    CreateSession {
        title: String,
        minutes: u32,
        allow_pause: bool,
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
    CreateTask {
        title: String,
    },
    UpdateTask {
        id: String,
        title: String,
        completed: bool,
    },
    SetPreferences {
        theme: String,
        default_minutes: u32,
    },
}
