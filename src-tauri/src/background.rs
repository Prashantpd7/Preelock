//! Independent user-session tracker.
//!
//! The desktop UI starts this process once, but it does not own it. The
//! process has its own SQLite connection and takes ownership of the browser
//! bridge whenever the UI heartbeat expires.
use crate::{attention_agent, database::Database, engine::Engine, platform};
use fs2::FileExt;
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const ARG: &str = "--background";
const HEARTBEAT: &str = "background-heartbeat";
const DESKTOP_HEARTBEAT: &str = "desktop-heartbeat";
const STALE_MS: i64 = 3_000;

pub fn is_invocation(args: &[String]) -> bool {
    args.iter().any(|arg| arg == ARG)
}

fn fresh(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(age) = modified.elapsed() else {
        return false;
    };
    age <= Duration::from_millis(STALE_MS as u64)
}

fn touch(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.set_len(0).map_err(|e| e.to_string())
}

pub fn tracker_alive(data_dir: &Path) -> bool {
    fresh(&data_dir.join(HEARTBEAT))
}

pub fn desktop_alive(data_dir: &Path) -> bool {
    fresh(&data_dir.join(DESKTOP_HEARTBEAT))
}

fn lock_held(data_dir: &Path) -> bool {
    let Ok(lock) = File::create(data_dir.join("background.lock")) else {
        return true;
    };
    if lock.try_lock_exclusive().is_ok() {
        let _ = lock.unlock();
        false
    } else {
        true
    }
}

fn attention_needed(path: &Path) -> bool {
    Database::open(path)
        .and_then(|db| db.load())
        .map(|data| {
            data.sessions.iter().any(|session| {
                matches!(
                    session.status,
                    crate::models::Status::Running | crate::models::Status::Preparing
                ) && session.focus.camera
                    && session.focus.attention.enabled
                    && session.focus.attention.verification == "camera_and_activity"
            })
        })
        .unwrap_or(false)
}

pub fn touch_desktop(data_dir: &Path) {
    if let Err(error) = touch(&data_dir.join(DESKTOP_HEARTBEAT)) {
        log::warn!("Could not update desktop heartbeat: {error}");
    }
}

#[cfg(target_os = "macos")]
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/com.preelock.desktop"))
        .ok_or("HOME is unavailable.".into())
}
#[cfg(target_os = "windows")]
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("com.preelock.desktop"))
        .ok_or("APPDATA is unavailable.".into())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn data_dir() -> Result<PathBuf, String> {
    Err("Background tracking is supported on macOS and Windows only.".into())
}

#[cfg(target_os = "macos")]
fn register_startup(exe: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable.")?;
    let agents = PathBuf::from(home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    let plist = agents.join("com.preelock.background.plist");
    let escaped = exe
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let contents = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.preelock.background</string>
<key>ProgramArguments</key><array><string>{escaped}</string><string>{ARG}</string></array>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
</dict></plist>"#
    );
    std::fs::write(&plist, contents).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn register_startup(exe: &Path) -> Result<(), String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| e.to_string())?;
    key.set_value(
        "Preelock Background",
        &format!("\"{}\" {}", exe.display(), ARG),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn register_startup(_: &Path) -> Result<(), String> {
    Ok(())
}

pub fn ensure_running(data_dir: &Path) -> Result<(), String> {
    touch_desktop(data_dir);
    if let Err(error) = platform::enable_background_startup() {
        log::warn!("Could not enable independent background startup: {error}");
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.to_string_lossy().to_lowercase().replace('/', "\\");
    let development = path.contains("\\target\\debug\\") || path.contains("\\target\\release\\");
    if !development && !platform::packaged() {
        register_startup(&exe)?;
    }
    if tracker_alive(data_dir) || lock_held(data_dir) {
        return Ok(());
    }
    Command::new(&exe)
        .arg(ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start background tracker: {e}"))
}

pub fn run() -> i32 {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let Ok(dir) = data_dir() else { return 1 };
    let path = dir.join("preelock.sqlite");
    let Ok(lock) = File::create(dir.join("background.lock")) else {
        return 1;
    };
    if lock.try_lock_exclusive().is_err() {
        return 0;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        let args: Vec<u16> = "--background\0".encode_utf16().collect();
        let _ = windows_sys::Win32::System::Recovery::RegisterApplicationRestart(args.as_ptr(), 0);
    }
    loop {
        if desktop_alive(&dir) {
            let needs_attention = attention_needed(&path);
            if needs_attention {
                if let Err(error) = attention_agent::ensure_running(&dir) {
                    log::warn!("Could not start attention agent: {error}");
                }
            }

            let _ = attention_agent::signal(&dir, needs_attention);
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        match Engine::open_background(&path) {
            Ok(mut engine) => {
                loop {
                    if desktop_alive(&dir) {
                        break;
                    }
                    let needs_attention = engine.needs_attention_agent();
                    if needs_attention {
                        if let Err(error) = attention_agent::ensure_running(&dir) {
                            log::warn!("Could not start attention agent: {error}");
                        }
                    }

                    let signal = attention_agent::signal(&dir, needs_attention).unwrap_or(
                        attention_agent::Signal {
                            status: "camera_unavailable".into(),
                            sampled_at_ms: 0,
                        },
                    );
                    engine.set_attention_camera_status(signal);
                    if let Err(error) = engine.tick() {
                        log::error!("Background engine tick failed: {error}");
                        break;
                    }
                    if let Err(error) = touch(&dir.join(HEARTBEAT)) {
                        log::warn!("Could not update background heartbeat: {error}");
                    }
                    thread::sleep(Duration::from_secs(1));
                }
                drop(engine);
            }
            Err(error) => {
                log::error!("Could not open background engine: {error}");
                thread::sleep(Duration::from_secs(1));
            }
        }

        if let Err(error) = touch(&dir.join(HEARTBEAT)) {
            log::warn!("Could not update background heartbeat: {error}");
        }
    }
}
