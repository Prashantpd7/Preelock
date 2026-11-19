pub mod adult_block;
mod allowances;
mod applocks;
pub mod attention;
pub mod attention_agent;
pub mod background;
mod blockedpage;
mod blockreasons;
pub mod browser;
pub(crate) mod camera;
pub mod capabilities;
pub mod cli;
mod clock;
pub mod database;
mod desktop_settings;
pub(crate) mod engine;
pub(crate) mod engine_lock;
pub mod focus;
mod frozen;
pub mod ipc;
pub mod launch;
mod lifecycle;
pub mod lifecycle_log;
mod locks;
pub mod models;
mod notifications;
pub mod platform;
pub mod pomodoro;
mod quit;
mod rewards;
mod schedules;
pub mod screen_time;
mod sessions;
mod settings;
mod setup;
mod stats;
mod tamper;
pub mod tray;
pub mod websites;
use ipc::IpcClient;
use models::{Action, Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Default)]
struct ExitApproved(AtomicBool);

pub struct GuiLock {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl GuiLock {
    pub fn try_acquire(dir: &std::path::Path) -> Option<Self> {
        use fs2::FileExt;
        let _ = std::fs::create_dir_all(dir);
        let lock_path = dir.join("gui.lock");
        for _ in 0..3 {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
            {
                if file.try_lock_exclusive().is_ok() {
                    use std::io::{Seek, SeekFrom, Write};
                    let mut f = file;
                    let _ = f.set_len(0);
                    let _ = f.seek(SeekFrom::Start(0));
                    let _ = writeln!(f, "{}", std::process::id());
                    let _ = f.flush();
                    return Some(Self {
                        file: f,
                        path: lock_path,
                    });
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
    }
}

impl Drop for GuiLock {
    fn drop(&mut self) {
        let _ = self.file.set_len(0);
        let _ = std::fs::remove_file(&self.path);
    }
}
/// M6: whether the main window was auto-hidden at this launch
/// (login launch, idle, no user login preference — see
/// `quit::login_window_action`). While set, the idle tray menu
/// also offers "Show Preelock"; cleared by `show_window`.
#[derive(Default)]
struct StartHidden(AtomicBool);
use tauri::Manager;

pub(crate) fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

// Run native work off the event loop via authenticated IPC to `--background`.
#[tauri::command]
async fn snapshot(app: tauri::AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<IpcClient>().snapshot())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn execute(action: Action, app: tauri::AppHandle) -> Result<Snapshot, String> {
    let app_for_task = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        app_for_task.state::<IpcClient>().execute(action)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Keep open-at-login in sync with session AND schedule state (step
    // 6a): registered while a session is active or any schedule is
    // enabled, removed otherwise.
    sync_login_for_sessions(&app);
    Ok(snapshot)
}

pub(crate) fn login_registration_decision(app: &tauri::AppHandle) -> bool {
    let user_pref = app
        .path()
        .app_data_dir()
        .map(|dir| crate::settings::login_user_pref_enabled(&dir))
        .unwrap_or(false);
    app.state::<IpcClient>()
        .snapshot()
        .map(|snapshot| {
            let has_active = !snapshot.preferences.paused
                && snapshot.sessions.iter().any(|s| {
                    matches!(
                        s.status,
                        models::Status::Running
                            | models::Status::Paused
                            | models::Status::Preparing
                    )
                });
            crate::schedules::login_registration_needed(has_active, &snapshot.schedules, user_pref)
        })
        .unwrap_or(user_pref)
}

fn sync_login_for_sessions(app: &tauri::AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let register = login_registration_decision(app);
    if register {
        if let Err(e) = desktop_settings::login(true) {
            log::warn!("Could not enable open-at-login: {e}");
        }
    } else if let Err(e) = desktop_settings::login(false) {
        log::warn!("Could not disable open-at-login: {e}");
    }
}
#[tauri::command]
async fn block_attempts(app: tauri::AppHandle) -> Result<Vec<crate::stats::BlockAttempt>, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<IpcClient>().block_attempts())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn export_statistics(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<IpcClient>().export_statistics())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_statistics(app: tauri::AppHandle) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<IpcClient>().delete_statistics())
        .await
        .map_err(|e| e.to_string())?
}
/// Screen Time row icon: the app's real icon as a base64 PNG (macOS
/// only), or null when the app has no resolvable icon. AppKit icon work
/// dispatches to the main thread inside the native call, so this is safe
/// off the event loop like the other commands. The frontend falls back to
/// a letter tile on null.
#[tauri::command]
async fn get_app_icon(identity: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || platform::app_icon(&identity))
        .await
        .ok()
        .flatten()
}
#[tauri::command]
fn capabilities() -> Vec<capabilities::Capability> {
    capabilities::list()
}
#[tauri::command]
async fn focus_setup(
    app: tauri::AppHandle,
    browser_id: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let applications = platform::applications().unwrap_or_default();
        let mut browsers: Vec<_> = applications
            .iter()
            .filter(|a| focus::supported_browser(&a.id))
            .cloned()
            .collect();
        for installed in platform::installed_browsers() {
            if focus::supported_browser(&installed.id)
                && !browsers
                    .iter()
                    .any(|b| b.id.eq_ignore_ascii_case(&installed.id))
            {
                browsers.push(installed);
            }
        }
        let extension_directory = if cfg!(debug_assertions) {
            setup::extension_dir(&app)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let default_browser = setup::default_browser_id();
        // Default to the first known browser, matching the wizard's dropdown.
        let browser_id = browser_id
            .filter(|s| !s.is_empty())
            .or_else(|| browsers.first().map(|b| b.id.clone()))
            .unwrap_or_default();
        let setup_info = app.state::<IpcClient>().browser_setup_info(browser_id)?;
        Ok(serde_json::json!({
            "platform":platform::name(),
            "native_available":platform::supported(),
            "applications":applications,
            "browsers":browsers,
            "camera_permission":platform::camera_permission(),
            "accessibility_permission":platform::web_permission(),
            "browser":setup_info.browser,
            "connected_browsers":setup_info.connected_browsers,
            "pairing_token":setup_info.pairing_token,
            "extension_directory":extension_directory,
            "default_browser":default_browser
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn open_setup(
    app: tauri::AppHandle,
    target: String,
    browser_id: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || setup::open(&app, &target, &browser_id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
fn request_camera() -> Result<(), String> {
    platform::request_camera()
}
#[tauri::command]
async fn reset_browser_pairing(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<IpcClient>().reset_browser_pairing())
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn settings_action(
    app: tauri::AppHandle,
    action: settings::SettingsAction,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || settings::run(&app, action))
        .await
        .map_err(|e| e.to_string())?
}
fn show_window(app: &tauri::AppHandle) {
    log::info!("lifecycle event=window_show");
    // M6: the window is visible again, so later tray-menu rebuilds
    // use the normal menu (no "Show Preelock" item).
    if let Some(hidden) = app.try_state::<StartHidden>() {
        hidden.0.store(false, Ordering::Release);
    }
    platform::activate_app();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// M6: tray menu for the current state. A live session shows the
/// timer (shared `tray::build_menu`); idle normally shows only
/// Quit — except while the main window is auto-hidden after a
/// login launch, where the menu also offers "Show Preelock" so
/// the window stays reachable without relaunching (on Windows
/// there is no Dock Reopen event to fall back on). Once the
/// window is shown, `show_window` clears the flag and later
/// rebuilds use the normal menu.
fn build_menu_for_state(
    app: &tauri::AppHandle,
    live: Option<&models::Session>,
    start_hidden: bool,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    if live.is_some() || !start_hidden {
        return crate::tray::build_menu(app, live);
    }
    let show = tauri::menu::MenuItem::with_id(app, "show", "Show Preelock", true, None::<&str>)?;
    let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit Preelock", true, None::<&str>)?;
    tauri::menu::Menu::with_items(app, &[&show, &quit])
}
fn notify_with_app_prefs(app: &tauri::AppHandle, title: &str, body: &str) {
    let (enabled, sound, volume) = app
        .try_state::<IpcClient>()
        .and_then(|c| c.ping().ok().map(|p| p.notification_prefs))
        .unwrap_or((true, true, 50));
    if enabled {
        desktop_settings::notify(title, body);
        if sound {
            desktop_settings::sound(volume);
        }
    }
}
fn blocked_quit(app: &tauri::AppHandle) {
    show_window(app);
    notify_with_app_prefs(
        app,
        "Session in progress",
        "Finish your focus session before closing Preelock.",
    );
}

fn active_session_for_quit(data_dir: &std::path::Path) -> bool {
    let db_path = crate::cli::db_path(data_dir);
    match crate::cli::load_snapshot_readonly(&db_path) {
        Ok(snapshot) => {
            return !snapshot.preferences.paused
                && snapshot.sessions.iter().any(|session| {
                    matches!(
                        session.status,
                        models::Status::Running
                            | models::Status::Paused
                            | models::Status::Preparing
                    )
                });
        }
        Err(error) => {
            log::warn!("Could not read session state while evaluating GUI quit: {error}");
        }
    }

    crate::lifecycle::read_checkpoint(&db_path.with_extension("lifecycle.json"))
        .and_then(|checkpoint| checkpoint.active_session)
        .is_some_and(|session| {
            matches!(session.status.as_str(), "running" | "paused" | "preparing")
        })
}

/// Authoritative UI Quit handler (Cmd+Q / Dock Quit / Tray Quit):
/// `--background` owns the session engine and remains running independently.
/// Therefore quitting the GUI exits only the GUI process cleanly (`ExitUiOnly`),
/// notifies `--background` (`ui_exiting`), and returns immediately:
/// - If any session is live, `--background` automatically relaunches the
///   Preelock UI and reconnects it to the intact session.
/// - If no session is live, `--background` continues background tracking and
///   schedules silently without reopening the UI.
fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let client = app.state::<IpcClient>().inner().clone();
        let ping = client.ping();
        let (reachable, has_active) = match &ping {
            Ok(info) => (true, info.has_active),
            Err(_) => {
                let active_in_store = active_session_for_quit(client.data_dir());
                let _ = background::ensure_running_headless(client.data_dir());
                (false, active_in_store)
            }
        };
        let decision = crate::quit::gui_quit_action(reachable, has_active);
        match decision {
            crate::quit::QuitDecision::CloseUiOnly => {
                let _ = background::ensure_running_headless(client.data_dir());
                blocked_quit(&app);
            }
            crate::quit::QuitDecision::ShutdownAndExit | crate::quit::QuitDecision::ExitUiOnly => {
                client.ui_exiting();
                disconnect_ui(&app);
            }
        }
    });
}
fn disconnect_ui(app: &tauri::AppHandle) {
    desktop_settings::approve_quit();
    app.state::<ExitApproved>().0.store(true, Ordering::Release);
    app.exit(0);
    // Hard exit fallback: if webviews or OS AppKit event loop hangs on exit, terminate cleanly after 500ms
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(500));
        std::process::exit(0);
    });
}
/// Step 12 Frozen Turkey overlay sync, run on every 1-second tick
/// from the tray thread (see `run`). The overlay is a fullscreen,
/// always-on-top Tauri window (label `frozen`) showing the frozen
/// timer; it is the visible lock only — the engine's app + website
/// blocking is the real enforcement and never depends on it.
/// A frozen Running session needs the window: it is created when
/// it does not exist (start, resume after kill/restart, or the
/// user closed it — re-created on the next tick) and unminimized
/// when it was minimized. A visible window is left alone:
/// re-focusing it every second would steal focus from the allowed
/// app. Anything else (expiry, natural completion, schedule window
/// end) destroys it: `destroy` is attempted first, and if the
/// platform's close handler prevents the destroy (the frozen
/// window's own close requests are always prevented, see
/// `on_window_event`), the window is hidden instead, which is
/// visually the same — and the next tick retries the destroy.
/// If creation fails, enforcement continues and the failure is
/// recorded as a report event on the frozen session (once per run
/// of failures, see `Engine::note_frozen_overlay_failure`); the
/// block never fail-opens because the window failed.
/// HONEST LIMITS (see `frozen.rs`): on macOS a fullscreen window
/// lives on its own Space and the user can switch Spaces away from
/// it; on Windows Task Manager can kill the process. Neither
/// defeats the engine blocking, which is why the overlay is a
/// deterrent, not the lock itself.
fn sync_frozen_overlay(app: &tauri::AppHandle, snapshot: &models::Snapshot) {
    // Step 14: a frozen session in grace is not yet frozen — the
    // overlay waits out the grace period exactly like enforcement
    // does, and appears when grace ends (or never, if the session
    // ends during grace).
    let now = crate::clock::now();
    let frozen_running = !snapshot.preferences.paused
        && snapshot.sessions.iter().any(|s| {
            crate::frozen::is_enforced(s.focus.frozen.enabled, s.status == models::Status::Running)
                && !crate::notifications::grace_active(s.grace_until_ms, now)
        });
    let window = app.get_webview_window("frozen");
    let action = crate::frozen::overlay_action(
        window.is_some(),
        window
            .as_ref()
            .and_then(|w| w.is_minimized().ok())
            .unwrap_or(false),
        frozen_running,
    );
    match action {
        crate::frozen::OverlayAction::None => {}
        crate::frozen::OverlayAction::Destroy => {
            if let Some(window) = window {
                // Destroy bypasses the close-requested handler on
                // most platforms; if the window still exists
                // afterwards, hide it as a fallback.
                let _ = window.destroy();
                if app.get_webview_window("frozen").is_some() {
                    let _ = window.hide();
                }
            }
        }
        crate::frozen::OverlayAction::Reassert => {
            if let Some(window) = window {
                let _ = window.unminimize();
                let _ = window.show();
            }
        }
        crate::frozen::OverlayAction::Create => {
            // The window loads the same frontend as the main
            // window; the frontend renders the overlay itself when
            // it sees its own window label is `frozen` (see
            // `App.tsx`) — no hash URL is needed, so there is no
            // asset-protocol fragment for a platform to drop.
            let result = tauri::WebviewWindowBuilder::new(
                app,
                "frozen",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Preelock — Frozen")
            .fullscreen(true)
            .always_on_top(true)
            .decorations(false)
            .skip_taskbar(true)
            .build();
            if let Err(e) = result {
                log::warn!("Could not open the frozen lock overlay: {e}");
                let _ = app.state::<IpcClient>().note_frozen_overlay_failure();
            }
        }
    }
}
// M2: the live-session predicate, timer text, and tray menu now live
// in `crate::tray`, shared by the GUI tray and the `--tray` process
// so the two icons can never drift apart.
fn migrate_legacy_data(new_dir: &std::path::Path) -> std::io::Result<()> {
    if new_dir.exists() {
        return Ok(());
    }
    let Some(parent) = new_dir.parent() else {
        return Ok(());
    };
    let legacy_dir = parent.join("com.vystriq.desktop");
    if !legacy_dir.is_dir() {
        return Ok(());
    }
    log::info!(
        "Migrating Preelock data from {} to {}",
        legacy_dir.display(),
        new_dir.display()
    );
    copy_directory(&legacy_dir, new_dir)?;
    for suffix in ["", "-wal", "-shm"] {
        let old = new_dir.join(format!("vystriq.sqlite{suffix}"));
        if old.exists() {
            std::fs::rename(old, new_dir.join(format!("preelock.sqlite{suffix}")))?;
        }
    }
    Ok(())
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            std::fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

pub fn run() {
    lifecycle_log::init();
    let correlation = lifecycle_log::log_process_start("gui");
    let dir = std::env::var_os("PREELOCK_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| background::data_dir().unwrap_or_default());
    let _ = std::fs::create_dir_all(&dir);

    // Single-instance enforcement BEFORE initializing Tauri / AppKit. A
    // losing launch asks LaunchServices to activate the winner, then exits
    // without creating another application instance.
    let gui_lock = match GuiLock::try_acquire(&dir) {
        Some(lock) => lock,
        None => {
            let activated = platform::activate_existing_instance();
            log::info!(
                "lifecycle event=second_instance correlation={correlation} activated={activated}"
            );
            platform::deactivate_and_exit();
        }
    };
    // The GUI owns the delegate before its run loop starts so a cold-launch
    // notification response is delivered, not dropped.
    platform::install_notification_delegate();
    let setup_dir = dir.clone();

    let builder = tauri::Builder::default();
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let result = builder
        .setup(move |app| {
            let dir = setup_dir;
            app.manage(gui_lock);
            desktop_settings::request_authorization();
            migrate_legacy_data(&dir)?;
            background::ensure_running(&dir).map_err(std::io::Error::other)?;
            background::touch_desktop(&dir);
            // Open-at-login is strictly session/schedule-conditional: the
            // app should only auto-open after a restart if there was an
            // ACTIVE session when the PC shut down, or if any schedule
            // is enabled (step 6a). Never enable it unconditionally.
            // In dev mode (not a bundled .app), always unregister to clean
            // up any bad registration of the raw debug binary (which would
            // launch via Terminal with a white screen).
            #[cfg(target_os = "macos")]
            {
                let bundled = std::env::current_exe().ok().is_some_and(|p| {
                    p.ancestors().any(|a| {
                        a.extension().is_some_and(|e| e == "app")
                            && a.join("Contents").join("Info.plist").exists()
                    })
                });
                if !bundled {
                    // Dev mode: remove any stale login registration of the
                    // debug binary. Never register the dev binary.
                    let _ = desktop_settings::login(false);
                }
            }
            #[cfg(target_os = "windows")]
            {
                // Dev mode: remove any stale login registration of the raw
                // dev exe (target\debug / target\release). Never register it.
                let is_dev = std::env::current_exe().ok().is_some_and(|p| {
                    let s = p.to_string_lossy().to_lowercase().replace('/', "\\");
                    s.contains("\\target\\debug\\") || s.contains("\\target\\release\\")
                });
                if is_dev {
                    let _ = desktop_settings::login(false);
                }
            }
            // Remove the legacy unconditional marker so the old behavior
            // (enable on first run) can never apply again.
            let _ = std::fs::remove_file(dir.join(".login_configured"));
            let client = IpcClient::new(&dir);
            let initial_ping = client
                .wait_ready(std::time::Duration::from_secs(12))
                .map_err(|err| {
                    let detail = background::startup_diagnostic(&dir)
                        .map(|d| format!("{err} Diagnostic: {d}"))
                        .unwrap_or(err);
                    std::io::Error::other(detail)
                })?;
            let _ = client.ui_heartbeat();
            let initial_snapshot = client.snapshot().map_err(std::io::Error::other)?;
            // A click consumes the handoff exactly once. It forces the window
            // visible even when the launch rules would start hidden.
            let pending_click = lifecycle_log::take_pending_notification_click(&dir);
            if let Some(click) = &pending_click {
                log::info!(
                    "lifecycle event=notification_open correlation={} request={} action={} kind={} received_at_ms={}",
                    click.correlation,
                    click.request_identifier,
                    click.action_identifier,
                    click.kind,
                    click.received_at_ms
                );
            }
            app.manage(client);
            app.manage(Mutex::new(settings::Runtime::default()));
            app.manage(ExitApproved::default());
            app.manage(StartHidden::default());
            desktop_settings::install_quit_guard(app.handle()).map_err(std::io::Error::other)?;
            sync_login_for_sessions(app.handle());
            crate::tray::wait_for_release(&dir, crate::tray::gui_takeover_wait());
            let start_hidden = pending_click.is_none()
                && crate::quit::login_window_action(
                    initial_ping.has_active,
                    crate::settings::login_user_pref_enabled(&dir),
                    initial_ping.recovered_after_os_shutdown,
                ) == crate::quit::LoginWindowAction::StartHidden;
            if start_hidden {
                if let Some(hidden) = app.try_state::<StartHidden>() {
                    hidden.0.store(true, Ordering::Release);
                }
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            } else {
                show_window(app.handle());
            }
            let app_handle = app.handle();
            let (initial_menu, mut showing_timer) = {
                let live = crate::tray::live_session(&initial_snapshot);
                (
                    build_menu_for_state(app_handle, live, start_hidden)?,
                    live.is_some(),
                )
            };
            let mut tray = tauri::tray::TrayIconBuilder::with_id("focus-controller")
                .tooltip("Preelock focus controller")
                .menu(&initial_menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "timer" => show_window(app),
                    "show" => show_window(app),
                    "quit" => self::quit(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }

            tray.build(app)?;
            let handle = app.handle().clone();
            let mut tray_menu = initial_menu;
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let client = handle.state::<IpcClient>().inner().clone();
                let _ = client.ui_heartbeat();
                let Ok(data) = client.snapshot() else {
                    continue;
                };
                sync_frozen_overlay(&handle, &data);
                blockedpage::sync_overlay(&handle, &data);
                let live = crate::tray::live_session(&data);
                let active = live.is_some();
                if active != showing_timer {
                    let hidden = handle
                        .try_state::<StartHidden>()
                        .map(|s| s.0.load(Ordering::Acquire))
                        .unwrap_or(false);
                    if let Ok(next) = build_menu_for_state(&handle, live, hidden) {
                        if let Some(tray) = handle.tray_by_id("focus-controller") {
                            if tray.set_menu(Some(next.clone())).is_ok() {
                                tray_menu = next;
                                showing_timer = active;
                            }
                        }
                    }
                    sync_login_for_sessions(&handle);
                } else if active {
                    if let (Some(session), Some(kind)) = (live, tray_menu.get("timer")) {
                        if let Some(item) = kind.as_menuitem() {
                            let _ = item
                                .set_text(crate::tray::timer_text(session, crate::clock::now()));
                        }
                    }
                }
                if let Ok(mut runtime) = handle.state::<Mutex<settings::Runtime>>().lock() {
                    runtime.update(&handle, &data);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "frozen" {
                    api.prevent_close();
                    return;
                }
                if window.label() == "blocked-app" {
                    let _ = window
                        .app_handle()
                        .state::<IpcClient>()
                        .dismiss_blocked_app_page();
                    return;
                }
                if lifecycle::ending() {
                    return;
                }
                let action = match window.app_handle().state::<IpcClient>().ping() {
                    Ok(info) => crate::quit::close_action(info.has_active, true),
                    Err(_) => crate::quit::close_action(false, false),
                };
                api.prevent_close();
                match action {
                    crate::quit::WindowCloseAction::HideWindow => {
                        let _ = window.hide();
                    }
                    crate::quit::WindowCloseAction::ShowBlockedNotice => {
                        blocked_quit(window.app_handle());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            execute,
            block_attempts,
            export_statistics,
            delete_statistics,
            get_app_icon,
            capabilities,
            focus_setup,
            request_camera,
            reset_browser_pairing,
            open_setup,
            settings_action,
            blockedpage::dismiss_blocked_app_page
        ])
        .build(context());
    match result {
        Ok(app) => app.run(|app, event| match event {
            tauri::RunEvent::ExitRequested { .. } if lifecycle::ending() => {
                desktop_settings::approve_quit();
                // Background engine handles OS shutdown checkpointing cleanly.
            }
            tauri::RunEvent::ExitRequested { api, .. }
                if !lifecycle::ending()
                    && !app.state::<ExitApproved>().0.load(Ordering::Acquire) =>
            {
                api.prevent_exit();
                quit(app);
            }
            tauri::RunEvent::ExitRequested { .. } => {
                desktop_settings::approve_quit();
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                log::info!("lifecycle event=reopen_apple_event");
                show_window(app);
            }
            _ => {}
        }),
        Err(error) => {
            log::error!("Preelock startup failed: {error}");
            eprintln!("Preelock could not start: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_fallback_treats_checkpointed_live_session_as_active() {
        let dir = tempfile::tempdir().unwrap();
        let lifecycle_path = crate::cli::db_path(dir.path()).with_extension("lifecycle.json");
        crate::lifecycle::write(
            &lifecycle_path,
            &crate::lifecycle::Checkpoint {
                active_session: Some(crate::lifecycle::SessionCheckpoint {
                    status: "running".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(active_session_for_quit(dir.path()));
    }

    #[test]
    fn gui_lock_single_instance_acquisition_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let lock1 = GuiLock::try_acquire(dir.path());
        assert!(lock1.is_some(), "First GUI lock acquisition must succeed");

        // Concurrent attempt in the same data directory must fail.
        let lock2 = GuiLock::try_acquire(dir.path());
        assert!(
            lock2.is_none(),
            "Concurrent GUI lock acquisition must be rejected"
        );

        // Dropping first lock must release flock immediately.
        drop(lock1);
        let lock3 = GuiLock::try_acquire(dir.path());
        assert!(
            lock3.is_some(),
            "Re-acquisition after drop must succeed cleanly"
        );
    }

    #[test]
    fn gui_lock_different_directories_are_independent() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let lock1 = GuiLock::try_acquire(dir1.path());
        let lock2 = GuiLock::try_acquire(dir2.path());
        assert!(lock1.is_some());
        assert!(lock2.is_some());
    }

    #[test]
    fn gui_lock_cleans_up_file_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("gui.lock");
        let lock = GuiLock::try_acquire(dir.path()).expect("acquire");
        assert!(lock_path.exists());
        #[cfg(unix)]
        {
            let pid_str = std::fs::read_to_string(&lock_path).unwrap();
            assert_eq!(pid_str.trim(), std::process::id().to_string());
        }
        drop(lock);
        assert!(!lock_path.exists(), "gui.lock must be removed on drop");
    }
}
