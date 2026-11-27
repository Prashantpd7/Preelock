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
    /// Whether the main display is currently asleep (off/blank).
    #[serde(default)]
    pub display_asleep: bool,
    /// Whether the desktop session is currently locked or in loginwindow.
    #[serde(default)]
    pub screen_locked: bool,
}
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
/// Native website identification (no browser extension): the foreground
/// browser's active page URL, plus why it is unavailable when it is.
/// `status` is one of `ok`, `permission_required`, `no_url`,
/// `unavailable` or `unsupported`. Only a page URL is ever returned — the
/// adapters never read page contents, titles or form values.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WebSample {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: String,
}

#[cfg(target_os = "macos")]
pub use macos::{
    activate_app, activate_existing_instance, app_icon, applications, approve_quit,
    browser_automation_status, camera_permission, camera_start, camera_status, camera_stop,
    close_blocked_tab, deactivate_and_exit, hide, hide_from_dock, init_background_daemon,
    install_notification_delegate, installed_browsers, pump_runloop, redirect_browser_blocked,
    redirect_safari_blocked, register_application_restart, request_camera, request_gui_activation,
    sample, web_sample,
};
#[cfg(target_os = "windows")]
pub use windows::{
    activate_app, activate_existing_instance, app_icon, applications, browser_automation_status,
    camera_permission, camera_start, camera_status, camera_stop, deactivate_and_exit, hide,
    installed_browsers, redirect_browser_blocked, redirect_safari_blocked,
    register_application_restart, request_camera, request_gui_activation, sample, web_sample,
};

/// Outcome of one GUI activation request, from any process mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuiActivation {
    /// This process already owns the GUI lock.
    SelfProcess,
    /// The OS accepted a request to bring the GUI to the front.
    Activated,
    /// The OS accepted a request to launch or activate the GUI.
    Launched,
    /// The operating system owns this cold launch (for example, a notification click).
    DeferredToSystem,
    /// Nothing could be activated or launched.
    Failed,
}

impl GuiActivation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfProcess => "self",
            Self::Activated => "activated",
            Self::Launched => "launched",
            Self::DeferredToSystem => "deferred_to_system",
            Self::Failed => "failed",
        }
    }
}

pub mod permissions;
pub use permissions::{PermissionManager, PermissionStatus};

static ACCESSIBILITY_MANAGER: PermissionManager =
    PermissionManager::new(std::time::Duration::from_millis(1500));

pub fn accessibility_status(force_refresh: bool) -> PermissionStatus {
    #[cfg(target_os = "macos")]
    {
        ACCESSIBILITY_MANAGER.check_status(force_refresh, macos::query_os_accessibility)
    }
    #[cfg(target_os = "windows")]
    {
        ACCESSIBILITY_MANAGER.check_status(force_refresh, windows::query_os_accessibility)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = force_refresh;
        PermissionStatus::NotGranted
    }
}

pub fn mark_accessibility_recheck_required() {
    ACCESSIBILITY_MANAGER.mark_recheck_required();
}

pub fn request_accessibility() {
    if !accessibility_status(false).is_granted() {
        #[cfg(target_os = "macos")]
        macos::request_web_permission();
        #[cfg(target_os = "windows")]
        windows::request_web_permission();
        mark_accessibility_recheck_required();
    }
}

pub fn web_permission() -> &'static str {
    accessibility_status(false).as_str()
}

pub fn request_web_permission() {
    request_accessibility();
}
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
    pub fn web_sample() -> WebSample {
        WebSample {
            url: String::new(),
            status: "unsupported".into(),
        }
    }
    pub fn web_permission() -> &'static str {
        "unsupported"
    }
    pub fn request_web_permission() {}
    pub fn redirect_safari_blocked() {}
    pub fn browser_automation_status(_: &str, _: bool) -> i32 {
        0
    }
    pub fn close_blocked_tab(_: &str, _: &str) -> bool {
        false
    }
    pub fn redirect_browser_blocked(_: &str, _: u32) {}
    pub fn register_application_restart(_args: &str) {}
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use unsupported::*;

pub mod browser_paths;
pub use browser_paths::browser_display_name;
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
#[cfg(target_os = "windows")]
pub fn enable_background_startup() -> Result<(), String> {
    settings_windows::enable_background_startup()
}
#[cfg(not(target_os = "windows"))]
pub fn enable_background_startup() -> Result<(), String> {
    Ok(())
}
/// M2: hide the tray-only process from the Dock / Cmd-Tab. macOS
/// provides this via `macos::hide_from_dock`; on Windows and Linux
/// a windowless process shows no taskbar entry, so this is a no-op.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn activate_existing_instance() -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn request_gui_activation(_source: &str, _correlation: &str) -> GuiActivation {
    GuiActivation::Failed
}

#[cfg(not(target_os = "macos"))]
pub fn hide_from_dock() {}

#[cfg(not(target_os = "macos"))]
pub fn init_background_daemon() {}

#[cfg(not(target_os = "macos"))]
pub fn approve_quit() {}

/// Non-macOS platforms have nothing to install: their notification click path
/// lives in the Tauri plugin / WinRT toast activation, not in a centre delegate.
#[cfg(not(target_os = "macos"))]
pub fn install_notification_delegate() {}

#[cfg(not(target_os = "macos"))]
pub fn pump_runloop(_seconds: f64) {}
