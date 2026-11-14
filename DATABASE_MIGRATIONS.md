# Preelock Database Migrations & Storage Architecture

Preelock uses an embedded SQLite database (`preelock.sqlite`) located in the user's application data folder:

- **macOS:** `~/Library/Application Support/com.preelock.desktop/preelock.sqlite`
- **Windows:** `%APPDATA%\com.preelock.desktop\preelock.sqlite`

This document defines schema history, migration rules, backup procedures, and crash recovery.

---

## 1. Migration Principles

1. **Strictly Additive & Backward-Compatible:** Migrations never alter or drop user tables or delete existing user data.
2. **Atomic Execution:** Every migration script runs inside an explicit `BEGIN IMMEDIATE; ... COMMIT;` transaction block.
3. **Automated Pre-Migration Backups:** Before running any additive migrations on an existing database (`version > 0 && version < LATEST`), Preelock automatically creates a timestamped safety backup:
   `preelock.pre_v12_v{version}.bak`
4. **Integrity Verification:** On every database open, `PRAGMA quick_check(1)` verifies database health. If corruption is detected, a copy `preelock.corrupt.{timestamp}.sqlite` is preserved for forensics.
5. **Future-Version Protection:** If `PRAGMA user_version` is higher than the compiled binary's supported schema (`> 12`), the app refuses to run and returns an error, preventing accidental corruption from running older binaries on newer schemas.

---

## 2. Schema Versions (v1 through v12)

| Version | Migration File               | Description                                                                           |
| :------ | :--------------------------- | :------------------------------------------------------------------------------------ |
| **v1**  | `001_initial.sql`            | Core tables: `sessions`, `tasks`, `preferences`.                                      |
| **v2**  | `002_focus_audit.sql`        | Append-only audit table: `focus_audit(session_id, at, kind, detail)`.                 |
| **v3**  | `003_attention.sql`          | Attention intervals table: `attention_intervals(session_id, at, payload)`.            |
| **v4**  | `004_deleted_tombstones.sql` | Deleted session tombstones: `deleted_sessions(id, deleted_at)`.                       |
| **v5**  | `005_schedules.sql`          | Recurring and calendar focus schedules: `schedules(id, payload)`.                     |
| **v6**  | `006_one_shots.sql`          | One-shot timers: `one_shots(id, payload)`.                                            |
| **v7**  | `007_allowances.sql`         | Daily and weekly usage allowances: `allowances(id, payload)`.                         |
| **v8**  | `008_block_attempts.sql`     | Block attempt logs: `block_attempts(id, at_ms, kind, target, session_id, reason)`.    |
| **v9**  | `009_screen_time.sql`        | Screen Time daily usage buckets: `screen_time_usage(day_ms, kind, target, name, ms)`. |
| **v10** | `010_app_locks.sql`          | Application and website locks: `app_locks(id, payload)`.                              |
| **v11** | `011_adult_block.sql`        | Adult website blocking configuration: `adult_block(id, payload)`.                     |
| **v12** | `012_strict_blocks.sql`      | Strict mode state for apps and websites: `strict_blocks(kind, payload)`.              |

---

## 3. SQLite Configuration Pragmas

On connection initialization, Preelock configures SQLite with high-reliability pragmas:

- `PRAGMA journal_mode=WAL;` — Write-Ahead Logging allows concurrent readers and author writes without blocking.
- `PRAGMA synchronous=FULL;` — Ensures all transactions are committed to durable physical disk storage before returning.
- `busy_timeout(5000)` — Waits up to 5 seconds if a lock contention occurs.

---

## 4. Disaster Recovery Procedure

If the database file is corrupted due to hardware crash or disk error:

1. Preelock detects the failure during `PRAGMA quick_check(1)`.
2. The damaged file is automatically preserved to:
   `preelock.corrupt.<timestamp>.sqlite`
3. A pre-migration backup is available at:
   `preelock.pre_v12_v*.bak`
4. To restore:
   - Stop the Preelock background daemon (`launchctl stop com.preelock.background` or `taskkill`).
   - Copy the backup file over `preelock.sqlite`.
   - Restart the background daemon and GUI.
