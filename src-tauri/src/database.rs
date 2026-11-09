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
        if version > 10 {
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
        // Additive only: existing sessions, tasks and preferences are
        // untouched; old databases gain an empty schedules table.
        if version < 5 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/005_schedules.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Additive only: existing data is untouched; old databases
        // gain an empty one_shots table (step 6).
        if version < 6 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/006_one_shots.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Additive only: existing data is untouched; old databases
        // gain an empty allowances table (step 8).
        if version < 7 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/007_allowances.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Additive only: existing data is untouched; old databases
        // gain an empty block_attempts table (step 13).
        if version < 8 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/008_block_attempts.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Additive only: persistent daily Screen Time buckets, enabled only on opt-in.
        if version < 9 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/009_screen_time.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Additive only: existing data is untouched; old databases
        // gain an empty app_locks table (app locks).
        if version < 10 {
            conn.execute_batch(concat!(
                "BEGIN IMMEDIATE;",
                include_str!("../migrations/010_app_locks.sql"),
                "COMMIT;"
            ))
            .map_err(|e| e.to_string())?;
        }
        // Prune attempts older than 90 days on open; a failure here
        // must never block opening the database, so it is ignored.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let cutoff = crate::stats::prune_cutoff_ms(now_ms);
        let _ = conn.execute(
            "DELETE FROM block_attempts WHERE at_ms < ?1",
            rusqlite::params![cutoff],
        );
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
            blocked_app_notice: None,
            sessions: self.read("SELECT payload FROM sessions WHERE id NOT IN (SELECT id FROM deleted_sessions) ORDER BY rowid")?,
            tasks: self.read("SELECT payload FROM tasks ORDER BY rowid")?,
            schedules: self.read("SELECT payload FROM schedules ORDER BY rowid")?,
            one_shots: self.read("SELECT payload FROM one_shots ORDER BY rowid")?,
            allowances: self.read("SELECT payload FROM allowances ORDER BY rowid")?,
            app_locks: self.read("SELECT payload FROM app_locks ORDER BY rowid")?,
            screen_time_usage: self.load_screen_time_usage()?,
            preferences: self
                .read("SELECT payload FROM preferences WHERE id=1")?
                .pop()
                .unwrap_or_default(),
        })
    }

    /// Step 13: record one block attempt. Rows are append-only
    /// here; pruning happens on database open (90 days).
    pub fn record_attempt(&self, attempt: &crate::stats::BlockAttempt) -> Result<(), String> {
        let kind = match attempt.kind {
            crate::stats::AttemptKind::App => "app",
            crate::stats::AttemptKind::Site => "site",
        };
        self.0
            .execute(
                "INSERT INTO block_attempts(at_ms, kind, target, session_id, reason) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    attempt.at_ms,
                    kind,
                    attempt.target,
                    attempt.session_id,
                    attempt.reason,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Step 13: newest attempts first, capped at `limit`.
    pub fn list_attempts(&self, limit: i64) -> Result<Vec<crate::stats::BlockAttempt>, String> {
        let mut stmt = self
            .0
            .prepare("SELECT at_ms, kind, target, session_id, reason FROM block_attempts ORDER BY at_ms DESC, id DESC LIMIT ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([limit], |r| {
                let kind_text: String = r.get(1)?;
                Ok(crate::stats::BlockAttempt {
                    at_ms: r.get(0)?,
                    kind: if kind_text == "site" {
                        crate::stats::AttemptKind::Site
                    } else {
                        crate::stats::AttemptKind::App
                    },
                    target: r.get(2)?,
                    session_id: r.get(3)?,
                    reason: r.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }
    /// Step 13: delete ALL block-attempt rows. Returns the count.
    /// Only this table is touched: sessions, schedules, tasks,
    /// allowances, preferences and lock data live elsewhere and
    /// are never deleted by statistics deletion.
    pub fn delete_attempts(&mut self) -> Result<u64, String> {
        let deleted = self
            .0
            .execute("DELETE FROM block_attempts", [])
            .map_err(|e| e.to_string())?;
        Ok(deleted as u64)
    }
    pub fn load_screen_time_usage(&self) -> Result<Vec<crate::models::ScreenTimeBucket>, String> {
        let mut stmt = self.0.prepare(
            "SELECT day_ms, kind, target, name, ms FROM screen_time_usage ORDER BY day_ms, ms DESC",
        ).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                let kind: String = r.get(1)?;
                Ok(crate::models::ScreenTimeBucket {
                    day_ms: r.get(0)?,
                    kind,
                    target: r.get(2)?,
                    name: r.get(3)?,
                    ms: r.get::<_, i64>(4)?.max(0) as u64,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    }
    pub fn record_screen_time(
        &mut self,
        buckets: &[crate::models::ScreenTimeBucket],
    ) -> Result<(), String> {
        let tx = self.0.transaction().map_err(|e| e.to_string())?;
        for bucket in buckets {
            if !["app", "site"].contains(&bucket.kind.as_str()) {
                return Err("Invalid Screen Time target kind.".into());
            }
            tx.execute(
                "INSERT INTO screen_time_usage(day_ms, kind, target, name, ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(day_ms, kind, target) DO UPDATE SET name=excluded.name, ms=screen_time_usage.ms + excluded.ms",
                params![bucket.day_ms, bucket.kind, bucket.target, bucket.name, bucket.ms.min(i64::MAX as u64) as i64],
            ).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
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
            tx.execute_batch("DELETE FROM attention_intervals; DELETE FROM focus_audit; DELETE FROM sessions; DELETE FROM tasks; DELETE FROM schedules; DELETE FROM one_shots; DELETE FROM allowances; DELETE FROM app_locks; DELETE FROM screen_time_usage;").map_err(|e| e.to_string())?;
        }
        let ids: Vec<_> = data.sessions.iter().map(|s| &s.id).collect();
        let task_ids: Vec<_> = data.tasks.iter().map(|t| &t.id).collect();
        let schedule_ids: Vec<_> = data.schedules.iter().map(|s| &s.id).collect();
        let one_shot_ids: Vec<_> = data.one_shots.iter().map(|s| &s.id).collect();
        let allowance_ids: Vec<_> = data.allowances.iter().map(|a| &a.id).collect();
        let app_lock_ids: Vec<_> = data.app_locks.iter().map(|l| &l.id).collect();
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
            "DELETE FROM schedules WHERE id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&schedule_ids).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM one_shots WHERE id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&one_shot_ids).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM allowances WHERE id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&allowance_ids).map_err(|e| e.to_string())?],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM app_locks WHERE id NOT IN (SELECT value FROM json_each(?1))",
            [serde_json::to_string(&app_lock_ids).map_err(|e| e.to_string())?],
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
        for s in &data.schedules {
            tx.execute("INSERT INTO schedules(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![s.id, serde_json::to_string(s).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        for s in &data.one_shots {
            tx.execute("INSERT INTO one_shots(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![s.id, serde_json::to_string(s).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        for a in &data.allowances {
            tx.execute("INSERT INTO allowances(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![a.id, serde_json::to_string(a).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        for l in &data.app_locks {
            tx.execute("INSERT INTO app_locks(id,payload) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![l.id, serde_json::to_string(l).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        }
        tx.execute("INSERT INTO preferences(id,payload) VALUES (1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(&data.preferences).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        for bucket in &data.screen_time_usage {
            tx.execute("INSERT INTO screen_time_usage(day_ms,kind,target,name,ms) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(day_ms,kind,target) DO UPDATE SET name=excluded.name,ms=excluded.ms", params![bucket.day_ms,bucket.kind,bucket.target,bucket.name,bucket.ms.min(i64::MAX as u64) as i64]).map_err(|e|e.to_string())?;
        }
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
                lock: Default::default(),
                lock_password: None,
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
    fn schedules_survive_reopen_and_do_not_disturb_old_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sched.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
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
        sessions::apply(
            &mut data,
            Action::CreateSchedule {
                title: "Night block".into(),
                days: vec![1, 2, 3, 4, 5],
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
        db.save(&data).unwrap();
        drop(db);
        // Reopening an existing database keeps sessions, tasks and
        // schedules (migration 005 is additive).
        let mut db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.tasks.len(), 1);
        assert_eq!(loaded.schedules.len(), 1);
        assert_eq!(loaded.schedules[0].title, "Night block");
        assert_eq!(loaded.schedules[0].start_time, "22:00");
        assert!(loaded.schedules[0].lock_changes);
        // Removing the schedule persists too (lock rules are
        // covered by the sessions tests; this checks storage only).
        let mut data = loaded;
        data.schedules.clear();
        db.save(&data).unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert!(loaded.schedules.is_empty());
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.tasks.len(), 1);
    }
    #[test]
    fn one_shots_survive_reopen_and_do_not_disturb_old_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("oneshot.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            },
            1,
        )
        .unwrap();
        sessions::apply(
            &mut data,
            Action::CreateOneShot {
                title: "Later".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                fire_at_ms: 5000,
            },
            1,
        )
        .unwrap();
        db.save(&data).unwrap();
        drop(db);
        // Reopening keeps the session and the timer (migration 006
        // is additive, like 005 before it).
        let mut db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.one_shots.len(), 1);
        assert_eq!(loaded.one_shots[0].title, "Later");
        assert_eq!(loaded.one_shots[0].fire_at_ms, 5000);
        // Removing the timer persists too.
        let mut data = loaded;
        data.one_shots.clear();
        db.save(&data).unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert!(loaded.one_shots.is_empty());
        assert_eq!(loaded.sessions.len(), 1);
    }

    #[test]
    fn allowances_survive_reopen_and_do_not_disturb_old_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("allow.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateAllowance {
                title: "YouTube".into(),
                kind: crate::allowances::TargetKind::Site,
                target: "youtube.com".into(),
                budget_minutes: 30,
                period: crate::allowances::Period::Day,
                mode: crate::allowances::ResetMode::Fixed,
                enabled: true,
            },
            1,
        )
        .unwrap();
        // Usage accrued before the restart is still there after it.
        crate::allowances::accrue(&mut data.allowances[0], 60_000, 5 * 60_000);
        db.save(&data).unwrap();
        drop(db);
        let mut db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.allowances.len(), 1);
        assert_eq!(loaded.allowances[0].title, "YouTube");
        assert_eq!(loaded.allowances[0].usage.len(), 1);
        // Removing the allowance persists too (deleting lifts its
        // block immediately, since exhaustion is computed, not stored).
        let mut data = loaded;
        data.allowances.clear();
        db.save(&data).unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        assert!(db.load().unwrap().allowances.is_empty());
    }

    #[test]
    fn app_locks_survive_reopen_and_prune_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("applock.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        let now = 1_800_000_000_000;
        sessions::apply(
            &mut data,
            Action::CreateAppLock {
                title: "Game".into(),
                kind: crate::allowances::TargetKind::App,
                target: "com.example.game".into(),
                rule: crate::applocks::AppLockRuleInput::Timed { minutes: 30 },
            },
            now,
        )
        .unwrap();
        sessions::apply(
            &mut data,
            Action::CreateAppLock {
                title: "Tube".into(),
                kind: crate::allowances::TargetKind::Site,
                target: "youtube.com".into(),
                rule: crate::applocks::AppLockRuleInput::DailyLimit { limit_minutes: 60 },
            },
            now,
        )
        .unwrap();
        // An expired timed lock: valid when created (validated
        // before its end), expired since — the state a restart
        // would load from disk. It must prune and never enforce.
        data.app_locks.push(
            crate::applocks::validate_lock(
                crate::applocks::AppLock {
                    id: "expired".into(),
                    title: "Old".into(),
                    kind: crate::allowances::TargetKind::Site,
                    target: "example.com".into(),
                    rule: crate::applocks::LockRule::Timed {
                        ends_at_ms: now - 1000,
                    },
                    enabled: true,
                    created_at: now - 60_000,
                },
                now - 2000,
            )
            .unwrap(),
        );
        db.save(&data).unwrap();
        drop(db);
        // Reopening keeps all three locks with fields intact
        // (migration 010 is additive, like 007 before it).
        let mut db = Database::open(&path).unwrap();
        let mut loaded = db.load().unwrap();
        assert_eq!(loaded.app_locks.len(), 3);
        let timed = loaded.app_locks.iter().find(|l| l.title == "Game").unwrap();
        assert_eq!(timed.target, "com.example.game");
        assert!(matches!(timed.kind, crate::allowances::TargetKind::App));
        assert!(matches!(
            timed.rule,
            crate::applocks::LockRule::Timed { ends_at_ms }
            if ends_at_ms == now + 30 * 60_000
        ));
        let daily = loaded.app_locks.iter().find(|l| l.title == "Tube").unwrap();
        assert!(matches!(
            daily.rule,
            crate::applocks::LockRule::DailyLimit { limit_ms }
            if limit_ms == 60 * 60_000
        ));
        // Prune the expired lock and save: it stays gone after
        // reopen (expiry is deletion, never a stale block).
        assert!(crate::applocks::prune_expired(&mut loaded.app_locks, now));
        db.save(&loaded).unwrap();
        drop(db);
        let mut db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.app_locks.len(), 2);
        assert!(loaded.app_locks.iter().all(|l| l.id != "expired"));
        // Deleting a lock persists too (lifts enforcement at once).
        let mut data = loaded;
        let id = data.app_locks[0].id.clone();
        sessions::apply(&mut data, Action::DeleteAppLock { id }, now).unwrap();
        db.save(&data).unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let loaded = db.load().unwrap();
        assert_eq!(loaded.app_locks.len(), 1);
        assert_eq!(loaded.app_locks[0].title, "Tube");
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
                lock: Default::default(),
                lock_password: None,
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
    #[test]
    fn block_attempts_roundtrip_prune_and_delete_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.sqlite");
        let mut db = Database::open(&path).unwrap();
        let mut data = Snapshot::default();
        sessions::apply(
            &mut data,
            Action::CreateSession {
                title: "Study".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            },
            1,
        )
        .unwrap();
        db.save(&data).unwrap();
        // A fresh attempt and a 91-day-old one (wall-clock
        // times: the 1970 epoch would itself be pruned as stale).
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        db.record_attempt(&crate::stats::BlockAttempt::app(
            now_ms,
            "com.example.Game",
            &data.sessions[0].id,
            "Blocked app",
        ))
        .unwrap();
        db.record_attempt(&crate::stats::BlockAttempt::app(
            now_ms - 91 * 86_400_000,
            "com.example.Old",
            &data.sessions[0].id,
            "Blocked app",
        ))
        .unwrap();
        // Reopening prunes the 91-day-old row but keeps the fresh
        // one, the session, and everything else.
        drop(db);
        let mut db = Database::open(&path).unwrap();
        let attempts = db.list_attempts(100).unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].target, "com.example.Game");
        assert_eq!(db.load().unwrap().sessions.len(), 1);
        // Deleting statistics removes only attempt rows.
        assert_eq!(db.delete_attempts().unwrap(), 1);
        assert!(db.list_attempts(100).unwrap().is_empty());
        assert_eq!(db.load().unwrap().sessions.len(), 1);
    }
}
