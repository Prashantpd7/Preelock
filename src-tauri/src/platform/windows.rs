use super::{Application, Sample};
use std::cell::RefCell;
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

struct Camera {
    capture: windows::Media::Capture::MediaCapture,
    detector: windows::Media::FaceAnalysis::FaceDetector,
}

thread_local! {
    static CAMERA: RefCell<Option<Camera>> = const { RefCell::new(None) };
    static CAMERA_STATUS: RefCell<String> = RefCell::new("not_requested".into());
}

fn set_camera_status(status: &str) {
    CAMERA_STATUS.with(|current| *current.borrow_mut() = status.into());
}

fn windows_error_status(error: &windows::core::Error) -> &'static str {
    match error.code().0 as u32 {
        0x80070005 => "denied",
        0xC00DABE0 | 0x80070490 => "no_camera",
        _ => "camera_unavailable",
    }
}

fn ensure_windows_runtime() -> Result<(), String> {
    unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
    }
    .map_err(|error| {
        if error.code().0 as u32 == 0x80010106 {
            String::new()
        } else {
            error.to_string()
        }
    })
}
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
    let err = unsafe { GetPackageFamilyName(handle as isize, &mut len, std::ptr::null_mut()) };
    let pfn = if err == 122 && len > 1 {
        let mut buffer = vec![0u16; len as usize];
        let ok = unsafe { GetPackageFamilyName(handle as isize, &mut len, buffer.as_mut_ptr()) };
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
    let screen_locked = {
        let lower = id.to_ascii_lowercase();
        lower.contains("lockapp.exe") || lower.contains("logonui.exe")
    };
    Ok(Sample {
        id,
        name,
        pid,
        idle_ms,
        path,
        title: window_title(hwnd),
        display_asleep: false,
        screen_locked,
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
    CAMERA_STATUS.with(|status| match status.borrow().as_str() {
        "granted"
        | "starting"
        | "person_detected"
        | "person_not_detected"
        | "extended_absence"
        | "camera_interrupted"
        | "camera_stalled" => "granted",
        "denied" => "denied",
        "no_camera" => "unavailable",
        _ => "not_requested",
    })
}
pub fn request_camera() -> Result<(), String> {
    if camera_start() {
        camera_stop();
    }
    Ok(())
}
pub fn camera_start() -> bool {
    if ensure_windows_runtime().is_err() {
        set_camera_status("camera_unavailable");
        return false;
    }
    CAMERA.with(|state| {
        let mut state = state.borrow_mut();
        if state.is_some() {
            return true;
        }
        use windows::Media::{
            Capture::{MediaCapture, MediaCaptureInitializationSettings, StreamingCaptureMode},
            FaceAnalysis::FaceDetector,
        };
        let settings = match MediaCaptureInitializationSettings::new() {
            Ok(settings) => settings,
            Err(_) => {
                set_camera_status("camera_unavailable");
                return false;
            }
        };
        if settings
            .SetStreamingCaptureMode(StreamingCaptureMode::Video)
            .is_err()
        {
            set_camera_status("camera_unavailable");
            return false;
        }
        let capture = match MediaCapture::new().and_then(|capture| {
            capture
                .InitializeWithSettingsAsync(&settings)?
                .join()
                .map(|_| capture)
        }) {
            Ok(capture) => capture,
            Err(error) => {
                set_camera_status(windows_error_status(&error));
                return false;
            }
        };
        let detector = match FaceDetector::CreateAsync().and_then(|operation| operation.join()) {
            Ok(detector) => detector,
            Err(_) => {
                let _ = capture.Close();
                set_camera_status("analysis_error");
                return false;
            }
        };
        if capture
            .StartPreviewAsync()
            .and_then(|operation| operation.join())
            .is_err()
        {
            let _ = capture.Close();
            set_camera_status("camera_unavailable");
            return false;
        }
        *state = Some(Camera { capture, detector });
        set_camera_status("starting");
        true
    })
}
pub fn camera_stop() {
    CAMERA.with(|state| {
        if let Some(camera) = state.borrow_mut().take() {
            let _ = camera
                .capture
                .StopPreviewAsync()
                .and_then(|operation| operation.join());
            let _ = camera.capture.Close();
        }
    });
    set_camera_status("off");
}
pub fn camera_status() -> String {
    CAMERA.with(|state| {
        let state = state.borrow();
        let Some(camera) = state.as_ref() else {
            return CAMERA_STATUS.with(|status| status.borrow().clone());
        };
        let frame = match camera
            .capture
            .GetPreviewFrameAsync()
            .and_then(|operation| operation.join())
        {
            Ok(frame) => frame,
            Err(_) => {
                set_camera_status("camera_interrupted");
                return "camera_interrupted".into();
            }
        };
        let bitmap = match frame.SoftwareBitmap() {
            Ok(bitmap) => bitmap,
            Err(_) => {
                set_camera_status("camera_stalled");
                return "camera_stalled".into();
            }
        };
        let bitmap = match windows::Graphics::Imaging::SoftwareBitmap::Convert(
            &bitmap,
            windows::Graphics::Imaging::BitmapPixelFormat::Bgra8,
        ) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                set_camera_status("analysis_error");
                return "analysis_error".into();
            }
        };
        let faces = match camera
            .detector
            .DetectFacesAsync(&bitmap)
            .and_then(|operation| operation.join())
            .and_then(|faces| faces.Size())
        {
            Ok(count) => count,
            Err(_) => {
                set_camera_status("analysis_error");
                return "analysis_error".into();
            }
        };
        let status = if faces > 0 {
            "person_detected"
        } else {
            "person_not_detected"
        };
        set_camera_status(status);
        status.into()
    })
}
/// Native website identification (no extension): the foreground
/// Chromium window's address bar value, read through UI Automation —
/// the supported accessibility interface for this, with no process
/// memory inspection and no debugging port. Returns `no_url` when the
/// browser is not Chromium, runs at a higher integrity level, or has no
/// address bar in its tree; the caller then keeps browser-level usage.
fn address_bar_value(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationValuePattern, TreeScope_Descendants,
        UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_ValuePatternId,
    };
    // The sampler thread is not COM-initialized; each thread initializes COM
    // once on first use. A failure here is harmless (another caller on this
    // thread may have already initialized it).
    thread_local! {
        static COM_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    COM_READY.with(|ready| {
        if !ready.get() {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            ready.set(true);
        }
    });
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let root = automation.ElementFromHandle(hwnd).ok()?;
        // One condition-filtered lookup instead of walking the subtree:
        // the first edit control in tree order is the address bar.
        let edit = VARIANT::from(UIA_EditControlTypeId.0);
        let condition = automation
            .CreatePropertyCondition(UIA_ControlTypePropertyId, &edit)
            .ok()?;
        let element = root.FindFirst(TreeScope_Descendants, &condition).ok()?;
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?;
        let value = pattern.CurrentValue().ok()?;
        let text = value.to_string();
        // Only a value that validates as an http(s) page is used, so a
        // page's own text field (or a typed query) is never stored as a
        // website.
        if crate::websites::domain_from_url(&text).is_some() {
            return Some(text);
        }
    }
    None
}
pub fn web_sample() -> super::WebSample {
    let empty = || super::WebSample {
        url: String::new(),
        status: "no_url".into(),
    };
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return empty();
    }
    // windows-sys reports the handle as a raw pointer while UI Automation
    // takes the windows crate's HWND wrapper: same handle, distinct types.
    let element = windows::Win32::Foundation::HWND(hwnd);
    match address_bar_value(element) {
        Some(url) => super::WebSample {
            url,
            status: "ok".into(),
        },
        None => empty(),
    }
}
/// UI Automation needs no separate user grant for same-integrity
/// processes; an elevated browser simply yields `no_url` above.
pub fn web_permission() -> &'static str {
    "granted"
}
pub fn request_web_permission() {}
pub fn redirect_safari_blocked() {}
pub fn redirect_browser_blocked(_: &str) {}
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
    fn expand_env_vars(input: &str, vars: &[(String, String)]) -> String {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after_start = &rest[start + 1..];
            if let Some(end) = after_start.find('%') {
                let name = &after_start[..end];
                if let Some((_, val)) = vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
                    out.push_str(val);
                } else {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
                rest = &after_start[end + 1..];
            } else {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
        out.push_str(rest);
        out
    }

    let env_vars: Vec<(String, String)> = std::env::vars().collect();
    let mut result: Vec<Application> = Vec::new();
    for raw_id in candidates {
        let id = expand_env_vars(&raw_id, &env_vars);
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

pub fn register_application_restart(args: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Recovery::RegisterApplicationRestart;
    let wide: Vec<u16> = OsStr::new(args).encode_wide().chain(Some(0)).collect();
    unsafe {
        let _ = RegisterApplicationRestart(wide.as_ptr(), 0);
    }
}
