CREATE TABLE IF NOT EXISTS screen_time_usage (
  day_ms INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('app', 'site')),
  target TEXT NOT NULL,
  name TEXT NOT NULL,
  ms INTEGER NOT NULL CHECK(ms >= 0),
  PRIMARY KEY(day_ms, kind, target)
);
CREATE INDEX IF NOT EXISTS screen_time_usage_day ON screen_time_usage(day_ms);
PRAGMA user_version=9;
