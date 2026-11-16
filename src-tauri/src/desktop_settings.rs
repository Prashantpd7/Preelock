//! Small native settings adapter; unsupported platforms return explicit errors.
#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, CStr, CString};
    unsafe extern "C" {
        fn preelock_install_quit_guard(callback: extern "C" fn(), shutdown: extern "C" fn())
            -> i32;
        fn preelock_settings_info() -> *mut c_char;
        fn preelock_login(enabled: i32) -> *mut c_char;
        fn preelock_notification_permission() -> i32;
        fn preelock_request_notification_authorization();
        fn preelock_notify_ctx(
            identifier: *const c_char,
            kind: *const c_char,
            title: *const c_char,
            body: *const c_char,
        );
        fn preelock_sound(volume: f32);
        fn preelock_file_panel(save: i32, filename: *const c_char) -> *mut c_char;
        fn preelock_free(p: *mut c_char);
    }
    fn text(p: *mut c_char) -> Result<String, String> {
        if p.is_null() {
            return Err("System settings unavailable.".into());
        }
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        unsafe { preelock_free(p) };
        Ok(s)
    }
    static QUIT_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
    extern "C" fn native_quit_requested() {
        if let Some(app) = QUIT_APP.get() {
            crate::quit(app);
        }
    }
    pub fn approve_quit() {
        crate::platform::approve_quit();
    }
    pub fn install_quit_guard(app: &tauri::AppHandle) -> Result<(), String> {
        QUIT_APP
            .set(app.clone())
            .map_err(|_| "Quit guard already installed.")?;
        if unsafe {
            preelock_install_quit_guard(native_quit_requested, crate::lifecycle::system_shutdown)
        } == 0
        {
            return Err("Could not install the Mac quit guard.".into());
        }
        Ok(())
    }
    pub fn info() -> Result<serde_json::Value, String> {
        serde_json::from_str(&text(unsafe { preelock_settings_info() })?).map_err(|e| e.to_string())
    }
    pub fn login(enabled: bool) -> Result<(), String> {
        let error = text(unsafe { preelock_login(i32::from(enabled)) })?;
        if error.is_empty() {
            Ok(())
        } else {
            Err(error)
        }
    }
    pub fn permission() -> bool {
        unsafe { preelock_notification_permission() != 0 }
    }
    pub fn request_authorization() {
        unsafe { preelock_request_notification_authorization() };
    }
    /// macOS notifications go through the native `preelock_notify_ctx`
    /// call, which needs no app handle: they can be shown from any
    /// process, including the background engine.
    pub fn can_notify() -> bool {
        true
    }
    pub fn notify(title: &str, body: &str) {
        notify_kind("general", title, body);
    }
    /// Post a notification with a non-sensitive kind (`reminder`, `warning`,
    /// `general`). The kind and a stable identifier travel with the request so a
    /// click that cold-launches the GUI can be understood without logging the
    /// notification text, and so one reminder replaces itself instead of
    /// stacking duplicate banners.
    pub fn notify_kind(kind: &str, title: &str, body: &str) {
        let identifier = super::stable_notification_id(kind, title, body);
        let identifier = CString::new(identifier).unwrap_or_default();
        let kind = CString::new(kind).unwrap_or_default();
        if let (Ok(t), Ok(b)) = (CString::new(title), CString::new(body)) {
            unsafe {
                preelock_notify_ctx(identifier.as_ptr(), kind.as_ptr(), t.as_ptr(), b.as_ptr())
            };
        }
    }
    pub fn sound(volume: u32) {
        if cfg!(test) || volume == 0 {
            return;
        }
        unsafe { preelock_sound(volume.min(100) as f32 / 100.0) };
    }
    pub fn file(save: bool, name: &str) -> Result<String, String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        text(unsafe { preelock_file_panel(i32::from(save), name.as_ptr()) })
    }
}
#[cfg(target_os = "macos")]
pub use mac::*;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod other {
    pub fn approve_quit() {}
    pub fn install_quit_guard(_: &tauri::AppHandle) -> Result<(), String> {
        Ok(())
    }
    pub fn info() -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"notifications":-1,"login":-1}))
    }
    pub fn login(_: bool) -> Result<(), String> {
        Err("Open at login is available on Mac only.".into())
    }
    pub fn permission() -> bool {
        false
    }
    pub fn request_authorization() {}
    /// Stub platform (Linux dev builds): `notify` is a no-op, so
    /// delivery is assumed — this keeps the engine's fire-once
    /// semantics identical to the shipped platforms. Only Windows
    /// gates on a real channel (see
    /// `settings_windows::can_notify`).
    pub fn can_notify() -> bool {
        true
    }
    pub fn notify(_: &str, _: &str) {}
    pub fn notify_kind(_: &str, _: &str, _: &str) {}
    pub fn sound(_: u32) {}
    pub fn file(_: bool, _: &str) -> Result<String, String> {
        Err("File tools are available on Mac only.".into())
    }
}
/// Stable notification identifier: the same kind and text map to the same id, so
/// a repeated reminder replaces the previous request rather than stacking.
/// FNV-1a, 64-bit — a stable hash across runs and processes (unlike
/// `DefaultHasher`, whose seed is not guaranteed).
/// Only the macOS module posts through it; Windows deliveries reuse the text
/// as-is (WinRT toast activation owns the click), so this stays test-covered
/// on those targets without being called in a production build.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn stable_notification_id(kind: &str, title: &str, body: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in kind
        .bytes()
        .chain(std::iter::once(0x1f_u8))
        .chain(title.bytes())
        .chain(std::iter::once(0x1f_u8))
        .chain(body.bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("preelock-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::stable_notification_id;

    #[test]
    fn notification_identifier_is_stable_and_kind_specific() {
        let first = stable_notification_id("reminder", "Time to focus", "Choose a task.");
        assert_eq!(
            first,
            stable_notification_id("reminder", "Time to focus", "Choose a task.")
        );
        assert!(first.starts_with("preelock-"));
        // Kind is part of the identity: a warning never overwrites a reminder.
        assert_ne!(
            first,
            stable_notification_id("warning", "Time to focus", "Choose a task.")
        );
        assert_ne!(
            first,
            stable_notification_id("reminder", "Time to focus", "Choose a task!")
        );
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use other::*;

#[cfg(target_os = "windows")]
pub use crate::platform::settings_windows::*;
