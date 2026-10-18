use super::{Application, Sample};
use std::ffi::{c_char, CStr};
unsafe extern "C" {
    fn vy_apps() -> *mut c_char;
    fn vy_sample() -> *mut c_char;
    fn vy_hide(pid: i32) -> i32;
    fn vy_free(p: *mut c_char);
    fn vy_camera_permission() -> i32;
    fn vy_request_camera();
    fn vy_camera_start() -> i32;
    fn vy_camera_stop();
    fn vy_camera_status() -> *mut c_char;
}
fn text(ptr: *mut c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("Native API returned no data.".into());
    }
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { vy_free(ptr) };
    Ok(value)
}
pub fn applications() -> Result<Vec<Application>, String> {
    serde_json::from_str(&text(unsafe { vy_apps() })?).map_err(|e| e.to_string())
}
pub fn sample() -> Result<Sample, String> {
    serde_json::from_str(&text(unsafe { vy_sample() })?).map_err(|e| e.to_string())
}
pub fn hide(sample: &Sample) -> bool {
    unsafe { vy_hide(sample.pid as i32) != 0 }
}
pub fn camera_permission() -> &'static str {
    match unsafe { vy_camera_permission() } {
        0 => "not_requested",
        1 => "restricted",
        2 => "denied",
        3 => "granted",
        _ => "unknown",
    }
}
pub fn request_camera() -> Result<(), String> {
    unsafe { vy_request_camera() };
    Ok(())
}
pub fn camera_start() -> bool {
    unsafe { vy_camera_start() != 0 }
}
pub fn camera_stop() {
    unsafe { vy_camera_stop() };
}
pub fn camera_status() -> String {
    text(unsafe { vy_camera_status() }).unwrap_or_else(|_| "camera_error".into())
}
