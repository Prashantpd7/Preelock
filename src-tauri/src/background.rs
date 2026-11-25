//! Independent user-session focus supervisor and authoritative Engine owner.
//!
//! Architecture:
//! - `--background` is the SINGLE authoritative owner of `Engine`, SQLite,
//!   `engine.lock`, `browser::Bridge` (127.0.0.1:47832), recurring schedules,
//!   one-shot timers, application/website enforcement, CLI request execution,
//!   and durable crash/reboot checkpoints.
//! - The GUI process connects over authenticated loopback IPC (`crate::ipc`)
//!   and renders the window, tray, and overlays. Closing or quitting the UI
//!   leaves `--background` operational.
//! - During any active session (`has_active()`), if the UI exits (Cmd+Q,
//!   macOS Force Quit, or Windows Task Manager), the supervisor relaunches
//!   the Preelock GUI in the user's interactive session. When idle, UI exit
//!   is never reopened.

use crate::{
    attention_agent,
    engine::Engine,
    ipc::{BrowserSetupInfo, IpcRequest, IpcResponse, IpcServer, PingInfo, UninstallCheck},
    platform,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const ARG: &str = "--background";
const HEARTBEAT: &str = "background-heartbeat";
const DESKTOP_HEARTBEAT: &str = "desktop-heartbeat";
const SERVICE_STATUS_FILE: &str = "background-service-status.json";
const META_FILE: &str = "background-meta.json";
#[allow(dead_code)]
const LAUNCH_AGENT_LABEL: &str = "com.preelock.background";
#[cfg(target_os = "windows")]
const WINDOWS_TASK_NAME: &str = "PreelockBackground";
const STALE_MS: i64 = 3_000;
/// Missing-UI debounce before a relaunch. A killed GUI never sends
/// `UiExiting`, so this delay is measured from the poll that first saw the
/// lock released and the recorded PID gone — 200 ms is enough to ride out a
/// handover and keeps the whole reopen inside the 1 s budget. The in-flight
/// grace window below, not this delay, is what prevents relaunch storms.
pub const RELAUNCH_DELAY_MS: u64 = 200;
pub const RELAUNCH_COOLDOWN_MS: u64 = 3_000;
/// Grace window after a supervisor launch (2026-11-17 loop fix). Once the
/// supervisor fires a launch, `evaluate` must not fire another one until
/// this window has elapsed — a freshly spawned GUI needs time to take
/// `gui.lock` / heartbeat before the next poll can see it, and the old
/// 3 s cooldown alone was short enough to fire 9 launches in 24 s while
/// every spawned copy died instantly. The window ends early as soon as any
/// liveness evidence reports the GUI alive (`poll_ui_alive` clears it).
pub const LAUNCH_GRACE_MS: u64 = 10_000;
/// Circuit breaker: stop relaunching after this many consecutive launches
/// where the UI never stayed alive. A GUI that dies on every launch would
/// otherwise be relaunched forever (one launch per grace window), filling
/// the Dock and hanging the machine. The counter resets as soon as the UI
/// is seen alive. (2026-11-23 fix: unlimited Dock icons + laptop hang.)
pub const MAX_CONSECUTIVE_RELAUNCHES: u32 = 3;

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

#[cfg(windows)]
unsafe extern "system" fn on_windows_ctrl(ctrl_type: u32) -> i32 {
    // 0 = CTRL_C_EVENT, 2 = CTRL_CLOSE_EVENT, 5 = CTRL_LOGOFF_EVENT, 6 = CTRL_SHUTDOWN_EVENT
    if matches!(ctrl_type, 0 | 2 | 5 | 6) {
        SHUTDOWN_REQUESTED.store(true, Ordering::Release);
        crate::lifecycle::system_shutdown();
        1
    } else {
        0
    }
}

#[cfg(windows)]
fn install_shutdown_handler() {
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler_routine: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(on_windows_ctrl), 1);
    }
}

#[cfg(not(any(unix, windows)))]
fn install_shutdown_handler() {}

pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Acquire) || crate::lifecycle::ending()
}

/// Whether some Preelock process currently owns the background lock.
/// Used by the startup safety net to distinguish an engine that is
/// merely slow to answer from one that does not exist.
pub fn engine_process_alive(data_dir: &Path) -> bool {
    lock_held(data_dir)
}

/// Stops the background engine for quit cleanup and removes its runtime
/// endpoint/meta so nothing can reconnect to a dying process.
///
/// Quitting the app must leave no engine behind: every respawn path is
/// closed (the LaunchAgent has no KeepAlive and the GUI is gone).
pub fn stop_background(data_dir: &Path) {
    let mut pids = Vec::new();
    if let Some(meta) = read_background_meta(data_dir) {
        pids.push(meta.pid);
    }
    if let Some(ep) = crate::ipc::read_endpoint(&data_dir.join(crate::ipc::ENDPOINT_FILE)) {
        if !pids.contains(&ep.pid) {
            pids.push(ep.pid);
        }
    }
    if let Some(holder) = find_preelock_lock_holder(&data_dir.join("background.lock")) {
        if !pids.contains(&holder) {
            pids.push(holder);
        }
    }
    for pid in pids {
        if process_alive(pid) {
            log::info!("lifecycle event=background_stop_for_quit pid={pid}");
            terminate_process(pid, data_dir);
        }
    }
    clear_stale_runtime_files(data_dir);
    let _ = std::fs::remove_file(data_dir.join(HEARTBEAT));
}

pub fn is_invocation(args: &[String]) -> bool {
    args.iter().any(|arg| arg == ARG)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceHealth {
    pub healthy: bool,
    pub state: String,
    pub detail: String,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BackgroundMeta {
    pid: u32,
    exe: String,
    ipc: bool,
    updated_ms: i64,
    /// Modification time of the binary that wrote this meta, ms
    /// since the Unix epoch; 0 when the writer predates this
    /// field. A rebuild rewrites the binary at the same path, so
    /// the path alone cannot tell a fresh binary from a stale one
    /// — the mtime can.
    #[serde(default)]
    exe_mtime_ms: u64,
}

/// Modification time of `exe`, ms since the Unix epoch; `None`
/// when it cannot be read.
fn exe_mtime_ms(exe: &Path) -> Option<u64> {
    std::fs::metadata(exe)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Whether the running background was started from a different
/// binary than the one now on disk: a different path, or the same
/// path rewritten by a rebuild (the mtime tells them apart). A
/// meta without an mtime predates this check, so it counts as
/// stale — the replacement writes a fresh meta, so this fires at
/// most once per upgrade. When the on-disk mtime cannot be read,
/// keep the running background rather than kill the engine on a
/// guess.
fn background_binary_mismatch(meta: &BackgroundMeta, exe: &Path) -> bool {
    if !meta.exe.is_empty() && meta.exe != exe.to_string_lossy() {
        return true;
    }
    match exe_mtime_ms(exe) {
        Some(current) => meta.exe_mtime_ms != current,
        None => false,
    }
}

fn write_service_health(data_dir: &Path, healthy: bool, state: &str, detail: &str) {
    let status = ServiceHealth {
        healthy,
        state: state.into(),
        detail: detail.into(),
        updated_ms: crate::clock::now(),
    };
    if let Ok(bytes) = serde_json::to_vec(&status) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(data_dir.join(SERVICE_STATUS_FILE), bytes);
    }
}

pub fn startup_diagnostic(data_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(data_dir.join(SERVICE_STATUS_FILE)).ok()?;
    let status = serde_json::from_slice::<ServiceHealth>(&bytes).ok()?;
    if status.healthy {
        None
    } else {
        Some(format!("{} ({})", status.detail, status.state))
    }
}

fn read_background_meta(data_dir: &Path) -> Option<BackgroundMeta> {
    let bytes = std::fs::read(data_dir.join(META_FILE)).ok()?;
    serde_json::from_slice::<BackgroundMeta>(&bytes).ok()
}

fn write_background_meta(data_dir: &Path, ipc_ready: bool) {
    let exe_path = std::env::current_exe().ok();
    let exe = exe_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let exe_mtime_ms = exe_path.as_ref().and_then(|p| exe_mtime_ms(p)).unwrap_or(0);
    let meta = BackgroundMeta {
        pid: std::process::id(),
        exe,
        ipc: ipc_ready,
        updated_ms: crate::clock::now(),
        exe_mtime_ms,
    };
    if let Ok(bytes) = serde_json::to_vec(&meta) {
        let _ = std::fs::create_dir_all(data_dir);
        let _ = std::fs::write(data_dir.join(META_FILE), bytes);
    }
}

fn clear_stale_runtime_files(data_dir: &Path) {
    crate::ipc::clear_endpoint(data_dir);
    let _ = std::fs::remove_file(data_dir.join(META_FILE));
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

pub fn touch_desktop(data_dir: &Path) {
    if let Err(error) = touch(&data_dir.join(DESKTOP_HEARTBEAT)) {
        log::warn!("Could not update desktop heartbeat: {error}");
    }
}

pub fn clear_desktop_heartbeat(data_dir: &Path) {
    let _ = std::fs::remove_file(data_dir.join(DESKTOP_HEARTBEAT));
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

#[cfg(target_os = "macos")]
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    if let Some(custom) = std::env::var_os("PREELOCK_DATA_DIR") {
        return Ok(PathBuf::from(custom));
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/com.preelock.desktop"))
        .ok_or("HOME is unavailable.".into())
}
#[cfg(target_os = "windows")]
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    if let Some(custom) = std::env::var_os("PREELOCK_DATA_DIR") {
        return Ok(PathBuf::from(custom));
    }
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("com.preelock.desktop"))
        .ok_or("APPDATA is unavailable.".into())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    Err("Background tracking is supported on macOS and Windows only.".into())
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Pure LaunchAgent plist builder (macOS 12+, user Aqua domain).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn launch_agent_plist(exe: &str, home: &str) -> String {
    let exe = xml_escape(exe);
    let home = xml_escape(home);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LAUNCH_AGENT_LABEL}</string>
<key>ProgramArguments</key><array><string>{exe}</string><string>{ARG}</string></array>
<key>RunAtLoad</key><true/>
<!-- No KeepAlive: launchd must NEVER respawn the engine on its own.
     A supervisor that ignores the GUI's circuit breaker kept filling
     the Dock with trying engines after the user quit (2026-11-24).
     The engine exits 0 when it cannot take the lock; the GUI starts it
     again on the next launch. -->
<key>ThrottleInterval</key><integer>5</integer>
<key>StandardOutPath</key><string>{home}/Library/Logs/{LAUNCH_AGENT_LABEL}.stdout.log</string>
<key>StandardErrorPath</key><string>{home}/Library/Logs/{LAUNCH_AGENT_LABEL}.stderr.log</string>
</dict></plist>"#
    )
}

/// Pure Windows Task Scheduler XML for per-user interactive background
/// supervision (`LogonTrigger` + `InteractiveToken` + `RestartOnFailure`).
#[cfg(any(target_os = "windows", test))]
pub(crate) fn scheduled_task_xml(exe: &str) -> String {
    let exe = xml_escape(exe);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{ARG}</Arguments>
    </Exec>
  </Actions>
</Task>"#
    )
}

#[cfg(any(target_os = "macos", test))]
/// Resolve the enclosing `.app` wrapper for a bundled executable. The GUI is
/// now launched only through the native coordinator (which uses
/// `NSBundle.mainBundle`), so this remains as the tested definition of what
/// "inside a bundle" means for the packaging checks.
#[allow(dead_code)]
pub(crate) fn app_bundle_root(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find_map(|ancestor| {
        (ancestor.extension().is_some_and(|e| e == "app")
            && ancestor.join("Contents").join("Info.plist").exists())
        .then(|| ancestor.to_path_buf())
    })
}

#[cfg(target_os = "macos")]
fn launchctl_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
fn register_startup(exe: &Path, data_dir: &Path) -> Result<bool, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable.")?;
    let agents = PathBuf::from(&home).join("Library/LaunchAgents");
    std::fs::create_dir_all(&agents).map_err(|e| e.to_string())?;
    let plist = agents.join(format!("{LAUNCH_AGENT_LABEL}.plist"));
    let expected_contents = launch_agent_plist(&exe.to_string_lossy(), &home.to_string_lossy());
    let needs_write =
        std::fs::read_to_string(&plist).ok().as_deref() != Some(expected_contents.as_str());
    if needs_write {
        std::fs::write(&plist, &expected_contents).map_err(|e| e.to_string())?;
    }

    // Bootstrap into the active Aqua GUI domain immediately so launchd starts
    // supervising crashes during the *current* session rather than next login.
    let domain = launchctl_domain();
    let target = format!("{domain}/{LAUNCH_AGENT_LABEL}");
    let disabled_check = Command::new("/bin/launchctl")
        .args(["print-disabled", &domain])
        .output();
    if let Ok(out) = disabled_check {
        let txt = String::from_utf8_lossy(&out.stdout);
        let disabled_marker = format!("\"{LAUNCH_AGENT_LABEL}\" => disabled");
        if txt.contains(&disabled_marker) {
            write_service_health(
                data_dir,
                false,
                "disabled",
                "Background item is disabled in System Settings -> Login Items.",
            );
            return Ok(false);
        }
    }

    let printed = Command::new("/bin/launchctl")
        .args(["print", &target])
        .output();
    let already_loaded_for_exe = printed.as_ref().is_ok_and(|out| {
        out.status.success()
            && String::from_utf8_lossy(&out.stdout).contains(exe.to_string_lossy().as_ref())
    });

    // Reload when the executable changed OR the plist contents did. A
    // loaded agent keeps its OLD configuration until it is booted out:
    // skipping this left the previous KeepAlive in force, so launchd
    // kept respawning engines after the GUI quit (2026-11-24).
    if !already_loaded_for_exe || needs_write {
        if printed.as_ref().is_ok_and(|out| out.status.success()) {
            let _ = Command::new("/bin/launchctl")
                .args(["bootout", &target])
                .output();
        }
        let bootstrap = Command::new("/bin/launchctl")
            .args(["bootstrap", &domain, &plist.to_string_lossy()])
            .output();
        match bootstrap {
            Ok(out) if out.status.success() => {
                write_service_health(data_dir, true, "bootstrapped", "LaunchAgent active.");
                return Ok(true);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                log::warn!("launchctl bootstrap returned non-zero: {stderr}");
                write_service_health(data_dir, false, "bootstrap_failed", &stderr);
                return Ok(false);
            }
            Err(error) => {
                write_service_health(data_dir, false, "bootstrap_error", &error.to_string());
                return Ok(false);
            }
        }
    }

    write_service_health(data_dir, true, "loaded", "LaunchAgent active.");
    if !lock_held(data_dir) {
        let _ = Command::new("/bin/launchctl")
            .args(["kickstart", &target])
            .output();
    }
    Ok(true)
}

#[cfg(target_os = "windows")]
fn register_startup(exe: &Path, data_dir: &Path) -> Result<bool, String> {
    use std::os::windows::process::CommandExt;
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    // Keep HKCU\Run fallback alongside Scheduled Task supervision.
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map_err(|e| e.to_string())?;
    key.set_value(
        "Preelock Background",
        &format!("\"{}\" {}", exe.display(), ARG),
    )
    .map_err(|e| e.to_string())?;

    // Per-user Scheduled Task provides non-admin RestartOnFailure crash recovery.
    let xml = scheduled_task_xml(&exe.to_string_lossy());
    let utf16: Vec<u16> = std::iter::once(0xFEFF).chain(xml.encode_utf16()).collect();
    let bytes: Vec<u8> = utf16.iter().flat_map(|u| u.to_le_bytes()).collect();
    let tmp_xml = data_dir.join("preelock-bg-task.xml");
    if let Err(e) = std::fs::create_dir_all(data_dir).and_then(|_| std::fs::write(&tmp_xml, bytes))
    {
        write_service_health(data_dir, false, "task_xml_error", &e.to_string());
        return Ok(false);
    }
    let res = Command::new("schtasks.exe")
        .args([
            "/Create",
            "/TN",
            WINDOWS_TASK_NAME,
            "/XML",
            &tmp_xml.to_string_lossy(),
            "/F",
        ])
        .creation_flags(0x08000000)
        .output();
    let _ = std::fs::remove_file(&tmp_xml);
    match res {
        Ok(out) if out.status.success() => {
            write_service_health(
                data_dir,
                true,
                "scheduled_task_active",
                "Task Scheduler active.",
            );
            // Run the scheduled task immediately so it runs independently under svchost.exe
            let _ = Command::new("schtasks.exe")
                .args(["/Run", "/TN", WINDOWS_TASK_NAME])
                .creation_flags(0x08000000)
                .output();
            Ok(true)
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            log::warn!("schtasks registration fell back to Run key: {err}");
            write_service_health(data_dir, false, "scheduled_task_failed", &err);
            Ok(false)
        }
        Err(error) => {
            write_service_health(data_dir, false, "scheduled_task_error", &error.to_string());
            Ok(false)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn register_startup(_: &Path, _: &Path) -> Result<bool, String> {
    Ok(false)
}

/// Ensure the independent background engine is registered (in packaged
/// non-MSIX builds) and running, touching `desktop-heartbeat` because this is
/// invoked during GUI launch.
pub fn ensure_running(data_dir: &Path) -> Result<(), String> {
    touch_desktop(data_dir);
    ensure_running_inner(data_dir)
}

/// Ensure the background engine is running without updating the GUI heartbeat
/// (used by IPC self-healing or headless probes).
pub fn ensure_running_headless(data_dir: &Path) -> Result<(), String> {
    ensure_running_inner(data_dir)
}

fn terminate_process(pid: u32, data_dir: &Path) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    let gui_held = std::fs::read_to_string(data_dir.join("gui.lock"))
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok())
        .is_some_and(|gui_pid| gui_pid == pid);
    if gui_held {
        log::warn!("Refusing to terminate PID {pid}: currently holding gui.lock");
        return;
    }
    #[cfg(unix)]
    unsafe {
        let is_preelock = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "args="])
            .output()
            .ok()
            .is_some_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_lowercase()
                    .contains("preelock")
            });
        if !is_preelock {
            log::warn!("Refusing to terminate PID {pid}: process is not preelock");
            return;
        }
        let _ = libc::kill(pid as libc::pid_t, libc::SIGTERM);
        for _ in 0..20 {
            if !process_alive(pid) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = libc::kill(pid as libc::pid_t, libc::SIGKILL);
        for _ in 0..10 {
            if !process_alive(pid) {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    #[cfg(windows)]
    {
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
                PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
            };
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                0,
                pid,
            );
            if !handle.is_null() {
                let mut buffer = vec![0u16; 32768];
                let mut size = buffer.len() as u32;
                let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
                let is_preelock = if ok != 0 {
                    let image_path =
                        String::from_utf16_lossy(&buffer[..size as usize]).to_lowercase();
                    image_path.ends_with("preelock.exe") || image_path.ends_with("preelock")
                } else {
                    false
                };

                if is_preelock {
                    let _ = TerminateProcess(handle, 1);
                } else {
                    log::warn!(
                        "Refusing to terminate PID {pid}: process image is not preelock.exe"
                    );
                }
                CloseHandle(handle);
            }
        }
        for _ in 0..20 {
            if !process_alive(pid) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(unix)]
fn find_preelock_lock_holder(lock_path: &Path) -> Option<u32> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-t", &lock_path.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let self_pid = std::process::id();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            if pid != 0 && pid != self_pid {
                let ps_out = Command::new("/bin/ps")
                    .args(["-p", &pid.to_string(), "-o", "args="])
                    .output()
                    .ok();
                if ps_out.as_ref().is_some_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .to_lowercase()
                        .contains("preelock")
                }) {
                    return Some(pid);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn find_preelock_lock_holder(lock_path: &Path) -> Option<u32> {
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let self_pid = std::process::id();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut candidate = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32ProcessID != self_pid && entry.th32ProcessID != 0 {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                    if exe_name == "preelock.exe" {
                        // Check if this process holds the GUI lock. If so, it's the active GUI, not a background process.
                        let gui_held = lock_path
                            .parent()
                            .map(|d| d.join("gui.lock"))
                            .and_then(|p| std::fs::read_to_string(p).ok())
                            .and_then(|content| content.trim().parse::<u32>().ok())
                            .is_some_and(|p| p == entry.th32ProcessID);
                        if !gui_held {
                            candidate = Some(entry.th32ProcessID);
                            break;
                        }
                    }
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        candidate
    }
}

#[cfg(not(any(unix, windows)))]
fn find_preelock_lock_holder(_lock_path: &Path) -> Option<u32> {
    None
}

fn stop_stale_background(data_dir: &Path) {
    let mut candidate_pids = Vec::new();
    if let Some(meta) = read_background_meta(data_dir) {
        candidate_pids.push(meta.pid);
    }
    if let Some(ep) = crate::ipc::read_endpoint(&data_dir.join(crate::ipc::ENDPOINT_FILE)) {
        if !candidate_pids.contains(&ep.pid) {
            candidate_pids.push(ep.pid);
        }
    }
    if let Some(holder) = find_preelock_lock_holder(&data_dir.join("background.lock")) {
        if !candidate_pids.contains(&holder) {
            candidate_pids.push(holder);
        }
    }
    for pid in candidate_pids {
        if process_alive(pid) {
            log::warn!("Stopping stale or non-IPC background process PID {pid}");
            terminate_process(pid, data_dir);
        }
    }
    clear_stale_runtime_files(data_dir);
    for _ in 0..20 {
        if !lock_held(data_dir) {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

struct SpawnGuardState {
    last_spawn_time_ms: u64,
    consecutive_failed_spawns: u32,
}

static SPAWN_GUARD: std::sync::Mutex<SpawnGuardState> = std::sync::Mutex::new(SpawnGuardState {
    last_spawn_time_ms: 0,
    consecutive_failed_spawns: 0,
});

pub fn reset_spawn_circuit_breaker() {
    if let Ok(mut guard) = SPAWN_GUARD.lock() {
        guard.consecutive_failed_spawns = 0;
        guard.last_spawn_time_ms = 0;
    }
}

pub fn manual_restart_background(data_dir: &Path) -> Result<(), String> {
    reset_spawn_circuit_breaker();
    ensure_running_inner(data_dir)
}

fn ensure_running_inner(data_dir: &Path) -> Result<(), String> {
    let client = crate::ipc::IpcClient::new(data_dir);
    // Fast path: if already alive and pingable, reset circuit breaker and return immediately.
    if client.ping().is_ok() {
        if let Ok(mut guard) = SPAWN_GUARD.lock() {
            guard.consecutive_failed_spawns = 0;
        }
        return Ok(());
    }

    // Process-wide lock: only ONE thread can evaluate, check, or spawn at a time.
    let mut guard = SPAWN_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    // Re-check ping after acquiring lock in case another thread just spawned it
    if client.ping().is_ok() {
        guard.consecutive_failed_spawns = 0;
        return Ok(());
    }

    // Circuit breaker check: MAX_CONSECUTIVE_RELAUNCHES = 3
    if guard.consecutive_failed_spawns >= MAX_CONSECUTIVE_RELAUNCHES {
        log::warn!(
            "Background engine spawn circuit breaker tripped after {} failed attempts; not spawning.",
            guard.consecutive_failed_spawns
        );
        return Err("Background engine unavailable — restart the app".to_string());
    }

    // Cooldown check: 5 seconds (5000 ms)
    let now_ms = crate::clock::continuous_ms();
    if guard.last_spawn_time_ms > 0 && now_ms.saturating_sub(guard.last_spawn_time_ms) < 5_000 {
        log::warn!("Background engine spawn cooldown in progress (last attempt was <5s ago); skipping spawn.");
        return Err("Background engine spawn cooldown active".to_string());
    }

    guard.last_spawn_time_ms = now_ms;

    if let Err(error) = platform::enable_background_startup() {
        log::warn!("Could not enable independent background startup: {error}");
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.to_string_lossy().to_lowercase().replace('/', "\\");
    let development = path.contains("\\target\\debug\\") || path.contains("\\target\\release\\");
    let mut supervised_started = false;
    if !development && !platform::packaged() {
        supervised_started = register_startup(&exe, data_dir)?;
    } else if development {
        write_service_health(
            data_dir,
            true,
            "dev_child",
            "Development direct background process.",
        );
    }
    if lock_held(data_dir) {
        let mut ipc_ok = client.ping().is_ok();
        if !ipc_ok {
            let meta = read_background_meta(data_dir);
            let starting_up = meta
                .as_ref()
                .is_some_and(|m| !m.ipc && (crate::clock::now() - m.updated_ms).abs() < 10_000);
            let max_polls = if starting_up { 100 } else { 10 };
            for _ in 0..max_polls {
                thread::sleep(Duration::from_millis(50));
                if client.ping().is_ok() {
                    ipc_ok = true;
                    break;
                }
                if !lock_held(data_dir) {
                    break;
                }
            }
        }
        if ipc_ok {
            // Dev only: a rebuild rewrites the binary at the same
            // path, so a path-only check keeps a stale background
            // serving IPC — and a stale background 400s any action
            // variant it predates (e.g. `set_app_lock_strict`),
            // which the GUI then retries for ~2 s before giving
            // up. The mtime catches the rebuild.
            let mismatch_dev = development
                && read_background_meta(data_dir)
                    .is_some_and(|m| background_binary_mismatch(&m, &exe));
            if !mismatch_dev {
                guard.consecutive_failed_spawns = 0;
                return Ok(());
            }
            log::info!("Development binary replacing existing background engine.");
            stop_stale_background(data_dir);
        } else if lock_held(data_dir) {
            log::warn!("Background lock holder is not responding to IPC; replacing it.");
            stop_stale_background(data_dir);
        }
    } else {
        clear_stale_runtime_files(data_dir);
    }
    if supervised_started {
        // Give launchd a brief moment to spawn the bootstrapped agent before
        // falling back to a direct spawn.
        for _ in 0..40 {
            if client.ping().is_ok() {
                guard.consecutive_failed_spawns = 0;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        if lock_held(data_dir) {
            if client.ping().is_ok() {
                guard.consecutive_failed_spawns = 0;
            }
            return Ok(());
        }
    }
    let mut command = Command::new(&exe);
    command
        .arg(ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS
        command.creation_flags(0x08000000 | 0x00000200 | 0x01000000 | 0x00000008);
    }
    command
        .spawn()
        .map_err(|e| format!("Could not start background engine: {e}"))?;

    // Wait for the spawned engine to initialize and answer ping (up to 3.0s)
    let mut ping_success = false;
    for _ in 0..60 {
        thread::sleep(Duration::from_millis(50));
        if client.ping().is_ok() {
            ping_success = true;
            break;
        }
    }

    if ping_success {
        guard.consecutive_failed_spawns = 0;
        log::info!("Background engine spawned and responded to ping successfully.");
        Ok(())
    } else {
        guard.consecutive_failed_spawns += 1;
        log::warn!(
            "Spawned background engine failed to respond to ping (consecutive failures: {}/{}).",
            guard.consecutive_failed_spawns,
            MAX_CONSECUTIVE_RELAUNCHES
        );
        if guard.consecutive_failed_spawns >= MAX_CONSECUTIVE_RELAUNCHES {
            Err("Background engine unavailable — restart the app".to_string())
        } else {
            Err("Background engine spawned but did not respond to ping in time".to_string())
        }
    }
}

struct BackgroundLock(File);

impl BackgroundLock {
    fn try_acquire(data_dir: &Path) -> Option<Self> {
        let _ = std::fs::create_dir_all(data_dir);
        let file = File::create(data_dir.join("background.lock")).ok()?;
        if file.try_lock_exclusive().is_ok() {
            Some(Self(file))
        } else {
            None
        }
    }
}

impl Drop for BackgroundLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Pure, unit-testable supervisor deciding when `--background` should reopen
/// the main UI process after Cmd+Q / Force Quit / Task Manager termination.
#[derive(Default, Debug)]
pub struct UiRelaunchSupervisor {
    known_ui_pid: Option<u32>,
    ui_exiting_at: Option<u64>,
    ui_missing_since: Option<u64>,
    last_relaunch_at: Option<u64>,
    /// End of the in-flight launch grace window (`LAUNCH_GRACE_MS` after
    /// the last launch `evaluate` authorised). While set and unexpired,
    /// `evaluate` never fires again — a previous launch is still in flight
    /// and the new GUI has not had time to register yet.
    pending_launch_until_ms: Option<u64>,
    /// Consecutive supervisor launches after which the UI never stayed
    /// alive. Reset to 0 as soon as the UI is seen alive. When it reaches
    /// `MAX_CONSECUTIVE_RELAUNCHES` the circuit breaker trips and
    /// `evaluate` refuses to launch again until the UI is seen alive.
    consecutive_relaunches: u32,
}

impl UiRelaunchSupervisor {
    pub fn mark_ui_alive(&mut self, pid: u32) {
        self.known_ui_pid = Some(pid);
        self.ui_exiting_at = None;
        self.ui_missing_since = None;
        self.pending_launch_until_ms = None;
        self.consecutive_relaunches = 0;
    }

    pub fn mark_ui_exiting(&mut self, pid: u32, now_ms: u64) {
        if self.known_ui_pid.is_none() || self.known_ui_pid == Some(pid) {
            self.ui_exiting_at = Some(now_ms);
            self.ui_missing_since.get_or_insert(now_ms);
        }
    }

    pub fn known_ui_pid(&self) -> Option<u32> {
        self.known_ui_pid
    }

    /// Reconcile the known PID, gui.lock, and heartbeat into a boolean
    /// `ui_alive`. The headless supervisor must not query AppKit: doing so can
    /// register this process as a LaunchServices application.
    pub fn poll_ui_alive(&mut self, data_dir: &Path) -> bool {
        let (gui_lock_held, gui_lock_pid) = gui_lock_status(data_dir);
        let mut known_ui_process_died = false;
        // A known PID counts only while it still owns gui.lock. PID
        //    existence alone is not proof: the OS can reuse a force-quit
        //    GUI's PID for an unrelated process.
        if let Some(pid) = self.known_ui_pid {
            let pid_matches = gui_lock_pid.map(|p| p == pid).unwrap_or(true);
            if gui_lock_held && pid_matches && process_alive(pid) {
                self.pending_launch_until_ms = None;
                return true;
            }
            known_ui_process_died = !process_alive(pid);
            self.known_ui_pid = None;
            self.ui_exiting_at = None;
            // Fall through: a new GUI may already have taken the lock
            // during a handover. Keep the heartbeat as a fallback unless
            // this known GUI process has actually died; then its fresh
            // heartbeat is only stale shutdown residue.
        }
        // The file lock is authoritative. Its text PID may be stale or
        //    reused after Force Quit, but a held exclusive lock proves a GUI
        //    still owns the singleton; a stale file without the lock does not.
        if gui_lock_held {
            self.known_ui_pid = gui_lock_pid
                .filter(|pid| *pid != 0 && *pid != std::process::id() && process_alive(*pid));
            self.pending_launch_until_ms = None;
            return true;
        }
        // A free lock plus a `gui.pid` whose process is gone is proof the GUI
        //    died right now (SIGKILL leaves the sidecar behind, but the OS
        //    releases the lock and reaps the PID immediately). Without this,
        //    a Force Quit waited out the 3 s heartbeat staleness before the
        //    relaunch delay even started — the whole 1 s budget spent on a
        //    signal that was never going to recover. `gui.pid` is absent in
        //    the handover case the heartbeat fallback exists for, so a GUI
        //    that has not written it yet still counts as alive by heartbeat.
        if !known_ui_process_died {
            if let Some(pid) = recorded_gui_pid(data_dir) {
                if !process_alive(pid) {
                    return false;
                }
            }
        }
        // Desktop heartbeat: only the GUI touches it (setup + its 1 s
        //    heartbeat thread via `UiHeartbeat`), so a fresh one means a
        //    GUI is alive even if every pid source above is stale.
        if !known_ui_process_died && desktop_alive(data_dir) {
            self.pending_launch_until_ms = None;
            return true;
        }
        false
    }

    /// Evaluate whether the UI should be relaunched at `now_ms`.
    /// Returns `true` only for an active session with no live UI, outside
    /// shutdown, after the missing delay and cooldown, and with no earlier
    /// launch still in flight inside `LAUNCH_GRACE_MS`.
    pub fn evaluate(
        &mut self,
        now_ms: u64,
        active_session: bool,
        ui_alive: bool,
        shutting_down: bool,
    ) -> bool {
        if shutting_down || !active_session {
            self.ui_missing_since = None;
            self.pending_launch_until_ms = None;
            return false;
        }
        if ui_alive {
            self.ui_missing_since = None;
            self.pending_launch_until_ms = None;
            self.consecutive_relaunches = 0;
            return false;
        }
        // Start (or keep) the missing clock BEFORE the grace check: the UI
        // has been missing throughout the grace window, so the 1 s delay
        // must be measured from when it first went missing — not restarted
        // when the window expires, or a relaunch would always slip a
        // further second after every grace window.
        let since = *self.ui_missing_since.get_or_insert(now_ms);
        if self
            .pending_launch_until_ms
            .is_some_and(|until| now_ms < until)
        {
            return false;
        }
        if now_ms.saturating_sub(since) < RELAUNCH_DELAY_MS {
            return false;
        }
        if self
            .last_relaunch_at
            .is_some_and(|t| now_ms.saturating_sub(t) < RELAUNCH_COOLDOWN_MS)
        {
            return false;
        }
        // Circuit breaker: if the UI died on every one of the last
        // `MAX_CONSECUTIVE_RELAUNCHES` launches, stop launching. The GUI
        // is crashing on startup; relaunching it forever fills the Dock
        // and hangs the machine. The user can reopen the app by hand.
        if self.consecutive_relaunches >= MAX_CONSECUTIVE_RELAUNCHES {
            log::warn!(
                "UI relaunch circuit breaker tripped after {} consecutive failed launches; not relaunching until the UI is seen alive.",
                self.consecutive_relaunches
            );
            return false;
        }
        self.last_relaunch_at = Some(now_ms);
        self.consecutive_relaunches += 1;
        self.ui_missing_since = None;
        self.pending_launch_until_ms = Some(now_ms.saturating_add(LAUNCH_GRACE_MS));
        true
    }
}

/// Win32 `ERROR_LOCK_VIOLATION` (33): what `LockFileEx` reports when a file
/// is already locked.
#[cfg(windows)]
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Whether `error` means "another process holds the file lock".
///
/// `fs2::FileExt::try_lock_exclusive` reports contention differently per OS:
/// Unix `flock` fails with `EWOULDBLOCK` (`ErrorKind::WouldBlock`), but
/// Windows `LockFileEx` fails with `ERROR_LOCK_VIOLATION`, which `std`
/// maps to `ErrorKind::Uncategorized` — never `WouldBlock` (see
/// `std::sys::io::error::windows::decode_error_kind`). Checking only
/// `WouldBlock` therefore reports a held lock as free on Windows, and
/// `poll_ui_alive` wrongly declares the UI dead during a handover. `std`'s
/// own `File::try_lock` works around the same gap by checking the raw OS
/// error, and this does the same.
fn is_lock_contention(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    // Windows only: `LockFileEx` contention is `ERROR_LOCK_VIOLATION`,
    // which `std` maps to `ErrorKind::Uncategorized`.
    #[cfg(windows)]
    return error.raw_os_error() == Some(ERROR_LOCK_VIOLATION);
    #[cfg(not(windows))]
    false
}

/// The PID the GUI wrote into `gui.pid` (Windows sidecar, and the unlocked
/// copy Unix keeps for readers). `None` when it is missing or unparsable.
fn recorded_gui_pid(data_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(data_dir.join("gui.pid"))
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok())
        .filter(|pid| *pid != 0)
}

fn gui_lock_status(data_dir: &Path) -> (bool, Option<u32>) {
    let path = data_dir.join("gui.lock");
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
    else {
        return (false, None);
    };

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            (false, None)
        }
        Err(error) if is_lock_contention(&error) => {
            let pid = std::fs::read_to_string(data_dir.join("gui.pid"))
                .or_else(|_| std::fs::read_to_string(&path))
                .ok()
                .and_then(|content| content.trim().parse::<u32>().ok());
            (true, pid)
        }
        Err(_) => (false, None),
    }
}

fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    unsafe {
        let rc = libc::kill(pid as libc::pid_t, 0);
        if rc == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let wait = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        wait == WAIT_TIMEOUT
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

fn spawn_gui_in_user_session(spawned: &mut Option<Child>) -> bool {
    // macOS: route through LaunchServices so a supervisor launch coalesces
    // with a concurrent Dock or notification launch.
    #[cfg(target_os = "macos")]
    {
        let _ = spawned;
        let correlation = crate::lifecycle_log::new_correlation("supervisor");
        let outcome = crate::platform::request_gui_activation("supervisor", &correlation);
        log::info!(
            "lifecycle event=gui_request result={} source=supervisor correlation={correlation}",
            outcome.as_str()
        );
        if outcome != crate::platform::GuiActivation::Failed {
            return true;
        }
        #[cfg(debug_assertions)]
        if let Ok(exe) = std::env::current_exe() {
            let bundled = exe.ancestors().any(|path| {
                path.extension().is_some_and(|extension| extension == "app")
                    && path.join("Contents").join("Info.plist").exists()
            });
            if !bundled {
                if std::net::TcpStream::connect_timeout(
                    &std::net::SocketAddr::from(([127, 0, 0, 1], 1420)),
                    Duration::from_millis(150),
                )
                .is_err()
                {
                    log::warn!(
                        "Vite dev server is unreachable at 127.0.0.1:1420; skipping raw debug GUI relaunch."
                    );
                    return false;
                }
                let mut cmd = Command::new(exe);
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                match cmd.spawn() {
                    Ok(child) => {
                        *spawned = Some(child);
                        return true;
                    }
                    Err(error) => {
                        log::warn!("Could not relaunch the debug Preelock UI: {error}");
                        return false;
                    }
                }
            }
        }
        // A Force Quit / `kill -9` leaves LaunchServices holding a record
        // for the dead instance, so the coalescing `open -a` above fails
        // with -600 (procNotFound) instead of starting a new GUI — the
        // supervisor then burned its whole circuit-breaker budget on
        // launches that could never happen. Force a new instance instead.
        // `GuiLock` still admits exactly one GUI: if a real launch is
        // already in flight it wins the lock and this instance exits.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(bundle) = app_bundle_root(&exe) {
                let launched = Command::new("/usr/bin/open")
                    .arg("-n")
                    .arg("-a")
                    .arg(&bundle)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false);
                if launched {
                    log::warn!(
                        "lifecycle event=gui_request result=forced_new_instance source=supervisor correlation={correlation}"
                    );
                    return true;
                }
                log::error!("Forced GUI relaunch failed for bundle {}", bundle.display());
            }
        }
        log::error!("Could not request a Preelock GUI launch through LaunchServices.");
    }

    #[cfg(target_os = "windows")]
    if crate::platform::packaged() {
        let _ = spawned;
        let correlation = crate::lifecycle_log::new_correlation("supervisor");
        let outcome = crate::platform::request_gui_activation("supervisor", &correlation);
        log::info!(
            "lifecycle event=gui_request result={} source=supervisor correlation={correlation}",
            outcome.as_str()
        );
        if outcome == crate::platform::GuiActivation::Failed {
            log::error!("Could not request a packaged Preelock GUI launch.");
        }
        return outcome != crate::platform::GuiActivation::Failed;
    }

    #[cfg(not(target_os = "macos"))]
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    #[cfg(not(target_os = "macos"))]
    let gui = crate::launch::gui_executable(&exe);

    #[cfg(not(target_os = "macos"))]
    {
        // In development debug builds, Tauri loads the UI from the Vite dev server (127.0.0.1:1420).
        // If Vite is not running, launching the raw debug binary results in a blank white window.
        // Guard against launching the unbundled debug binary if the dev server is unreachable.
        #[cfg(debug_assertions)]
        let dev_server_unreachable = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], 1420)),
            Duration::from_millis(150),
        )
        .is_err();
        #[cfg(not(debug_assertions))]
        let dev_server_unreachable = false;
        if dev_server_unreachable {
            log::warn!(
                "Vite dev server is unreachable at 127.0.0.1:1420; skipping raw debug GUI relaunch to avoid white window."
            );
            false
        } else {
            let mut cmd = Command::new(&gui);
            if let Some(parent) = gui.parent() {
                cmd.current_dir(parent);
            }
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x00000200); // CREATE_NEW_PROCESS_GROUP
            }
            match cmd.spawn() {
                Ok(child) => {
                    *spawned = Some(child);
                    true
                }
                Err(error) => {
                    log::warn!("Could not relaunch Preelock UI {}: {error}", gui.display());
                    false
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        false
    }
}

fn reap_child(child_slot: &mut Option<Child>) {
    if let Some(child) = child_slot.as_mut() {
        if matches!(child.try_wait(), Ok(Some(_))) {
            *child_slot = None;
        }
    }
}

fn handle_ipc_request(
    engine: &mut Engine,
    dir: &Path,
    supervisor: &mut UiRelaunchSupervisor,
    req: IpcRequest,
) -> IpcResponse {
    match req {
        IpcRequest::Ping => IpcResponse::success(&PingInfo {
            pid: std::process::id(),
            has_active: engine.has_active(),
            has_active_strict: engine.has_active_strict(),
            recovered_after_os_shutdown: engine.recovered_after_os_shutdown(),
            notification_prefs: engine.notification_prefs(),
        }),
        IpcRequest::UiHeartbeat { pid } => {
            supervisor.mark_ui_alive(pid);
            touch_desktop(dir);
            IpcResponse::success(&PingInfo {
                pid: std::process::id(),
                has_active: engine.has_active(),
                has_active_strict: engine.has_active_strict(),
                recovered_after_os_shutdown: engine.recovered_after_os_shutdown(),
                notification_prefs: engine.notification_prefs(),
            })
        }
        IpcRequest::UiExiting { pid } => {
            supervisor.mark_ui_exiting(pid, crate::clock::continuous_ms());
            clear_desktop_heartbeat(dir);
            let _ = engine.save_checkpoint();
            IpcResponse::success(&true)
        }
        IpcRequest::TestNotification { title, body } => {
            crate::desktop_settings::notify(&title, &body);
            IpcResponse::success(&serde_json::json!({ "sent": true }))
        }
        IpcRequest::Snapshot => match engine.snapshot() {
            Ok(s) => IpcResponse::success(&s),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::Execute { action } => match engine.execute(action) {
            Ok(s) => IpcResponse::success(&s),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::BlockAttempts => match engine.block_attempts() {
            Ok(list) => IpcResponse::success(&list),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::ExportStatistics => match engine.export_statistics() {
            Ok(json_str) => IpcResponse::success(&json_str),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::DeleteStatistics => match engine.delete_statistics() {
            Ok(count) => IpcResponse::success(&count),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::Backup => match engine.backup() {
            Ok(snap) => IpcResponse::success(&snap),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::ReplaceData { data } => match engine.replace_data(data) {
            Ok(()) => IpcResponse::success(&true),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::TestCamera => match engine.test_camera() {
            Ok(()) => IpcResponse::success(&true),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::StopTest => {
            engine.stop_test();
            IpcResponse::success(&true)
        }
        IpcRequest::ResetBrowserPairing => match engine.reset_pairing(dir) {
            Ok(()) => IpcResponse::success(&true),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::TestBrowserConnection => match engine.test_browser() {
            Ok(()) => IpcResponse::success(&true),
            Err(e) => IpcResponse::err(e),
        },
        IpcRequest::BrowserTestResult => IpcResponse::success(&engine.browser_test_result()),
        IpcRequest::DismissBlockedAppPage => {
            engine.dismiss_blocked_app_page();
            IpcResponse::success(&true)
        }
        IpcRequest::BrowserSetupInfo { browser_id } => {
            engine.ensure_bridge_ready();
            IpcResponse::success(&BrowserSetupInfo {
                browser: engine.browser_status_for(&browser_id),
                connected_browsers: engine.connected_browsers(),
                pairing_token: engine.pairing_token(),
            })
        }
        IpcRequest::NoteFrozenOverlayFailure => {
            engine.note_frozen_overlay_failure();
            IpcResponse::success(&true)
        }
        IpcRequest::RefreshBrowserConnection => {
            engine.refresh_browser();
            IpcResponse::success(&true)
        }
        IpcRequest::CheckUninstall => {
            if engine.has_active_strict() {
                IpcResponse::success(&UninstallCheck {
                    allowed: false,
                    reason: "An active strict focus session is currently running. Finish the session before uninstalling Preelock.".into(),
                })
            } else {
                IpcResponse::success(&UninstallCheck {
                    allowed: true,
                    reason: String::new(),
                })
            }
        }
    }
}

pub fn run() -> i32 {
    crate::platform::init_background_daemon();
    crate::lifecycle_log::init();
    let correlation = crate::lifecycle_log::log_process_start("background");
    install_shutdown_handler();
    #[cfg(target_os = "macos")]
    crate::dns_proxy::cleanup_stale_redirect();
    // The headless process must not install a notification delegate or query
    // AppKit to locate/activate Preelock. Notification Center launches the GUI
    // for clicks; only that GUI owns the delegate and application identity.
    let _ = correlation;
    #[cfg(target_os = "windows")]
    if let Ok(exe) = std::env::current_exe() {
        if crate::launch::launch_mode(
            &exe.file_stem()
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default(),
            &[],
        ) == crate::launch::LaunchMode::Background
        {
            crate::platform::register_application_restart("");
        } else {
            crate::platform::register_application_restart(ARG);
        }
    }
    let Ok(dir) = data_dir() else {
        return 1;
    };
    let Some(_lock) = BackgroundLock::try_acquire(&dir) else {
        return 0;
    };
    write_background_meta(&dir, false);
    let path = dir.join("preelock.sqlite");
    let _ = touch(&dir.join(HEARTBEAT));

    // Retry briefly if an exiting previous background process is still
    // releasing engine.lock during upgrade/restart.
    let mut engine = None;
    for attempt in 0..25 {
        match Engine::open_background(&path) {
            Ok(e) if e.owns_engine_lock() => {
                engine = Some(e);
                break;
            }
            Ok(e) => {
                drop(e);
                if attempt == 24 {
                    break;
                }
            }
            Err(error) => {
                log::warn!("Background engine open retry ({attempt}): {error}");
            }
        }
        if shutdown_requested() {
            clear_stale_runtime_files(&dir);
            return 0;
        }
        thread::sleep(Duration::from_millis(200));
    }
    let Some(mut engine) = engine else {
        log::error!("Could not acquire authoritative background engine ownership.");
        write_service_health(
            &dir,
            false,
            "engine_lock_failed",
            "Could not acquire background engine lock.",
        );
        clear_stale_runtime_files(&dir);
        return 1;
    };

    let (ipc_tx, ipc_rx) = mpsc::channel();
    let _ipc_server = match IpcServer::start(&dir, ipc_tx) {
        Ok(server) => {
            write_background_meta(&dir, true);
            server
        }
        Err(error) => {
            log::error!("Could not start local IPC server: {error}");
            write_service_health(
                &dir,
                false,
                "ipc_bind_failed",
                &format!("Could not start local IPC server: {error}"),
            );
            clear_stale_runtime_files(&dir);
            return 1;
        }
    };

    let mut supervisor = UiRelaunchSupervisor::default();
    let mut spawned_gui: Option<Child> = None;
    let mut next_tick = std::time::Instant::now();
    let mut next_supervisor_check = std::time::Instant::now();

    let flush_shutdown = |engine: &mut Engine| {
        crate::lifecycle::system_shutdown();
        if let Err(error) = engine.save_checkpoint() {
            log::warn!("Could not save shutdown checkpoint: {error}");
        }
        clear_stale_runtime_files(&dir);
    };

    loop {
        if shutdown_requested() {
            flush_shutdown(&mut engine);
            #[cfg(target_os = "macos")]
            crate::dns_proxy::cleanup_at_exit();
            return 0;
        }

        #[cfg(target_os = "macos")]
        crate::platform::pump_runloop(0.02);

        let now = std::time::Instant::now();
        let wait = next_supervisor_check
            .saturating_duration_since(now)
            .min(next_tick.saturating_duration_since(now))
            .min(Duration::from_millis(100));
        if let Ok((req, reply)) = ipc_rx.recv_timeout(wait) {
            let resp = handle_ipc_request(&mut engine, &dir, &mut supervisor, req);
            let _ = reply.send(resp);
            while let Ok((queued, q_reply)) = ipc_rx.try_recv() {
                let r = handle_ipc_request(&mut engine, &dir, &mut supervisor, queued);
                let _ = q_reply.send(r);
            }
        }

        if shutdown_requested() {
            flush_shutdown(&mut engine);
            #[cfg(target_os = "macos")]
            crate::dns_proxy::cleanup_at_exit();
            return 0;
        }

        reap_child(&mut spawned_gui);

        let now_instant = std::time::Instant::now();

        if now_instant >= next_supervisor_check {
            next_supervisor_check = now_instant + Duration::from_millis(200);

            let ui_alive = supervisor.poll_ui_alive(&dir);
            // Anti-bypass: reopen for EVERY live session kind, not only
            // Strict. `has_active()` covers Running, Preparing and Paused
            // sessions (instant, scheduled, one-shot and Pomodoro), and is
            // false only when no session is live or the global power
            // toggle is off (an explicit user release, not a quit bypass).
            let should_supervise = engine.has_active();
            if supervisor.evaluate(
                crate::clock::continuous_ms(),
                should_supervise,
                ui_alive,
                shutdown_requested(),
            ) && !spawn_gui_in_user_session(&mut spawned_gui)
                && should_supervise
            {
                if let Err(error) = crate::tray::ensure_running(&dir) {
                    log::warn!("Could not start tray after GUI launch failure: {error}");
                }
            }
        }

        if now_instant >= next_tick {
            next_tick = now_instant + Duration::from_secs(1);

            let needs_attention = engine.needs_attention_agent();
            if needs_attention {
                if let Err(error) = attention_agent::ensure_running(&dir) {
                    log::warn!("Could not start attention agent: {error}");
                }
            }
            let signal =
                attention_agent::signal(&dir, needs_attention).unwrap_or(attention_agent::Signal {
                    status: "camera_unavailable".into(),
                    sampled_at_ms: 0,
                });
            engine.set_attention_camera_status(signal);

            // Process CLI commands directly inside `--background` so CLI control
            // works reliably whether or not the GUI window is open.
            let cli_now = crate::clock::now();
            let _ = crate::cli::write_heartbeat(&dir, cli_now);
            crate::cli::cleanup_stale_results(&dir, cli_now);
            for cli_req in crate::cli::take_requests(&dir, cli_now) {
                let outcome = crate::cli::execute_request_on_engine(&mut engine, &cli_req);
                let _ = crate::cli::write_result(&dir, &outcome);
            }

            if let Err(error) = engine.tick() {
                log::error!("Background engine tick failed: {error}");
            }

            if let Err(error) = touch(&dir.join(HEARTBEAT)) {
                log::warn!("Could not update background heartbeat: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold_gui_lock(data_dir: &Path, pid: u32) -> File {
        use std::io::Write;

        let _ = std::fs::write(data_dir.join("gui.pid"), format!("{pid}\n"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(data_dir.join("gui.lock"))
            .unwrap();
        writeln!(file, "{pid}").unwrap();
        file.try_lock_exclusive().unwrap();
        file
    }

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
        // launchd must NOT respawn the engine: no KeepAlive at all. A
        // supervisor that ignored the breaker kept spawning engines (and
        // Dock tiles) after the GUI was gone (2026-11-24).
        assert!(!plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<key>ThrottleInterval</key><integer>5</integer>"));
        assert!(plist.contains(
            "<string>/Users/test/Library/Logs/com.preelock.background.stdout.log</string>"
        ));
        assert!(plist.contains(
            "<string>/Users/test/Library/Logs/com.preelock.background.stderr.log</string>"
        ));
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

    #[test]
    fn windows_scheduled_task_xml_uses_interactive_session_and_restart_supervision() {
        let xml = scheduled_task_xml(r"C:\Users\Alice\AppData\Local\Preelock\Preelock.exe");
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<RestartOnFailure>"));
        assert!(xml.contains("<Arguments>--background</Arguments>"));
        assert!(!xml.contains("S-1-5-18")); // Never SYSTEM / Session 0
    }

    #[test]
    fn ui_relaunch_supervisor_relaunches_active_sessions_after_the_relaunch_delay() {
        let mut sup = UiRelaunchSupervisor::default();

        // Idle never relaunches regardless of elapsed absence.
        assert!(!sup.evaluate(1_000, false, false, false));
        assert!(!sup.evaluate(20_000, false, false, false));

        // UI alive during an active session clears the missing timer.
        sup.mark_ui_alive(1234);
        assert!(!sup.evaluate(25_000, true, true, false));

        // Cmd+Q / Force Quit -> wait the debounce after confirmed
        // disappearance.
        let missing_at = 30_000u64;
        let launched = missing_at + RELAUNCH_DELAY_MS;
        assert!(!sup.evaluate(missing_at, true, false, false));
        assert!(!sup.evaluate(launched - 1, true, false, false));

        assert!(sup.evaluate(launched, true, false, false));
        // In-flight grace window (10 s) prevents duplicate spawns while the
        // GUI is booting — this is what stopped the 2026-11-17 loop, where
        // the old 3 s cooldown alone fired 9 launches in 24 s.
        assert!(!sup.evaluate(launched, true, false, false));
        assert!(!sup.evaluate(launched + 400, true, false, false));
        assert!(!sup.evaluate(launched + LAUNCH_GRACE_MS - 1, true, false, false));

        // After the grace window passes, if UI still not alive, can relaunch.
        let second = launched + LAUNCH_GRACE_MS;
        assert!(sup.evaluate(second, true, false, false));

        // A launch that registers (UI seen alive) ends the grace early:
        // the next disappearance still needs its own missing delay,
        // but no 10 s grace blocks it.
        assert!(!sup.evaluate(second + 100, true, true, false));
        assert!(!sup.evaluate(second + 200, true, false, false));
        // Still inside the 3 s cooldown from the second launch.
        assert!(!sup.evaluate(second + RELAUNCH_COOLDOWN_MS - 1, true, false, false));
        assert!(sup.evaluate(second + RELAUNCH_COOLDOWN_MS, true, false, false));

        // OS shutdown suppresses relaunch even during an active session.
        assert!(!sup.evaluate(50_000, true, false, true));

        // The session ends -> timer is cancelled, never relaunches.
        assert!(!sup.evaluate(60_000, true, false, false));
        assert!(!sup.evaluate(60_500, false, false, false));
        assert!(!sup.evaluate(70_000, false, false, false));
    }

    #[test]
    fn dead_known_gui_pid_does_not_get_a_grace_period_from_its_heartbeat() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        sup.mark_ui_alive(0);
        touch_desktop(dir.path());

        let ui_alive = sup.poll_ui_alive(dir.path());
        assert!(
            !ui_alive,
            "a fresh heartbeat from a previously known, dead GUI is stale"
        );
        assert!(!sup.evaluate(0, true, ui_alive, false));
        assert!(sup.evaluate(RELAUNCH_DELAY_MS, true, ui_alive, false));
    }

    #[test]
    fn poll_ui_alive_stale_gui_lock_alone_never_declares_ui_dead() {
        // 2026-11-17 loop: gui.lock pointed at a dead pid while a GUI was
        // alive and heartbeating, and the old code returned false on the
        // dead lock pid alone — so the supervisor kept launching copies
        // that died instantly. A stale lock must never by itself declare
        // the UI dead while fresher evidence (here: the heartbeat) says
        // a GUI is alive.
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        std::fs::write(dir.path().join("gui.lock"), "99999999\n").unwrap();
        touch_desktop(dir.path());
        assert!(desktop_alive(dir.path()));

        assert!(sup.poll_ui_alive(dir.path()));
    }

    #[test]
    fn poll_ui_alive_dead_lock_and_no_heartbeat_is_dead() {
        // The complement: when NOTHING says a GUI is alive — dead lock pid,
        // no heartbeat — poll must still report dead,
        // so a genuinely quit GUI is relaunched (once, under the grace
        // window) instead of being missed forever.
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        std::fs::write(dir.path().join("gui.lock"), "99999999\n").unwrap();
        assert!(!desktop_alive(dir.path()));
        assert!(!sup.poll_ui_alive(dir.path()));
    }

    #[test]
    fn poll_ui_alive_does_not_trust_a_live_pid_without_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        std::fs::write(
            dir.path().join("gui.lock"),
            format!("{}\n", std::process::id()),
        )
        .unwrap();

        assert!(!sup.poll_ui_alive(dir.path()));
    }

    #[cfg(windows)]
    #[test]
    fn lock_violation_error_is_lock_contention() {
        // `LockFileEx` reports a held lock as `ERROR_LOCK_VIOLATION` (33),
        // and `std` maps it to `ErrorKind::Uncategorized` — never
        // `WouldBlock`. `gui_lock_status` must still read it as "held", or
        // `poll_ui_alive` wrongly declares the UI dead on Windows (the
        // 2026-11-18 CI failure: both lock-based tests failed while the
        // identical code passed on Linux, where `flock` reports
        // `EWOULDBLOCK`).
        let error = std::io::Error::from_raw_os_error(33);
        assert_ne!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(is_lock_contention(&error));
    }

    #[test]
    fn poll_ui_alive_marks_alive_when_pid_in_gui_lock_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        #[cfg(unix)]
        let test_pid = unsafe { libc::getppid() as u32 };
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd")
                .args(["/c", "ping 127.0.0.1 -n 10 >nul"])
                .creation_flags(0x08000000)
                .spawn()
                .expect("spawn test process")
        };
        #[cfg(windows)]
        let test_pid = child.id();

        let _lock = hold_gui_lock(dir.path(), test_pid);
        assert!(sup.poll_ui_alive(dir.path()));
        // The pid inside gui.lock is only readable where locks are
        // advisory (Unix). On Windows the lock is mandatory: a read from
        // a different handle fails with ERROR_LOCK_VIOLATION, so
        // `gui_lock_status` reports the lock held but no pid — and the
        // supervisor still (correctly) reports the UI alive.
        #[cfg(unix)]
        assert_eq!(sup.known_ui_pid(), Some(test_pid));

        #[cfg(windows)]
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[test]
    fn poll_ui_alive_falls_through_to_gui_lock_when_known_pid_died() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        // Handover: the supervisor still knows the old (dead) GUI pid, but a
        // new GUI has already taken gui.lock and is starting (no heartbeat
        // yet). poll_ui_alive must report the UI alive, not launch a
        // duplicate GUI whose Dock tile would stick as a ghost icon.
        #[cfg(unix)]
        let live_pid = unsafe { libc::getppid() as u32 };
        #[cfg(windows)]
        let mut child = {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("cmd")
                .args(["/c", "ping 127.0.0.1 -n 10 >nul"])
                .creation_flags(0x08000000)
                .spawn()
                .expect("spawn test process")
        };
        #[cfg(windows)]
        let live_pid = child.id();

        sup.mark_ui_alive(99999999);
        let _lock = hold_gui_lock(dir.path(), live_pid);
        assert!(sup.poll_ui_alive(dir.path()));
        // Pid readable only where locks are advisory (Unix); on Windows
        // the mandatory lock blocks the read (see above), so only the
        // liveness verdict is asserted there.
        #[cfg(unix)]
        assert_eq!(sup.known_ui_pid(), Some(live_pid));

        #[cfg(windows)]
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    #[test]
    fn supervisor_never_double_spawns_within_one_tick_or_window() {
        // The exact 2026-11-17 failure, at the supervisor level: polls run
        // every 200 ms and the old 3 s cooldown let a second launch fire
        // while the first was still in flight (two GUIs spawned in the same
        // millisecond at 19:39:49). After one authorised launch, every
        // evaluate inside the grace window must refuse — even across many
        // rapid ticks — until the window expires or the GUI registers.
        let mut sup = UiRelaunchSupervisor::default();
        assert!(!sup.evaluate(1_000, true, false, false));
        assert!(sup.evaluate(2_000, true, false, false));
        for t in [2_000, 2_100, 2_200, 2_500, 3_000, 5_000, 9_999, 11_999] {
            assert!(
                !sup.evaluate(t, true, false, false),
                "second launch fired at {t} ms, inside the grace window"
            );
        }
        assert!(sup.evaluate(12_000, true, false, false));
    }

    #[test]
    fn supervisor_loop_scenario_from_2026_11_17_log_does_not_recur() {
        // A fresh heartbeat keeps the supervisor from launching while the
        // lock PID is stale. Once the heartbeat disappears, relaunches are
        // bounded by the in-flight grace window.
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();

        // Stale lock, live GUI heartbeat.
        std::fs::write(dir.path().join("gui.lock"), "99999999\n").unwrap();
        touch_desktop(dir.path());
        let mut launches = 0;
        for t in (0..=25_000).step_by(200) {
            let alive = sup.poll_ui_alive(dir.path());
            assert!(alive, "poll said dead at {t} ms while GUI was live");
            if sup.evaluate(t as u64, true, alive, false) {
                launches += 1;
            }
        }
        assert_eq!(launches, 0, "supervisor launched while GUI was live");

        // GUI truly gone: no heartbeat (remove the heartbeat file), lock
        // dead. The supervisor's known pid is reset to a dead pid to model
        // the live GUI process actually exiting.
        sup.mark_ui_alive(99999999);
        let _ = std::fs::remove_file(dir.path().join("desktop-heartbeat"));
        // A genuinely dead GUI must still be relaunched — but at the grace
        // cadence (one launch per 10 s window), never the 3 s burst of the
        // 2026-11-17 log (9 launches in 24 s). Over 25 s that is exactly
        // three launches, spaced 10 s apart.
        let mut launch_at: Vec<u64> = Vec::new();
        for t in (0..=25_000).step_by(200) {
            let alive = sup.poll_ui_alive(dir.path());
            assert!(!alive, "poll said alive at {t} ms with all evidence dead");
            if sup.evaluate(t as u64, true, alive, false) {
                launch_at.push(t as u64);
            }
        }
        assert_eq!(
            launch_at,
            vec![
                RELAUNCH_DELAY_MS,
                RELAUNCH_DELAY_MS + LAUNCH_GRACE_MS,
                RELAUNCH_DELAY_MS + 2 * LAUNCH_GRACE_MS
            ],
            "expected one relaunch per grace window, got {launch_at:?}"
        );
    }

    #[test]
    fn supervisor_circuit_breaker_stops_after_three_failed_relaunches() {
        // A GUI that dies on every launch must not be relaunched forever.
        // After three consecutive launches with no live UI, the breaker
        // trips and `evaluate` refuses until the UI is seen alive again.
        let mut sup = UiRelaunchSupervisor::default();
        let mut launches = 0;
        // Simulate 60 s of a crashing GUI: UI never alive.
        for t in (0..=60_000).step_by(200) {
            if sup.evaluate(t as u64, true, false, false) {
                launches += 1;
            }
        }
        assert_eq!(
            launches, 3,
            "circuit breaker should allow exactly 3 relaunches, got {launches}"
        );
        // Seeing the UI alive resets the breaker: a later crash gets a fresh budget.
        sup.mark_ui_alive(1234);
        let mut reset_launch = false;
        for t in (61_000..=65_000).step_by(200) {
            if sup.evaluate(t as u64, true, false, false) {
                reset_launch = true;
                break;
            }
        }
        assert!(
            reset_launch,
            "breaker should reset once the UI is seen alive"
        );
    }

    #[test]
    fn app_bundle_root_resolves_macos_app_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let contents = dir.path().join("Preelock.app").join("Contents");
        let macos = contents.join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(contents.join("Info.plist"), "<plist/>").unwrap();
        let exe = macos.join("preelock");
        assert_eq!(app_bundle_root(&exe), Some(dir.path().join("Preelock.app")));
        assert_eq!(
            app_bundle_root(Path::new("/tmp/dev/target/debug/preelock")),
            None
        );
    }

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

    #[test]
    fn background_ipc_starts_strict_schedule_serves_cli_and_monitors_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("preelock.sqlite");
        let mut engine = Engine::open(&db_path).unwrap();

        let now = crate::clock::now();
        let (weekday, minute) = crate::schedules::local_weekday_and_minutes(now);
        let start_min = minute.saturating_sub(5);
        let end_min = (minute + 30).min(1439);
        if end_min > start_min + 1 {
            let start_time = format!("{:02}:{:02}", start_min / 60, start_min % 60);
            let end_time = format!("{:02}:{:02}", end_min / 60, end_min % 60);
            engine
                .execute(crate::models::Action::CreateSchedule {
                    title: "Weekday Strict".into(),
                    days: vec![weekday],
                    start_time,
                    end_time,
                    focus: crate::focus::FocusConfig {
                        mode: crate::focus::Mode::Strict,
                        apps: vec!["com.microsoft.VSCode".into()],
                        ..Default::default()
                    },
                    enabled: true,
                    lock_changes: false,
                    lock: Default::default(),
                    lock_password: None,
                })
                .unwrap();
        } else {
            let created = engine
                .execute(crate::models::Action::CreateSession {
                    title: "Weekday Strict".into(),
                    minutes: 25,
                    allow_pause: false,
                    focus: crate::focus::FocusConfig {
                        mode: crate::focus::Mode::Strict,
                        apps: vec!["com.microsoft.VSCode".into()],
                        ..Default::default()
                    },
                    lock: Default::default(),
                    lock_password: None,
                })
                .unwrap();
            let id = created.sessions[0].id.clone();
            engine
                .execute(crate::models::Action::StartSession { id })
                .unwrap();
        }

        // Tick without any UI open -> schedule starts automatically (or strict manual session runs).
        engine.tick().unwrap();
        assert!(engine.has_active_strict());

        let (tx, rx) = mpsc::channel();
        let server = IpcServer::start(dir.path(), tx).unwrap();
        let client = crate::ipc::IpcClient::new(dir.path());
        let dir_path = dir.path().to_path_buf();
        let handle = thread::spawn(move || {
            let mut supervisor = UiRelaunchSupervisor::default();
            for _ in 0..4 {
                if let Ok((req, reply)) = rx.recv_timeout(Duration::from_secs(3)) {
                    let resp = handle_ipc_request(&mut engine, &dir_path, &mut supervisor, req);
                    let _ = reply.send(resp);
                }
            }
            (engine, supervisor)
        });

        let ping = client.ping().unwrap();
        assert!(ping.has_active_strict);
        let uninstall = client.check_uninstall().unwrap();
        assert!(!uninstall.allowed);
        assert!(uninstall.reason.contains("strict focus session"));

        client.ui_heartbeat().unwrap();
        client.ui_exiting();

        let (engine, mut supervisor) = handle.join().unwrap();
        drop(server);

        let t0 = crate::clock::continuous_ms();
        assert!(!supervisor.evaluate(t0, engine.has_active(), false, false));
        assert!(supervisor.evaluate(t0 + 500, engine.has_active(), false, false));

        // Verify CLI check-uninstall reflects active strict state inside dir.
        let cli_check = crate::cli::uninstall_check_in_dir(dir.path());
        assert!(!cli_check.allowed);

        // Mark session ended in SQLite and confirm check allows uninstall once idle.
        drop(engine);
        let mut db = crate::database::Database::open(&db_path).unwrap();
        let mut snap = db.load().unwrap();
        for s in &mut snap.sessions {
            s.status = crate::models::Status::Ended;
        }
        db.save(&snap).unwrap();
        let _ = std::fs::remove_file(db_path.with_extension("lifecycle.json"));
        let cli_idle = crate::cli::uninstall_check_in_dir(dir.path());
        assert!(cli_idle.allowed);
    }

    #[test]
    fn background_binary_mismatch_detects_rebuilt_binary() {
        // A rebuild rewrites the binary at the same path, so the
        // path alone cannot tell a fresh binary from a stale one
        // — this is what left a pre-`set_app_lock_strict`
        // background serving IPC while the GUI already sent the
        // new action (HTTP 400, retried ~2 s, toggle reverted).
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("preelock");
        std::fs::write(&exe, b"fake binary").unwrap();
        let mtime = exe_mtime_ms(&exe).unwrap();
        let meta = |exe_str: &str, mtime_ms: u64| BackgroundMeta {
            pid: 1,
            exe: exe_str.into(),
            ipc: true,
            updated_ms: 0,
            exe_mtime_ms: mtime_ms,
        };
        let current = exe.to_string_lossy().into_owned();

        // Same path, same mtime: current binary, keep running.
        assert!(!background_binary_mismatch(&meta(&current, mtime), &exe));
        // Same path, rewritten since the background started: stale.
        assert!(background_binary_mismatch(
            &meta(&current, mtime + 1_000),
            &exe
        ));
        // Different path: stale (the pre-existing check).
        assert!(background_binary_mismatch(
            &meta("/other/preelock", mtime),
            &exe
        ));
        // No mtime recorded: writer predates this check, stale.
        assert!(background_binary_mismatch(&meta(&current, 0), &exe));
        // Empty path but matching mtime: same binary, keep running.
        assert!(!background_binary_mismatch(&meta("", mtime), &exe));
    }

    #[test]
    fn write_background_meta_records_exe_mtime_and_old_meta_still_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_background_meta(dir.path(), true);
        let meta = read_background_meta(dir.path()).unwrap();
        // The running binary's own mtime, recorded.
        let current = exe_mtime_ms(&std::env::current_exe().unwrap()).unwrap();
        assert_eq!(meta.exe_mtime_ms, current);
        assert!(meta.exe_mtime_ms > 0);
        // A meta written before this field existed still parses,
        // with mtime 0 — and therefore counts as stale.
        let old: BackgroundMeta =
            serde_json::from_slice(br#"{"pid":1,"exe":"x","ipc":true,"updated_ms":1}"#).unwrap();
        assert_eq!(old.exe_mtime_ms, 0);
        assert!(background_binary_mismatch(
            &old,
            &std::env::current_exe().unwrap()
        ));
    }

    #[test]
    fn windows_strict_session_relaunch_evaluates_correctly() {
        let mut sup = UiRelaunchSupervisor::default();
        let now = 10_000;

        // When a strict session is active (supervise_active = true) and UI is dead (ui_alive = false),
        // missing time starts at `now`.
        assert!(!sup.evaluate(now, true, false, false));
        // After RELAUNCH_DELAY_MS (500 ms), supervisor triggers relaunch.
        assert!(
            sup.evaluate(now + RELAUNCH_DELAY_MS, true, false, false),
            "Supervisor must trigger relaunch during active strict session when UI dies"
        );

        // When no strict session is active (supervise_active = false) even if UI is dead,
        // supervisor must evaluate to false (do NOT relaunch).
        let mut sup2 = UiRelaunchSupervisor::default();
        assert!(!sup2.evaluate(now, false, false, false));
        assert!(
            !sup2.evaluate(now + RELAUNCH_DELAY_MS, false, false, false),
            "Supervisor must NOT trigger relaunch when no strict session is active"
        );
    }

    #[test]
    fn supervisor_relaunches_for_every_live_session_kind_not_only_strict() {
        // Anti-bypass: the supervisor used to check `has_active_strict()`,
        // so Cmd+Q / Force Quit escaped every Standard, Pomodoro,
        // scheduled and one-shot session. It now supervises `has_active()`,
        // which is exactly the Running | Preparing | Paused set.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("preelock.sqlite");
        let mut engine = Engine::open(&db_path).unwrap();
        let created = engine
            .execute(crate::models::Action::CreateSession {
                title: "Standard Focus".into(),
                minutes: 25,
                allow_pause: true,
                focus: crate::focus::FocusConfig {
                    mode: crate::focus::Mode::Standard,
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = created.sessions[0].id.clone();
        engine
            .execute(crate::models::Action::StartSession { id: id.clone() })
            .unwrap();

        assert!(engine.has_active());
        assert!(
            !engine.has_active_strict(),
            "a Standard session is live but not strict"
        );

        // A Standard session that the user started by hand gets the same
        // relaunch treatment as a Strict one.
        let mut sup = UiRelaunchSupervisor::default();
        let t0 = crate::clock::continuous_ms();
        assert!(!sup.evaluate(t0, engine.has_active(), false, false));
        assert!(
            sup.evaluate(t0 + RELAUNCH_DELAY_MS, engine.has_active(), false, false),
            "a live Standard session must trigger a relaunch"
        );

        // A paused session is still live (`has_active()` matches Paused):
        // the user must not be able to quit out of it while it is paused.
        engine
            .execute(crate::models::Action::PauseSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            })
            .unwrap();
        assert!(engine.has_active(), "a paused session is still live");

        // A session that was properly ended never reopens the window.
        engine
            .execute(crate::models::Action::EndSession {
                id: id.clone(),
                password: None,
                challenge_answer: None,
            })
            .unwrap();
        assert!(!engine.has_active());
        let mut sup = UiRelaunchSupervisor::default();
        assert!(!sup.evaluate(t0, engine.has_active(), false, false));
        assert!(!sup.evaluate(t0 + 60_000, engine.has_active(), false, false));
    }

    #[test]
    fn supervisor_stays_closed_when_idle_or_powered_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("idle.sqlite")).unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        let t0 = crate::clock::continuous_ms();
        assert!(!engine.has_active());
        for t in [t0, t0 + 500, t0 + 5_000, t0 + 60_000] {
            assert!(
                !sup.evaluate(t, engine.has_active(), false, false),
                "an idle app must never reopen"
            );
        }

        // The global power toggle is an explicit user release: it lifts
        // enforcement (`has_active()` is false while paused), so it also
        // stops the relaunch. This is the one user-initiated exemption and
        // is intentional — see the report.
        let created = engine
            .execute(crate::models::Action::CreateSession {
                title: "Standard Focus".into(),
                minutes: 25,
                allow_pause: true,
                focus: crate::focus::FocusConfig::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        engine
            .execute(crate::models::Action::StartSession {
                id: created.sessions[0].id.clone(),
            })
            .unwrap();
        assert!(engine.has_active());
        engine.execute(crate::models::Action::TogglePower).unwrap();
        assert!(engine.is_paused());
        assert!(!engine.has_active());
        let mut sup = UiRelaunchSupervisor::default();
        assert!(!sup.evaluate(t0, engine.has_active(), false, false));
        assert!(!sup.evaluate(t0 + 60_000, engine.has_active(), false, false));
    }

    #[test]
    fn repeated_user_quits_keep_reopening_because_a_healthy_gui_resets_the_breaker() {
        // The circuit breaker exists to stop a GUI that crashes on every
        // launch (2026-11-23 Dock storm), not to let a user escape a session
        // by quitting four times. Every relaunch that is seen alive resets
        // the consecutive-launch counter, so user quits keep reopening.
        let mut sup = UiRelaunchSupervisor::default();
        let mut t = 0u64;
        for round in 0..4 {
            assert!(!sup.evaluate(t, true, false, false));
            assert!(
                sup.evaluate(t + RELAUNCH_DELAY_MS, true, false, false),
                "user quit {round} must still reopen the GUI"
            );
            // The relaunched healthy GUI registers within its boot.
            sup.mark_ui_alive(1_000 + round);
            assert!(!sup.evaluate(t + RELAUNCH_DELAY_MS + 200, true, true, false));
            t += RELAUNCH_COOLDOWN_MS + RELAUNCH_DELAY_MS;
        }
    }

    #[test]
    fn windows_gui_pid_sidecar_detects_dead_gui_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();

        // Write a dead PID into gui.pid and touch heartbeat
        let dead_pid = 99999999;
        std::fs::write(dir.path().join("gui.pid"), format!("{dead_pid}\n")).unwrap();
        touch_desktop(dir.path());

        // Marking the GUI alive with dead_pid
        sup.mark_ui_alive(dead_pid);

        // poll_ui_alive must detect that dead_pid is not alive and return false
        assert!(
            !sup.poll_ui_alive(dir.path()),
            "poll_ui_alive must detect dead PID from gui.pid immediately"
        );
    }

    #[test]
    fn dead_gui_pid_sidecar_beats_a_fresh_heartbeat() {
        // Force Quit: the GUI is gone, `gui.pid` still names it, and the
        // heartbeat it wrote a moment earlier is still inside STALE_MS.
        // The supervisor must not wait those 3 s — the 1 s reopen budget
        // would already be gone. A sidecar naming a dead process is proof.
        let dir = tempfile::tempdir().unwrap();
        let mut sup = UiRelaunchSupervisor::default();
        std::fs::write(dir.path().join("gui.pid"), "99999999\n").unwrap();
        touch_desktop(dir.path());
        assert!(
            !sup.poll_ui_alive(dir.path()),
            "a dead gui.pid must be reported dead without waiting for heartbeat staleness"
        );

        // While that process really is alive, the same sidecar means alive.
        #[cfg(unix)]
        {
            let live = unsafe { libc::getppid() as u32 };
            let mut sup = UiRelaunchSupervisor::default();
            std::fs::write(dir.path().join("gui.pid"), format!("{live}\n")).unwrap();
            assert!(sup.poll_ui_alive(dir.path()));
        }
    }

    #[test]
    fn windows_relaunch_grace_throttling_prevents_process_storms() {
        let mut sup = UiRelaunchSupervisor::default();
        let t0 = 1_000;

        // Missing at t0, authorized after RELAUNCH_DELAY_MS
        assert!(!sup.evaluate(t0, true, false, false));
        assert!(sup.evaluate(t0 + RELAUNCH_DELAY_MS, true, false, false));

        // Rapid subsequent evaluations inside the grace window (LAUNCH_GRACE_MS = 10_000)
        // must all be rejected, preventing duplicate spawns / process storms.
        let authorized_time = t0 + RELAUNCH_DELAY_MS;
        for t in [
            authorized_time + 100,
            authorized_time + 1_000,
            authorized_time + 5_000,
            authorized_time + LAUNCH_GRACE_MS - 1,
        ] {
            assert!(
                !sup.evaluate(t, true, false, false),
                "Relaunch must be throttled during grace period at t={t}"
            );
        }

        // After grace period expires and delay passes, next relaunch is allowed if GUI is still dead
        assert!(
            sup.evaluate(
                authorized_time + LAUNCH_GRACE_MS + RELAUNCH_DELAY_MS,
                true,
                false,
                false
            ),
            "Relaunch allowed after grace period has elapsed"
        );
    }
}
