//! Structured lifecycle diagnostics for the installed application.
//!
//! The notification-after-Quit defect could only be diagnosed by looking at
//! LaunchServices state and process identity, so every lifecycle decision now
//! emits one `key=value` line. Lines go to stderr (launchd captures the
//! background service's stderr at
//! `~/Library/Logs/com.preelock.background.stderr.log`) and are mirrored into a
//! small rotating file:
//! - macOS: `~/Library/Logs/Preelock/lifecycle.log`
//! - Windows: `%LOCALAPPDATA%\Preelock\Logs\lifecycle.log`
//!
//! `scripts/collect-macos-diagnostics.sh` bundles that file with the
//! LaunchServices / process state the fix depends on.
//!
//! NEVER log notification titles, bodies, session titles or any other user
//! content here: this file is collected from users' machines.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Once;

/// Bundle identifier of the macOS application. Pinned by
/// `bundle_identifier_matches_tauri_config` against `tauri.conf.json` so the
/// diagnostics can never drift from the shipped bundle.
#[cfg(target_os = "macos")]
pub const MACOS_BUNDLE_ID: &str = "com.preelock.desktop";

/// Process environment variable carrying the launch source across a
/// LaunchServices launch (`notification`, `tray`, `supervisor`,
/// `second-instance`, `cli`, or absent for a direct launch).
pub const LAUNCH_SOURCE_ENV: &str = "PREELOCK_LAUNCH_SOURCE";
/// Process environment variable carrying the correlation id across a cold
/// launch, so the native click handler, the coordinator and the GUI log the
/// same interaction.
pub const CORRELATION_ENV: &str = "PREELOCK_CORRELATION_ID";

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

struct TeeWriter {
    file: std::sync::Mutex<Option<std::fs::File>>,
}

impl TeeWriter {
    fn open() -> Self {
        let file = log_file().and_then(|path| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // One rotation is enough for a diagnostic trace: keep the previous
            // run as `.1` rather than growing without bound.
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
                let rotated = path.with_extension("log.1");
                let _ = std::fs::remove_file(&rotated);
                let _ = std::fs::rename(&path, &rotated);
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        Self {
            file: std::sync::Mutex::new(file),
        }
    }
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.write_all(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        if let Ok(mut guard) = self.file.lock() {
            if let Some(file) = guard.as_mut() {
                let _ = file.flush();
            }
        }
        Ok(())
    }
}

/// Where the lifecycle trace is written, when the platform has a log home.
pub fn log_file() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Logs/Preelock/lifecycle.log"))
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("TEMP"))
            .map(PathBuf::from)?;
        Some(base.join("Preelock").join("Logs").join("lifecycle.log"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Some(std::env::temp_dir().join("preelock-lifecycle.log"))
    }
}

/// Initialize logging once per process, for every process mode. Called from
/// `main.rs` before mode dispatch (and idempotent, so tests and library
/// callers are safe).
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("warn,preelock_lib=info"),
        );
        builder.target(env_logger::Target::Pipe(Box::new(TeeWriter::open())));
        let _ = builder.try_init();
    });
}

/// A new correlation id for one interaction: `<kind>-<epoch_ms>-<seq>`.
pub fn new_correlation(kind: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{kind}-{}-{}",
        crate::clock::now(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// The launch source recorded in the process environment by the process that
/// requested this launch (`notification`, `tray`, `supervisor`, ...).
pub fn launch_source() -> String {
    std::env::var(LAUNCH_SOURCE_ENV).unwrap_or_else(|_| "direct".to_string())
}

/// The correlation id carried across a cold launch, if any.
pub fn incoming_correlation() -> Option<String> {
    std::env::var(CORRELATION_ENV)
        .ok()
        .filter(|id| !id.is_empty())
}

fn parent_pid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getppid() as u32
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn executable_path() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn bundle_identifier() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        MACOS_BUNDLE_ID
    }
    #[cfg(not(target_os = "macos"))]
    {
        ""
    }
}

/// Emit the `process_start` line for a mode and return this process's
/// correlation id (the incoming one for a coordinated launch, otherwise a new
/// one). No user content is included.
pub fn log_process_start(mode: &str) -> String {
    let correlation = incoming_correlation().unwrap_or_else(|| new_correlation(mode));
    log::info!(
        "lifecycle event=process_start mode={mode} ts_ms={} platform={} version={} pid={} ppid={} exe={} bundle_id={} launch_source={} correlation={correlation}",
        crate::clock::now(),
        std::env::consts::OS,
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        parent_pid(),
        executable_path(),
        bundle_identifier(),
        launch_source()
    );
    correlation
}

/// One notification click handed to a cold-launched (or running) GUI. Contains
/// identifiers only — never the notification title or body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingNotificationClick {
    #[serde(default)]
    pub correlation: String,
    #[serde(default)]
    pub request_identifier: String,
    #[serde(default)]
    pub action_identifier: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub received_at_ms: i64,
}

pub fn pending_click_path(dir: &Path) -> PathBuf {
    dir.join("notification-click.json")
}

/// Read AND consume the pending click. A second call returns `None`, so a
/// notification response is handled at most once even if the GUI restarts
/// during startup.
pub fn take_pending_notification_click(dir: &Path) -> Option<PendingNotificationClick> {
    let path = pending_click_path(dir);
    let data = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    match serde_json::from_slice::<PendingNotificationClick>(&data) {
        Ok(click) => Some(click),
        Err(error) => {
            log::warn!("lifecycle event=notification_click_corrupt error={error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_ids_are_unique_and_prefixed() {
        let first = new_correlation("notification");
        let second = new_correlation("notification");
        assert!(first.starts_with("notification-"));
        assert_ne!(first, second);
    }

    #[test]
    fn pending_click_is_consumed_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(take_pending_notification_click(dir.path()).is_none());
        let click = PendingNotificationClick {
            correlation: "notification-1".into(),
            request_identifier: "preelock-reminder-deadbeef".into(),
            action_identifier: "com.apple.UNNotificationDefaultActionIdentifier".into(),
            kind: "reminder".into(),
            received_at_ms: 1_700_000_000_000,
        };
        std::fs::write(
            pending_click_path(dir.path()),
            serde_json::to_vec(&click).unwrap(),
        )
        .unwrap();
        assert_eq!(
            take_pending_notification_click(dir.path()),
            Some(click.clone())
        );
        // Consumed: the file is gone and a duplicate delivery is a no-op.
        assert!(!pending_click_path(dir.path()).exists());
        assert!(take_pending_notification_click(dir.path()).is_none());
    }

    #[test]
    fn corrupt_pending_click_is_dropped_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(pending_click_path(dir.path()), b"not json").unwrap();
        assert!(take_pending_notification_click(dir.path()).is_none());
        assert!(!pending_click_path(dir.path()).exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bundle_identifier_matches_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"].as_str(), Some(MACOS_BUNDLE_ID));
    }
}
