CREATE TABLE IF NOT EXISTS deleted_sessions(id TEXT PRIMARY KEY, deleted_at INTEGER NOT NULL);
PRAGMA user_version=4;