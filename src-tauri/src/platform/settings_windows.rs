//! Windows settings use native dialogs, per-user startup registration and OS alerts.
use crate::platform::packaging::{self, StartupState};
use std::sync::OnceLock;
use tauri_plugin_notification::NotificationExt;
use winreg::{enums::*, RegKey};
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// An MSIX package installs under `WindowsApps` and virtualises HKCU writes,
/// so a registry Run entry is invisible at sign-in. Installed packages use the
/// manifest `startupTask` extension through the Windows API instead.
pub fn is_packaged() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(packaging::is_packaged_exe))
        .unwrap_or(false)
}
fn startup_task() -> Result<windows::ApplicationModel::StartupTask, String> {
    use windows::core::HSTRING;
    windows::ApplicationModel::StartupTask::GetAsync(&HSTRING::from(packaging::STARTUP_TASK_ID))
        .and_then(|operation| operation.join())
        .map_err(|error| error.message())
}
fn packaged_login_value() -> i32 {
    match startup_task().and_then(|task| task.State().map_err(|error| error.message())) {
        Ok(state) => StartupState::from_winrt(state.0).login_value(),
        Err(error) => {
            log::warn!("Could not read the Windows startup task: {error}");
            -1
        }
    }
}
fn set_packaged_login(on: bool) -> Result<(), String> {
    let task = startup_task()?;
    if on {
        task.RequestEnableAsync()
            .and_then(|operation| operation.join())
            .map_err(|error| error.message())?;
    } else {
        task.Disable().map_err(|error| error.message())?;
    }
    // Report what Windows actually stored instead of assuming the request won.
    let state = StartupState::from_winrt(task.State().map_err(|error| error.message())?.0);
    let stored_as_requested = match on {
        true => state.is_on(),
        false => !state.is_on(),
    };
    if stored_as_requested {
        return Ok(());
    }
    Err(match state {
        StartupState::PolicyOff => {
            "Windows policy prevents Preelock from opening when you sign in.".into()
        }
        StartupState::PolicyOn => {
            "Windows policy keeps Preelock opening when you sign in; change it in your organization's policy.".into()
        }
        _ if on => {
            "Windows did not enable Preelock at sign-in. Enable it in Settings > Apps > Startup."
                .into()
        }
        _ => "Windows did not disable Preelock at sign-in.".into(),
    })
}
pub fn install_quit_guard(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    if let Err(error) = migrate_login_label() {
        log::warn!("Startup label migration deferred: {error}");
    }
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
// Rename only an existing opt-in registration. Preserve Windows' disabled state
// before creating the new value; never enable startup for a user without one.
fn migrate_login_label() -> Result<bool, String> {
    if is_packaged() {
        return Ok(false);
    }
    let Ok(key) =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_READ | KEY_WRITE)
    else {
        return Ok(false);
    };
    let legacy = key.get_value::<String, _>("VYSTRIQ").ok();
    let mut migrated = false;
    if key.get_value::<String, _>("Preelock").is_err() {
        let Some(legacy) = legacy.as_ref() else {
            return Ok(false);
        };
        key.set_value("Preelock", legacy)
            .map_err(|e| e.to_string())?;
        migrated = true;
    }
    let approval = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    if let Ok(state) =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(approval, KEY_READ | KEY_WRITE)
    {
        if state.get_raw_value("Preelock").is_err() {
            if let Ok(value) = state.get_raw_value("VYSTRIQ") {
                state
                    .set_raw_value("Preelock", &value)
                    .map_err(|e| e.to_string())?;
                migrated = true;
            }
        }
    }
    if legacy.is_some() {
        key.delete_value("VYSTRIQ").map_err(|e| e.to_string())?;
    }
    Ok(migrated)
}
pub fn info() -> Result<serde_json::Value, String> {
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut time);
    }
    let packaged = is_packaged();
    let login = if packaged {
        packaged_login_value()
    } else if RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN)
        .ok()
        .and_then(|k| k.get_value::<String, _>("Preelock").ok())
        .is_some()
    {
        1
    } else {
        0
    };
    // A packaged app's per-app notification switch lives under its package
    // family name, not the unpackaged identifier, so only the global switch is
    // reported there rather than claiming a state we cannot read.
    let notifications = enabled(
        r"Software\Microsoft\Windows\CurrentVersion\PushNotifications",
        "ToastEnabled",
    ) && (packaged
        || enabled(
            r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\com.vystriq.desktop",
            "Enabled",
        ));
    Ok(serde_json::json!({
        "login": login,
        "notifications": if notifications {2} else {1},
        "day": time.wDayOfWeek,
        "date": format!("{}-{}-{}", time.wYear,time.wMonth,time.wDay),
        "time": format!("{:02}:{:02}",time.wHour,time.wMinute)
    }))
}
pub fn login(on: bool) -> Result<(), String> {
    // Never register a dev/build binary for open-at-login: the registry
    // would launch the raw exe outside its installed context. Only the
    // installed app (NSIS/MSIX) qualifies. Always allow unregister
    // (cleanup of a bad entry).
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let p = exe.to_string_lossy().to_lowercase().replace('/', "\\");
        if p.contains("\\target\\debug\\") || p.contains("\\target\\release\\") {
            return Err("Open at login is available in the installed app.".into());
        }
    }
    if is_packaged() {
        return set_packaged_login(on);
    }
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(RUN)
        .map_err(|e| e.to_string())?;
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value("Preelock", &format!("\"{}\"", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value("Preelock") {
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
        .add_filter("Preelock files", &[extension]);
    let path = if save {
        dialog.save_file()
    } else {
        dialog.pick_file()
    };
    Ok(path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default())
}
