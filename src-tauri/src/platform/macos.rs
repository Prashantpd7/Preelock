use super::{Application, Sample};
use std::ffi::{c_char, CStr};
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
    ];
    known
        .iter()
        .filter(|(_, _, app)| std::path::Path::new("/Applications").join(app).exists())
        .map(|(id, name, _)| Application {
            id: id.to_string(),
            name: name.to_string(),
        })
        .collect()
}
