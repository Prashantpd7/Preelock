//! Fixed setup actions. No arbitrary URLs or shell commands are accepted.
use tauri::Manager;

/// Bundle id of the user's default browser (macOS), via LaunchServices.
#[cfg(target_os = "macos")]
fn default_browser_id() -> Option<String> {
    let script = "ObjC.import('AppKit'); const ws = $.NSWorkspace.sharedWorkspace; \
        const url = $.NSURL.URLWithString('https://example.com'); \
        const appUrl = ws.URLForApplicationToOpenURL(url); \
        appUrl ? appUrl.path.js : '';";
    let output = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let output = std::process::Command::new("mdls")
        .args(["-name", "kMDItemCFBundleIdentifier", "-r", &path])
        .output()
        .ok()?;
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if id.is_empty() || id == "(null)" {
        None
    } else {
        Some(id)
    }
}

/// Executable name of the user's default browser (Windows), via the
/// https URL association in the registry.
#[cfg(target_os = "windows")]
fn default_browser_id() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
            "/v",
            "ProgId",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let prog_id = text
        .lines()
        .find_map(|l| l.split_whitespace().last())?
        .to_ascii_lowercase();
    // Map common Chromium ProgIds to their executables.
    let exe = if prog_id.contains("chrome") {
        "chrome.exe"
    } else if prog_id.contains("msedge") {
        "msedge.exe"
    } else if prog_id.contains("brave") {
        "brave.exe"
    } else if prog_id.contains("vivaldi") {
        "vivaldi.exe"
    } else if prog_id.contains("comet") {
        "comet.exe"
    } else {
        return None;
    };
    Some(exe.to_string())
}
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
        "website-controls.js",
        "adult-domains.js",
        "youtube-controls.js",
        "youtube.js",
        "page-bridge.js",
        "self-test.js",
        "popup.html",
        "popup.css",
        "popup.js",
        "blocked.html",
        "blocked.css",
        "blocked.js",
    ] {
        let bytes = std::fs::read(source.join(file)).map_err(|e| e.to_string())?;
        let dest = target.join(file);
        if std::fs::read(&dest).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(dest, bytes).map_err(|e| e.to_string())?;
        }
    }
    Ok(target)
}
/// Official download pages for the browsers Preelock can enforce rules in,
/// keyed by the same ids the app uses (bundle id or executable name). This
/// is an allowlist: the frontend can never hand an arbitrary URL to the
/// shell, and nothing is downloaded or installed by Preelock itself.
fn browser_download_url(browser_id: &str) -> Result<&'static str, String> {
    let id = browser_id.to_ascii_lowercase();
    let leaf = id.rsplit(['/', '\\']).next().unwrap_or(&id);
    Ok(match leaf {
        "com.google.chrome" | "chrome.exe" => "https://www.google.com/chrome/",
        "com.microsoft.edgemac" | "msedge.exe" => "https://www.microsoft.com/edge",
        "com.brave.browser" | "brave.exe" => "https://brave.com/download/",
        "com.vivaldi.vivaldi" | "vivaldi.exe" => "https://vivaldi.com/download/",
        "ai.perplexity.comet" | "comet.exe" => "https://www.perplexity.ai/comet",
        _ => return Err("Unknown browser.".into()),
    })
}
/// The installed supported browser to launch for session setup, or an
/// error. The id comes from the app's own browser list, and is checked
/// against the installed/known applications before anything is spawned,
/// so no arbitrary application or command can be started from here.
fn launch_target(browser_id: &str) -> Result<String, String> {
    let id = browser_id.trim();
    // A path is only acceptable as a Windows executable path; anything
    // else (a shell command, a URL) is never a browser id.
    let path_like = id.contains('/') && !id.to_ascii_lowercase().ends_with(".exe");
    if id.is_empty() || path_like || !crate::focus::supported_browser(id) {
        return Err("Unknown browser.".into());
    }
    let mut candidates = crate::platform::installed_browsers();
    if let Ok(apps) = crate::platform::applications() {
        candidates.extend(apps);
    }
    candidates
        .into_iter()
        .find(|a| a.id.eq_ignore_ascii_case(id) && crate::focus::supported_browser(&a.id))
        .map(|a| a.id)
        .ok_or_else(|| {
            "Couldn't open browser. Open it manually and refresh the app list.".to_string()
        })
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
            // Official download page for a supported browser the user does
            // not have yet (session browser compatibility). Only the
            // allowlisted official URLs; Preelock never installs anything.
            "browser_download" => {
                cmd.arg(browser_download_url(browser_id)?);
            }
            // Bring the browser the user just chose to the front. `open -b`
            // activates the running instance instead of starting a second
            // copy, so no extra window is created.
            "browser" => {
                let id = launch_target(browser_id)?;
                cmd.args(["-b", &id]);
            }
            "extensions" => {
                // Empty id = use the user's default browser.
                let default_id;
                let resolved: &str = if browser_id.is_empty() {
                    match default_browser_id() {
                        Some(id) => {
                            default_id = id;
                            &default_id
                        }
                        None => return Err("Could not find your default browser.".into()),
                    }
                } else {
                    browser_id
                };
                if !crate::focus::supported_browser(resolved) {
                    return Err("Your default browser is not supported. Open Comet, Chrome, Edge, Brave or Vivaldi first.".into());
                }
                cmd.args(["-b", resolved, "chrome://extensions/"]);
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
                // Empty id = use the user's default browser.
                let default_id;
                let resolved: &str =
                    if browser_id.is_empty() {
                        match default_browser_id() {
                            Some(id) => {
                                default_id = id;
                                &default_id
                            }
                            None => return Err(
                                "Could not find your default browser. Open it first, then refresh."
                                    .into(),
                            ),
                        }
                    } else {
                        browser_id
                    };
                if !crate::focus::supported_browser(resolved) {
                    return Err("Your default browser is not supported. Open Comet, Chrome, Edge, Brave or Vivaldi first.".into());
                }
                // When a specific browser was requested, verify it is installed.
                // The default browser from the registry is resolved to its full
                // executable path when available so Command::new avoids PATH lookups.
                let installed = crate::platform::installed_browsers();
                let apps = crate::platform::applications()?;
                let matched_path = installed
                    .iter()
                    .chain(apps.iter())
                    .find(|a| {
                        a.id.eq_ignore_ascii_case(resolved)
                            || std::path::Path::new(&a.id)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .is_some_and(|f| f.eq_ignore_ascii_case(resolved))
                    })
                    .map(|a| a.id.as_str());
                if !browser_id.is_empty() && matched_path.is_none() {
                    return Err("Open your browser first, then refresh.".into());
                }
                cmd = std::process::Command::new(matched_path.unwrap_or(resolved));
                cmd.arg("chrome://extensions/");
            }
            "browser_download" => {
                cmd = std::process::Command::new("explorer.exe");
                cmd.arg(browser_download_url(browser_id)?);
            }
            // Chromium browsers are single-instance: starting an installed
            // executable hands off to the running copy rather than opening a
            // second window.
            "browser" => {
                let exe = launch_target(browser_id)?;
                cmd = std::process::Command::new(exe);
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
        Err("Desktop setup is unavailable.".into())
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        cmd.spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}
#[cfg(test)]
mod download_links {
    use super::{browser_download_url, launch_target};
    #[test]
    fn launch_refuses_anything_but_a_supported_browser_id() {
        // Empty, Safari, arbitrary URLs and shell strings never reach
        // the launcher, whatever the frontend sends.
        for bad in [
            "",
            "   ",
            "com.apple.Safari",
            "https://example.com/",
            "open -a Calculator",
            "/tmp/evil.sh",
        ] {
            assert!(launch_target(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn download_links_are_official_and_allowlisted() {
        assert_eq!(
            browser_download_url("com.google.Chrome").unwrap(),
            "https://www.google.com/chrome/"
        );
        assert_eq!(
            browser_download_url(
                r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"
            )
            .unwrap(),
            "https://brave.com/download/"
        );
        assert_eq!(
            browser_download_url("ai.perplexity.comet").unwrap(),
            "https://www.perplexity.ai/comet"
        );
        // Anything else — including Safari and arbitrary URLs — is refused.
        assert!(browser_download_url("com.apple.Safari").is_err());
        assert!(browser_download_url("https://example.com/install").is_err());
        assert!(browser_download_url("").is_err());
    }
}
