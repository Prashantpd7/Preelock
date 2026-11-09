use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Application {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sample {
    pub id: String,
    pub name: String,
    pub pid: u32,
    pub idle_ms: u64,
    /// Step 4: the foreground app's path — the exe path on Windows, the
    /// bundle path for .app processes on macOS and the executable path
    /// otherwise. Serde-default (empty) so old native JSON still parses;
    /// the engine falls back to `id` when it looks like a path.
    #[serde(default)]
    pub path: String,
    /// Step 4: the frontmost window's title (empty when unavailable).
    /// Serde-default so old native JSON still parses.
    #[serde(default)]
    pub title: String,
}
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "windows")]
pub use windows::*;
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}
pub fn name() -> &'static str {
    std::env::consts::OS
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod unsupported {
    use super::*;
    pub fn applications() -> Result<Vec<Application>, String> {
        Err("Unsupported desktop platform.".into())
    }
    pub fn sample() -> Result<Sample, String> {
        Err("Unsupported desktop platform.".into())
    }
    pub fn hide(_: &Sample) -> bool {
        false
    }
    pub fn camera_permission() -> &'static str {
        "unsupported"
    }
    pub fn request_camera() -> Result<(), String> {
        Err("Camera integration unavailable.".into())
    }
    pub fn camera_start() -> bool {
        false
    }
    pub fn camera_stop() {}
    pub fn camera_status() -> String {
        "unsupported".into()
    }
    pub fn installed_browsers() -> Vec<Application> {
        Vec::new()
    }
    pub fn app_icon(_identity: &str) -> Option<String> {
        None
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use unsupported::*;

#[cfg(any(target_os = "windows", test))]
mod browser_paths;
pub mod packaging;
#[cfg(target_os = "windows")]
pub mod settings_windows;

/// True only for an installed MSIX package (Microsoft Store or sideload).
#[cfg(target_os = "windows")]
pub fn packaged() -> bool {
    settings_windows::is_packaged()
}
#[cfg(not(target_os = "windows"))]
pub fn packaged() -> bool {
    false
}
