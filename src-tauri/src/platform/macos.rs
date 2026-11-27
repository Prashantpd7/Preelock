use super::{Application, Sample};
use std::ffi::{c_char, CStr, CString};
unsafe extern "C" {
    fn preelock_apps() -> *mut c_char;
    fn preelock_sample() -> *mut c_char;
    fn preelock_hide(pid: i32) -> i32;
    fn preelock_free(p: *mut c_char);
    fn preelock_camera_permission() -> i32;
    fn preelock_request_camera();
    fn preelock_camera_start() -> i32;
    fn preelock_camera_stop();
    fn preelock_camera_status() -> *mut c_char;
    fn preelock_app_icon(identity: *const c_char) -> *mut c_char;
    fn preelock_hide_from_dock();
    fn preelock_init_background_daemon();
    fn preelock_pump_runloop(seconds: f64);
    fn preelock_activate_app();
    fn preelock_request_gui_activation(source: *const c_char, correlation: *const c_char) -> i32;
    fn preelock_install_notification_delegate();
    fn preelock_approve_quit();
    fn preelock_web_sample() -> *mut c_char;
    fn preelock_web_permission() -> i32;
    fn preelock_web_request_permission();
    fn preelock_browser_automation(bundle_id: *const c_char, ask_user: i32) -> i32;
    fn preelock_close_blocked_tab(blocked_host: *const c_char, bundle_id: *const c_char) -> i32;
    fn preelock_redirect_safari_blocked();
    fn preelock_redirect_browser_blocked(app_name: *const c_char, pid: i32);
}
fn text(ptr: *mut c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("Native API returned no data.".into());
    }
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { preelock_free(ptr) };
    Ok(value)
}
pub fn applications() -> Result<Vec<Application>, String> {
    serde_json::from_str(&text(unsafe { preelock_apps() })?).map_err(|e| e.to_string())
}
pub fn sample() -> Result<Sample, String> {
    serde_json::from_str(&text(unsafe { preelock_sample() })?).map_err(|e| e.to_string())
}
pub fn hide(sample: &Sample) -> bool {
    unsafe { preelock_hide(sample.pid as i32) != 0 }
}
pub fn camera_permission() -> &'static str {
    match unsafe { preelock_camera_permission() } {
        0 => "not_requested",
        1 => "restricted",
        2 => "denied",
        3 => "granted",
        _ => "unknown",
    }
}
pub fn request_camera() -> Result<(), String> {
    unsafe { preelock_request_camera() };
    Ok(())
}
pub fn camera_start() -> bool {
    unsafe { preelock_camera_start() != 0 }
}
pub fn camera_stop() {
    unsafe { preelock_camera_stop() };
}
pub fn camera_status() -> String {
    text(unsafe { preelock_camera_status() }).unwrap_or_else(|_| "camera_error".into())
}
/// Base64 PNG (96px) of the app's real icon, or None when the app has no
/// resolvable icon. Used by the Screen Time list; the frontend falls back
/// to a letter tile on None.
pub fn app_icon(identity: &str) -> Option<String> {
    if identity.is_empty() {
        return None;
    }
    let identity = std::ffi::CString::new(identity).ok()?;
    text(unsafe { preelock_app_icon(identity.as_ptr()) }).ok()
}
/// M2: hide a tray-only process (`Preelock --tray`) from the Dock
/// and Cmd-Tab, so it is a menu-bar icon only. No-op on other
/// platforms (see `super::hide_from_dock`). Call on the main thread
/// once the app exists; harmless if called again.
pub fn hide_from_dock() {
    unsafe { preelock_hide_from_dock() };
}
/// Sets the process type permanently to background-only so it never appears in the macOS Dock.
pub fn init_background_daemon() {
    unsafe { preelock_init_background_daemon() };
}
/// Install the notification-centre delegate in the GUI process.
pub fn install_notification_delegate() {
    unsafe { preelock_install_notification_delegate() }
}
/// Pump the macOS CFRunLoop on the main thread for the given duration.
pub fn pump_runloop(seconds: f64) {
    unsafe { preelock_pump_runloop(seconds) };
}
/// Activate existing GUI application instance.
pub fn activate_app() {
    unsafe {
        preelock_activate_app();
    }
}

/// Ask LaunchServices to activate the existing GUI after a second-instance
/// launch. The OS coalesces this request with the running application.
pub fn activate_existing_instance() -> bool {
    request_gui_activation("second_instance", "") != super::GuiActivation::Failed
}

/// Ask LaunchServices to open the GUI without using `NSWorkspace` to locate
/// the Preelock instance from a headless process.
pub fn request_gui_activation(source: &str, correlation: &str) -> super::GuiActivation {
    let source = std::ffi::CString::new(source).unwrap_or_default();
    let correlation = std::ffi::CString::new(correlation).unwrap_or_default();
    match unsafe { preelock_request_gui_activation(source.as_ptr(), correlation.as_ptr()) } {
        3 => super::GuiActivation::SelfProcess,
        1 => super::GuiActivation::Activated,
        2 => super::GuiActivation::Launched,
        4 => super::GuiActivation::DeferredToSystem,
        _ => super::GuiActivation::Failed,
    }
}

/// A second instance exits WITHOUT touching AppKit: creating an NSApplication
/// (even with a Prohibited policy) is what registered a competing
/// com.preelock.desktop identity and produced the ghost Dock icon.
pub fn deactivate_and_exit() -> ! {
    std::process::exit(0);
}
/// Approve quit request so AppKit termination is not cancelled.
pub fn approve_quit() {
    unsafe { preelock_approve_quit() };
}
/// Native website identification: the frontmost browser window's page
/// URL through the Accessibility API. Returns `permission_required`
/// until the user grants Accessibility permission, and `no_url` for
/// windows that expose no page (browser UI, private windows the browser
/// keeps opaque, elevated processes).
pub fn web_sample() -> super::WebSample {
    match text(unsafe { preelock_web_sample() }) {
        Ok(value) => serde_json::from_str(&value).unwrap_or(super::WebSample {
            url: String::new(),
            status: "unavailable".into(),
        }),
        Err(_) => super::WebSample {
            url: String::new(),
            status: "unavailable".into(),
        },
    }
}
pub fn query_os_accessibility() -> Result<bool, String> {
    Ok(unsafe { preelock_web_permission() } == 1)
}
/// Ask macOS for Accessibility permission once (the system dialog). The
/// caller requests at most once per process.
pub fn request_web_permission() {
    unsafe { preelock_web_request_permission() };
}
/// macOS Automation (Apple Events) state for one browser: `0` allowed,
/// `-1743` denied by the user, `-1744` consent would be needed, `-600`
/// the browser is not running, `-50` a bad bundle id. `ask` shows the
/// system consent dialog when the decision has not been made yet; pass
/// `false` to only read the current state.
pub fn browser_automation_status(bundle_id: &str, ask: bool) -> i32 {
    let Ok(id) = CString::new(bundle_id) else {
        return -50;
    };
    unsafe { preelock_browser_automation(id.as_ptr(), i32::from(ask)) }
}
pub fn redirect_safari_blocked() {
    unsafe { preelock_redirect_safari_blocked() };
}
/// Closes the frontmost tab when it is showing `blocked_host` in the
/// frontmost instance of `app_id`, using Accessibility only (no Apple Events,
/// no clipboard, no typing). Returns whether a tab was closed. The native side
/// re-checks the frontmost app, the focused page and the keyboard focus, so a
/// stale caller can never close a tab the user is reading or typing in.
pub fn close_blocked_tab(blocked_host: &str, app_id: &str) -> i32 {
    let (Ok(host), Ok(app)) = (CString::new(blocked_host), CString::new(app_id)) else {
        return 0;
    };
    unsafe { preelock_close_blocked_tab(host.as_ptr(), app.as_ptr()) }
}
pub fn redirect_browser_blocked(app_name: &str, pid: u32) {
    if let Ok(c_str) = std::ffi::CString::new(app_name) {
        unsafe { preelock_redirect_browser_blocked(c_str.as_ptr(), pid as i32) };
    }
}
/// Browsers installed on this Mac (whether running or not), so the user can
/// pick their browser without having to open it first.
pub fn installed_browsers() -> Vec<Application> {
    let known = [
        ("com.google.Chrome", "Google Chrome", "Google Chrome.app"),
        (
            "com.microsoft.edgemac",
            "Microsoft Edge",
            "Microsoft Edge.app",
        ),
        ("com.brave.Browser", "Brave", "Brave Browser.app"),
        ("com.vivaldi.Vivaldi", "Vivaldi", "Vivaldi.app"),
        ("org.chromium.Chromium", "Chromium", "Chromium.app"),
        ("ai.perplexity.comet", "Comet", "Comet.app"),
        ("company.thebrowser.Browser", "Arc", "Arc.app"),
        ("com.operasoftware.Opera", "Opera", "Opera.app"),
        ("com.operasoftware.OperaGX", "Opera GX", "Opera GX.app"),
        ("com.apple.Safari", "Safari", "Safari.app"),
    ];
    let user_apps = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join("Applications"))
        .ok();
    known
        .iter()
        .filter(|(_, _, app)| {
            std::path::Path::new("/Applications").join(app).exists()
                || std::path::Path::new("/System/Applications")
                    .join(app)
                    .exists()
                || user_apps
                    .as_ref()
                    .map(|p| p.join(app).exists())
                    .unwrap_or(false)
        })
        .map(|(id, name, _)| Application {
            id: id.to_string(),
            name: name.to_string(),
        })
        .collect()
}

pub fn register_application_restart(_args: &str) {}
