//! Preelock Privileged Helper Management Module (SelfControl-Style Kernel Blocking).
//!
//! Controls the root-level helper daemon (`preelock-helper`) which enforces
//! unbypassable domain-level blocking via `/etc/hosts` and macOS Packet Filter (`pf`).
//!
//! Provides a seamless fallback to the local DNS proxy and user-level hosts blocking
//! when the privileged helper is not installed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

pub const DEFAULT_HELPER_SOCKET: &str = "/var/run/preelock-helper.sock";
pub const HELPER_PLIST_PATH: &str = "/Library/LaunchDaemons/com.preelock.helper.plist";
pub const HELPER_BINARY_SYSTEM_PATH: &str = "/Library/PrivilegedHelperTools/preelock-helper";
pub const HELPER_SUDOERS_PATH: &str = "/etc/sudoers.d/preelock-helper";
pub const HELPER_VERSION: u32 = 2;

pub const HOSTS_START_MARKER: &str = "# === PREELOCK BLOCKLIST START ===";
pub const HOSTS_END_MARKER: &str = "# === PREELOCK BLOCKLIST END ===";

/// Owners and their domains as last requested by this process.
///
/// The helper keeps the same registry on its side. This copy exists so
/// the *fallback* path (helper unreachable) can rewrite `/etc/hosts`
/// with the union of every owner's domains instead of clearing the
/// whole section. Without it, `clear_block("app_lock")` while an empty
/// app-lock list ran on every engine tick erased the session's hosts
/// entries a second after they were written — the block visibly
/// flickered on and off (2026-11-24 diagnosis).
static ACTIVE_OWNERS: std::sync::Mutex<Option<std::collections::BTreeMap<String, OwnerBlock>>> =
    std::sync::Mutex::new(None);

#[derive(Clone, PartialEq, Eq)]
struct OwnerBlock {
    domains: Vec<String>,
    strict_until: Option<u64>,
}

/// Records an owner's desired block and returns `(union, changed)`.
/// `changed == false` means the engine is re-publishing an identical
/// policy on its 1-second tick; the caller must not touch the helper or
/// `/etc/hosts` in that case. This is what stops the resolver + pfctl
/// storm that starved IPC and flipped hosts on and off every second.
fn remember_owner(
    owner: &str,
    domains: &[String],
    strict_until: Option<u64>,
) -> (Vec<String>, bool) {
    let mut guard = ACTIVE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    let next = OwnerBlock {
        domains: domains.to_vec(),
        strict_until,
    };
    if domains.is_empty() {
        let removed = map.remove(owner).is_some();
        return (union_of(map), removed);
    }
    let changed = map.get(owner) != Some(&next);
    map.insert(owner.to_string(), next);
    (union_of(map), changed)
}

fn non_adult_union() -> Vec<String> {
    let guard = ACTIVE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(map) = guard.as_ref() else {
        return Vec::new();
    };
    let mut all: Vec<String> = map
        .iter()
        .filter(|(owner, _)| owner.as_str() != "adult_block")
        .flat_map(|(_, b)| b.domains.clone())
        .collect();
    all.sort();
    all.dedup();
    all
}

/// Removes an owner and returns `(union, removed_domains)`. A clear for an
/// owner that was never applied is a no-op: it must never rewrite (and
/// accidentally erase) another owner's hosts section or reset reloaded state.
fn forget_owner(owner: &str) -> (Vec<String>, Option<Vec<String>>) {
    let mut guard = ACTIVE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    let removed = map.remove(owner).map(|ob| ob.domains);
    (union_of(map), removed)
}

fn reset_owners() {
    if let Ok(mut guard) = ACTIVE_OWNERS.lock() {
        guard.get_or_insert_with(Default::default).clear();
    }
}

fn union_of(map: &std::collections::BTreeMap<String, OwnerBlock>) -> Vec<String> {
    let mut all: Vec<String> = map.values().flat_map(|b| b.domains.clone()).collect();
    all.sort();
    all.dedup();
    all
}

/// Fast, non-blocking check that the privileged helper socket exists.
/// Never probes with a socket round-trip; callers that need a probe
/// (`get_system_blocking_status`) do it themselves.
fn helper_socket_present() -> bool {
    Path::new(DEFAULT_HELPER_SOCKET).exists()
}

fn helper_binary_candidates(include_system: bool) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if include_system {
        // 1. Installed system privileged tools path
        candidates.push(PathBuf::from(HELPER_BINARY_SYSTEM_PATH));
    }

    // 2. Current executable directory (release bundle and macOS app structure)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("../Resources/binaries/preelock-helper"));
            candidates.push(parent.join("binaries").join("preelock-helper"));
            candidates.push(parent.join("preelock-helper"));
            candidates.push(parent.join("../Resources/helper/build/preelock-helper"));
        }
    }

    // 3. Standard application bundle location
    candidates.push(PathBuf::from(
        "/Applications/Preelock.app/Contents/Resources/binaries/preelock-helper",
    ));

    // 4. Local cargo / app target bundle directories & workspace paths
    candidates.push(PathBuf::from("src-tauri/helper/build/preelock-helper"));
    candidates.push(PathBuf::from("helper/build/preelock-helper"));
    candidates.push(PathBuf::from("src-tauri/binaries/preelock-helper"));
    candidates.push(PathBuf::from("binaries/preelock-helper"));
    candidates.push(PathBuf::from(
        "/Users/Prashant/My_Workspace/Preelock/src-tauri/helper/build/preelock-helper",
    ));
    candidates.push(PathBuf::from(
        "/Users/Prashant/My_Workspace/Preelock/src-tauri/binaries/preelock-helper",
    ));
    candidates.push(PathBuf::from(
        "/Users/Prashant/My_Workspace/Preelock/src-tauri/target/release/bundle/macos/Preelock.app/Contents/Resources/binaries/preelock-helper",
    ));
    candidates
}

/// Candidate paths to locate the compiled helper binary on the host system.
pub fn find_helper_binary() -> Option<PathBuf> {
    for candidate in &helper_binary_candidates(true) {
        if candidate.exists() {
            log::info!(
                "[PRIVILEGED_HELPER] Found helper binary candidate at: {:?}",
                candidate
            );
            return Some(candidate.clone());
        }
    }
    log::warn!("[PRIVILEGED_HELPER] Helper binary not found in any candidate paths");
    None
}

fn find_bundled_helper_binary() -> Option<PathBuf> {
    helper_binary_candidates(false)
        .into_iter()
        .find(|candidate| candidate.exists())
}

#[cfg(target_os = "macos")]
static HELPER_UPDATE_CHECKED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Automatically upgrades an already-installed privileged helper daemon when the
/// running app ships a newer `HELPER_VERSION` and passwordless sudo is authorized.
#[cfg(target_os = "macos")]
fn maybe_update_installed_helper() {
    if HELPER_UPDATE_CHECKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if !helper_socket_present() {
        return;
    }
    let current_ver = send_helper_socket_command(r#"{"command":"status"}"#)
        .ok()
        .and_then(|resp| serde_json::from_str::<serde_json::Value>(&resp).ok())
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
        .unwrap_or(1);
    if current_ver >= u64::from(HELPER_VERSION) {
        return;
    }
    let Some(bundled) = find_bundled_helper_binary() else {
        return;
    };
    let Some(bundled_str) = bundled.to_str() else {
        return;
    };
    log::info!(
        "[PRIVILEGED_HELPER] Updating helper daemon from version {current_ver} to {HELPER_VERSION} using {bundled_str}"
    );
    let status = std::process::Command::new("sudo")
        .args([
            "-n",
            HELPER_BINARY_SYSTEM_PATH,
            "update",
            "--binary",
            bundled_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if matches!(status, Ok(s) if s.success()) {
        for _ in 0..15 {
            if send_helper_socket_command(r#"{"command":"status"}"#).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Checks whether the privileged helper daemon is installed and operational.
pub fn is_helper_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        // 1. Live Unix domain socket file. Checking the file (not a
        // socket round-trip) keeps this instant: it is called on the
        // UI status path and must never block a request.
        if helper_socket_present() {
            return true;
        }

        // 2. Check if LaunchDaemon plist exists on disk
        if Path::new(HELPER_PLIST_PATH).exists() && Path::new(HELPER_BINARY_SYSTEM_PATH).exists() {
            return true;
        }

        // 3. Check passwordless sudo execution
        let status = std::process::Command::new("sudo")
            .args(["-n", HELPER_BINARY_SYSTEM_PATH, "status"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        matches!(status, Ok(s) if s.success())
    }
    #[cfg(target_os = "windows")]
    {
        // On Windows, check if elevated hosts/firewall permissions are ready
        !crate::windows_hosts::is_admin_setup_required()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

/// Runs one-time administrator authorization to install the root LaunchDaemon and sudoers entry.
///
/// NEVER called at app startup or in the background engine. Triggered strictly
/// by explicit user click in the UI.
pub fn run_admin_install_helper() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        log::info!("[PRIVILEGED_HELPER] run_admin_install_helper called");
        if is_helper_installed() {
            log::info!(
                "[PRIVILEGED_HELPER] Helper is already installed and operational; skipping install"
            );
            return Ok(());
        }
        let source_binary = find_helper_binary().ok_or_else(|| {
            "preelock-helper binary not found on disk. Build it first.".to_string()
        })?;

        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .or_else(|_| {
                std::process::Command::new("/usr/bin/id")
                    .arg("-un")
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            })
            .map_err(|e| format!("Could not determine username: {e}"))?;

        let bin_src = source_binary.to_str().ok_or("Invalid binary path")?;
        log::info!("[PRIVILEGED_HELPER] Installing helper for user={user} from source={bin_src}");

        let script = format!(
            r#"#!/bin/sh
set -e
mkdir -p /Library/PrivilegedHelperTools
mkdir -p /var/db/preelock
cp '{bin_src}' {HELPER_BINARY_SYSTEM_PATH}
chown root:wheel {HELPER_BINARY_SYSTEM_PATH}
chmod 755 {HELPER_BINARY_SYSTEM_PATH}

cat << 'EOF' > {HELPER_PLIST_PATH}
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.preelock.helper</string>
    <key>ProgramArguments</key>
    <array>
        <string>{HELPER_BINARY_SYSTEM_PATH}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/var/log/preelock-helper.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/preelock-helper.log</string>
</dict>
</plist>
EOF
chown root:wheel {HELPER_PLIST_PATH}
chmod 644 {HELPER_PLIST_PATH}

mkdir -p /etc/sudoers.d
echo '{user} ALL=(ALL) NOPASSWD: {HELPER_BINARY_SYSTEM_PATH}' > {HELPER_SUDOERS_PATH}
chown root:wheel {HELPER_SUDOERS_PATH}
chmod 440 {HELPER_SUDOERS_PATH}

/usr/sbin/visudo -cf {HELPER_SUDOERS_PATH} || (rm -f {HELPER_SUDOERS_PATH} && exit 1)

launchctl bootout system/com.preelock.helper 2>/dev/null || true
launchctl bootstrap system {HELPER_PLIST_PATH} 2>/dev/null || launchctl kickstart -k system/com.preelock.helper 2>/dev/null || true
"#
        );

        let tmp_script = format!("/tmp/preelock_helper_install_{}.sh", std::process::id());
        std::fs::write(&tmp_script, &script)
            .map_err(|e| format!("Failed to write installation script: {e}"))?;

        let osa_cmd = format!(r#"do shell script "sh {tmp_script}" with administrator privileges"#);

        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &osa_cmd])
            .output();

        let _ = std::fs::remove_file(&tmp_script);

        let output = output.map_err(|e| format!("Failed to execute authorization dialog: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("[PRIVILEGED_HELPER] osascript failed with stderr: {stderr}");
            if stderr.contains("-128") || stderr.contains("User canceled") {
                return Err("Administrator authorization was cancelled by the user.".into());
            }
            return Err(format!("Helper installation failed: {stderr}"));
        }

        // Give launchd up to 3 seconds to bootstrap and open the socket
        let mut ready = false;
        for _ in 0..15 {
            if is_helper_installed() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        if ready {
            log::info!(
                "[PRIVILEGED_HELPER] Helper successfully installed and registered with launchd."
            );
        } else {
            log::warn!(
                "[PRIVILEGED_HELPER] Helper files installed, but daemon is still starting up."
            );
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_hosts::ensure_hosts_writable()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(())
    }
}

/// Uninstalls the privileged helper daemon and cleans up system modifications.
pub fn uninstall_helper() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        log::info!("[PRIVILEGED_HELPER] uninstall_helper called");
        // 1. Send remove command to helper if accessible
        let _ = send_helper_socket_command(r#"{"command":"remove"}"#);

        let script = format!(
            r#"#!/bin/sh
launchctl bootout system/com.preelock.helper 2>/dev/null || true
rm -f {HELPER_PLIST_PATH}
rm -f {HELPER_SUDOERS_PATH}
rm -f {HELPER_BINARY_SYSTEM_PATH}
rm -f {DEFAULT_HELPER_SOCKET}
rm -rf /var/db/preelock
"#
        );

        let tmp_script = format!("/tmp/preelock_helper_uninstall_{}.sh", std::process::id());
        std::fs::write(&tmp_script, &script)
            .map_err(|e| format!("Failed to write uninstallation script: {e}"))?;

        let osa_cmd = format!(r#"do shell script "sh {tmp_script}" with administrator privileges"#);

        let output = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &osa_cmd])
            .output();

        let _ = std::fs::remove_file(&tmp_script);

        if let Ok(out) = output {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                log::warn!("[PRIVILEGED_HELPER] uninstall osascript returned stderr: {stderr}");
            }
        }

        log::info!("[PRIVILEGED_HELPER] Helper uninstalled successfully.");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// Sends a JSON command to the helper over its Unix domain socket or via sudo -n fallback.
pub fn send_helper_command(json_payload: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        // Preferred: fast Unix domain socket
        if let Ok(res) = send_helper_socket_command(json_payload) {
            return Ok(res);
        }

        // Fallback: execute helper binary via sudo -n
        if Path::new(HELPER_BINARY_SYSTEM_PATH).exists() {
            let output = std::process::Command::new("sudo")
                .args(["-n", HELPER_BINARY_SYSTEM_PATH, "apply"]) // or pipe json
                .output()
                .map_err(|e| format!("Failed to invoke helper via sudo: {e}"))?;

            if output.status.success() {
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
        }
        Err("Could not communicate with privileged helper".into())
    }
    #[cfg(not(unix))]
    {
        let _ = json_payload;
        Err("Privileged helper socket only available on Unix".into())
    }
}

/// Connects directly to `/var/run/preelock-helper.sock` to exchange a JSON message.
pub fn send_helper_socket_command(msg: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        let mut stream = UnixStream::connect(DEFAULT_HELPER_SOCKET)
            .map_err(|e| format!("Socket connection error: {e}"))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;

        let formatted = format!("{}\n", msg.trim());
        stream
            .write_all(formatted.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .map_err(|e| format!("Read failed: {e}"))?;

        String::from_utf8(buf).map_err(|e| format!("UTF-8 decode failed: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = msg;
        Err("Unix socket not supported".into())
    }
}

#[cfg(target_os = "macos")]
static PREVIOUSLY_RELOADED: std::sync::Mutex<Option<HashSet<String>>> = std::sync::Mutex::new(None);

#[cfg(target_os = "macos")]
pub fn reset_reloaded_domains() {
    if let Ok(mut lock) = PREVIOUSLY_RELOADED.lock() {
        *lock = None;
    }
}

#[cfg(not(target_os = "macos"))]
pub fn reset_reloaded_domains() {}

#[cfg(target_os = "macos")]
pub fn prune_reloaded_domains(remaining_union: &[String]) {
    if let Ok(mut lock) = PREVIOUSLY_RELOADED.lock() {
        if remaining_union.is_empty() {
            *lock = None;
        } else if let Some(ref mut set) = *lock {
            let remaining: HashSet<String> = remaining_union
                .iter()
                .map(|d| d.trim().trim_start_matches("www.").to_lowercase())
                .filter(|d| !d.is_empty())
                .collect();
            set.retain(|d| remaining.contains(d));
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn prune_reloaded_domains(_remaining_union: &[String]) {}

#[cfg(target_os = "macos")]
pub fn compute_newly_blocked(domains: &[String]) -> Vec<String> {
    if domains.is_empty() {
        return Vec::new();
    }
    let current_set: HashSet<String> = domains
        .iter()
        .map(|d| d.trim().trim_start_matches("www.").to_lowercase())
        .filter(|d| !d.is_empty())
        .collect();

    let mut prev = PREVIOUSLY_RELOADED
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let new_items: Vec<String> = match &*prev {
        Some(old_set) => current_set.difference(old_set).cloned().collect(),
        None => current_set.iter().cloned().collect(),
    };
    *prev = Some(current_set);
    new_items
}

#[cfg(not(target_os = "macos"))]
pub fn compute_newly_blocked(_domains: &[String]) -> Vec<String> {
    Vec::new()
}

/// Expands user-blocked domains with known service subdomains and DoH/Private-Relay
/// canary hosts for hosts-file blocking (without bloating the 50k adult list).
pub fn expand_user_domains_for_hosts(
    all_domains: &[String],
    user_domains: &[String],
) -> Vec<String> {
    let mut out: HashSet<String> = all_domains
        .iter()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty() && !d.contains(' ') && !d.contains('/'))
        .collect();
    if out.is_empty() {
        return Vec::new();
    }

    const SERVICE_SUBDOMAINS: &[(&str, &[&str])] = &[
        (
            "instagram.com",
            &[
                "i.instagram.com",
                "graph.instagram.com",
                "api.instagram.com",
                "m.instagram.com",
                "l.instagram.com",
                "edge-chat.instagram.com",
                "cdninstagram.com",
                "static.cdninstagram.com",
                "scontent.cdninstagram.com",
            ],
        ),
        (
            "facebook.com",
            &[
                "m.facebook.com",
                "web.facebook.com",
                "l.facebook.com",
                "graph.facebook.com",
                "edge-mqtt.facebook.com",
                "mbasic.facebook.com",
            ],
        ),
        (
            "youtube.com",
            &[
                "m.youtube.com",
                "music.youtube.com",
                "studio.youtube.com",
                "youtu.be",
            ],
        ),
        (
            "reddit.com",
            &[
                "old.reddit.com",
                "new.reddit.com",
                "sh.reddit.com",
                "oauth.reddit.com",
                "gateway.reddit.com",
                "gql.reddit.com",
                "redd.it",
            ],
        ),
        ("x.com", &["api.x.com", "mobile.x.com", "pro.x.com"]),
        ("twitter.com", &["api.twitter.com", "mobile.twitter.com"]),
        ("tiktok.com", &["m.tiktok.com", "vm.tiktok.com"]),
        ("whatsapp.com", &["web.whatsapp.com"]),
        ("discord.com", &["ptb.discord.com", "canary.discord.com"]),
    ];

    for raw in user_domains {
        let clean = raw.trim().to_ascii_lowercase();
        if clean.is_empty() || clean.contains(' ') || clean.contains('/') {
            continue;
        }
        let bare = clean.strip_prefix("www.").unwrap_or(&clean);
        out.insert(bare.to_string());
        out.insert(format!("www.{bare}"));
        for (parent, subs) in SERVICE_SUBDOMAINS {
            if bare == *parent {
                for sub in *subs {
                    out.insert((*sub).to_string());
                }
            }
        }
        for prefix in ["m.", "mobile.", "api.", "app.", "web."] {
            if !bare.starts_with(prefix) && bare != "use-application-dns.net" {
                out.insert(format!("{prefix}{bare}"));
            }
        }
    }

    for canary in [
        "mask.icloud.com",
        "mask-h2.icloud.com",
        "use-application-dns.net",
    ] {
        out.insert(canary.to_string());
    }

    let mut res: Vec<String> = out.into_iter().collect();
    res.sort();
    res
}

#[cfg(target_os = "macos")]
const MACOS_BROWSER_APPS: &[(&str, &str)] = &[
    ("com.apple.safari", "Safari"),
    ("com.google.chrome", "Google Chrome"),
    ("com.brave.browser", "Brave Browser"),
    ("com.microsoft.edgemac", "Microsoft Edge"),
    ("ai.perplexity.comet", "Comet"),
    ("company.thebrowser.browser", "Arc"),
    ("com.vivaldi.vivaldi", "Vivaldi"),
    ("com.operasoftware.opera", "Opera"),
    ("com.operasoftware.operagx", "Opera GX"),
    ("org.chromium.chromium", "Chromium"),
];

#[cfg(target_os = "macos")]
static REDIRECT_DEBOUNCE: std::sync::Mutex<Option<std::collections::HashMap<String, u64>>> =
    std::sync::Mutex::new(None);

#[cfg(target_os = "macos")]
fn blocked_redirect_url(host: &str) -> String {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], crate::browser::PORT));
    if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(40)).is_ok() {
        let clean = crate::browser::sanitize_blocked_host(host);
        if clean.is_empty() {
            format!("http://127.0.0.1:{}/blocked", crate::browser::PORT)
        } else {
            format!(
                "http://127.0.0.1:{}/blocked?h={clean}",
                crate::browser::PORT
            )
        }
    } else {
        "about:blank".to_string()
    }
}

#[cfg(target_os = "macos")]
fn is_browser_process_running(app_name: &str) -> bool {
    // Only target applications that are actually running. Never use `pgrep -f`
    // which matches full command-line arguments of background system agents
    // (such as SafariBookmarksSyncAgent or Google Chrome Helper).
    std::process::Command::new("/usr/bin/pgrep")
        .arg("-x")
        .arg(app_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn url_host_and_path(raw: &str) -> Option<(String, String)> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() || matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") {
        return None;
    }
    Some((host, parsed.path().to_string()))
}

#[cfg(target_os = "macos")]
fn enforce_tabs_in_browser<F>(app_name: &str, matcher: &F)
where
    F: Fn(&str, &str) -> bool,
{
    if !is_browser_process_running(app_name) {
        return;
    }
    let list_script = format!(
        "if application \"{app_name}\" is running then\n\
        tell application \"{app_name}\"\n\
        set out to \"\"\n\
        set wIdx to 0\n\
        repeat with w in windows\n\
        set wIdx to wIdx + 1\n\
        try\n\
        set tIdx to 0\n\
        repeat with t in tabs of w\n\
        set tIdx to tIdx + 1\n\
        try\n\
        set u to URL of t\n\
        if u is not missing value then\n\
        set out to out & (wIdx as string) & (ASCII character 9) & (tIdx as string) & (ASCII character 9) & u & linefeed\n\
        end if\n\
        end try\n\
        end repeat\n\
        end try\n\
        end repeat\n\
        return out\n\
        end tell\n\
        else\n\
        return \"\"\n\
        end if"
    );

    let Ok(out) = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &list_script])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut updates: Vec<(usize, usize, String)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let Some(w_str) = parts.next() else { continue };
        let Some(t_str) = parts.next() else { continue };
        let Some(tab_url) = parts.next() else {
            continue;
        };
        let Ok(w_idx) = w_str.trim().parse::<usize>() else {
            continue;
        };
        let Ok(t_idx) = t_str.trim().parse::<usize>() else {
            continue;
        };
        if let Some((host, path)) = url_host_and_path(tab_url) {
            if matcher(&host, &path) {
                updates.push((w_idx, t_idx, blocked_redirect_url(&host)));
            }
        }
    }

    if updates.is_empty() {
        return;
    }

    let mut set_script = format!("tell application \"{app_name}\"\n");
    for (w_idx, t_idx, target_url) in updates {
        set_script.push_str(&format!(
            "try\nset URL of tab {t_idx} of window {w_idx} to \"{target_url}\"\nend try\n"
        ));
    }
    set_script.push_str("end tell");

    let _ = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", &set_script])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(target_os = "macos")]
pub fn reload_blocked_browser_tabs(domains: &[String]) {
    let newly_blocked = compute_newly_blocked(domains);
    if newly_blocked.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        let matcher = |host: &str, path: &str| {
            newly_blocked
                .iter()
                .any(|rule| crate::focus::rule_matches(rule, host, path))
        };
        for &(_, app_name) in MACOS_BROWSER_APPS {
            enforce_tabs_in_browser(app_name, &matcher);
        }
        // Second sweep after 450ms catches tabs that were still loading
        // or had PF state kills completing right after apply returned.
        std::thread::sleep(Duration::from_millis(450));
        for &(_, app_name) in MACOS_BROWSER_APPS {
            enforce_tabs_in_browser(app_name, &matcher);
        }
    });
}

#[cfg(target_os = "macos")]
pub fn reload_adult_browser_tabs() {
    std::thread::spawn(|| {
        let matcher = |host: &str, _path: &str| crate::adult_block::is_adult_domain(host);
        for &(_, app_name) in MACOS_BROWSER_APPS {
            enforce_tabs_in_browser(app_name, &matcher);
        }
    });
}

#[cfg(target_os = "macos")]
static LAST_ALLOWLIST_TABS: std::sync::Mutex<Option<(Vec<String>, bool)>> =
    std::sync::Mutex::new(None);

/// Immediately redirects any already-open browser tabs outside the session's
/// allowlist across all running macOS browsers (only fires when the allowlist changes).
#[cfg(target_os = "macos")]
pub fn enforce_allowlist_browser_tabs(allowed_domains: &[String], skip_youtube: bool) {
    let mut normalized: Vec<String> = allowed_domains
        .iter()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    {
        let mut guard = LAST_ALLOWLIST_TABS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let next = (normalized.clone(), skip_youtube);
        if guard.as_ref() == Some(&next) {
            return;
        }
        *guard = Some(next);
    }
    std::thread::spawn(move || {
        let matcher = |host: &str, path: &str| {
            if skip_youtube && (host == "youtube.com" || host.ends_with(".youtube.com")) {
                return false;
            }
            !normalized
                .iter()
                .any(|rule| crate::focus::rule_matches(rule, host, path))
        };
        for &(_, app_name) in MACOS_BROWSER_APPS {
            enforce_tabs_in_browser(app_name, &matcher);
        }
    });
}

/// Redirects open tabs matching `blocked_domain` inside a specific foreground
/// browser (`app_id`), debounced per `(app_id, blocked_domain)` so repeated
/// 1-second ticks cannot spam AppleEvents while still immediately stopping
/// Service-Worker-cached, DoH, or browser-VPN-proxied visits.
#[cfg(target_os = "macos")]
pub fn redirect_blocked_tab_for_browser(app_id: &str, blocked_domain: &str) {
    let Some(&(_, app_name)) = MACOS_BROWSER_APPS
        .iter()
        .find(|&&(id, _)| crate::browser::same_browser(id, app_id))
    else {
        return;
    };
    let domain_norm = blocked_domain
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if domain_norm.is_empty() {
        return;
    }

    let now = crate::clock::continuous_ms();
    let key = format!("{app_name}:{domain_norm}");
    {
        let mut guard = REDIRECT_DEBOUNCE.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(Default::default);
        if let Some(&last_at) = map.get(&key) {
            if now.saturating_sub(last_at) < 800 {
                return;
            }
        }
        map.insert(key, now);
        if map.len() > 128 {
            map.retain(|_, ts| now.saturating_sub(*ts) < 10_000);
        }
    }

    std::thread::spawn(move || {
        enforce_tabs_in_browser(app_name, &|host: &str, path: &str| {
            crate::focus::rule_matches(&domain_norm, host, path)
                || host == domain_norm
                || host.ends_with(&format!(".{domain_norm}"))
        });
    });
}

#[cfg(not(target_os = "macos"))]
pub fn reload_blocked_browser_tabs(_domains: &[String]) {}

#[cfg(not(target_os = "macos"))]
pub fn reload_adult_browser_tabs() {}

#[cfg(not(target_os = "macos"))]
pub fn enforce_allowlist_browser_tabs(_allowed_domains: &[String], _skip_youtube: bool) {}

#[cfg(not(target_os = "macos"))]
pub fn redirect_blocked_tab_for_browser(_app_id: &str, _blocked_domain: &str) {}

/// Applies website blocking rules across both hosts and PF firewall.
///
/// Caller identifies themselves with `owner` (e.g. `"session"` or `"app_lock"`).
/// If `strict_until` is provided, strict mode prevents cancellation before that timestamp.
pub fn apply_block(
    owner: &str,
    domains: &[String],
    strict_until: Option<u64>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        maybe_update_installed_helper();
        let (union, changed) = remember_owner(owner, domains, strict_until);
        if !changed {
            return Ok(());
        }
        let expanded_union = expand_user_domains_for_hosts(&union, &non_adult_union());
        let res = if is_helper_installed() {
            let json = serde_json::json!({
                "command": "apply",
                "owner": owner,
                "domains": domains,
                "strict_until": strict_until
            });
            match send_helper_socket_command(&json.to_string()) {
                Ok(resp) => {
                    log::info!("[PRIVILEGED_HELPER] Applied block for owner {owner}: {resp}");
                    Ok(())
                }
                Err(e) => {
                    log::warn!(
                        "[PRIVILEGED_HELPER] Socket call failed ({e}), falling back to hosts"
                    );
                    // Fallback writes the union of every owner's domains,
                    // never just this owner's: clearing or applying one
                    // owner must not drop another owner's blocks.
                    if expanded_union.is_empty() {
                        crate::macos_hosts::clear_blocks()
                    } else {
                        crate::macos_hosts::apply_blocks(&expanded_union)
                    }
                }
            }
        } else if expanded_union.is_empty() {
            crate::macos_hosts::clear_blocks()
        } else {
            crate::macos_hosts::apply_blocks(&expanded_union)
        };

        if res.is_ok() {
            // Already-open tabs: redirect any tab whose host matches the
            // newly blocked sites so it cannot keep running or serving
            // from Service Workers / browser caches.
            if owner == "adult_block" {
                reload_adult_browser_tabs();
            } else {
                reload_blocked_browser_tabs(domains);
            }
        }
        res
    }
    #[cfg(target_os = "windows")]
    {
        let (union, changed) = remember_owner(owner, domains, strict_until);
        if !changed {
            return Ok(());
        }
        let user_doms = non_adult_union();
        let expanded_union = expand_user_domains_for_hosts(&union, &user_doms);
        if expanded_union.is_empty() {
            crate::windows_hosts::clear_blocks()?;
        } else {
            crate::windows_hosts::apply_blocks(&expanded_union)?;
        }
        if owner != "adult_block" {
            crate::windows_firewall::apply_firewall_rules(&user_doms)?;
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (owner, domains, strict_until);
        Ok(())
    }
}

/// Clears website blocking rules for the given owner.
///
/// If other owners still have active rules (e.g. App Lock active while a session ends),
/// the shared domains remain blocked.
pub fn clear_block(owner: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (union, removed) = forget_owner(owner);
        let Some(removed_domains) = removed else {
            return Ok(());
        };
        if !removed_domains.is_empty() {
            prune_reloaded_domains(&union);
        }
        if owner == "session" {
            if let Ok(mut guard) = LAST_ALLOWLIST_TABS.lock() {
                *guard = None;
            }
        }
        let expanded_union = expand_user_domains_for_hosts(&union, &non_adult_union());
        if is_helper_installed() {
            let json = serde_json::json!({
                "command": "clear",
                "owner": owner,
                "force": true
            });
            match send_helper_socket_command(&json.to_string()) {
                Ok(resp) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                        if v.get("status").and_then(|s| s.as_str()) == Some("ok") {
                            log::info!(
                                "[PRIVILEGED_HELPER] Cleared block for owner {owner}: {resp}"
                            );
                            return Ok(());
                        } else {
                            log::warn!(
                                "[PRIVILEGED_HELPER] Helper returned error on clear: {resp}"
                            );
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[PRIVILEGED_HELPER] Socket call failed ({e}), falling back to hosts"
                    );
                }
            }
        }
        // Fallback: rewrite hosts with the domains still owned by other
        // owners; only clear the section when nothing is left.
        if expanded_union.is_empty() {
            crate::macos_hosts::clear_blocks()
        } else {
            crate::macos_hosts::apply_blocks(&expanded_union)
        }
    }
    #[cfg(target_os = "windows")]
    {
        let (union, removed) = forget_owner(owner);
        if removed.is_none() {
            return Ok(());
        }
        let user_doms = non_adult_union();
        let expanded_union = expand_user_domains_for_hosts(&union, &user_doms);
        if expanded_union.is_empty() {
            crate::windows_hosts::clear_blocks()?;
        } else {
            crate::windows_hosts::apply_blocks(&expanded_union)?;
        }
        if owner != "adult_block" {
            crate::windows_firewall::apply_firewall_rules(&user_doms)?;
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = owner;
        Ok(())
    }
}

/// Queries the live system-level blocking status from /etc/hosts, the privileged helper, and PF firewall.
pub fn get_system_blocking_status() -> serde_json::Value {
    let mut hosts_entries = 0;
    if let Ok(hosts) = std::fs::read_to_string("/etc/hosts") {
        let mut in_block = false;
        for line in hosts.lines() {
            let trimmed = line.trim();
            if trimmed == HOSTS_START_MARKER {
                in_block = true;
            } else if trimmed == HOSTS_END_MARKER {
                in_block = false;
            } else if in_block && !trimmed.starts_with('#') && !trimmed.is_empty() {
                hosts_entries += 1;
            }
        }
    }

    let mut pf_rules = 0;
    if let Ok(anchor) = std::fs::read_to_string("/etc/pf.anchors/com.preelock") {
        for line in anchor.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') && !trimmed.is_empty() {
                pf_rules += 1;
            }
        }
    }

    let helper_installed = is_helper_installed();
    let mut helper_domains = Vec::new();
    let mut strict_until = None;

    if helper_installed {
        if let Ok(resp) = send_helper_socket_command(r#"{"command":"status"}"#) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(arr) = v.get("domains").and_then(|d| d.as_array()) {
                    for d in arr {
                        if let Some(s) = d.as_str() {
                            helper_domains.push(s.to_string());
                        }
                    }
                }
                strict_until = v.get("strict_until").and_then(|u| u.as_u64());
            }
        }
    }

    let is_clean = hosts_entries == 0 && pf_rules == 0 && helper_domains.is_empty();

    serde_json::json!({
        "hosts_entries": hosts_entries,
        "pf_rules": pf_rules,
        "helper_installed": helper_installed,
        "helper_domains": helper_domains,
        "strict_until": strict_until,
        "clean": is_clean,
    })
}

/// Flushes all blocks across the entire machine (privileged helper remove + /etc/hosts clear).
pub fn flush_all_blocks() -> Result<(), String> {
    unblock_everything("manual flush")
}

/// Removes **every** Preelock block, without needing the background engine.
///
/// The helper daemon is a separate root process: it is asked to remove all
/// owners (hosts + PF anchor + states). If it is unreachable, the hosts
/// section is still stripped directly and the anchor file emptied, so the
/// user can never stay blocked by a dead engine. The result is verified by
/// reading `/etc/hosts` and the anchor back, never by trusting a return code.
pub fn unblock_everything(reason: &str) -> Result<(), String> {
    let _ = reason;
    #[cfg(target_os = "macos")]
    {
        reset_owners();
        reset_reloaded_domains();
        if let Ok(mut guard) = LAST_ALLOWLIST_TABS.lock() {
            *guard = None;
        }
        let mut helper_ok = false;
        if is_helper_installed() {
            for attempt in 0..3 {
                match send_helper_socket_command(r#"{"command":"remove"}"#) {
                    Ok(resp) => {
                        helper_ok = serde_json::from_str::<serde_json::Value>(&resp)
                            .ok()
                            .and_then(|v| {
                                v.get("status").and_then(|s| s.as_str()).map(|s| s == "ok")
                            })
                            .unwrap_or(false);
                        log::info!(
                            "[UNBLOCK] Helper remove ({reason}) attempt {}: {resp}",
                            attempt + 1
                        );
                        if helper_ok {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "[UNBLOCK] Helper remove ({reason}) attempt {} failed: {e}",
                            attempt + 1
                        );
                    }
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
        let _ = crate::macos_hosts::clear_blocks();
        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();
        let _ = std::process::Command::new("/usr/bin/killall")
            .args(["-HUP", "mDNSResponder"])
            .status();

        // Verify by reading the files back — a cleanup that does not
        // verify is not a cleanup.
        let hosts_clean = std::fs::read_to_string("/etc/hosts")
            .map(|c| !c.contains(HOSTS_START_MARKER))
            .unwrap_or(true);
        let anchor_clean = std::fs::read_to_string("/etc/pf.anchors/com.preelock")
            .map(|c| {
                !c.lines()
                    .any(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
            })
            .unwrap_or(true);
        log::info!(
            "[UNBLOCK] {reason}: hosts_clean={hosts_clean} anchor_clean={anchor_clean} helper_remove_ok={helper_ok}"
        );
        if hosts_clean && anchor_clean {
            Ok(())
        } else {
            Err(format!(
                "Blocks remain after {reason}: hosts_clean={hosts_clean} anchor_clean={anchor_clean}"
            ))
        }
    }
    #[cfg(target_os = "windows")]
    {
        reset_owners();
        crate::windows_hosts::clear_blocks()?;
        crate::windows_firewall::clear_firewall_rules()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(())
    }
}

// ============================================================================
// PURE UNIT-TESTABLE FUNCTIONS (No root required)
// ============================================================================

/// Generates `/etc/hosts` blocking lines for domain candidates.
/// Emits both bare domain and www variant for IPv4 (`0.0.0.0`) and IPv6 (`::`).
pub fn generate_hosts_block_section(domains: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push(HOSTS_START_MARKER.to_string());
    lines.push("# Managed by Preelock. Do not edit this section manually.".to_string());

    let mut clean_domains = HashSet::new();
    for d in domains {
        let trimmed = d.trim().to_ascii_lowercase();
        if !trimmed.is_empty() && !trimmed.contains(' ') && !trimmed.contains('/') {
            let bare = if let Some(stripped) = trimmed.strip_prefix("www.") {
                stripped.to_string()
            } else {
                trimmed
            };
            clean_domains.insert(bare);
        }
    }

    let mut sorted: Vec<String> = clean_domains.into_iter().collect();
    sorted.sort();

    for d in sorted {
        lines.push(format!("0.0.0.0 {d}"));
        lines.push(format!("0.0.0.0 www.{d}"));
        lines.push(format!(":: {d}"));
        lines.push(format!(":: www.{d}"));
    }

    lines.push(HOSTS_END_MARKER.to_string());
    lines.join("\n") + "\n"
}

/// Formats PF anchor blocking rule syntax for a set of resolved IP addresses.
pub fn generate_pf_rule_text(ips: &[String]) -> String {
    let mut out = String::from("# Preelock packet filter rules\n");
    let mut sorted = ips.to_vec();
    sorted.sort();
    sorted.dedup();

    for ip in sorted {
        let norm = ip.trim();
        if !norm.is_empty() {
            out.push_str(&format!(
                "block return out quick proto tcp from any to {norm}\n"
            ));
            out.push_str(&format!(
                "block return out quick proto udp from any to {norm}\n"
            ));
        }
    }
    out
}

/// Reference-counting owner manager for tracking multi-owner domain blocking.
#[derive(Default, Debug, Clone)]
pub struct DomainOwnerTracker {
    owners: std::collections::HashMap<String, HashSet<String>>,
}

impl DomainOwnerTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, owner: &str, domains: &[String]) -> Vec<String> {
        let mut set = HashSet::new();
        for d in domains {
            let norm = d.trim().to_ascii_lowercase();
            if !norm.is_empty() {
                set.insert(norm);
            }
        }
        self.owners.insert(owner.to_string(), set);
        self.all_blocked_domains()
    }

    pub fn clear(&mut self, owner: &str) -> Vec<String> {
        self.owners.remove(owner);
        self.all_blocked_domains()
    }

    pub fn all_blocked_domains(&self) -> Vec<String> {
        let mut union_set = HashSet::new();
        for domains in self.owners.values() {
            for d in domains {
                union_set.insert(d.clone());
            }
        }
        let mut res: Vec<String> = union_set.into_iter().collect();
        res.sort();
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hosts_line_generation() {
        let domains = vec![
            "instagram.com".to_string(),
            "www.facebook.com".to_string(),
            "invalid domain".to_string(),
        ];
        let section = generate_hosts_block_section(&domains);

        assert!(section.contains(HOSTS_START_MARKER));
        assert!(section.contains(HOSTS_END_MARKER));
        assert!(section.contains("0.0.0.0 instagram.com"));
        assert!(section.contains("0.0.0.0 www.instagram.com"));
        assert!(section.contains(":: instagram.com"));
        assert!(section.contains(":: www.instagram.com"));
        assert!(section.contains("0.0.0.0 facebook.com"));
        assert!(section.contains("0.0.0.0 www.facebook.com"));
        assert!(!section.contains("invalid domain"));
    }

    #[test]
    fn test_pf_rule_text_generation() {
        let ips = vec![
            "157.240.22.35".to_string(),
            "2a03:2880:f12c:83:face:b00c:0:25de".to_string(),
        ];
        let rules = generate_pf_rule_text(&ips);

        assert!(rules.contains("block return out quick proto tcp from any to 157.240.22.35"));
        assert!(rules.contains("block return out quick proto udp from any to 157.240.22.35"));
        assert!(rules.contains(
            "block return out quick proto tcp from any to 2a03:2880:f12c:83:face:b00c:0:25de"
        ));
        assert!(rules.contains(
            "block return out quick proto udp from any to 2a03:2880:f12c:83:face:b00c:0:25de"
        ));
    }

    #[test]
    fn test_owner_reference_count_overlapping() {
        let mut tracker = DomainOwnerTracker::new();

        // 1. Independent App Lock blocks instagram.com
        let blocked = tracker.apply("app_lock", &["instagram.com".to_string()]);
        assert_eq!(blocked, vec!["instagram.com"]);

        // 2. Focus Session starts, blocking instagram.com AND facebook.com
        let blocked_session = tracker.apply(
            "session",
            &["instagram.com".to_string(), "facebook.com".to_string()],
        );
        assert_eq!(blocked_session, vec!["facebook.com", "instagram.com"]);

        // 3. Focus Session finishes -> clears "session" owner
        let remaining = tracker.clear("session");

        // CRITICAL CHECK: instagram.com must NOT be unblocked because "app_lock" still owns it!
        assert_eq!(remaining, vec!["instagram.com"]);
        assert!(remaining.contains(&"instagram.com".to_string()));
        assert!(!remaining.contains(&"facebook.com".to_string()));

        // 4. User unblocks instagram.com in App Lock
        let final_remaining = tracker.clear("app_lock");
        assert!(final_remaining.is_empty());
    }

    #[test]
    fn test_find_helper_binary_locates_existing_binary() {
        // As long as the binary was built via build.sh, find_helper_binary should locate it
        if Path::new("helper/build/preelock-helper").exists()
            || Path::new("src-tauri/helper/build/preelock-helper").exists()
        {
            let found = find_helper_binary();
            assert!(found.is_some(), "Helper binary was expected to be found");
            assert!(found.unwrap().exists());
        }
    }

    #[test]
    fn test_unowned_clear_does_not_arm_reload() {
        reset_reloaded_domains();
        reset_owners();

        let d = vec!["facebook.com".to_string()];
        // 1. Initial block computation treats facebook.com as newly blocked
        let newly = compute_newly_blocked(&d);
        assert_eq!(newly, vec!["facebook.com"]);

        // Subsequent check without clearing shows it is already known (not newly blocked)
        assert!(compute_newly_blocked(&d).is_empty());

        // 2. Clear unowned session (was never applied in ACTIVE_OWNERS)
        let res = clear_block("session");
        assert!(res.is_ok());

        // 3. Must NOT re-arm: facebook.com must still be considered already blocked!
        let re_check = compute_newly_blocked(&d);
        assert!(
            re_check.is_empty(),
            "Expected already-blocked domain not to be reloaded after unowned clear, got {re_check:?}"
        );
    }

    #[test]
    fn test_expand_user_domains_includes_subdomains_and_canaries() {
        let user = vec!["instagram.com".to_string()];
        let expanded = expand_user_domains_for_hosts(&user, &user);
        assert!(expanded.contains(&"instagram.com".to_string()));
        assert!(expanded.contains(&"www.instagram.com".to_string()));
        assert!(expanded.contains(&"i.instagram.com".to_string()));
        assert!(expanded.contains(&"graph.instagram.com".to_string()));
        assert!(expanded.contains(&"cdninstagram.com".to_string()));
        assert!(expanded.contains(&"static.cdninstagram.com".to_string()));
        assert!(expanded.contains(&"mask.icloud.com".to_string()));
        assert!(expanded.contains(&"mask-h2.icloud.com".to_string()));
        assert!(expanded.contains(&"use-application-dns.net".to_string()));

        // Empty blocklist must stay empty (never inject canary domains when nothing is blocked).
        assert!(expand_user_domains_for_hosts(&[], &[]).is_empty());
    }
}
