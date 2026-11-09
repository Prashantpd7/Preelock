CREATE TABLE IF NOT EXISTS focus_audit (
  session_id TEXT NOT NULL,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL,
  detail TEXT NOT NULL,
  UNIQUE(session_id,at,kind,detail)
);
CREATE INDEX IF NOT EXISTS focus_audit_by_session ON focus_audit(session_id,at);
PRAGMA user_version=2;
