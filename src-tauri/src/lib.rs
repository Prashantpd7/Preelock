pub mod capabilities;
mod database;
mod engine;
pub mod models;
pub mod platform;
mod sessions;

use engine::Engine;
use models::{Action, Snapshot};
use std::sync::Mutex;
use tauri::Manager;

#[tauri::command]
fn snapshot(engine: tauri::State<'_, Mutex<Engine>>) -> Result<Snapshot, String> {
    engine
        .lock()
        .map_err(|_| "Session engine is unavailable. Restart VYSTRIQ.")?
        .snapshot()
}
#[tauri::command]
fn execute(action: Action, engine: tauri::State<'_, Mutex<Engine>>) -> Result<Snapshot, String> {
    engine
        .lock()
        .map_err(|_| "Session engine is unavailable. Restart VYSTRIQ.")?
        .execute(action)
}
#[tauri::command]
fn capabilities() -> Vec<capabilities::Capability> {
    capabilities::list()
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let result = tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let engine =
                Engine::open(&dir.join("vystriq.sqlite")).map_err(std::io::Error::other)?;
            app.manage(Mutex::new(engine));
            // Continue the engine while the webview is suspended or not rendering.
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let state = handle.state::<Mutex<Engine>>();
                if let Ok(mut engine) = state.lock() {
                    if engine.snapshot().is_err() {
                        log::error!("Session checkpoint failed; retrying on next tick");
                    }
                };
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![snapshot, execute, capabilities])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        log::error!("VYSTRIQ failed to start: {error}");
        panic!("VYSTRIQ could not start. See application logs and DEVELOPMENT.md recovery steps.");
    }
}
