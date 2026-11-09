//! Step 15: command-line control for power users.
//!
//! The binary is dual-purpose: with no CLI flag it launches the GUI
//! exactly as before (see `main.rs`); with a CLI flag it runs one
//! command and exits. No new crates are used: argument parsing and
//! the file protocol below are plain std, and the database reads use
//! the existing rusqlite dependency.
//!
//! Protocol (all files live in the app-data directory, next to
//! `preelock.sqlite`, so the GUI and the CLI always agree on the
//! location — see `app_data_dir`):
//!
//! * Heartbeat: the GUI's 1-second tray thread rewrites
//!   `preelock-cli-heartbeat.json` (`{"updated_ms": ...}`) on every
//!   tick. A heartbeat older than 5 seconds means the app is not
//!   running (or its tray thread is stuck), and control commands
//!   refuse to write anything.
//! * Request: a control command writes one
//!   `preelock-cli-request-<id>.json` file (`CliRequest`) and polls
//!   for its result. The tray thread *consumes* the file (deletes it
//!   on pickup, before executing), so a request can never fire twice
//!   and never sits around to fire unexpectedly later. A request
//!   older than 60 seconds that the tray thread finds is ignored and
//!   deleted, and a malformed request file is deleted, never executed.
//! * Result: after executing, the tray thread writes
//!   `preelock-cli-result-<id>.json` (`CliOutcome`); the CLI prints
//!   it and deletes it. Result files older than 60 seconds are
//!   cleaned up by the tray thread.
//!
//! Safety invariants, enforced here and in `lib.rs`:
//!
//! * Read-only commands (`--status`, `--list-blocks`) never write to
//!   the user's database: they open it read-only, and fall back to
//!   reading a throwaway copy in the temp directory if the live
//!   database cannot be opened read-only (for example while its WAL
//!   is being recovered). They never run migrations or pruning.
//! * Control commands never touch the engine directly. They go
//!   through the tray thread, which executes them with the existing
//!   `Engine::execute` paths, so step-10 lock gates, step-12 frozen
//!   no-override, step-5 time-range locks and step-14 grace rules
//!   apply exactly as in the GUI. A rejected stop reports the
//!   engine's reason; it never bypasses a gate.
//! * A CLI start never extends a session: it starts a Ready session,
//!   resumes a Paused one, or creates a fresh session from a saved
//!   setup or a schedule — always with that template's own duration
//!   (bounded 1..=480 minutes by the engine) — and never touches a
//!   live session's remaining time.
//! * Nothing here can create a permanent lockout: requests expire
//!   (60 s), heartbeats go stale (5 s), and a CLI-started session
//!   ends on its own timer exactly like a GUI-started one.
//!
//! Windows note: release builds use the `windows_subsystem =
//! "windows"` attribute (see `main.rs`), so a release binary
//! launched from a terminal does not reliably print to that
//! terminal. CLI output is reliable on macOS and Linux and in
//! Windows dev builds; on Windows release builds the commands still
//! run (requests, heartbeats and results are files), but their
//! printed output may not be visible. Changing the subsystem would
//! pop a console window on every GUI launch, so it is left alone.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Tauri application identifier (see `tauri.conf.json`). The
/// app-data directory is derived from it on every platform.
pub const APP_ID: &str = "com.preelock.desktop";
/// A heartbeat this old (or older) means the app is not running.
pub const HEARTBEAT_MAX_AGE_MS: i64 = 5_000;
/// A request this old (or older) is stale: ignored and deleted.
pub const REQUEST_MAX_AGE_MS: i64 = 60_000;
/// How long a control command waits for the tray thread's result.
const RESULT_WAIT_MS: u64 = 20_000;
const RESULT_POLL_MS: u64 = 200;

const HEARTBEAT_FILE: &str = "preelock-cli-heartbeat.json";
const REQUEST_PREFIX: &str = "preelock-cli-request-";
const RESULT_PREFIX: &str = "preelock-cli-result-";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const HELP: &str = "Preelock — command-line control\n\nUsage: preelock [command]\n\nCommands:\n  --help                  Show this help.\n  --version               Show the Preelock version.\n  --status                Show the live session state (read-only).\n  --list-blocks           Show schedules, one-shots, saved setups and\n                          the live session summary (read-only).\n  --start <id>            Start a Ready session, resume a Paused\n                          session, or start a fresh session from a\n                          saved setup or schedule with this id.\n                          Requires the app to be running.\n  --stop                  End the live session. Requires the app to\n                          be running. A locked or frozen session is\n                          rejected with its reason, never bypassed.\n\nRead-only commands work whether or not the app is running. Control\ncommands refuse with \"Preelock isn't running — start it first\" when\nthe app is not running, and never leave a pending request behind.\nWith no command, Preelock launches its normal window instead.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
    ListBlocks,
    Start(String),
    Stop,
}

/// The flags that switch the binary into CLI mode. Any other first
/// argument (including an unknown `--flag`) launches the GUI, so a
/// launcher that passes its own arguments can never accidentally
/// turn the app into a one-shot CLI run.
pub fn is_cli_invocation(args: &[String]) -> bool {
    matches!(
        args.first().map(String::as_str),
        Some(
            "--help"
                | "-h"
                | "--version"
                | "-V"
                | "--status"
                | "--list-blocks"
                | "--start"
                | "--stop"
        )
    )
}

pub fn parse_args(args: &[String]) -> Result<Command, String> {
    match args.first().map(String::as_str) {
        Some("--help") | Some("-h") => Ok(Command::Help),
        Some("--version") | Some("-V") => Ok(Command::Version),
        Some("--status") => Ok(Command::Status),
        Some("--list-blocks") => Ok(Command::ListBlocks),
        Some("--start") => match args.get(1) {
            Some(id) if !id.trim().is_empty() => Ok(Command::Start(id.trim().to_string())),
            _ => Err("Usage: preelock --start <session, saved-setup, or schedule id>".into()),
        },
        Some("--stop") => Ok(Command::Stop),
        Some(other) => Err(format!("Unknown command: {other}\n{HELP}")),
        None => Err(HELP.into()),
    }
}

/// App-data directory for the current platform, matching Tauri's
/// `app_data_dir` for this identifier.
pub fn app_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or("HOME is not set.")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(APP_ID))
    }
    #[cfg(target_os = "windows")]
    {
        let data = std::env::var_os("APPDATA").ok_or("APPDATA is not set.")?;
        Ok(PathBuf::from(data).join(APP_ID))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let base = match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => {
                let home = std::env::var_os("HOME").ok_or("HOME is not set.")?;
                PathBuf::from(home).join(".local").join("share")
            }
        };
        Ok(base.join(APP_ID))
    }
}

pub fn db_path(dir: &Path) -> PathBuf {
    dir.join("preelock.sqlite")
}

fn heartbeat_path(dir: &Path) -> PathBuf {
    dir.join(HEARTBEAT_FILE)
}

// Test-only helper: production code builds request paths inline
// in `write_request`/`take_requests` via the prefix.
#[cfg(test)]
fn request_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{REQUEST_PREFIX}{id}.json"))
}

fn result_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{RESULT_PREFIX}{id}.json"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Heartbeat {
    updated_ms: i64,
}

/// True when the heartbeat was written recently enough to prove the
/// app's tray thread is alive. Future timestamps never count: a
/// clock jump must not fake a live app.
pub fn heartbeat_fresh(updated_ms: i64, now_ms: i64) -> bool {
    let age = now_ms.saturating_sub(updated_ms);
    updated_ms <= now_ms && age < HEARTBEAT_MAX_AGE_MS
}

/// One control request, written by the CLI and consumed by the tray
/// thread. `kind` is `"start"` (with `target`) or `"stop"` (without).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliRequest {
    pub id: String,
    pub kind: String,
    pub target: Option<String>,
    pub created_ms: i64,
}

/// One control result, written by the tray thread and read by the CLI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliOutcome {
    pub id: String,
    pub ok: bool,
    pub message: String,
    pub created_ms: i64,
}

/// Strict validation for a request file. Anything that fails here is
/// deleted by the tray thread and never executed: ids are opaque
/// template/session/schedule identifiers, so the target charset is
/// deliberately narrow.
pub fn validate_request(request: &CliRequest, now_ms: i64) -> Result<(), String> {
    if request.id.trim().is_empty() || request.id.len() > 128 {
        return Err("Request id is missing or too long.".into());
    }
    match request.kind.as_str() {
        "start" => {
            let target = request.target.as_deref().unwrap_or("").trim();
            if target.is_empty() || target.len() > 200 {
                return Err("A start request needs a session, saved-setup, or schedule id.".into());
            }
            if !target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err("The id contains characters that are not allowed.".into());
            }
        }
        "stop" => {
            if request.target.is_some() {
                return Err("A stop request takes no id.".into());
            }
        }
        other => return Err(format!("Unknown request kind: {other}")),
    }
    if request.created_ms > now_ms + 5_000
        || now_ms.saturating_sub(request.created_ms) >= REQUEST_MAX_AGE_MS
    {
        return Err("Request is stale or from the future.".into());
    }
    Ok(())
}

/// Read the heartbeat timestamp, if a readable one exists.
pub fn read_heartbeat(dir: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(heartbeat_path(dir)).ok()?;
    serde_json::from_str::<Heartbeat>(&text)
        .ok()
        .map(|h| h.updated_ms)
}

/// Rewrite the heartbeat file (tray thread, every tick). Failures
/// are the caller's problem to ignore: a missed heartbeat must never
/// break the tick itself. The write goes through a temp file plus
/// rename so the CLI never reads a half-written heartbeat.
pub fn write_heartbeat(dir: &Path, now_ms: i64) -> std::io::Result<()> {
    let text =
        serde_json::to_string(&Heartbeat { updated_ms: now_ms }).map_err(std::io::Error::other)?;
    write_atomic(dir, HEARTBEAT_FILE, &text, "heartbeat")
}

fn write_atomic(dir: &Path, file_name: &str, text: &str, tag: &str) -> std::io::Result<()> {
    let tmp_name = format!(".preelock-cli-{tag}-{}.tmp", std::process::id());
    let tmp = dir.join(&tmp_name);
    std::fs::write(&tmp, text)?;
    let result = std::fs::rename(&tmp, dir.join(file_name));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Write a control request file. Callers must have checked the
/// heartbeat first: this function itself does not check liveness.
pub fn write_request(dir: &Path, request: &CliRequest) -> std::io::Result<()> {
    let text = serde_json::to_string(request).map_err(std::io::Error::other)?;
    let file_name = format!("{REQUEST_PREFIX}{}.json", request.id);
    write_atomic(dir, &file_name, &text, &format!("request-{}", request.id))
}

/// Write a control result file for a consumed request.
pub fn write_result(dir: &Path, outcome: &CliOutcome) -> std::io::Result<()> {
    let text = serde_json::to_string(outcome).map_err(std::io::Error::other)?;
    let file_name = format!("{RESULT_PREFIX}{}.json", outcome.id);
    write_atomic(dir, &file_name, &text, &format!("result-{}", outcome.id))
}

/// Consume every pending request file in `dir`: each file is deleted
/// on pickup, and only valid, fresh requests are returned. Stale,
/// future-dated, malformed, or otherwise invalid files are deleted
/// and never executed. Callers execute the returned requests and
/// then write one result file each.
pub fn take_requests(dir: &Path, now_ms: i64) -> Vec<CliRequest> {
    let mut taken = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return taken;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_prefix(REQUEST_PREFIX)
            .and_then(|rest| rest.strip_suffix(".json"))
        else {
            continue;
        };
        let path = entry.path();
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<CliRequest>(&text).ok());
        // Consumed either way: a request file must never survive
        // pickup, valid or not, so it can never fire later.
        let _ = std::fs::remove_file(&path);
        let Some(request) = parsed else { continue };
        if request.id != id {
            continue;
        }
        if validate_request(&request, now_ms).is_ok() {
            taken.push(request);
        }
    }
    taken
}

/// Delete result files older than 60 seconds, so a result the CLI
/// never collected (crashed, killed) does not linger.
pub fn cleanup_stale_results(dir: &Path, now_ms: i64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(RESULT_PREFIX) || !name.ends_with(".json") {
            continue;
        }
        let path = entry.path();
        let stale = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<CliOutcome>(&text).ok())
            .is_none_or(|outcome| now_ms.saturating_sub(outcome.created_ms) >= REQUEST_MAX_AGE_MS);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Read one result file, deleting it whether or not it parsed: a
/// result is single-use, like its request.
fn take_result(dir: &Path, id: &str) -> Option<CliOutcome> {
    let path = result_path(dir, id);
    let outcome = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<CliOutcome>(&text).ok());
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    outcome
}

/// Window length of a schedule in minutes, from its `HH:MM` times.
/// Overnight windows (end earlier than start) wrap past midnight;
/// an equal start/end is not a valid window.
pub fn schedule_window_minutes(start_time: &str, end_time: &str) -> Option<u32> {
    let start = crate::schedules::parse_time(start_time).ok()?;
    let end = crate::schedules::parse_time(end_time).ok()?;
    if start == end {
        return None;
    }
    let minutes = if end > start {
        end - start
    } else {
        end + 24 * 60 - start
    };
    Some(minutes)
}

fn status_name(status: &crate::models::Status) -> &'static str {
    match status {
        crate::models::Status::Ready => "ready",
        crate::models::Status::Preparing => "preparing",
        crate::models::Status::Running => "running",
        crate::models::Status::Paused => "paused",
        crate::models::Status::Completed => "completed",
        crate::models::Status::Ended => "ended",
        crate::models::Status::Interrupted => "interrupted",
    }
}

fn live_session(snapshot: &crate::models::Snapshot) -> Option<&crate::models::Session> {
    snapshot.sessions.iter().find(|s| {
        matches!(
            s.status,
            crate::models::Status::Running
                | crate::models::Status::Paused
                | crate::models::Status::Preparing
        )
    })
}

fn format_clock(total_ms: u64) -> String {
    let secs = total_ms / 1000;
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// One-line-per-fact status, for humans and scripts alike.
pub fn format_status(snapshot: &crate::models::Snapshot, now_ms: i64) -> String {
    let mut out = String::from("Preelock status\n");
    match live_session(snapshot) {
        Some(session) => {
            out.push_str(&format!(
                "Live session: \"{}\" ({})\n",
                session.title, session.id
            ));
            if crate::notifications::grace_active(session.grace_until_ms, now_ms) {
                out.push_str(&format!(
                    "State: starting in {}s (grace period; blocking has not begun)\n",
                    crate::notifications::grace_remaining_secs(session.grace_until_ms, now_ms)
                ));
            } else {
                out.push_str(&format!(
                    "State: {} — {} left\n",
                    status_name(&session.status),
                    format_clock(session.remaining_ms)
                ));
            }
            if session.focus.frozen.enabled {
                out.push_str("Frozen: yes (cannot be ended early)\n");
            }
        }
        None => out.push_str("Live session: none\n"),
    }
    let enabled = snapshot.schedules.iter().filter(|s| s.enabled).count();
    out.push_str(&format!(
        "Schedules: {} ({} enabled)\nOne-shots: {}\nSaved setups: {}\n",
        snapshot.schedules.len(),
        enabled,
        snapshot.one_shots.len(),
        snapshot.preferences.saved_setups.len()
    ));
    out
}

/// Schedules, one-shots, saved setups, plus the live session summary.
pub fn format_blocks(snapshot: &crate::models::Snapshot, now_ms: i64) -> String {
    let mut out = format_status(snapshot, now_ms);
    out.push_str("\nBlocks:\n");
    if snapshot.schedules.is_empty() && snapshot.one_shots.is_empty() {
        out.push_str("(no schedules or one-shots)\n");
    }
    for schedule in &snapshot.schedules {
        out.push_str(&format!(
            "schedule {} \"{}\" {} {}–{} {}\n",
            schedule.id,
            schedule.title,
            schedule
                .days
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            schedule.start_time,
            schedule.end_time,
            if schedule.enabled {
                "enabled"
            } else {
                "disabled"
            }
        ));
    }
    for shot in &snapshot.one_shots {
        out.push_str(&format!(
            "one-shot {} \"{}\" fires at {} ms, {} min\n",
            shot.id, shot.title, shot.fire_at_ms, shot.minutes
        ));
    }
    if !snapshot.preferences.saved_setups.is_empty() {
        out.push_str("\nSaved setups (start with --start <id>):\n");
        for setup in &snapshot.preferences.saved_setups {
            out.push_str(&format!(
                "setup {} \"{}\" {} min\n",
                setup.id, setup.name, setup.minutes
            ));
        }
    }
    out
}

fn read_snapshot_rows<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    query: &str,
) -> Result<Vec<T>, String> {
    let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.map(|r| serde_json::from_str(&r.map_err(|e| e.to_string())?).map_err(|e| e.to_string()))
        .collect()
}

fn load_from_connection(conn: &rusqlite::Connection) -> Result<crate::models::Snapshot, String> {
    Ok(crate::models::Snapshot {
        browser: None,
        blocked_app_notice: None,
        sessions: read_snapshot_rows(
            conn,
            "SELECT payload FROM sessions WHERE id NOT IN (SELECT id FROM deleted_sessions) ORDER BY rowid",
        )?,
        tasks: read_snapshot_rows(conn, "SELECT payload FROM tasks ORDER BY rowid")?,
        schedules: read_snapshot_rows(conn, "SELECT payload FROM schedules ORDER BY rowid")?,
        one_shots: read_snapshot_rows(conn, "SELECT payload FROM one_shots ORDER BY rowid")?,
        allowances: read_snapshot_rows(conn, "SELECT payload FROM allowances ORDER BY rowid")?,
        app_locks: read_snapshot_rows(conn, "SELECT payload FROM app_locks ORDER BY rowid")?,
        screen_time_usage: {
            let mut stmt = conn
                .prepare("SELECT day_ms, kind, target, name, ms FROM screen_time_usage ORDER BY day_ms, ms DESC")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(crate::models::ScreenTimeBucket {
                        day_ms: r.get(0)?,
                        kind: r.get(1)?,
                        target: r.get(2)?,
                        name: r.get(3)?,
                        ms: r.get::<_, i64>(4)?.max(0) as u64,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.map(|row| row.map_err(|e| e.to_string())).collect::<Result<Vec<_>, _>>()?
        },
        preferences: read_snapshot_rows(conn, "SELECT payload FROM preferences WHERE id=1")?
            .pop()
            .unwrap_or_default(),
    })
}

/// Load the snapshot without writing anything to the user's
/// database: first a read-only connection, and if that cannot open
/// (WAL recovery needs write access), a throwaway copy in the temp
/// directory, deleted afterwards. Never runs migrations or pruning,
/// so the on-disk database is byte-identical afterwards.
pub fn load_snapshot_readonly(db: &Path) -> Result<crate::models::Snapshot, String> {
    if !db.exists() {
        return Err("No Preelock database found yet — start the app once first.".into());
    }
    if let Ok(conn) =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    {
        if let Ok(snapshot) = load_from_connection(&conn) {
            return Ok(snapshot);
        }
    }
    let scratch = std::env::temp_dir().join(format!("preelock-cli-{}.db", std::process::id()));
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    let scratch_wal = PathBuf::from(format!("{}-wal", scratch.display()));
    let copied = std::fs::copy(db, &scratch).is_ok()
        && (!wal.exists() || std::fs::copy(&wal, &scratch_wal).is_ok());
    if !copied {
        let _ = std::fs::remove_file(&scratch);
        let _ = std::fs::remove_file(&scratch_wal);
        return Err("Could not read the Preelock database (it may be locked).".into());
    }
    let result = rusqlite::Connection::open(&scratch)
        .map_err(|e| e.to_string())
        .and_then(|conn| load_from_connection(&conn));
    let _ = std::fs::remove_file(&scratch);
    let _ = std::fs::remove_file(&scratch_wal);
    let _ = std::fs::remove_file(PathBuf::from(format!("{}-shm", scratch.display())));
    result
}

/// Run one CLI invocation. Returns the process exit code: 0 on
/// success, 1 on a runtime failure, 2 on a usage error.
pub fn run(args: &[String]) -> i32 {
    let command = match parse_args(args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    match command {
        Command::Help => {
            println!("{HELP}");
            0
        }
        Command::Version => {
            println!("Preelock {VERSION}");
            0
        }
        Command::Status | Command::ListBlocks => {
            let dir = match app_data_dir() {
                Ok(dir) => dir,
                Err(message) => {
                    eprintln!("Error: {message}");
                    return 1;
                }
            };
            match load_snapshot_readonly(&db_path(&dir)) {
                Ok(snapshot) => {
                    let now = crate::clock::now();
                    if command == Command::Status {
                        println!("{}", format_status(&snapshot, now));
                    } else {
                        println!("{}", format_blocks(&snapshot, now));
                    }
                    0
                }
                Err(message) => {
                    eprintln!("Error: {message}");
                    1
                }
            }
        }
        Command::Start(_) | Command::Stop => {
            let dir = match app_data_dir() {
                Ok(dir) => dir,
                Err(message) => {
                    eprintln!("Error: {message}");
                    return 1;
                }
            };
            // Liveness first: without a fresh heartbeat, no request
            // file is ever written, so nothing can fire later.
            let fresh = read_heartbeat(&dir)
                .is_some_and(|updated| heartbeat_fresh(updated, crate::clock::now()));
            if !fresh {
                eprintln!("Preelock isn't running — start it first.");
                return 1;
            }
            let now = crate::clock::now();
            let (kind, target) = match &command {
                Command::Start(target) => ("start", Some(target.clone())),
                Command::Stop => ("stop", None),
                _ => unreachable!(),
            };
            let request = CliRequest {
                id: format!("{}-{now}", std::process::id()),
                kind: kind.to_string(),
                target,
                created_ms: now,
            };
            if let Err(message) = validate_request(&request, now) {
                eprintln!("Error: {message}");
                return 1;
            }
            if let Err(error) = write_request(&dir, &request) {
                eprintln!("Error: could not write the control request: {error}");
                return 1;
            }
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_millis(RESULT_WAIT_MS);
            loop {
                if let Some(outcome) = take_result(&dir, &request.id) {
                    if outcome.ok {
                        println!("{}", outcome.message);
                        return 0;
                    }
                    eprintln!("Error: {}", outcome.message);
                    return 1;
                }
                if std::time::Instant::now() >= deadline {
                    eprintln!(
                        "Error: Preelock did not respond within {} seconds. \
                         No request is left behind: it was consumed on pickup \
                         or expired after 60 seconds.",
                        RESULT_WAIT_MS / 1000
                    );
                    return 1;
                }
                std::thread::sleep(std::time::Duration::from_millis(RESULT_POLL_MS));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Preferences, Snapshot, Status};

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("preelock-cli-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn empty_snapshot() -> Snapshot {
        Snapshot {
            sessions: vec![],
            tasks: vec![],
            schedules: vec![],
            one_shots: vec![],
            allowances: vec![],
            app_locks: vec![],
            screen_time_usage: vec![],
            preferences: Preferences::default(),
            browser: None,
            blocked_app_notice: None,
        }
    }

    #[test]
    fn parses_every_command() {
        let arg = |s: &str| vec![s.to_string()];
        assert_eq!(parse_args(&arg("--help")).unwrap(), Command::Help);
        assert_eq!(parse_args(&arg("-h")).unwrap(), Command::Help);
        assert_eq!(parse_args(&arg("--version")).unwrap(), Command::Version);
        assert_eq!(parse_args(&arg("--status")).unwrap(), Command::Status);
        assert_eq!(
            parse_args(&arg("--list-blocks")).unwrap(),
            Command::ListBlocks
        );
        assert_eq!(parse_args(&arg("--stop")).unwrap(), Command::Stop);
        assert_eq!(
            parse_args(&["--start".to_string(), "abc-123".to_string()]).unwrap(),
            Command::Start("abc-123".to_string())
        );
        assert!(parse_args(&arg("--start")).is_err());
        assert!(parse_args(&arg("--bogus")).is_err());
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn cli_invocation_only_for_known_flags() {
        assert!(is_cli_invocation(&["--status".to_string()]));
        assert!(is_cli_invocation(&["--start".to_string()]));
        assert!(!is_cli_invocation(&["--bogus".to_string()]));
        assert!(!is_cli_invocation(&[]));
    }

    #[test]
    fn heartbeat_freshness_boundaries() {
        assert!(heartbeat_fresh(10_000, 14_999));
        assert!(!heartbeat_fresh(10_000, 15_000));
        assert!(!heartbeat_fresh(10_000, 9_999));
        assert!(!heartbeat_fresh(10_000, 10_000 + HEARTBEAT_MAX_AGE_MS));
    }

    fn request(kind: &str, target: Option<&str>, created_ms: i64) -> CliRequest {
        CliRequest {
            id: "1-1000".into(),
            kind: kind.into(),
            target: target.map(str::to_string),
            created_ms,
        }
    }

    #[test]
    fn request_validation_rejects_bad_and_stale() {
        let now = 100_000;
        assert!(validate_request(&request("start", Some("abc-1"), now), now).is_ok());
        assert!(validate_request(&request("stop", None, now), now).is_ok());
        assert!(validate_request(&request("start", None, now), now).is_err());
        assert!(validate_request(&request("start", Some(""), now), now).is_err());
        assert!(validate_request(&request("start", Some("a/b"), now), now).is_err());
        assert!(validate_request(&request("start", Some("a b"), now), now).is_err());
        assert!(validate_request(&request("stop", Some("x"), now), now).is_err());
        assert!(validate_request(&request("pause", None, now), now).is_err());
        assert!(validate_request(
            &request("start", Some("abc"), now - REQUEST_MAX_AGE_MS),
            now
        )
        .is_err());
        assert!(validate_request(&request("start", Some("abc"), now + 5_001), now).is_err());
        assert!(validate_request(&request("start", Some("abc"), now - 59_999), now).is_ok());
    }

    #[test]
    fn request_files_are_consumed_and_stale_ones_dropped() {
        let dir = temp_dir("requests");
        let now = 100_000;
        let fresh = request("stop", None, now);
        write_request(&dir, &fresh).unwrap();
        assert!(request_path(&dir, &fresh.id).exists());
        let taken = take_requests(&dir, now);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].kind, "stop");
        // Consumed: the file is gone, so a second sweep finds nothing.
        assert!(!request_path(&dir, &fresh.id).exists());
        assert!(take_requests(&dir, now).is_empty());

        let stale = request("stop", None, now - REQUEST_MAX_AGE_MS);
        write_request(&dir, &stale).unwrap();
        assert!(take_requests(&dir, now).is_empty());
        assert!(!request_path(&dir, &stale.id).exists());

        // Malformed files are deleted, never executed.
        std::fs::write(request_path(&dir, "junk"), "{not json").unwrap();
        assert!(take_requests(&dir, now).is_empty());
        assert!(!request_path(&dir, "junk").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn result_files_expire_after_sixty_seconds() {
        let dir = temp_dir("results");
        let now = 100_000;
        let old = CliOutcome {
            id: "old".into(),
            ok: true,
            message: "done".into(),
            created_ms: now - REQUEST_MAX_AGE_MS,
        };
        write_result(&dir, &old).unwrap();
        cleanup_stale_results(&dir, now);
        assert!(!result_path(&dir, "old").exists());
        let fresh = CliOutcome {
            id: "fresh".into(),
            ok: true,
            message: "done".into(),
            created_ms: now,
        };
        write_result(&dir, &fresh).unwrap();
        cleanup_stale_results(&dir, now);
        assert!(result_path(&dir, "fresh").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schedule_window_minutes_handles_overnight() {
        assert_eq!(schedule_window_minutes("09:00", "10:30"), Some(90));
        assert_eq!(schedule_window_minutes("22:00", "06:00"), Some(480));
        assert_eq!(schedule_window_minutes("09:00", "09:00"), None);
        // parse_time accepts a single-digit hour, so 9:00 is valid.
        assert_eq!(schedule_window_minutes("9:00", "10:00"), Some(60));
        assert_eq!(schedule_window_minutes("25:00", "26:00"), None);
    }

    #[test]
    fn status_and_blocks_format_without_a_live_session() {
        let snapshot = empty_snapshot();
        let text = format_status(&snapshot, 1000);
        assert!(text.contains("Live session: none"));
        assert!(text.contains("Schedules: 0"));
        let blocks = format_blocks(&snapshot, 1000);
        assert!(blocks.contains("no schedules or one-shots"));
    }

    #[test]
    fn status_shows_grace_instead_of_the_full_target() {
        let mut snapshot = empty_snapshot();
        let mut session = crate::models::Session {
            id: "s1".into(),
            title: "Focus".into(),
            duration_minutes: 25,
            remaining_ms: 25 * 60_000,
            focused_ms: 0,
            allow_pause: true,
            status: Status::Running,
            started_at: Some(1000),
            ended_at: None,
            deadline_ms: None,
            focus: Default::default(),
            report: Default::default(),
            attention: Default::default(),
            force_quit: false,
            schedule_id: None,
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: Some(45_000),
        };
        snapshot.sessions.push(session.clone());
        let text = format_status(&snapshot, 1000);
        assert!(text.contains("starting in 44s"));
        session.grace_until_ms = None;
        snapshot.sessions[0] = session;
        let text = format_status(&snapshot, 1000);
        assert!(text.contains("25:00 left"));
    }

    #[test]
    fn readonly_snapshot_loads_a_real_database_unchanged() {
        let dir = temp_dir("db");
        let db = dir.join("preelock.sqlite");
        let mut database = crate::database::Database::open(&db).unwrap();
        let mut snapshot = empty_snapshot();
        snapshot.preferences.default_minutes = 42;
        database.save(&snapshot).unwrap();
        drop(database);
        let before = std::fs::read(&db).unwrap();
        let loaded = load_snapshot_readonly(&db).unwrap();
        assert_eq!(loaded.preferences.default_minutes, 42);
        // The read-only load wrote nothing to the database file.
        assert_eq!(std::fs::read(&db).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
