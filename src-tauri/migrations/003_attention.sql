CREATE TABLE IF NOT EXISTS attention_intervals(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, at INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS attention_session_time ON attention_intervals(session_id,at);
PRAGMA user_version=3;
