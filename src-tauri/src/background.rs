//! Independent user-session tracker.
//!
//! The desktop UI starts this process once, but it does not own it. The
//! process has its own SQLite connection and takes ownership of the browser
//! bridge whenever the UI heartbeat expires.
use crate::{attention_agent, database::Database, engine::Engine, platform};
use fs2::FileExt;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
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

/// M6: set by the Unix shutdown-signal handler below. launchd sends
/// SIGTERM to this LaunchAgent job on logout/shutdown/restart, and
/// a plain Rust process has no signal API in `std` — `libc` (already
/// in Cargo.lock) is the minimal way to catch it. The handler does
/// nothing but set this flag (async-signal-safe: no locks, no I/O);
/// the main loop notices it within a second, checkpoints the
/// engine, and exits gracefully. Without this, the process would
/// die mid-tick and recovery would rely solely on the periodic
/// ~5 s commit checkpoint — correct, but up to 5 s of productive
/// time could be lost on every shutdown. On Windows there is no
/// equivalent signal for a windowless process: the GUI's
/// `WM_ENDSESSION` handler marks the shutdown, and
/// `RegisterApplicationRestart` (called in `run()` below)
/// re-registers the background for the next login.
#[cfg(unix)]
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_shutdown_signal(_signal: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Release);
}

#[cfg(unix)]
fn install_shutdown_handler() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            on_shutdown_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            on_shutdown_signal as *const () as libc::sighandler_t,
        );
    }
}

/// M6: whether an OS shutdown signal arrived (Unix only; always
/// false elsewhere — see the `SHUTDOWN_REQUESTED` docs).
pub fn shutdown_requested() -> bool {
    #[cfg(unix)]
    {
        SHUTDOWN_REQUESTED.load(Ordering::Acquire)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

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
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/com.preelock.desktop"))
        .ok_or("HOME is unavailable.".into())
}
#[cfg(target_os = "windows")]
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("com.preelock.desktop"))
        .ok_or("APPDATA is unavailable.".into())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    Err("Background tracking is supported on macOS and Windows only.".into())
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The LaunchAgent plist contents for the background tracker (M7).
/// Pure so it can be tested on any platform; only `register_startup`
/// (macOS) writes it.
///
/// Lifecycle semantics, pinned by the tests below:
/// - `RunAtLoad`: launchd starts `--background` at login, inside
///   the user's login session (an Aqua agent, never a LaunchDaemon
///   — nothing here needs, or could use, a system-context daemon:
///   the engine needs the user session, the camera, and the
///   browser bridge).
/// - `KeepAlive` with `SuccessfulExit` false: launchd restarts the
///   job when it crashes or is killed, but NOT after a clean exit.
///   The old bare `<true/>` restarted even a clean exit — including
///   the duplicate `--background` that exits 0 on the already-held
///   `background.lock`, which launchd would then respawn in a loop,
///   and the graceful SIGTERM exit at logout/shutdown (M6), which
///   needs no restart because the machine is going down.
/// - `StandardOutPath`/`StandardErrorPath`: the background is a
///   headless process, so without these its log output is discarded
///   by launchd. `~/Library/Logs` always exists on macOS, so
///   launchd can create the files there.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn launch_agent_plist(exe: &str, home: &str) -> String {
    let exe = xml_escape(exe);
    let home = xml_escape(home);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.preelock.background</string>
<key>ProgramArguments</key><array><string>{exe}</string><string>{ARG}</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>StandardOutPath</key><string>{home}/Library/Logs/com.preelock.background.stdout.log</string>
<key>StandardErrorPath</key><string>{home}/Library/Logs/com.preelock.background.stderr.log</string>
</dict></plist>"#
    )
}

#[cfg(target_os = "macos")]
fn register_startup(exe: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable.")?;
    let agents = PathBuf::from(&home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    let plist = agents.join("com.preelock.background.plist");
    // launchd reads this directory at login, so a plist written
    // mid-session takes effect at the next login; the current
    // session is covered because `ensure_running` also spawns
    // `--background` directly. Rewriting it on every GUI launch
    // keeps it pointing at the current install location if the
    // app was moved.
    std::fs::write(
        &plist,
        launch_agent_plist(&exe.to_string_lossy(), &home.to_string_lossy()),
    )
    .map_err(|e| e.to_string())?;
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
    // M6: catch the logout/shutdown signal (Unix) so the engine can
    // checkpoint before the process exits; see `SHUTDOWN_REQUESTED`.
    #[cfg(unix)]
    install_shutdown_handler();
    #[cfg(target_os = "windows")]
    unsafe {
        let args: Vec<u16> = "--background\0".encode_utf16().collect();
        let _ = windows_sys::Win32::System::Recovery::RegisterApplicationRestart(args.as_ptr(), 0);
    }
    loop {
        // M6: no engine is open in the desktop-alive branch, so
        // there is nothing to checkpoint — just exit promptly.
        if shutdown_requested() {
            return 0;
        }
        if desktop_alive(&dir) {
            // The desktop GUI's tray thread drives the attention agent
            // while it is alive; this process must not issue its own
            // camera commands, or the two drivers will flap the camera
            // whenever they disagree. The background only takes over
            // driving once the desktop heartbeat expires (below).
            let needs_attention = attention_needed(&path);
            if needs_attention {
                if let Err(error) = attention_agent::ensure_running(&dir) {
                    log::warn!("Could not start attention agent: {error}");
                }
            }

            thread::sleep(Duration::from_secs(1));
            continue;
        }
        match Engine::open_background(&path) {
            Ok(mut engine) => {
                loop {
                    if desktop_alive(&dir) {
                        break;
                    }
                    // M6: an OS shutdown signal arrived while this
                    // engine owns the session — checkpoint it as-is
                    // (live sessions stay live) and exit gracefully
                    // instead of dying mid-tick. Dropping the engine
                    // then stops the camera and clears the website
                    // policy, which is correct: the machine is going
                    // down, and the session resumes from the
                    // checkpoint at the next login.
                    if shutdown_requested() {
                        if let Err(error) = engine.save_checkpoint() {
                            log::warn!("Could not save shutdown checkpoint: {error}");
                        }
                        return 0;
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
                    // M2: the GUI is down and a session is live — make
                    // sure the tray process owns the menu-bar icon so
                    // the session stays visible. This is the path that
                    // covers force-quit / crash, where no exit handler
                    // ran to spawn it. Never spawned while the GUI is
                    // alive (the desktop-alive branch above returns
                    // before reaching here).
                    if engine.has_active() {
                        let _ = crate::tray::ensure_running(&dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_agent_plist_registers_a_supervised_login_agent() {
        let plist = launch_agent_plist(
            "/Applications/Preelock.app/Contents/MacOS/Preelock",
            "/Users/test",
        );
        assert!(plist.contains("<key>Label</key><string>com.preelock.background</string>"));
        assert!(plist.contains(
            "<string>/Applications/Preelock.app/Contents/MacOS/Preelock</string><string>--background</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        // Crash-only supervision: restarts on a crash/kill, never
        // after a clean exit (no respawn loop for the lock-held
        // duplicate, no restart after the M6 SIGTERM exit).
        assert!(
            plist.contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>")
        );
        assert!(!plist.contains("<key>KeepAlive</key><true/>"));
        // Headless output goes to files instead of being discarded.
        assert!(plist.contains(
            "<string>/Users/test/Library/Logs/com.preelock.background.stdout.log</string>"
        ));
        assert!(plist.contains(
            "<string>/Users/test/Library/Logs/com.preelock.background.stderr.log</string>"
        ));
        // A user-session agent, never a system daemon.
        assert!(!plist.contains("LaunchDaemon"));
        assert!(!plist.contains("LimitLoadToSessionType"));
    }

    #[test]
    fn launch_agent_plist_escapes_xml_in_paths() {
        let plist = launch_agent_plist("/tmp/A&B <Preelock>/Preelock", "/Users/a&b");
        assert!(plist.contains("/tmp/A&amp;B &lt;Preelock&gt;/Preelock"));
        assert!(plist.contains("/Users/a&amp;b/Library/Logs/"));
        assert!(!plist.contains("A&B <Preelock>"));
    }

    /// M6: the shutdown-signal handler only sets the flag — the
    /// loop does the checkpointing. Raising SIGTERM against the
    /// test process proves the handler is installed and that the
    /// flag is what the loop reads. The default disposition is
    /// restored and the flag cleared so no later test inherits it.
    #[cfg(unix)]
    #[test]
    fn shutdown_signal_sets_the_flag() {
        install_shutdown_handler();
        assert!(!shutdown_requested());
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        assert!(shutdown_requested());
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
        }
        SHUTDOWN_REQUESTED.store(false, Ordering::Release);
        assert!(!shutdown_requested());
    }
}
