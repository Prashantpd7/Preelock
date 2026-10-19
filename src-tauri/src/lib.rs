pub mod attention;
mod browser;
pub mod capabilities;
mod clock;
mod database;
mod desktop_settings;
mod engine;
pub mod focus;
pub mod models;
pub mod platform;
mod sessions;
mod settings;
mod setup;
use engine::Engine;
use models::{Action, Snapshot};
use std::sync::Mutex;
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
async fn focus_setup(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move||{
        let applications=platform::applications()?;
        let browsers:Vec<_>=applications.iter().filter(|a|focus::supported_browser(&a.id)).cloned().collect();
        let extension_directory=setup::extension_dir(&app)?;
        let state=app.state::<Mutex<Engine>>();let engine=state.lock().map_err(|_|"Session engine unavailable.")?;
        Ok(serde_json::json!({"platform":platform::name(),"native_available":platform::supported(),"applications":applications,"browsers":browsers,"camera_permission":platform::camera_permission(),"browser":engine.browser_status(),"pairing_token":engine.pairing_token(),"extension_directory":extension_directory}))
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
fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(mut engine) = app.state::<Mutex<Engine>>().lock() {
            engine.shutdown();
        }
        app.exit(0);
    });
}
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let result = tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            app.manage(Mutex::new(
                Engine::open_native(&dir.join("vystriq.sqlite")).map_err(std::io::Error::other)?,
            ));
            app.manage(Mutex::new(settings::Runtime::default()));
            let show = tauri::menu::MenuItem::with_id(
                app,
                "show",
                "Open VYSTRIQ / Emergency exit",
                true,
                None::<&str>,
            )?;
            let quit = tauri::menu::MenuItem::with_id(
                app,
                "quit",
                "Quit and release restrictions",
                true,
                None::<&str>,
            )?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &quit])?;
            let mut tray = tauri::tray::TrayIconBuilder::with_id("focus-controller")
                .tooltip("VYSTRIQ focus controller")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_window(app),
                    "quit" => self::quit(app),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            let handle = app.handle().clone();
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
                if let Ok(mut runtime) = handle.state::<Mutex<settings::Runtime>>().lock() {
                    runtime.update(&handle, &data);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
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
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } => {
                api.prevent_exit();
                quit(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(app),
            _ => {}
        }),
        Err(error) => {
            log::error!("VYSTRIQ startup failed: {error}");
            panic!("VYSTRIQ could not start. See DEVELOPMENT.md recovery steps.");
        }
    }
}
