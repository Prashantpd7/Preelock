use crate::models::Snapshot;
use rusqlite::{params, Connection};
use serde::de::DeserializeOwned;
use std::path::Path;

pub struct Database(Connection);
impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(|e| e.to_string())?;
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > 4 {
            return Err("Database was created by a newer version of Preelock.".into());
        }
        if version == 0 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/001_initial.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        if version < 2 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/002_focus_audit.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        if version < 3 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/003_attention.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        if version < 4 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/004_deleted_tombstones.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        Ok(Self(conn))
    }
    fn read<T: DeserializeOwned>(&self, query: &str) -> Result<Vec<T>, String> {
        let mut stmt = self.0.prepare(query).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn load(&self) -> Result<Snapshot, String> {
        Ok(Snapshot {
            browser: None,
            sessions: self.read("SELECT payload FROM sessions WHERE id NOT IN (SELECT id FROM deleted_sessions) ORDER BY rowid")?,
            tasks: self.read("SELECT payload FROM tasks ORDER BY rowid")?,
            preferences: self
                .read("SELECT payload FROM preferences WHERE id=1")?
                .pop()
                .unwrap_or_default(),
        })
    }
    pub fn backup(&self) -> Result<Snapshot, String> {
        let mut data = self.load()?;
        for session in &mut data.sessions {
            let mut stmt = self
                .0
                .prepare("SELECT payload FROM attention_intervals WHERE session_id=?1 ORDER BY at")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([&session.id], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            session.attention.intervals = rows
                .map(|r| {
                    serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut audit = self
                .0
                .prepare("SELECT at,kind,detail FROM focus_audit WHERE session_id=?1 ORDER BY at")
                .map_err(|e| e.to_string())?;
            session.report.events = audit
                .query_map([&session.id], |r| {
                    Ok(crate::focus::Event {
                        at: r.get(0)?,
                        kind: r.get(1)?,
                        detail: r.get(2)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
        }
        Ok(data)
    }
    pub fn save(&mut self, data: &Snapshot) -> Result<(), String> {
        self.write(data, false)
    }
    pub fn replace(&mut self, data: &Snapshot) -> Result<(), String> {
        self.write(data, true)
    }
    fn write(&mut self, data: &Snapshot, replace: bool) -> Result<(), String> {
        let tx = self.0.transaction().map_err(|e| e.to_string())?;
        if replace {
            tx.execute_batch("DELETE FROM attention_intervals; DELETE FROM focus_audit; DELETE FROM sessions; DELETE FROM tasks;").map_err(|e| e.to_string())?;
        }
        let ids: Vec<_> = data.sessions.iter().map(|s| &s.id).collect();
        let task_ids: Vec<_> = data.tasks.iter().map(|t| &t.id).collect();
        let ids_json = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
        if replace {
            // Explicit backup restore may restore deleted IDs; ordinary saves
            // must still reject stale writers.
            tx.execute(
                "DELETE FROM deleted_sessions WHERE id IN (SELECT value FROM json_each(?1))",
                params![ids_json.clone()],
            )
            .map_err(|e| e.to_string())?;
        }
        // Tombstone anything disappearing from the snapshot. Once deleted, a
        // session must never come back, even if a stale writer re-inserts it.
        tx.execute(
            "INSERT OR IGNORE INTO deleted_sessions(id, deleted_at) SELECT id, strftime('%s','now')*1000 FROM sessions WHERE id NOT IN (SELECT value FROM json_each(?1))",
            params![ids_json.clone()],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM sessions WHERE id NOT IN (SELECT value FROM json_each(?1))",
            params![ids_json],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM tasks WHERE id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&task_ids).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM attention_intervals WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM focus_audit WHERE session_id NOT IN (SELECT id FROM sessions)",
            [],
        )
        .map_err(|e| e.to_string())?;
        for s in &data.sessions {
            tx.execute("INSERT INTO sessions(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![s.id, serde_json::to_string(s).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        for s in &data.sessions {
            for interval in &s.attention.intervals {
                tx.execute("INSERT INTO attention_intervals(id,session_id,at,payload) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",params![interval.id,s.id,interval.at,serde_json::to_string(interval).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
            }
            for event in &s.report.events {
                tx.execute("INSERT OR IGNORE INTO focus_audit(session_id,at,kind,detail) VALUES (?1,?2,?3,?4)",params![s.id,event.at,event.kind,event.detail]).map_err(|e|e.to_string())?;
            }
        }
        // A stale writer may have re-inserted a tombstoned row just above;
        // drop it again so a delete is always final.
        tx.execute(
            "DELETE FROM sessions WHERE id IN (SELECT id FROM deleted_sessions)",
            [],
        )
        .map_err(|e| e.to_string())?;
        for t in &data.tasks {
            tx.execute("INSERT INTO tasks(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![t.id, serde_json::to_string(t).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        tx.execute("INSERT INTO preferences(id,payload) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(&data.preferences).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::Action, sessions};
    #[test]
    fn delete_and_replace_survive_reopen_and_preserve_full_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
            },
            1,
        )
        .unwrap();
        sessions::apply(
            &mut data,
            Action::CreateTask {
                title: "Task".into(),
            },
            1,
        )
        .unwrap();
        data.sessions[0]
            .attention
            .intervals
            .push(crate::attention::Interval {
                id: "interval".into(),
                at: 1,
                duration_ms: 1000,
                state: crate::attention::State::Focused,
                reason: "Work".into(),
                corrected_ms: 0,
                correction_note: String::new(),
            });
        data.sessions[0].report.event(1, "test", "Retain audit");
        db.save(&data).unwrap();
        data.sessions[0].attention.intervals.clear();
        data.sessions[0].report.events.clear();
        db.save(&data).unwrap();
        let backup = db.backup().unwrap();
        assert_eq!(backup.sessions[0].attention.intervals.len(), 1);
        assert_eq!(backup.sessions[0].report.events.len(), 1);
        data.sessions.clear();
        db.save(&data).unwrap();
        drop(db);
        let mut db = Database::open(&path).unwrap();
        assert!(db.load().unwrap().sessions.is_empty());
        assert_eq!(db.load().unwrap().tasks.len(), 1);
        db.replace(&backup).unwrap();
        assert_eq!(
            db.backup().unwrap().sessions[0].attention.intervals.len(),
            1
        );
        db.replace(&Snapshot::default()).unwrap();
        assert!(db.load().unwrap().tasks.is_empty());
    }
    #[test]
    fn deleted_sessions_never_resurface() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tomb.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Gone".into(),
                minutes: 1,
                allow_pause: true,
                focus: Default::default(),
            },
            1,
        )
        .unwrap();
        let id = data.sessions[0].id.clone();
        let payload = serde_json::to_string(&data.sessions[0]).unwrap();
        db.save(&data).unwrap();
        // Deleting tombstones the id.
        data.sessions.clear();
        db.save(&data).unwrap();
        // A stale writer re-inserts the deleted row (old process, old restore).
        db.0.execute(
            "INSERT INTO sessions(id,payload) VALUES (?1,?2)",
            rusqlite::params![id, payload],
        )
        .unwrap();
        drop(db);
        // After reopen the deleted session stays gone...
        let mut db = Database::open(&path).unwrap();
        assert!(db.load().unwrap().sessions.is_empty());
        // ...and the next save purges the resurrected row for good.
        let data = db.load().unwrap();
        db.save(&data).unwrap();
        let count: i64 =
            db.0.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
                .unwrap();
        assert_eq!(count, 0);
        let tombed: i64 =
            db.0.query_row("SELECT count(*) FROM deleted_sessions", [], |r| r.get(0))
                .unwrap();
        assert_eq!(tombed, 1);
    }
}
