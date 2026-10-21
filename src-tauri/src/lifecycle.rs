//! Distinguish OS restarts/logouts from a process killed during the same boot.
//! The marker contains no session data; productive time remains in SQLite.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    boot: Option<String>,
    orderly: bool,
}
static CHECKPOINT: OnceLock<(PathBuf, Option<String>)> = OnceLock::new();
static ENDING: AtomicBool = AtomicBool::new(false);

fn write(path: &Path, checkpoint: &Checkpoint) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(checkpoint).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
pub fn startup(path: &Path, boot: Option<String>) -> Result<bool, String> {
    let previous = std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Checkpoint>(&data).ok());
    let resume = previous.is_some_and(|old| {
        old.orderly || matches!((&old.boot, &boot), (Some(a), Some(b)) if a != b)
    });
    write(
        path,
        &Checkpoint {
            boot,
            orderly: false,
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
pub fn ending() -> bool {
    ENDING.load(Ordering::Acquire)
}
pub extern "C" fn system_shutdown() {
    ENDING.store(true, Ordering::Release);
    // Never acquire the engine lock on the OS event thread: sampling may be
    // waiting on that same thread. SQLite already checkpoints every 5 seconds.
    if let Some((path, boot)) = CHECKPOINT.get() {
        if let Err(error) = write(
            path,
            &Checkpoint {
                boot: boot.clone(),
                orderly: true,
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
                boot: None,
                orderly: true,
            },
        )
        .unwrap();
        assert!(startup(&path, None).unwrap());
        assert!(!startup(&path, None).unwrap());
        std::fs::write(&path, "incomplete").unwrap();
        assert!(!startup(&path, None).unwrap());
    }
}
