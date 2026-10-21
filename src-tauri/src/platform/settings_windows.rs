//! Windows settings use native dialogs, per-user startup registration and OS alerts.
use std::sync::OnceLock;
use tauri_plugin_notification::NotificationExt;
use winreg::{enums::*, RegKey};
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub fn install_quit_guard(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    APP.set(app.clone())
        .map_err(|_| "Settings already initialized.".to_string())?;
    let window = app
        .get_webview_window("main")
        .ok_or("Main window unavailable")?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    if unsafe {
        windows_sys::Win32::UI::Shell::SetWindowSubclass(hwnd.0.cast(), Some(shutdown_proc), 1, 0)
    } == 0
    {
        return Err("Could not install Windows shutdown handler".into());
    }
    Ok(())
}
// Observe the OS's confirmed shutdown, not a query that can be cancelled.
unsafe extern "system" fn shutdown_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::{
        Shell::DefSubclassProc,
        WindowsAndMessaging::{WM_ENDSESSION, WM_QUERYENDSESSION},
    };
    if message == WM_QUERYENDSESSION {
        return 1;
    }
    if message == WM_ENDSESSION && wparam != 0 {
        crate::lifecycle::system_shutdown();
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
fn enabled(key: &str, value: &str) -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(key)
        .ok()
        .and_then(|k| k.get_value::<u32, _>(value).ok())
        .unwrap_or(1)
        != 0
}
pub fn info() -> Result<serde_json::Value, String> {
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut time);
    }
    let registered = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN)
        .ok()
        .and_then(|k| k.get_value::<String, _>("VYSTRIQ").ok())
        .is_some();
    let notifications = enabled(
        r"Software\Microsoft\Windows\CurrentVersion\PushNotifications",
        "ToastEnabled",
    ) && enabled(
        r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\com.vystriq.desktop",
        "Enabled",
    );
    Ok(serde_json::json!({
        "login": if registered {1} else {0},
        "notifications": if notifications {2} else {1},
        "day": time.wDayOfWeek,
        "date": format!("{}-{}-{}", time.wYear,time.wMonth,time.wDay),
        "time": format!("{:02}:{:02}",time.wHour,time.wMinute)
    }))
}
pub fn login(on: bool) -> Result<(), String> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN)
        .map_err(|e| e.to_string())?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value("VYSTRIQ", &format!("\"{}\"", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value("VYSTRIQ") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
pub fn permission() -> bool {
    info().is_ok_and(|i| i["notifications"] == 2)
}
pub fn notify(title: &str, body: &str) {
    if let Some(app) = APP.get() {
        if let Err(error) = app.notification().builder().title(title).body(body).show() {
            log::warn!("Windows notification failed: {error}");
        }
    }
}
pub fn sound(volume: u32) {
    if volume == 0 {
        return;
    }
    // A short PCM chime with its own gain; never change the system volume.
    std::thread::spawn(move || {
        use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT};
        let count = 11025u32;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + count * 2).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(44100u32.to_le_bytes());
        wav.extend(88200u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((count * 2).to_le_bytes());
        for i in 0..count {
            let envelope = (1.0 - i as f32 / count as f32).min(i as f32 / 441.0);
            let sample = ((i as f32 * 880.0 * std::f32::consts::TAU / 44100.0).sin()
                * envelope
                * volume.min(100) as f32
                / 100.0
                * 12000.0) as i16;
            wav.extend(sample.to_le_bytes());
        }
        unsafe {
            PlaySoundW(
                wav.as_ptr().cast(),
                std::ptr::null_mut(),
                SND_MEMORY | SND_NODEFAULT,
            );
        }
    });
}
pub fn file(save: bool, name: &str) -> Result<String, String> {
    let extension = if name.ends_with(".csv") {
        "csv"
    } else {
        "json"
    };
    let dialog = rfd::FileDialog::new()
        .set_file_name(name)
        .add_filter("VYSTRIQ files", &[extension]);
    let path = if save {
        dialog.save_file()
    } else {
        dialog.pick_file()
    };
    Ok(path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default())
}
