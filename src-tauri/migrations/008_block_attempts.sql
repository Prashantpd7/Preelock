CREATE TABLE IF NOT EXISTS block_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL, session_id TEXT, reason TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS block_attempts_at_idx ON block_attempts(at_ms);
PRAGMA user_version=8;
