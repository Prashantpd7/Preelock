pub mod attention;
mod browser;
mod camera;
pub mod capabilities;
mod clock;
mod database;
mod desktop_settings;
mod engine;
pub mod focus;
mod lifecycle;
pub mod models;
pub mod platform;
mod sessions;
mod settings;
mod setup;
use engine::Engine;
use models::{Action, Snapshot};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

#[derive(Default)]
struct ExitApproved(AtomicBool);
use tauri::Manager;

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
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Mutex<Engine>>()
            .lock()
            .map_err(|_| "Session engine unavailable.")?
            .execute(action)
    })
    .await
    .map_err(|e| e.to_string())?
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
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn blocked_quit(app: &tauri::AppHandle) {
    show_window(app);
    desktop_settings::notify(
        "Session in progress",
        "Finish your focus session before closing Preelock.",
    );
}
fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    // Never wait on the engine from the UI thread: native sampling can be
    // waiting for that same thread. Check and shut down under one lock.
    tauri::async_runtime::spawn_blocking(move || {
        let approved = {
            let state = app.state::<Mutex<Engine>>();
            let result = match state.lock() {
                Ok(mut engine) => {
                    if engine.shutdown_if_idle() {
                        app.state::<ExitApproved>().0.store(true, Ordering::Release);
                        true
                    } else {
                        false
                    }
                }
                Err(_) => false,
            };
            result
        };
        if approved {
            app.exit(0);
        } else {
            blocked_quit(&app);
        }
    });
}
fn live_session(snapshot: &models::Snapshot) -> Option<&models::Session> {
    snapshot.sessions.iter().find(|s| {
        matches!(
            s.status,
            models::Status::Running | models::Status::Paused | models::Status::Preparing
        )
    })
}
fn tray_timer_text(session: &models::Session) -> String {
    let secs = session.remaining_ms / 1000;
    let clock = format!("{:02}:{:02}", secs / 60, secs % 60);
    match session.status {
        models::Status::Paused => format!("Paused - {clock}"),
        models::Status::Preparing => "Starting session...".to_string(),
        _ => format!("{clock} left"),
    }
}
fn build_tray_menu(
    app: &tauri::AppHandle,
    live: Option<&models::Session>,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    match live {
        Some(session) => {
            let timer = tauri::menu::MenuItem::with_id(
                app,
                "timer",
                tray_timer_text(session),
                true,
                None::<&str>,
            )?;
            tauri::menu::Menu::with_items(app, &[&timer])
        }
        None => {
            let quit =
                tauri::menu::MenuItem::with_id(app, "quit", "Quit Preelock", true, None::<&str>)?;
            tauri::menu::Menu::with_items(app, &[&quit])
        }
    }
}

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
            app.manage(Mutex::new(
                Engine::open_native(&dir.join("preelock.sqlite")).map_err(std::io::Error::other)?,
            ));
            app.manage(Mutex::new(settings::Runtime::default()));
            app.manage(ExitApproved::default());
            desktop_settings::install_quit_guard(app.handle()).map_err(std::io::Error::other)?;
            let app_handle = app.handle();
            let (initial_menu, mut showing_timer) = {
                let state = app.state::<Mutex<Engine>>();
                let engine = state.lock().map_err(|_| "Session engine unavailable.")?;
                let snapshot = engine.current();
                let live = live_session(&snapshot);
                (build_tray_menu(app_handle, live)?, live.is_some())
            };
            let mut tray = tauri::tray::TrayIconBuilder::with_id("focus-controller")
                .tooltip("Preelock focus controller")
                .menu(&initial_menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "timer" => show_window(app),
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
                let data = {
                    let state = handle.state::<Mutex<Engine>>();
                    let mut engine = match state.lock() {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    if engine.tick().is_err() {
                        log::error!("Focus worker failed; restrictions released where possible.");
                    }
                    engine.current()
                };
                // Tray menu follows the session: live timer while a session
                // runs (no quit item), plain Quit when idle.
                let live = live_session(&data);
                let active = live.is_some();
                if active != showing_timer {
                    if let Ok(next) = build_tray_menu(&handle, live) {
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
                            let _ = item.set_text(tray_timer_text(session));
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
                if lifecycle::ending() {
                    return;
                }
                // Covers the Mac close button/Cmd+W and Windows X/Alt+F4/
                // taskbar close. A busy/poisoned engine fails closed.
                api.prevent_close();
                let state = window.app_handle().state::<Mutex<Engine>>();
                let blocked = match state.try_lock() {
                    Ok(engine) if !engine.has_active() => {
                        let _ = window.hide();
                        false
                    }
                    _ => true,
                };
                if blocked {
                    blocked_quit(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            execute,
            capabilities,
            focus_setup,
            request_camera,
            reset_browser_pairing,
            open_setup,
            settings_action
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|app, event| match event {
            // Covers Cmd+Q, application-menu Quit, Dock Quit and app.exit(code).
            // Only the checked shutdown helper can approve the final exit.
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
