use crate::{
    database::Database,
    models::{Action, Snapshot},
    sessions,
};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub struct Engine {
    db: Database,
    data: Snapshot,
    anchor: Instant,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
impl Engine {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut db = Database::open(path)?;
        let mut data = db.load()?;
        sessions::restore(&mut data, now());
        db.save(&data)?;
        Ok(Self {
            db,
            data,
            anchor: Instant::now(),
        })
    }
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.commit(None)
    }
    pub fn execute(&mut self, action: Action) -> Result<Snapshot, String> {
        self.commit(Some(action))
    }
    fn commit(&mut self, action: Option<Action>) -> Result<Snapshot, String> {
        let instant = Instant::now();
        let timestamp = now();
        let elapsed = instant
            .duration_since(self.anchor)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let mut next = self.data.clone();
        for s in &mut next.sessions {
            sessions::advance(s, elapsed, timestamp);
        }
        let transitioned = next
            .sessions
            .iter()
            .zip(&self.data.sessions)
            .any(|(a, b)| a.status != b.status);
        let changed = action.is_some();
        if let Some(action) = action {
            sessions::apply(&mut next, action, timestamp)?;
        }
        if changed || transitioned {
            self.db.save(&next)?;
        }
        self.data = next;
        self.anchor = instant;
        Ok(self.data.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sqlite_roundtrip_persists_tasks_preferences_and_paused_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine
            .execute(Action::CreateTask {
                title: "Read".into(),
            })
            .unwrap();
        engine
            .execute(Action::SetPreferences {
                theme: "light".into(),
                default_minutes: 45,
            })
            .unwrap();
        let state = engine
            .execute(Action::CreateSession {
                title: "Focus".into(),
                minutes: 1,
                allow_pause: true,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        engine.execute(Action::PauseSession { id }).unwrap();
        drop(engine);
        let mut restored = Engine::open(&path).unwrap();
        let state = restored.snapshot().unwrap();
        assert_eq!(state.tasks[0].title, "Read");
        assert_eq!(state.preferences.theme, "light");
        assert_eq!(state.sessions[0].status, crate::models::Status::Paused);
    }
    #[test]
    fn rejected_action_does_not_persist_partial_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        assert!(engine
            .execute(Action::CreateSession {
                title: "Invalid".into(),
                minutes: 0,
                allow_pause: false
            })
            .is_err());
        assert!(engine.snapshot().unwrap().sessions.is_empty());
    }
}
