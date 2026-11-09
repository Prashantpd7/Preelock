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
            EnumChildWindows, EnumWindows, GetForegroundWindow, GetWindowTextLengthW,
            GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, ShowWindowAsync,
            SW_MINIMIZE,
        },
    },
};
// GetPackageFamilyName lives in kernel32 but windows-sys 0.61 does not
// expose it under the features this crate enables, so declare it
// directly (HANDLE = isize, PWSTR = *mut u16 in windows-sys terms).
// Two-call pattern: first call with a null buffer returns
// ERROR_INSUFFICIENT_BUFFER (122) and the required length; a process
// with no package returns APPMODEL_ERROR_NO_PACKAGE (15700).
extern "system" {
    fn GetPackageFamilyName(
        process: isize,
        package_family_name_length: *mut u32,
        package_family_name: *mut u16,
    ) -> i32;
}
fn image_of_pid(pid: u32) -> Option<String> {
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
    (ok != 0).then(|| String::from_utf16_lossy(&buffer[..size as usize]))
}
fn identity(hwnd: HWND) -> Option<(String, u32)> {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    image_of_pid(pid).map(|path| (path, pid))
}
fn file_name(path: &str) -> &str {
    path.rsplit(&['\\', '/'][..]).next().unwrap_or(path)
}
fn is_application_frame_host(exe_path: &str) -> bool {
    file_name(exe_path).eq_ignore_ascii_case("ApplicationFrameHost.exe")
}
/// Step 4: the foreground window's title, or empty when unavailable.
fn window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; len as usize + 1];
    let n = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if n <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buffer[..n as usize])
    }
}
/// Step 4: resolve the Store app hosted inside an ApplicationFrameHost
/// window. UWP/MSIX apps run hosted: the foreground window belongs to
/// ApplicationFrameHost.exe, while the real app window is a child window
/// owned by the app's own process. Enumerate the child windows, take the
/// first child whose process has a package family name, and return
/// (store id, hosted pid, display name). None when no child resolves —
/// the caller must then fail open (keep skipping the host), never block
/// ApplicationFrameHost itself.
struct HostedSearch {
    afh_pid: u32,
    found: Option<(String, u32, String)>,
}
unsafe extern "system" fn collect_hosted(hwnd: HWND, param: LPARAM) -> i32 {
    let search = unsafe { &mut *(param as *mut HostedSearch) };
    if search.found.is_some() {
        return 0;
    }
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 || pid == search.afh_pid {
        return 1;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return 1;
    }
    let mut len = 0u32;
    let err = unsafe { GetPackageFamilyName(handle, &mut len, std::ptr::null_mut()) };
    let pfn = if err == 122 && len > 1 {
        let mut buffer = vec![0u16; len as usize];
        let ok = unsafe { GetPackageFamilyName(handle, &mut len, buffer.as_mut_ptr()) };
        unsafe {
            CloseHandle(handle);
        }
        if ok != 0 {
            return 1;
        }
        // Read up to the first NUL rather than trusting the returned
        // length's null-terminator convention.
        String::from_utf16_lossy(&buffer)
            .trim_end_matches('\0')
            .to_string()
    } else {
        unsafe {
            CloseHandle(handle);
        }
        return 1;
    };
    let Some(store_id) = crate::focus::store_id_from_pfn(&pfn) else {
        return 1;
    };
    let name = image_of_pid(pid)
        .map(|exe| display_name(&exe))
        .unwrap_or_default();
    search.found = Some((store_id, pid, name));
    0
}
fn hosted_store_app(hwnd: HWND, afh_pid: u32) -> Option<(String, u32, String)> {
    let mut search = HostedSearch {
        afh_pid,
        found: None,
    };
    unsafe {
        EnumChildWindows(
            hwnd,
            Some(collect_hosted),
            &mut search as *mut HostedSearch as LPARAM,
        );
    }
    search.found
}
/// Windows system host processes that are never real user apps — skip them.
const SYSTEM_PROCESSES: &[&str] = &[
    "ApplicationFrameHost.exe",
    "TextInputHost.exe",
    "ShellExperienceHost.exe",
    "SystemSettings.exe",
    "SearchHost.exe",
    "StartMenuExperienceHost.exe",
    "RuntimeBroker.exe",
    "sihost.exe",
    "ctfmon.exe",
    "dwm.exe",
    "explorer.exe",
];
fn is_system_process(exe_path: &str) -> bool {
    let exe = exe_path.rsplit(&['\\', '/'][..]).next().unwrap_or(exe_path);
    SYSTEM_PROCESSES.iter().any(|s| s.eq_ignore_ascii_case(exe))
}
/// Proper display name for an exe: read the FileDescription from its version
/// resource (e.g. Code.exe -> "Visual Studio Code"). Falls back to the file
/// name without the .exe extension.
fn display_name(exe_path: &str) -> String {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let wide: Vec<u16> = OsStr::new(exe_path).encode_wide().chain(Some(0)).collect();
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
    if size != 0 {
        let mut data = vec![0u8; size as usize];
        if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr() as _) } != 0 {
            for lang in ["040904E4", "040904B0"] {
                let sub: Vec<u16> =
                    OsStr::new(&format!("\\StringFileInfo\\{lang}\\FileDescription"))
                        .encode_wide()
                        .chain(Some(0))
                        .collect();
                let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                let mut len = 0u32;
                if unsafe { VerQueryValueW(data.as_ptr() as _, sub.as_ptr(), &mut ptr, &mut len) }
                    != 0
                    && !ptr.is_null()
                    && len > 1
                {
                    let slice = unsafe {
                        std::slice::from_raw_parts(ptr as *const u16, (len - 1) as usize)
                    };
                    let s = String::from_utf16_lossy(slice).trim().to_string();
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
    }
    let file = exe_path.rsplit(&['\\', '/'][..]).next().unwrap_or(exe_path);
    file.strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(file)
        .to_string()
}
unsafe extern "system" fn collect(hwnd: HWND, param: LPARAM) -> i32 {
    if unsafe { IsWindowVisible(hwnd) } != 0 {
        if let Some((id, pid)) = identity(hwnd) {
            let list = unsafe { &mut *(param as *mut Vec<Application>) };
            // Step 4: a running Store app appears under its store id
            // with its display name, so it can be picked in the wizard.
            // Unresolvable hosts keep the old behavior: skipped.
            if is_application_frame_host(&id) {
                if let Some((store_id, _, name)) = hosted_store_app(hwnd, pid) {
                    if !list.iter().any(|a| a.id == store_id) {
                        list.push(Application { id: store_id, name });
                    }
                }
                return 1;
            }
            if is_system_process(&id) {
                return 1;
            }
            if !list.iter().any(|a| a.id == id) {
                let name = display_name(&id);
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
    let (mut id, mut pid) =
        identity(hwnd).ok_or("Foreground application is unavailable or protected.")?;
    let mut input = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut input) } == 0 {
        return Err("Idle-time API unavailable.".into());
    }
    let idle_ms = unsafe { GetTickCount() }.wrapping_sub(input.dwTime) as u64;
    // Step 4: ApplicationFrameHost is never a real app. Resolve the
    // hosted Store app (its store id, hosted pid, display name, and
    // exe path for folder rules); on failure keep the host sample,
    // which the engine never blocks (ApplicationFrameHost.exe is
    // protected), so this fails open.
    let mut path = id.clone();
    let mut name = display_name(&id);
    if is_application_frame_host(&id) {
        if let Some((store_id, hosted_pid, hosted_name)) = hosted_store_app(hwnd, pid) {
            path = image_of_pid(hosted_pid).unwrap_or_default();
            id = store_id;
            pid = hosted_pid;
            name = hosted_name;
        }
    }
    Ok(Sample {
        id,
        name,
        pid,
        idle_ms,
        path,
        title: window_title(hwnd),
    })
}
pub fn hide(sample: &Sample) -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    // Step 4: a store sample's id never equals the host exe path, so
    // re-resolve the hosted app and compare its pid instead.
    if crate::focus::is_store_id(&sample.id) {
        if let Some((_, afh_pid)) = identity(hwnd) {
            if let Some((id, pid, _)) = hosted_store_app(hwnd, afh_pid) {
                if id == sample.id && pid == sample.pid {
                    return unsafe { ShowWindowAsync(hwnd, SW_MINIMIZE) != 0 };
                }
            }
        }
        return false;
    }
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
/// Screen Time row icons on Windows: not implemented. Extracting a real
/// .exe icon to PNG would need GDI rasterization, which this crate's
/// windows-sys features do not enable, so Windows rows fall back to the
/// frontend's letter tile rather than risking the Windows build.
pub fn app_icon(_identity: &str) -> Option<String> {
    None
}
