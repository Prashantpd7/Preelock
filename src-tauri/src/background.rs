//! Independent user-session tracker.
//!
//! The desktop UI starts this process once, but it does not own it. The
//! process has its own SQLite connection and takes ownership of the browser
//! bridge whenever the UI heartbeat expires.
use crate::{
    browser::{Bridge, Policy},
    clock,
    database::Database,
    models::Status,
    platform, screen_time,
};
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
    if !development {
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

fn publish_session(bridge: &Bridge, data: &crate::models::Snapshot) {
    let active = data.sessions.iter().find(|session| {
        matches!(
            session.status,
            Status::Running | Status::Paused | Status::Preparing
        )
    });
    let policy =
        active.map(|session| Policy::new(&session.id, &session.focus, session.remaining_ms));
    bridge.publish(policy);
    bridge.set_screen_time_tracking(true);
}

fn sample(db: &mut Database, bridge: Option<&Bridge>, last: &mut u64) -> Result<(), String> {
    let now = clock::now();
    let current = clock::continuous_ms();
    let delta = current.saturating_sub(*last);
    *last = current;
    if delta == 0 || delta > 3_000 {
        return Ok(());
    }
    let sample = platform::sample()?;
    let Some(bridge) = bridge else {
        if sample.idle_ms >= 60_000 {
            return Ok(());
        }
        let mut buckets = Vec::new();
        screen_time::add_usage(
            &mut buckets,
            screen_time::local_day_start_ms(now),
            "app",
            &sample.id,
            &sample.name,
            delta,
        );
        return db.record_screen_time(&buckets);
    };
    let status = bridge.status();
    let mut buckets = Vec::new();
    let day = screen_time::local_day_start_ms(now);
    if sample.idle_ms < 60_000 && !sample.id.is_empty() {
        screen_time::add_usage(&mut buckets, day, "app", &sample.id, &sample.name, delta);
    }
    if status.activity_fresh && status.browser_focused && !status.active_domain.is_empty() {
        screen_time::add_usage(
            &mut buckets,
            day,
            "site",
            &status.active_domain,
            &status.active_domain,
            delta,
        );
    }
    db.record_screen_time(&buckets)
}

pub fn run() -> i32 {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let Ok(dir) = data_dir() else { return 1 };
    let path = dir.join("preelock.sqlite");
    let Ok(mut db) = Database::open(&path) else {
        return 1;
    };
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
    let mut bridge: Option<Bridge> = None;
    let mut last = clock::continuous_ms();
    loop {
        if desktop_alive(&dir) {
            if let Some(current) = bridge.take() {
                current.stop();
            }
        } else if bridge.is_none() {
            bridge = Bridge::start(&dir).ok();
            if let Some(current) = &bridge {
                current.set_screen_time_tracking(true);
            }
        }
        if let Some(current) = &bridge {
            if let Ok(data) = db.load() {
                publish_session(current, &data);
            }
            current.refresh_screen_time_tracking();
        }
        if !desktop_alive(&dir) {
            if let Err(error) = sample(&mut db, bridge.as_ref(), &mut last) {
                log::warn!("Background screen-time sample failed: {error}");
            }
        } else {
            last = clock::continuous_ms();
        }
        if let Err(error) = touch(&dir.join(HEARTBEAT)) {
            log::warn!("Could not update background heartbeat: {error}");
        }
        thread::sleep(Duration::from_secs(1));
    }
}
