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
//! - During an active strict focus session (`has_active_strict()`), if the UI
//!   exits (Cmd+Q, macOS Force Quit, or Windows Task Manager), the supervisor
//!   relaunches the Preelock GUI after ~5 seconds in the user's interactive
//!   session. Outside strict sessions, UI exit is never reopened.

use crate::{
    attention_agent,
    engine::Engine,
    ipc::{BrowserSetupInfo, IpcRequest, IpcResponse, IpcServer, PingInfo, UninstallCheck},
    launch, platform,
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
#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "com.preelock.background";
#[cfg(target_os = "windows")]
const WINDOWS_TASK_NAME: &str = "PreelockBackground";
const STALE_MS: i64 = 3_000;
pub const RELAUNCH_DELAY_MS: u64 = 5_000;
pub const RELAUNCH_COOLDOWN_MS: u64 = 8_000;

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
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let meta = BackgroundMeta {
        pid: std::process::id(),
        exe,
        ipc: ipc_ready,
        updated_ms: crate::clock::now(),
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
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
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

    if !already_loaded_for_exe {
        if printed.is_ok_and(|out| out.status.success()) {
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
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            log::warn!("schtasks registration fell back to Run key: {err}");
            write_service_health(data_dir, false, "scheduled_task_failed", &err);
        }
        Err(error) => {
            write_service_health(data_dir, false, "scheduled_task_error", &error.to_string());
        }
    }
    Ok(false)
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

fn terminate_process(pid: u32) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    #[cfg(unix)]
    unsafe {
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
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(0x08000000)
            .output();
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

#[cfg(not(unix))]
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
            terminate_process(pid);
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

fn ensure_running_inner(data_dir: &Path) -> Result<(), String> {
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
    let client = crate::ipc::IpcClient::new(data_dir);
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
            let mismatch_dev = development
                && read_background_meta(data_dir)
                    .is_some_and(|m| !m.exe.is_empty() && m.exe != exe.to_string_lossy());
            if !mismatch_dev {
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
        for _ in 0..20 {
            if client.ping().is_ok() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        if lock_held(data_dir) {
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
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x08000000 | 0x00000200);
    }
    command
        .spawn()
        .map_err(|e| format!("Could not start background engine: {e}"))?;
    Ok(())
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
}

impl UiRelaunchSupervisor {
    pub fn mark_ui_alive(&mut self, pid: u32) {
        self.known_ui_pid = Some(pid);
        self.ui_exiting_at = None;
        self.ui_missing_since = None;
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

    /// Reconcile known PID + heartbeat state into a boolean `ui_alive`.
    pub fn poll_ui_alive(&mut self, data_dir: &Path) -> bool {
        if let Some(pid) = self.known_ui_pid {
            if !process_alive(pid) {
                self.known_ui_pid = None;
                self.ui_exiting_at = None;
                clear_desktop_heartbeat(data_dir);
                return false;
            }
            if self.ui_exiting_at.is_some() {
                // Process still shutting down cleanly; wait for PID exit.
                return true;
            }
            return desktop_alive(data_dir);
        }
        desktop_alive(data_dir)
    }

    /// Evaluate whether the UI should be relaunched at `now_ms`.
    /// CRITICAL INVARIANT: returns `true` ONLY when `strict_active == true`,
    /// `ui_alive == false`, `shutting_down == false`, missing duration >= 5s,
    /// and cooldown since previous relaunch has elapsed.
    pub fn evaluate(
        &mut self,
        now_ms: u64,
        strict_active: bool,
        ui_alive: bool,
        shutting_down: bool,
    ) -> bool {
        if shutting_down || !strict_active {
            self.ui_missing_since = None;
            return false;
        }
        if ui_alive {
            self.ui_missing_since = None;
            return false;
        }
        let since = *self.ui_missing_since.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) < RELAUNCH_DELAY_MS {
            return false;
        }
        if self
            .last_relaunch_at
            .is_some_and(|t| now_ms.saturating_sub(t) < RELAUNCH_COOLDOWN_MS)
        {
            return false;
        }
        self.last_relaunch_at = Some(now_ms);
        self.ui_missing_since = None;
        true
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

fn spawn_gui_in_user_session(spawned: &mut Option<Child>) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let gui = launch::gui_executable(&exe);
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle_root(&gui) {
        // Pass `-n` so LaunchServices launches a fresh GUI window instance even
        // when the headless `--tray` accessory process is currently running.
        match Command::new("/usr/bin/open")
            .arg("-n")
            .arg("-a")
            .arg(&bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                *spawned = Some(child);
            }
            Err(error) => {
                log::warn!(
                    "Could not relaunch Preelock bundle {}: {error}",
                    bundle.display()
                );
            }
        }
        return;
    }

    // In development debug builds, Tauri loads the UI from the Vite dev server (127.0.0.1:1420).
    // If Vite is not running, launching the raw debug binary results in a blank white window.
    // Guard against launching the unbundled debug binary if the dev server is unreachable.
    #[cfg(debug_assertions)]
    {
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], 1420)),
            Duration::from_millis(150),
        )
        .is_err()
        {
            log::warn!(
                "Vite dev server is unreachable at 127.0.0.1:1420; skipping raw debug GUI relaunch to avoid white window."
            );
            return;
        }
    }

    let mut cmd = Command::new(&gui);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.spawn() {
        Ok(child) => {
            *spawned = Some(child);
        }
        Err(error) => {
            log::warn!("Could not relaunch Preelock UI {}: {error}", gui.display());
        }
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    install_shutdown_handler();
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
            return 0;
        }

        let now = std::time::Instant::now();
        let wait = next_tick
            .saturating_duration_since(now)
            .min(Duration::from_millis(250));
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
            return 0;
        }

        reap_child(&mut spawned_gui);

        if std::time::Instant::now() >= next_tick {
            next_tick = std::time::Instant::now() + Duration::from_secs(1);

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

            let ui_alive = supervisor.poll_ui_alive(&dir);
            if !ui_alive && engine.has_active() {
                let _ = crate::tray::ensure_running(&dir);
            }
            if supervisor.evaluate(
                crate::clock::continuous_ms(),
                engine.has_active_strict(),
                ui_alive,
                shutdown_requested(),
            ) {
                spawn_gui_in_user_session(&mut spawned_gui);
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
        assert!(
            plist.contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>")
        );
        assert!(plist.contains("<key>ThrottleInterval</key><integer>5</integer>"));
        assert!(!plist.contains("<key>KeepAlive</key><true/>"));
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
    fn ui_relaunch_supervisor_only_relaunches_after_5s_during_active_strict() {
        let mut sup = UiRelaunchSupervisor::default();

        // Idle or non-strict never relaunches regardless of elapsed absence.
        assert!(!sup.evaluate(1_000, false, false, false));
        assert!(!sup.evaluate(20_000, false, false, false));

        // UI alive during strict clears missing timer and does not relaunch.
        sup.mark_ui_alive(1234);
        assert!(!sup.evaluate(25_000, true, true, false));

        // Cmd+Q / Force Quit -> missing for < 5s does not relaunch early.
        assert!(!sup.evaluate(30_000, true, false, false));
        assert!(!sup.evaluate(34_999, true, false, false));

        // Reaches ~5s missing -> relaunches once!
        assert!(sup.evaluate(35_000, true, false, false));
        // Cooldown prevents duplicate spawns while GUI is booting.
        assert!(!sup.evaluate(35_500, true, false, false));
        assert!(!sup.evaluate(40_500, true, false, false));

        // OS shutdown suppresses relaunch even in active strict.
        assert!(!sup.evaluate(50_000, true, false, true));

        // Strict ends before 5s elapses -> timer cancelled, never relaunches.
        assert!(!sup.evaluate(60_000, true, false, false));
        assert!(!sup.evaluate(62_000, false, false, false));
        assert!(!sup.evaluate(70_000, false, false, false));
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
        assert!(!supervisor.evaluate(t0, engine.has_active_strict(), false, false));
        assert!(supervisor.evaluate(t0 + 5_001, engine.has_active_strict(), false, false));

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
}
