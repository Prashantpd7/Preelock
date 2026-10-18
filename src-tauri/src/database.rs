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
        if version > 1 {
            return Err("Database was created by a newer version of VYSTRIQ.".into());
        }
        if version == 0 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/001_initial.sql"),
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
            sessions: self.read("SELECT payload FROM sessions ORDER BY rowid")?,
            tasks: self.read("SELECT payload FROM tasks ORDER BY rowid")?,
            preferences: self
                .read("SELECT payload FROM preferences WHERE id=1")?
                .pop()
                .unwrap_or_default(),
        })
    }
    pub fn save(&mut self, data: &Snapshot) -> Result<(), String> {
        let tx = self.0.transaction().map_err(|e| e.to_string())?;
        for s in &data.sessions {
            tx.execute("INSERT INTO sessions(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![s.id, serde_json::to_string(s).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        for t in &data.tasks {
            tx.execute("INSERT INTO tasks(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![t.id, serde_json::to_string(t).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        tx.execute("INSERT INTO preferences(id,payload) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(&data.preferences).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}
