use super::{Application, Sample};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM},
    System::{
        SystemInformation::GetTickCount,
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::{
        Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
        WindowsAndMessaging::{
            EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
            ShowWindowAsync, SW_MINIMIZE,
        },
    },
};
fn identity(hwnd: HWND) -> Option<(String, u32)> {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe {
        CloseHandle(handle);
    }
    (ok != 0).then(|| (String::from_utf16_lossy(&buffer[..size as usize]), pid))
}
unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> i32 {
    if unsafe { IsWindowVisible(hwnd) } != 0 {
        if let Some((id, _)) = identity(hwnd) {
            let list = unsafe { &mut *(param as *mut Vec<Application>) };
            if !list.iter().any(|a| a.id == id) {
                let name = id.rsplit('\\').next().unwrap_or(&id).to_string();
                list.push(Application { id, name });
            }
        }
    }
    1
}
pub fn applications() -> Result<Vec<Application>, String> {
    let mut apps = vec![];
    if unsafe { EnumWindows(Some(collect), &mut apps as *mut Vec<Application> as LPARAM) } == 0 {
        return Err("Cannot enumerate application windows.".into());
    }
    Ok(apps)
}
pub fn sample() -> Result<Sample, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    let (id, pid) = identity(hwnd).ok_or("Foreground application is unavailable or protected.")?;
    let mut input = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut input) } == 0 {
        return Err("Idle-time API unavailable.".into());
    }
    let idle_ms = unsafe { GetTickCount() }.wrapping_sub(input.dwTime) as u64;
    let name = id.rsplit('\\').next().unwrap_or(&id).to_string();
    Ok(Sample {
        id,
        name,
        pid,
        idle_ms,
    })
}
pub fn hide(sample: &Sample) -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if identity(hwnd).is_some_and(|(id, pid)| id == sample.id && pid == sample.pid) {
        unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE) != 0 }
    } else {
        false
    }
}
pub fn camera_permission() -> &'static str {
    "unsupported"
}
pub fn request_camera() -> Result<(), String> {
    Err("Windows camera presence integration is not implemented.".into())
}
pub fn camera_start() -> bool {
    false
}
pub fn camera_stop() {}
pub fn camera_status() -> String {
    "unsupported".into()
}
pub fn installed_browsers() -> Vec<super::Application> {
    use winreg::{enums::*, RegKey};
    let mut candidates = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Ok(root) = RegKey::predef(hive)
                .open_subkey_with_flags(r"SOFTWARE\Clients\StartMenuInternet", KEY_READ | view)
            {
                for name in root.enum_keys().flatten() {
                    if let Ok(command) = root.open_subkey(format!(r"{name}\shell\open\command")) {
                        if let Ok(value) = command.get_value::<String, _>("") {
                            if let Some(path) = super::browser_paths::executable(&value) {
                                candidates.push(path);
                            }
                        }
                    }
                }
            }
        }
    }
    for base in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Some(root) = std::env::var_os(base) {
            for relative in [
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
                r"Vivaldi\Application\vivaldi.exe",
                r"Chromium\Application\chrome.exe",
                r"Perplexity\Comet\Application\comet.exe",
                r"Comet\Application\comet.exe",
            ] {
                candidates.push(
                    std::path::PathBuf::from(&root)
                        .join(relative)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let mut result: Vec<Application> = Vec::new();
    for mut id in candidates {
        for (key, value) in std::env::vars() {
            id = id.replace(&format!("%{key}%"), &value);
        }
        if !crate::focus::supported_browser(&id)
            || !std::path::Path::new(&id).is_file()
            || result.iter().any(|a| a.id.eq_ignore_ascii_case(&id))
        {
            continue;
        }
        let name = super::browser_paths::name(&id).to_string();
        result.push(Application { id, name });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}
