//! Fixed setup actions. No arbitrary URLs or shell commands are accepted.
use tauri::Manager;
pub fn extension_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let source = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("browser-extension");
    // Development uses the repository extension; release uses bundled resources.
    let source = if source.join("manifest.json").is_file() {
        source
    } else if cfg!(debug_assertions) {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../browser-extension")
    } else {
        return Err("Extension files are missing. Reinstall Preelock.".into());
    };
    // Store (MSIX) packages virtualise AppData into a private per-package
    // store. The browser and Explorer are separate processes, so they cannot
    // read a copy kept there. Packaged builds therefore deploy the extension
    // to a normal user-writable folder outside AppData.
    let base = if crate::platform::packaged() {
        let home = app.path().home_dir().map_err(|e| e.to_string())?;
        let legacy = home.join("VYSTRIQ"); // Existing packaged extension path; preserve pairing.
        if legacy.join("browser-extension").exists() {
            legacy // Preserve the path Chromium already registered and paired.
        } else {
            home.join("Preelock")
        }
    } else {
        app.path().app_data_dir().map_err(|e| e.to_string())?
    };
    let target = base.join("browser-extension");
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    for file in [
        "manifest.json",
        "icon16.png",
        "icon32.png",
        "icon48.png",
        "icon128.png",
        "worker.js",
        "identity.js",
        "rules.js",
        "self-test.js",
        "popup.html",
        "popup.css",
        "popup.js",
        "blocked.html",
        "blocked.css",
    ] {
        let bytes = std::fs::read(source.join(file)).map_err(|e| e.to_string())?;
        let dest = target.join(file);
        if std::fs::read(&dest).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(dest, bytes).map_err(|e| e.to_string())?;
        }
    }
    Ok(target)
}
pub fn open(app: &tauri::AppHandle, target: &str, browser_id: &str) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let mut cmd;
    #[cfg(target_os = "macos")]
    {
        cmd = std::process::Command::new("open");
        match target {
            "extension_folder" => {
                cmd.arg(extension_dir(app)?);
            }
            "camera_settings" => {
                cmd.arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera");
            }
            "notification_settings" => {
                cmd.arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension");
            }
            "privacy_settings" => {
                cmd.arg("x-apple.systempreferences:com.apple.preference.security?Privacy");
            }
            "extensions" => {
                if !crate::focus::supported_browser(browser_id) {
                    return Err("Open Comet, Chrome, Edge, Brave or Vivaldi first.".into());
                }
                cmd.args(["-b", browser_id, "chrome://extensions/"]);
            }
            _ => return Err("Unknown setup action.".into()),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match target {
            "extension_folder" => {
                cmd = std::process::Command::new("explorer.exe");
                cmd.arg(extension_dir(app)?);
            }
            "extensions" => {
                if !crate::focus::supported_browser(browser_id)
                    || !crate::platform::installed_browsers()
                        .iter()
                        .chain(crate::platform::applications()?.iter())
                        .any(|a| a.id.eq_ignore_ascii_case(browser_id))
                {
                    return Err("Open your browser first, then refresh.".into());
                }
                cmd = std::process::Command::new(browser_id);
                cmd.arg("chrome://extensions/");
            }
            "camera_settings" | "notification_settings" | "privacy_settings" => {
                cmd = std::process::Command::new("explorer.exe");
                cmd.arg(match target {
                    "camera_settings" => "ms-settings:privacy-webcam",
                    "notification_settings" => "ms-settings:notifications",
                    _ => "ms-settings:privacy",
                });
            }
            _ => return Err("This setup action is unavailable on Windows.".into()),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, target, browser_id);
        return Err("Desktop setup is unavailable.".into());
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        cmd.spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}
