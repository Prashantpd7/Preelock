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
pub mod launch;
mod lifecycle;
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
use engine::Engine;
use models::{Action, Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Default)]
struct ExitApproved(AtomicBool);
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

// Run native work off the event loop. macOS adapters dispatch AppKit calls to the main thread.
#[tauri::command]
async fn snapshot(app: tauri::AppHandle) -> Result<Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .snapshot()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn execute(action: Action, app: tauri::AppHandle) -> Result<Snapshot, String> {
    let app_for_task = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        app_for_task
            .state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .execute(action)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Keep open-at-login in sync with session AND schedule state (step
    // 6a): registered while a session is active or any schedule is
    // enabled, removed otherwise. Schedule and one-shot mutations all
    // flow through `execute`, so they re-sync here too — there is no
    // second login mechanism.
    sync_login_for_sessions(&app);
    Ok(snapshot)
}
/// Step 15: execute one CLI control request through the existing
/// engine paths — never around them. Starting resolves the id in
/// order: a session (Ready starts, Paused resumes, anything else is
/// refused with its status), a saved setup, or a schedule. A
/// setup/schedule start creates a fresh session from that template
/// (its own duration, bounded 1..=480 minutes by the engine, its own
/// focus and lock) and then starts it, exactly as the GUI's
/// create-then-start flow does; a schedule start is a manual run of
/// the schedule's template, not window-bound and with no grace
/// (manual sessions never get grace, step 14). Stopping ends the
/// single live session with no lock proofs, so the step-10 gate, the
/// step-12 frozen no-override, the step-5 time-range lock and the
/// Strict early-end rule all reject it with their own reasons when
/// they apply. Nothing here extends a session or bypasses a gate.
fn cli_start_template(
    engine: &mut Engine,
    before: &Snapshot,
    title: &str,
    minutes: u32,
    focus: crate::focus::FocusConfig,
    lock: crate::locks::LockConfig,
) -> Result<String, String> {
    engine.execute(Action::CreateSession {
        title: title.to_string(),
        minutes,
        allow_pause: true,
        focus,
        lock,
        lock_password: None,
    })?;
    let after = engine.current();
    let new_id = after
        .sessions
        .iter()
        .map(|s| &s.id)
        .find(|id| !before.sessions.iter().any(|s| &s.id == *id))
        .cloned();
    let Some(new_id) = new_id else {
        return Err("The session was created but could not be found to start it.".into());
    };
    match engine.execute(Action::StartSession { id: new_id }) {
        Ok(_) => Ok(format!("Started \"{title}\" ({minutes} minutes).")),
        Err(error) => Err(format!(
            "Created \"{title}\" as a ready session but could not start it: {error}"
        )),
    }
}

fn execute_cli_request(engine: &mut Engine, request: &cli::CliRequest) -> cli::CliOutcome {
    let outcome = |ok: bool, message: String| cli::CliOutcome {
        id: request.id.clone(),
        ok,
        message,
        created_ms: crate::clock::now(),
    };
    match request.kind.as_str() {
        "stop" => {
            let snapshot = engine.current();
            let Some(live) = snapshot.sessions.iter().find(|s| {
                matches!(
                    s.status,
                    models::Status::Running | models::Status::Paused | models::Status::Preparing
                )
            }) else {
                return outcome(false, "No session is running.".into());
            };
            let (id, title) = (live.id.clone(), live.title.clone());
            match engine.execute(Action::EndSession {
                id,
                password: None,
                challenge_answer: None,
            }) {
                Ok(_) => outcome(true, format!("Ended \"{title}\".")),
                Err(error) => outcome(false, error),
            }
        }
        "start" => {
            let target = request.target.clone().unwrap_or_default();
            let snapshot = engine.current();
            if let Some(session) = snapshot.sessions.iter().find(|s| s.id == target) {
                let (title, status) = (session.title.clone(), session.status.clone());
                let action = match session.status {
                    models::Status::Ready => Action::StartSession { id: target },
                    models::Status::Paused => Action::ResumeSession { id: target },
                    _ => {
                        return outcome(
                            false,
                            format!(
                                "Session \"{title}\" is {:?} and cannot be started from the CLI.",
                                status
                            ),
                        );
                    }
                };
                return match engine.execute(action) {
                    Ok(_) => outcome(true, format!("Started \"{title}\".")),
                    Err(error) => outcome(false, error),
                };
            }
            if let Some(setup) = snapshot
                .preferences
                .saved_setups
                .iter()
                .find(|p| p.id == target)
            {
                let (name, minutes, focus) =
                    (setup.name.clone(), setup.minutes, setup.focus.clone());
                return match cli_start_template(
                    engine,
                    &snapshot,
                    &name,
                    minutes,
                    focus,
                    crate::locks::LockConfig::default(),
                ) {
                    Ok(message) => outcome(true, message),
                    Err(error) => outcome(false, error),
                };
            }
            if let Some(schedule) = snapshot.schedules.iter().find(|s| s.id == target) {
                let (title, focus, lock) = (
                    schedule.title.clone(),
                    schedule.focus.clone(),
                    schedule.lock.clone(),
                );
                let minutes =
                    cli::schedule_window_minutes(&schedule.start_time, &schedule.end_time);
                let Some(minutes) = minutes.filter(|m| (1..=480).contains(m)) else {
                    return outcome(
                        false,
                        "That schedule's window is longer than 480 minutes or invalid; start it from the app.".into(),
                    );
                };
                return match cli_start_template(engine, &snapshot, &title, minutes, focus, lock) {
                    Ok(message) => outcome(true, message),
                    Err(error) => outcome(false, error),
                };
            }
            outcome(
                false,
                format!("No session, saved setup, or schedule with id '{target}'."),
            )
        }
        other => outcome(false, format!("Unknown request kind: {other}")),
    }
}

/// Register for open-at-login iff the user enabled the "Open at login"
/// preference themselves OR a session is currently active (Running,
/// Paused, or Preparing) OR any schedule is enabled.
/// Rationale: the preference is the user's own ask for the app at
/// login; schedules can only fire if the app runs, so creating an
/// enabled schedule is explicit consent to login launch; and a live
/// session must survive a shutdown, so its registration must never
/// depend on the preference (mandatory recovery). Otherwise — preference
/// off, no active session, no enabled schedule — unregister, so the app
/// never auto-opens without a reason. No-op in dev mode (macos.m
/// refuses registration there; unregistration is always allowed).
/// The decision lives in `login_registration_decision` below — the
/// single rule, applied here after every `execute` (session AND
/// schedule/one-shot mutations all flow through `execute`) and once at
/// launch in `setup`, and in the Settings `Login` action (which needs
/// the same decision in one OS call so its errors surface).
/// Documented edge case: on a login launch, schedule evaluation runs
/// on the first tick (step-5 behavior); if a block starts, normal app
/// behavior; if no block is active, the app simply opens to the
/// dashboard. No login-launch detection or auto-quit hacks are built:
/// they are unreliable across macOS/Windows login flows.
/// NOTE: the pre-step-6 code only ever registered here and had no
/// removal branch, despite comments implying removal; step 6 adds
/// the removal branch, which is the documented rule.
/// The single open-at-login decision (see the doc above): register iff
/// the user preference, a live session, or an enabled schedule says so.
/// Reads the user's preference from the `login-user-enabled` marker
/// file and the session/schedule state from the engine snapshot.
/// If the engine lock is poisoned and the snapshot cannot be read,
/// the user's explicit preference alone decides — never silently
/// unregister on a broken engine read.
pub(crate) fn login_registration_decision(app: &tauri::AppHandle) -> bool {
    let user_pref = app
        .path()
        .app_data_dir()
        .map(|dir| crate::settings::login_user_pref_enabled(&dir))
        .unwrap_or(false);
    app.state::<Mutex<Engine>>()
        .lock()
        .map(|engine| {
            let snapshot = engine.current();
            // Screen Time is always on, so it no longer keeps the app
            // launching at login; login registration stays tied to
            // the user preference, live sessions and enabled schedules.
            crate::schedules::login_registration_needed(
                engine.has_active(),
                &snapshot.schedules,
                user_pref,
            )
        })
        .unwrap_or(user_pref)
}

fn sync_login_for_sessions(app: &tauri::AppHandle) {
    // `tauri dev` runs the raw debug binary, which cannot be registered as
    // an OS login item. Startup cleanup above handles stale registrations.
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
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .block_attempts()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn export_statistics(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .export_statistics()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn delete_statistics(app: tauri::AppHandle) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .delete_statistics()
    })
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
    tauri::async_runtime::spawn_blocking(move||{
        let applications=platform::applications()?;
        let mut browsers:Vec<_>=applications.iter().filter(|a|focus::supported_browser(&a.id)).cloned().collect();
        for installed in platform::installed_browsers() {
            if focus::supported_browser(&installed.id) && !browsers.iter().any(|b| b.id == installed.id) {
                browsers.push(installed);
            }
        }
        let extension_directory=setup::extension_dir(&app)?;
        // Default to the first known browser, matching the wizard's dropdown.
        let browser_id=browser_id.filter(|s|!s.is_empty()).or_else(||browsers.first().map(|b|b.id.clone())).unwrap_or_default();
        let state=app.state::<Mutex<Engine>>();let engine=state.lock().map_err(|_|"Session engine unavailable.")?;
        Ok(serde_json::json!({"platform":platform::name(),"native_available":platform::supported(),"applications":applications,"browsers":browsers,"camera_permission":platform::camera_permission(),"browser":engine.browser_status_for(&browser_id),"connected_browsers":engine.connected_browsers(),"pairing_token":engine.pairing_token(),"extension_directory":extension_directory}))
    }).await.map_err(|e|e.to_string())?
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
    tauri::async_runtime::spawn_blocking(move || {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .reset_pairing(&dir)
    })
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
    // M6: the window is visible again, so later tray-menu rebuilds
    // use the normal menu (no "Show Preelock" item).
    if let Some(hidden) = app.try_state::<StartHidden>() {
        hidden.0.store(false, Ordering::Release);
    }
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
/// M3: hide (never destroy) the main window, so the tray timer click
/// can reopen it with `show_window`. Hiding is what "close only the
/// main UI" means during a live session.
fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}
fn blocked_quit(app: &tauri::AppHandle) {
    show_window(app);
    desktop_settings::notify(
        "Session in progress",
        "Finish your focus session before closing Preelock.",
    );
}
/// M3: every user quit request — tray menu Quit, Cmd+Q /
/// application-menu Quit / Dock Quit (via the native quit guard and
/// via `ExitRequested`) — goes through the one state-dependent
/// decision in `Engine::quit_decision`, driven by authoritative
/// engine state, never by UI-local state:
/// - Live session → hide only the main window. The GUI process, its
///   tray icon, the background engine, the browser bridge and the
///   camera agent all keep running; the session is untouched.
/// - Idle + this engine owns the lock → shut the engine down and
///   exit the UI + tray. The separate `--background` process is NOT
///   stopped: screen-time tracking and scheduled sessions keep
///   running there.
/// - Idle + another engine owns the lock → exit only this UI
///   process, with no shutdown and no "finish your session" notice
///   (the M1 transient fix).
fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    // Never wait on the engine from the UI thread: native sampling can be
    // waiting for that same thread. Check and shut down under one lock.
    tauri::async_runtime::spawn_blocking(move || {
        let decision = {
            let state = app.state::<Mutex<Engine>>();
            // Bind the match result first: as the block's tail
            // expression its temporaries would outlive `state`.
            let decision = match state.lock() {
                // A poisoned engine mutex is a broken engine: take the
                // safe direction and hide the UI instead of exiting
                // or shutting anything down.
                Ok(mut engine) => engine.quit_decision(),
                Err(_) => crate::quit::QuitDecision::CloseUiOnly,
            };
            decision
        };
        match decision {
            crate::quit::QuitDecision::CloseUiOnly => {
                hide_main_window(&app);
                desktop_settings::notify(
                    "Session in progress",
                    "Preelock keeps running in the menu bar — finish your focus session to quit.",
                );
            }
            crate::quit::QuitDecision::ShutdownAndExit => {
                app.state::<ExitApproved>().0.store(true, Ordering::Release);
                app.exit(0);
            }
            crate::quit::QuitDecision::ExitUiOnly => {
                disconnect_ui(&app);
            }
        }
    });
}
fn disconnect_ui(app: &tauri::AppHandle) {
    app.state::<ExitApproved>().0.store(true, Ordering::Release);
    app.exit(0);
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
    let frozen_running = snapshot.sessions.iter().any(|s| {
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
                if let Ok(mut engine) = app.state::<Mutex<Engine>>().lock() {
                    engine.note_frozen_overlay_failure();
                }
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let builder = tauri::Builder::default();
    #[cfg(target_os = "windows")]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let result = builder
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            migrate_legacy_data(&dir)?;
            std::fs::create_dir_all(&dir)?;
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
            app.manage(Mutex::new(
                Engine::open_native(&dir.join("preelock.sqlite")).map_err(std::io::Error::other)?,
            ));
            app.manage(Mutex::new(settings::Runtime::default()));
            app.manage(ExitApproved::default());
            app.manage(StartHidden::default());
            desktop_settings::install_quit_guard(app.handle()).map_err(std::io::Error::other)?;
            // Sync open-at-login on every launch: registered while a
            // session is active (recovery after restart) or while any
            // schedule is enabled (schedules can only fire if the app
            // runs); otherwise unregistered. In dev mode this
            // unregisters the debug binary (macos.m guards
            // registration, but an old bad entry may still exist).
            sync_login_for_sessions(app.handle());
            // M2: a tray process may own the menu-bar icon right now
            // (it took over when this GUI exited or was killed). It
            // exits within a second of the desktop heartbeat going
            // fresh — the heartbeat was touched at setup start — so
            // wait for it to release the tray lock before building
            // this GUI's tray, or there would be two icons. If it
            // does not exit in time, build anyway: the tray process
            // still exits on its next tick.
            crate::tray::wait_for_release(&dir, crate::tray::gui_takeover_wait());
            // M6 req 9: after an OS shutdown/logout/restart with no
            // live session — and no user "Start Preelock at login"
            // preference — the main window does not pop up at login.
            // The background engine still tracks and fires schedules,
            // the tray icon stays available, and the window is only
            // hidden (never destroyed): the tray's "Show Preelock"
            // item or a Dock click reopens it. A manual launch, a
            // live session, or the user's own login preference all
            // show the window (see `quit::login_window_action`).
            let start_hidden = {
                let state = app.state::<Mutex<Engine>>();
                let engine = state.lock().map_err(|_| "Session engine unavailable.")?;
                crate::quit::login_window_action(
                    engine.has_active(),
                    crate::settings::login_user_pref_enabled(&dir),
                    engine.recovered_after_os_shutdown(),
                ) == crate::quit::LoginWindowAction::StartHidden
            };
            if start_hidden {
                if let Some(hidden) = app.try_state::<StartHidden>() {
                    hidden.0.store(true, Ordering::Release);
                }
                // Setup runs before the run loop starts, so no window
                // is ever presented (the same timing the --tray
                // process relies on to destroy its windows).
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            let app_handle = app.handle();
            let (initial_menu, mut showing_timer) = {
                let state = app.state::<Mutex<Engine>>();
                let engine = state.lock().map_err(|_| "Session engine unavailable.")?;
                let snapshot = engine.current();
                let live = crate::tray::live_session(&snapshot);
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
            let cli_dir = dir.clone();
            let mut tray_menu = initial_menu;
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                background::touch_desktop(&cli_dir);
                let data = {
                    let state = handle.state::<Mutex<Engine>>();
                    let mut engine = match state.lock() {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let needs_attention = engine.needs_attention_agent();
                    // M6: the OS is shutting down / logging out — the
                    // process is about to die, so checkpoint the
                    // engine as-is (live sessions stay live) before
                    // the tick below runs. The tick itself is still
                    // run: at worst a schedule starting here is
                    // checkpointed as Running and resumes at login.
                    if lifecycle::ending() {
                        if let Err(error) = engine.save_checkpoint() {
                            log::warn!("Could not save shutdown checkpoint: {error}");
                        }
                    }
                    if needs_attention {
                        if let Err(error) = attention_agent::ensure_running(&cli_dir) {
                            log::warn!("Could not start attention agent: {error}");
                        }
                    }
                    let signal = attention_agent::signal(&cli_dir, needs_attention).unwrap_or(
                        attention_agent::Signal {
                            status: "camera_unavailable".into(),
                            sampled_at_ms: 0,
                        },
                    );
                    engine.set_attention_camera_status(signal);
                    if engine.tick().is_err() {
                        log::error!("Focus worker failed; restrictions released where possible.");
                    }
                    engine.current()
                };
                // Step 12: the frozen overlay follows the tick, so
                // it appears on start/resume, re-appears if closed
                // or minimized, and disappears on expiry — all
                // within a second of the session state changing.
                sync_frozen_overlay(&handle, &data);
                // "Why am I blocked" app page: shows the blocked app +
                // its true reason instead of the app just vanishing.
                blockedpage::sync_overlay(&handle, &data);
                // Tray menu follows the session: live timer while a session
                // runs (no quit item), plain Quit when idle — plus a
                // "Show Preelock" item while the window is auto-hidden
                // after a login launch (M6 req 9).
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
                // Step 15 CLI: heartbeat + request consumption, every
                // tick. The heartbeat proves the app is alive for
                // control commands; requests are consumed (deleted)
                // on pickup by `take_requests`, executed through the
                // normal engine paths, and answered with a result
                // file — so no request can fire twice or linger.
                let cli_now = crate::clock::now();
                let _ = crate::cli::write_heartbeat(&cli_dir, cli_now);
                crate::cli::cleanup_stale_results(&cli_dir, cli_now);
                for request in crate::cli::take_requests(&cli_dir, cli_now) {
                    let mut locked = false;
                    let mut result = None;
                    if let Ok(mut engine) = handle.state::<Mutex<Engine>>().lock() {
                        locked = true;
                        result = Some(execute_cli_request(&mut engine, &request));
                    }
                    let result = result.unwrap_or_else(|| cli::CliOutcome {
                        id: request.id.clone(),
                        ok: false,
                        message: "Session engine unavailable.".into(),
                        created_ms: cli_now,
                    });
                    // A CLI start/stop changes session state, so
                    // open-at-login stays in sync exactly as after a
                    // GUI execute — but only when the engine lock
                    // succeeded, so a poisoned engine never triggers
                    // a spurious unregister here.
                    if locked {
                        sync_login_for_sessions(&handle);
                    }
                    let _ = crate::cli::write_result(&cli_dir, &result);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Step 12: the frozen overlay has no close
                // affordance at all — its close requests (window
                // chrome, Cmd+W, Alt+F4) are always prevented, and
                // its lifecycle is owned by the tick sync, which
                // destroys it when the frozen session ends. The
                // decorations are already off, so this only covers
                // keyboard/system close attempts.
                if window.label() == "frozen" {
                    api.prevent_close();
                    return;
                }
                // "Why am I blocked" app page: purely informational
                // and trivially dismissible — any close request clears
                // the notice (the tick sync then destroys the window
                // instead of re-creating it). The block itself is
                // untouched. Unlike the frozen overlay, this close is
                // never prevented.
                if window.label() == "blocked-app" {
                    if let Ok(mut engine) = window.app_handle().state::<Mutex<Engine>>().lock() {
                        engine.dismiss_blocked_app_page();
                    }
                    return;
                }
                if lifecycle::ending() {
                    return;
                }
                // M3: closing the main window closes only the main UI —
                // never the process, the engine, or the tray — whether
                // a session is live or not. A strict session is owned
                // by the background engine; closing the UI must not end
                // it. The window is hidden (not destroyed) so the tray
                // timer click can reopen it. Only a poisoned engine
                // mutex keeps the window visible with the blocked-quit
                // notice instead of hiding it.
                let action = {
                    let state = window.app_handle().state::<Mutex<Engine>>();
                    // Bind the match result first: as the block's tail
                    // expression its temporaries would outlive `state`.
                    let action = match state.try_lock() {
                        Ok(engine) => crate::quit::close_action(engine.has_active(), true),
                        Err(_) => crate::quit::close_action(false, false),
                    };
                    action
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
            // M6: OS shutdown/logout/restart (`lifecycle::ending()`).
            // The process is about to die, so checkpoint the engine
            // as-is — live sessions stay live with their remaining
            // time — then let the exit proceed (never `prevent_exit`
            // here: blocking the OS shutdown is not ours to do).
            // The lock is only taken with `try_lock`: the tray
            // thread may hold the engine mutex, and waiting on it
            // from the run loop could stall the shutdown. A missed
            // save is bounded by the periodic ~5 s checkpoint.
            tauri::RunEvent::ExitRequested { .. } if lifecycle::ending() => {
                if let Ok(mut engine) = app.state::<Mutex<Engine>>().try_lock() {
                    if let Err(error) = engine.save_checkpoint() {
                        log::warn!("Could not save shutdown checkpoint: {error}");
                    }
                }
            }
            // Covers Cmd+Q, application-menu Quit, Dock Quit and app.exit(code).
            // M3: every user quit request goes through the same
            // state-dependent decision as tray Quit — see `quit()`.
            // Live session: only the main window hides; the process,
            // tray, engine, bridge and camera agent keep running, so
            // the M2 tray spawn that used to live on the active branch
            // is gone (the GUI process no longer exits mid-session, so
            // its own tray icon survives and no handover is needed).
            // Idle: the engine owner shuts down and exits; an idle
            // observer exits only the UI process. OS shutdown/logout
            // (`lifecycle::ending()`) still exits fully — the arm
            // above checkpoints first, then lets it exit.
            tauri::RunEvent::ExitRequested { api, .. }
                if !lifecycle::ending()
                    && !app.state::<ExitApproved>().0.load(Ordering::Acquire) =>
            {
                api.prevent_exit();
                quit(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(app),
            _ => {}
        }),
        Err(error) => {
            log::error!("Preelock startup failed: {error}");
            panic!("Preelock could not start. See DEVELOPMENT.md recovery steps.");
        }
    }
}
