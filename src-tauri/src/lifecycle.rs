//! Distinguish OS restarts/logouts from a process killed during the same boot.
//! The marker contains no session data; productive time remains in SQLite.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        OnceLock,
    },
};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCheckpoint {
    pub session_id: String,
    pub kind: String,
    pub status: String,
    pub mode: String,
    pub schedule_id: Option<String>,
    pub schedule_title: Option<String>,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    pub started_at_ms: Option<i64>,
    pub intended_end_at_ms: Option<i64>,
    pub remaining_ms: u64,
    #[serde(default)]
    pub blocked_apps: Vec<String>,
    #[serde(default)]
    pub allowed_apps: Vec<String>,
    #[serde(default)]
    pub blocked_sites: Vec<String>,
    #[serde(default)]
    pub allowed_sites: Vec<String>,
    pub recovery_status: String,
    pub last_checkpoint_ms: i64,
    pub last_enforcement_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub boot: Option<String>,
    pub orderly: bool,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub corrupt_recovered: bool,
    #[serde(default)]
    pub active_session: Option<SessionCheckpoint>,
}

static CHECKPOINT: OnceLock<(PathBuf, Option<String>)> = OnceLock::new();
static ENDING: AtomicBool = AtomicBool::new(false);
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Crash-safe atomic write: serialize into a sibling temp file, fsync its
/// contents, rename over `path`, and clean up the temp file on failure so
/// a sudden power loss can never truncate `lifecycle.json`.
pub fn write(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Invalid lifecycle checkpoint path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(checkpoint).map_err(|e| e.to_string())?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".preelock.lifecycle.{}.{seq}.tmp", std::process::id());
    let tmp = parent.join(tmp_name);
    let write_res = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if write_res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_res.map_err(|e| e.to_string())
}

pub fn read_checkpoint(path: &Path) -> Option<Checkpoint> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice::<Checkpoint>(&data).ok()
}

pub fn startup(path: &Path, boot: Option<String>) -> Result<bool, String> {
    let (previous, corrupt_recovered) = match std::fs::read(path) {
        Ok(data) => match serde_json::from_slice::<Checkpoint>(&data) {
            Ok(parsed) => (Some(parsed), false),
            Err(error) => {
                log::warn!(
                    "Recovering corrupted lifecycle checkpoint {}: {error}",
                    path.display()
                );
                (None, true)
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, false),
        Err(error) => {
            log::warn!(
                "Could not read lifecycle checkpoint {}: {error}",
                path.display()
            );
            (None, false)
        }
    };
    let resume = previous.as_ref().is_some_and(|old| {
        old.orderly || matches!((&old.boot, &boot), (Some(a), Some(b)) if a != b)
    });
    let active_session = previous.and_then(|old| old.active_session);
    write(
        path,
        &Checkpoint {
            boot,
            orderly: false,
            updated_at_ms: crate::clock::now(),
            corrupt_recovered,
            active_session,
        },
    )?;
    Ok(resume)
}

pub fn initialize(database: &Path) -> Result<bool, String> {
    let path = database.with_extension("lifecycle.json");
    let boot = boot_id();
    let resume = startup(&path, boot.clone())?;
    CHECKPOINT
        .set((path, boot))
        .map_err(|_| "Lifecycle already initialized".to_string())?;
    Ok(resume)
}

/// Update only the durable active-session summary (keeping current `boot`
/// and clearing `orderly` only if not shutting down).
pub fn write_session_checkpoint(
    path: &Path,
    boot: Option<String>,
    orderly: bool,
    active_session: Option<SessionCheckpoint>,
) -> Result<(), String> {
    let corrupt_recovered = read_checkpoint(path).is_some_and(|c| c.corrupt_recovered);
    write(
        path,
        &Checkpoint {
            boot,
            orderly,
            updated_at_ms: crate::clock::now(),
            corrupt_recovered,
            active_session,
        },
    )
}

pub fn current_boot_id() -> Option<String> {
    CHECKPOINT
        .get()
        .and_then(|(_, b)| b.clone())
        .or_else(boot_id)
}

pub fn ending() -> bool {
    ENDING.load(Ordering::Acquire)
}

// Called from the macOS quit guard (desktop_settings.rs) and the
// Windows settings path (platform/settings_windows.rs). Linux has
// no call site, so this is dead code there only — never delete it
// for a Linux-only warning, or the macOS/Windows builds lose their
// orderly-shutdown marker.
#[allow(dead_code)]
pub extern "C" fn system_shutdown() {
    ENDING.store(true, Ordering::Release);
    // Never acquire the engine lock on the OS event thread: sampling may be
    // waiting on that same thread. SQLite already checkpoints every 5 seconds.
    if let Some((path, boot)) = CHECKPOINT.get() {
        let existing_session = read_checkpoint(path).and_then(|c| c.active_session);
        if let Err(error) = write(
            path,
            &Checkpoint {
                boot: boot.clone(),
                orderly: true,
                updated_at_ms: crate::clock::now(),
                corrupt_recovered: false,
                active_session: existing_session,
            },
        ) {
            log::error!("Could not mark system shutdown: {error}");
        }
    }
}
#[cfg(target_os = "macos")]
fn boot_id() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.bootsessionuuid"])
        .output()
        .ok()?;
    let id = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && !id.is_empty()).then_some(id)
}
#[cfg(target_os = "windows")]
fn boot_id() -> Option<String> {
    use std::os::windows::process::CommandExt;
    // A stable OS boot timestamp, not wall-clock-minus-uptime (which changes
    // when the user adjusts the clock). Fast Startup is covered by WM_ENDSESSION.
    let mut child = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().Ticks",
        ])
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    while child.try_wait().ok()?.is_none() {
        if start.elapsed() > std::time::Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let output = child.wait_with_output().ok()?;
    let id = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (output.status.success() && id.len() >= 16 && id.chars().all(|c| c.is_ascii_digit()))
        .then_some(id)
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn boot_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reboot_resumes_but_same_boot_force_quit_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lifecycle.json");
        assert!(!startup(&path, Some("boot-a".into())).unwrap());
        assert!(!startup(&path, Some("boot-a".into())).unwrap());
        assert!(startup(&path, Some("boot-b".into())).unwrap());
        assert!(!startup(&path, Some("boot-b".into())).unwrap());
    }
    #[test]
    fn orderly_shutdown_marker_is_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lifecycle.json");
        write(
            &path,
            &Checkpoint {
                orderly: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(startup(&path, None).unwrap());
        assert!(!startup(&path, None).unwrap());
        std::fs::write(&path, "incomplete").unwrap();
        assert!(!startup(&path, None).unwrap());
        assert!(read_checkpoint(&path).unwrap().corrupt_recovered);
    }
    #[test]
    fn session_checkpoint_and_legacy_json_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lifecycle.json");
        std::fs::write(&path, r#"{"boot":"old-boot","orderly":false}"#).unwrap();
        assert!(startup(&path, Some("new-boot".into())).unwrap());

        let summary = SessionCheckpoint {
            session_id: "sess-1".into(),
            kind: "scheduled".into(),
            status: "running".into(),
            mode: "strict".into(),
            schedule_id: Some("sched-1".into()),
            schedule_title: Some("Morning Deep Work".into()),
            weekdays: vec![1, 2, 3, 4, 5],
            start_time: Some("09:00".into()),
            end_time: Some("11:00".into()),
            started_at_ms: Some(1_700_000_000_000),
            intended_end_at_ms: Some(1_700_007_200_000),
            remaining_ms: 4_800_000,
            blocked_apps: vec!["com.apple.Safari".into()],
            allowed_apps: vec![],
            blocked_sites: vec![],
            allowed_sites: vec!["docs.rs".into()],
            recovery_status: "clean".into(),
            last_checkpoint_ms: 1_700_002_400_000,
            last_enforcement_at_ms: Some(1_700_002_400_000),
        };
        write_session_checkpoint(&path, Some("new-boot".into()), false, Some(summary.clone()))
            .unwrap();
        let loaded = read_checkpoint(&path).unwrap();
        assert_eq!(loaded.active_session, Some(summary));
        // Clean atomic rename leaves no dangling temp files.
        let temps: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty());
    }
}
