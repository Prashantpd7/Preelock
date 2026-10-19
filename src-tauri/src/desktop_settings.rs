//! Small native settings adapter; unsupported platforms return explicit errors.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, CStr, CString};
    unsafe extern "C" {
        fn vy_settings_info() -> *mut c_char;
        fn vy_login(enabled: i32) -> *mut c_char;
        fn vy_notification_permission() -> i32;
        fn vy_notify(title: *const c_char, body: *const c_char);
        fn vy_sound(volume: f32);
        fn vy_file_panel(save: i32, filename: *const c_char) -> *mut c_char;
        fn vy_free(p: *mut c_char);
    }
    fn text(p: *mut c_char) -> Result<String, String> {
        if p.is_null() {
            return Err("System settings unavailable.".into());
        }
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { vy_free(p) };
        Ok(s)
    }
    pub fn info() -> Result<serde_json::Value, String> {
        serde_json::from_str(&text(unsafe { vy_settings_info() })?).map_err(|e| e.to_string())
    }
    pub fn login(enabled: bool) -> Result<(), String> {
        let error = text(unsafe { vy_login(i32::from(enabled)) })?;
        if error.is_empty() {
            Ok(())
        } else {
            Err(error)
        }
    }
    pub fn permission() -> bool {
        unsafe { vy_notification_permission() != 0 }
    }
    pub fn notify(title: &str, body: &str) {
        if let (Ok(t), Ok(b)) = (CString::new(title), CString::new(body)) {
            unsafe { vy_notify(t.as_ptr(), b.as_ptr()) };
        }
    }
    pub fn sound(volume: u32) {
        unsafe { vy_sound(volume.min(100) as f32 / 100.0) };
    }
    pub fn file(save: bool, name: &str) -> Result<String, String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        text(unsafe { vy_file_panel(i32::from(save), name.as_ptr()) })
    }
}
#[cfg(target_os = "macos")]
pub use mac::*;
#[cfg(not(target_os = "macos"))]
mod other {
    pub fn info() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"notifications":-1,"login":-1}))
    }
    pub fn login(_: bool) -> Result<(), String> {
        Err("Open at login is available on Mac only.".into())
    }
    pub fn permission() -> bool {
        false
    }
    pub fn notify(_: &str, _: &str) {}
    pub fn sound(_: u32) {}
    pub fn file(_: bool, _: &str) -> Result<String, String> {
        Err("File tools are available on Mac only.".into())
    }
}
#[cfg(not(target_os = "macos"))]
pub use other::*;
