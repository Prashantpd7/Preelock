//! Preelock Native DNS Proxy Manager Module (Phase 8).
//!
//! Controls the local user-space DNS proxy sidecar (`preelock-dns-proxy`)
//! via its local Unix domain control socket, applies session website blocking rules,
//! manages loopback system DNS redirection, and ensures graceful degradation when inactive.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;

const DEFAULT_SOCKET_PATH: &str = "/tmp/preelock-dns-control.sock";
const DEFAULT_POLICY_PATH: &str = "/tmp/preelock_active_policy.json";
#[allow(dead_code)]
const ORIGINAL_DNS_PATH: &str = "/tmp/preelock_original_dns.txt";
#[allow(dead_code)]
const UPSTREAM_DNS_PATH: &str = "/tmp/preelock_upstream_dns.txt";
const DNS_CONSENT_FILE: &str = "dns_proxy_consent.json";

/// Returns the path to the Unix domain socket for controlling the DNS proxy.
/// Namespaced to the current user UID on Unix so root and standard user sockets never conflict.
pub fn control_socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/preelock-dns-control-{uid}.sock"))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(DEFAULT_SOCKET_PATH)
    }
}

/// Returns the path to the user's DNS proxy consent file in the application data directory.
pub fn consent_file_path() -> PathBuf {
    if let Ok(dir) = crate::background::data_dir() {
        return dir.join(DNS_CONSENT_FILE);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join("Library/Application Support/com.preelock.desktop")
            .join(DNS_CONSENT_FILE);
    }
    PathBuf::from("/tmp").join(DNS_CONSENT_FILE)
}

/// Checks whether the user has given consent to redirect system DNS to 127.0.0.1.
/// Defaults to `Some(true)` so website rules are natively enforced via DNS automatically without annoying popups.
pub fn has_user_consent() -> Option<bool> {
    let path = consent_file_path();
    if !path.exists() {
        return Some(true);
    }
    let data = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    json.get("consent").and_then(|v| v.as_bool()).or(Some(true))
}

/// Records the user's consent choice to the persistent consent file.
pub fn set_user_consent(granted: bool) -> Result<(), String> {
    let path = consent_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::json!({
        "consent": granted,
        "updated_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });
    std::fs::write(&path, json.to_string())
        .map_err(|e| format!("Failed to write consent file: {e}"))?;

    if !granted {
        let _ = stop_session_blocking();
    }
    Ok(())
}

#[allow(dead_code)]
#[derive(Default)]
struct DnsProxyState {
    child_pid: Option<u32>,
    socket_path: String,
    active_service: Option<String>,
    is_active: bool,
    last_policy_domains: Vec<String>,
    last_is_allowlist: bool,
    last_block_adult: bool,
    last_start_attempt_ms: u64,
}

static STATE: Mutex<Option<DnsProxyState>> = Mutex::new(None);

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut DnsProxyState) -> R,
{
    let mut lock = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if lock.is_none() {
        *lock = Some(DnsProxyState {
            child_pid: None,
            socket_path: control_socket_path().to_string_lossy().to_string(),
            active_service: None,
            is_active: false,
            last_policy_domains: Vec::new(),
            last_is_allowlist: false,
            last_block_adult: false,
            last_start_attempt_ms: 0,
        });
    }
    f(lock.as_mut().unwrap())
}

/// Milliseconds since the Unix epoch, for retry throttling.
#[allow(dead_code)]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Normalizes domain rules: strips scheme, path, port, www-prefix, and trims dots.
pub fn normalize_domain(input: &str) -> String {
    let mut s = input.trim().to_ascii_lowercase();
    if let Some(idx) = s.find("://") {
        s = s[idx + 3..].to_string();
    }
    if let Some(idx) = s.find('/') {
        s = s[..idx].to_string();
    }
    if let Some(idx) = s.find(':') {
        s = s[..idx].to_string();
    }
    if let Some(stripped) = s.strip_prefix("www.") {
        s = stripped.to_string();
    }
    s.trim_matches('.').to_string()
}

/// Locates the `preelock-dns-proxy` sidecar binary on the filesystem.
pub fn find_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    // 1. The binary shipped inside the running .app. This MUST come
    //    before any workspace path: the installed app used to spawn the
    //    dev build every ~19 s, and a leftover `--help` probe process
    //    from that path hung forever (2026-11-24 diagnosis).
    if let Ok(exe) = std::env::current_exe() {
        let parent = exe.parent().unwrap_or(Path::new(""));
        candidates.push(parent.join("binaries").join("preelock-dns-proxy"));
        candidates.push(parent.join("../Resources/binaries/preelock-dns-proxy"));
        candidates.push(parent.join("preelock-dns-proxy"));
        candidates.push(parent.join("../Resources/dns-proxy/build/preelock-dns-proxy"));
    }

    // 2. Standard application bundle location
    candidates.push(PathBuf::from(
        "/Applications/Preelock.app/Contents/Resources/binaries/preelock-dns-proxy",
    ));

    // 3. Development paths, debug builds only (never for an installed app).
    #[cfg(debug_assertions)]
    {
        candidates.push(PathBuf::from(
            "src-tauri/dns-proxy/build/preelock-dns-proxy",
        ));
        candidates.push(PathBuf::from("dns-proxy/build/preelock-dns-proxy"));
        candidates.push(PathBuf::from(
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/build/preelock-dns-proxy",
        ));
    }
    candidates.push(PathBuf::from(
        "/Users/Prashant/My_Workspace/Preelock/src-tauri/target/release/bundle/macos/Preelock.app/Contents/Resources/binaries/preelock-dns-proxy",
    ));

    candidates.into_iter().find(|candidate| candidate.exists())
}

/// Builds a scoped sudoers NOPASSWD line for the given user and binary path.
pub fn build_sudoers_entry(user: &str, binary_path: &str) -> String {
    format!("{user} ALL=(ALL) NOPASSWD: {binary_path}\n")
}

/// Builds a scoped sudoers NOPASSWD line using the current system user (or `%admin`).
pub fn build_sudoers_entry_default(binary_path: &str) -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "%admin".to_string());
    build_sudoers_entry(&user, binary_path)
}

/// Checks whether one-time administrator setup is required to enforce native website blocking.
pub fn is_admin_setup_required() -> bool {
    #[cfg(target_os = "macos")]
    {
        // When the privileged helper is installed and operational,
        // no additional administrator setup is needed.
        if crate::privileged_helper::is_helper_installed() {
            return false;
        }
        let hosts_required = crate::macos_hosts::is_admin_setup_required();
        let binary_required = match find_binary() {
            Some(b) => {
                let status = std::process::Command::new("sudo")
                    .args(["-n", b.to_str().unwrap_or(""), "--help"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                !matches!(status, Ok(s) if s.success())
            }
            None => true,
        };
        hosts_required || binary_required
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_hosts::is_admin_setup_required()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

/// Executes the one-time administrator GUI setup for native OS-level website blocking.
///
/// On macOS: launches osascript with administrator privileges to install a scoped
/// `/etc/sudoers.d/preelock-dns` file with mode 0440, verified with `visudo -c`.
/// On Windows: triggers standard Windows UAC to grant write permission on the hosts file.
pub fn run_admin_setup() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let binary =
            find_binary().ok_or_else(|| "preelock-dns-proxy binary not found".to_string())?;
        let user = std::env::var("USER")
            .map_err(|e| format!("Could not read USER environment variable: {e}"))?;

        // Collect all binary candidates on this system
        let mut binary_paths = Vec::new();
        binary_paths.push(binary.clone());
        let standard_app = PathBuf::from(
            "/Applications/Preelock.app/Contents/Resources/binaries/preelock-dns-proxy",
        );
        if !binary_paths.contains(&standard_app) {
            binary_paths.push(standard_app);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let in_bundle = parent.join("../Resources/binaries/preelock-dns-proxy");
                if !binary_paths.contains(&in_bundle) {
                    binary_paths.push(in_bundle);
                }
            }
        }
        let dev_path = PathBuf::from(
            "/Users/Prashant/My_Workspace/Preelock/src-tauri/dns-proxy/build/preelock-dns-proxy",
        );
        if dev_path.exists() && !binary_paths.contains(&dev_path) {
            binary_paths.push(dev_path);
        }

        let mut lines = String::from("# Preelock scoped DNS proxy privilege (Phase 10)\n");
        let mut seen = std::collections::HashSet::new();
        for p in &binary_paths {
            let s = p.to_str().unwrap_or("");
            if !s.is_empty() && seen.insert(s.to_string()) {
                lines.push_str(&build_sudoers_entry(&user, s));
            }
            if let Ok(canon) = p.canonicalize() {
                let cs = canon.to_str().unwrap_or("");
                if !cs.is_empty() && seen.insert(cs.to_string()) {
                    lines.push_str(&build_sudoers_entry(&user, cs));
                }
            }
        }

        let tmp_path = "/tmp/preelock-dns-sudoers.tmp";
        std::fs::write(tmp_path, &lines)
            .map_err(|e| format!("Failed to write temporary sudoers file: {e}"))?;

        // Pre-validate syntax before prompting user
        let visudo_check = std::process::Command::new("/usr/sbin/visudo")
            .args(["-cf", tmp_path])
            .output();
        if let Ok(out) = visudo_check {
            if !out.status.success() {
                let _ = std::fs::remove_file(tmp_path);
                return Err(format!(
                    "Generated sudoers syntax is invalid: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        }

        let install_script = format!(
            "do shell script \"cp {tmp_path} /etc/sudoers.d/preelock-dns && chown root:wheel /etc/sudoers.d/preelock-dns && chmod 0440 /etc/sudoers.d/preelock-dns && chown root:admin /etc/hosts && chmod 664 /etc/hosts && /usr/sbin/visudo -c || (rm -f /etc/sudoers.d/preelock-dns && exit 1)\" with administrator privileges"
        );

        let osa_res = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &install_script])
            .output();

        let _ = std::fs::remove_file(tmp_path);

        let out = osa_res.map_err(|e| format!("Failed to launch osascript: {e}"))?;
        if !out.status.success() {
            let err_msg = String::from_utf8_lossy(&out.stderr);
            log::warn!("[DNS_PROXY] Administrator setup declined or failed: {err_msg}");
            return Err("Administrator setup declined or failed".to_string());
        }

        log::info!("[DNS_PROXY] Scoped sudoers entry installed successfully to /etc/sudoers.d/preelock-dns");
        let _ = crate::privileged_helper::run_admin_install_helper();
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        crate::windows_hosts::run_admin_setup()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(())
    }
}

/// Sends a one-line command to the local control socket and returns the response.
pub fn send_control_command(command: &str) -> Result<String, String> {
    #[cfg(unix)]
    {
        let socket_path = with_state(|s| s.socket_path.clone());
        let mut stream = UnixStream::connect(&socket_path)
            .map_err(|e| format!("Could not connect to DNS proxy socket at {socket_path}: {e}"))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;

        let msg = format!("{}\n", command.trim());
        stream
            .write_all(msg.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .map_err(|e| format!("Read failed: {e}"))?;

        String::from_utf8(buf).map_err(|e| format!("UTF-8 decode failed: {e}"))
    }

    #[cfg(not(unix))]
    {
        let _ = command;
        Err("DNS Proxy control socket is only supported on Unix platforms".into())
    }
}

/// A recently resolved domain from the DNS proxy, for the engine to know
/// which website the user is on without the browser extension and without
/// macOS Accessibility permission.
#[derive(Clone, Debug)]
pub struct RecentDomain {
    pub domain: String,
    /// Seconds since the Unix epoch when the query was answered.
    pub timestamp_secs: f64,
    pub blocked: bool,
}

/// Asks the DNS proxy for its most recently resolved domains (newest
/// last). Returns an empty list when the proxy is not running, the
/// command fails, or the response cannot be parsed — never an error, so
/// a missing proxy only pauses the timer instead of breaking the tick.
pub fn recent_domains() -> Vec<RecentDomain> {
    let response = match send_control_command("recent-domains") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&response) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(list) = parsed.get("domains").and_then(|d| d.as_array()) {
        for item in list {
            let domain = item
                .get("domain")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let timestamp_secs = item
                .get("timestamp")
                .and_then(|t| t.as_f64())
                .unwrap_or(0.0);
            let blocked = item
                .get("blocked")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            if !domain.is_empty() {
                out.push(RecentDomain {
                    domain,
                    timestamp_secs,
                    blocked,
                });
            }
        }
    }
    out
}

/// The most recently resolved website domain (if any) from the DNS proxy,
/// for the engine to know which site the user is on without the browser
/// extension and without Accessibility permission. `now_ms` is the
/// engine's current time in milliseconds. Only a domain resolved within
/// the last 10 seconds counts: older entries are from a previous page,
/// and a blank/new tab makes no website query at all, so there is nothing
/// recent and the timer pauses. Returns the domain string, or None.
/// Asset/CDN domains that never identify the site the user is on by
/// themselves: they serve many sites (fbcdn.net serves Facebook, Instagram
/// and WhatsApp; licdn.com is LinkedIn's CDN; googlevideo/ytimg/ggpht are
/// Google/YouTube assets). Skipped when picking the active domain, so a
/// background asset fetch cannot make the timer think the user left their
/// work site — and cannot poison the sticky domain with a CDN host that
/// matches no session rule.
pub(crate) const ASSET_ONLY_DOMAINS: &[&str] = &[
    "fbcdn.net",
    "licdn.com",
    "googlevideo.com",
    "ytimg.com",
    "ggpht.com",
    "cdninstagram.com",
    "whatsapp.net",
];

fn is_asset_only_domain(domain: &str) -> bool {
    ASSET_ONLY_DOMAINS
        .iter()
        .any(|base| domain == *base || domain.ends_with(&format!(".{base}")))
}

pub fn recent_active_domain(now_ms: i64) -> Option<String> {
    let now_secs = now_ms as f64 / 1000.0;
    // Newest first: the proxy returns entries oldest-first, so walk back.
    // Skip asset/CDN-only domains: they never identify the site itself.
    for entry in recent_domains().iter().rev() {
        if now_secs - entry.timestamp_secs <= 10.0 && !is_asset_only_domain(&entry.domain) {
            return Some(entry.domain.clone());
        }
    }
    None
}

/// Detects the active macOS network service (e.g. "Wi-Fi").
#[cfg(target_os = "macos")]
pub fn detect_active_network_service() -> String {
    // 1. Get default interface from route
    let route_out = std::process::Command::new("/sbin/route")
        .args(["-n", "get", "default"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut iface = "en0".to_string();
    for line in route_out.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("interface:") {
            if let Some(name) = trimmed.split(':').nth(1) {
                iface = name.trim().to_string();
                break;
            }
        }
    }

    // 2. Map interface to hardware port
    let hw_out = std::process::Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();

    let mut current_port = String::new();
    for line in hw_out.lines() {
        if line.starts_with("Hardware Port: ") {
            current_port = line
                .trim_start_matches("Hardware Port: ")
                .trim()
                .to_string();
        } else if line.starts_with("Device: ") {
            let dev = line.trim_start_matches("Device: ").trim();
            if dev == iface && !current_port.is_empty() {
                return current_port;
            }
        }
    }

    "Wi-Fi".to_string()
}

#[cfg(not(target_os = "macos"))]
pub fn detect_active_network_service() -> String {
    "Ethernet".to_string()
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn capture_current_upstream_servers() -> Vec<String> {
    let mut servers = Vec::new();
    if let Ok(resolv) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in resolv.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("nameserver ") {
                if let Some(ip) = trimmed.split_whitespace().nth(1) {
                    let ip = ip.trim();
                    if ip != "127.0.0.1"
                        && !ip.starts_with("127.")
                        && !servers.contains(&ip.to_string())
                    {
                        servers.push(ip.to_string());
                    }
                }
            }
        }
    }
    if servers.is_empty() {
        if let Ok(out) = std::process::Command::new("/usr/sbin/scutil")
            .arg("--dns")
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("nameserver[") {
                    if let Some(colon) = trimmed.find(':') {
                        let ip = trimmed[colon + 1..].trim();
                        if ip != "127.0.0.1"
                            && !ip.starts_with("127.")
                            && !servers.contains(&ip.to_string())
                        {
                            servers.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }
    if servers.is_empty() {
        servers.push("1.1.1.1".to_string());
        servers.push("8.8.8.8".to_string());
    }
    servers
}

/// Sends a real DNS query to 127.0.0.1:53 and confirms a valid response.
/// This proves a proxy is actually answering DNS on port 53 — not just
/// that a control socket exists (a stray proxy on another port also owns
/// that socket). Never apply the system DNS redirect unless this passes,
/// or the machine loses all DNS (2026-11-23 fix: DNS set to 127.0.0.1
/// while nothing listened on port 53, whole laptop went offline).
#[cfg(target_os = "macos")]
fn verify_dns_resolves() -> bool {
    use std::net::UdpSocket;
    use std::time::Duration;

    // Minimal DNS query for example.com, type A.
    let mut query = Vec::with_capacity(32);
    let id: u16 = 0x1234;
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, RD
    query.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    query.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    query.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    query.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
                                                  // QNAME: example.com
    query.push(7);
    query.extend_from_slice(b"example");
    query.push(3);
    query.extend_from_slice(b"com");
    query.push(0);
    query.extend_from_slice(&1u16.to_be_bytes()); // QTYPE A
    query.extend_from_slice(&1u16.to_be_bytes()); // QCLASS IN

    let socket = match UdpSocket::bind("127.0.0.1:0") {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
    if socket.send_to(&query, "127.0.0.1:53").is_err() {
        return false;
    }
    let mut buf = [0u8; 512];
    let (len, _) = match socket.recv_from(&mut buf) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if len < 12 {
        return false;
    }
    // Response ID must match, QR bit set, RCODE == 0.
    let resp_id = u16::from_be_bytes([buf[0], buf[1]]);
    if resp_id != id {
        return false;
    }
    let flags = u16::from_be_bytes([buf[2], buf[3]]);
    if flags & 0x8000 == 0 {
        return false; // not a response
    }
    if flags & 0x000f != 0 {
        return false; // error (e.g. NXDOMAIN / SERVFAIL)
    }
    true
}

/// RULE 1: Never redirect macOS system DNS.
/// Preelock enforces unbypassable domain blocking exclusively via /etc/hosts and PF firewall.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn apply_dns_redirect(_service: &str) -> Result<(), String> {
    log::info!(
        "[DNS_PROXY] System DNS redirection permanently disabled (RULE 1: never touch system DNS)"
    );
    Ok(())
}

/// Restores original macOS system DNS on the active network service.
#[cfg(target_os = "macos")]
fn restore_system_dns(service: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = service;
        Ok(())
    }

    #[cfg(not(test))]
    {
        let restore_args: Vec<String> = if Path::new(ORIGINAL_DNS_PATH).exists() {
            let saved = std::fs::read_to_string(ORIGINAL_DNS_PATH).unwrap_or_default();
            let _ = std::fs::remove_file(ORIGINAL_DNS_PATH);

            if saved.contains("There aren't any DNS Servers set") || saved.trim().is_empty() {
                vec!["Empty".to_string()]
            } else {
                let filtered: Vec<String> = saved
                    .split_whitespace()
                    .filter(|s| !s.starts_with("127.") && !s.contains("localhost"))
                    .map(|s| s.to_string())
                    .collect();
                if filtered.is_empty() {
                    vec!["Empty".to_string()]
                } else {
                    filtered
                }
            }
        } else {
            vec!["Empty".to_string()]
        };

        let _ = std::fs::remove_file(UPSTREAM_DNS_PATH);

        let mut cmd = std::process::Command::new("/usr/sbin/networksetup");
        cmd.args(["-setdnsservers", service]);
        for arg in &restore_args {
            cmd.arg(arg);
        }

        let _ = cmd.status();

        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();

        Ok(())
    }
}

/// Well-known canary and Apple Private Relay domains that force Safari, Firefox,
/// and Chromium browsers to immediately disable DoH/Private Relay and respect OS DNS.
pub const CANARY_AND_RELAY_DOMAINS: &[&str] = &[
    "mask.icloud.com",
    "mask-h2.icloud.com",
    "mask-api.icloud.com",
    "mask.apple-dns.net",
    "mask-tfo.apple-dns.net",
    "mask-api.fe.apple-dns.net",
    "mask-api.fe2.apple-dns.net",
    "use-application-dns.net",
];

/// Writes the policy JSON file to disk.
pub fn write_policy_file(
    domains: &[String],
    is_allowlist: bool,
    block_adult: bool,
) -> Result<(), String> {
    let mode = if is_allowlist {
        "allowlist"
    } else {
        "blocklist"
    };

    let mut effective_domains = domains.to_vec();
    if !is_allowlist {
        for canary in CANARY_AND_RELAY_DOMAINS {
            let s = canary.to_string();
            if !effective_domains.contains(&s) {
                effective_domains.push(s);
            }
        }
    }

    let policy_json = serde_json::json!({
        "version": 2,
        "name": "Preelock Session Policy",
        "mode": mode,
        "domains": effective_domains,
        "block_doh": true,
        "block_adult": block_adult
    });

    let json_bytes =
        serde_json::to_vec_pretty(&policy_json).map_err(|e| format!("Serialization error: {e}"))?;

    std::fs::write(DEFAULT_POLICY_PATH, json_bytes)
        .map_err(|e| format!("Failed to write {DEFAULT_POLICY_PATH}: {e}"))?;

    Ok(())
}

/// Generates and pushes an active policy file to the running proxy.
pub fn sync_policy(
    domains: &[String],
    is_allowlist: bool,
    block_adult: bool,
) -> Result<(), String> {
    write_policy_file(domains, is_allowlist, block_adult)?;

    let cmd = format!("set-policy {DEFAULT_POLICY_PATH}");
    let _ = send_control_command(&cmd);

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();
    }

    with_state(|s| {
        s.last_policy_domains = domains.to_vec();
        s.last_is_allowlist = is_allowlist;
        s.last_block_adult = block_adult;
    });

    Ok(())
}

#[allow(dead_code)]
static WATCHDOG_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Watchdog thread that monitors proxy health and immediately restores system DNS if the proxy dies.
#[allow(dead_code)]
fn ensure_watchdog_started() {
    #[cfg(target_os = "macos")]
    {
        if WATCHDOG_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(1500));
            let (is_active, active_service) =
                with_state(|s| (s.is_active, s.active_service.clone()));
            if is_active && send_control_command("status").is_err() {
                log::error!("[DNS_PROXY_WATCHDOG] DNS proxy sidecar is unresponsive! Restoring system DNS immediately.");
                let service = active_service.unwrap_or_else(detect_active_network_service);
                let _ = restore_system_dns(&service);
                with_state(|s| {
                    s.is_active = false;
                });
            }
        });
    }
}

/// Startup safety net: checks all network services on the machine.
/// If any service has DNS pointing to 127.0.0.1 or localhost, resets it to Empty immediately.
/// Also removes stale sockets, policy files, and flushes the macOS DNS cache.
pub fn cleanup_stale_redirect() {
    #[cfg(target_os = "macos")]
    {
        let _ = crate::privileged_helper::clear_block("engine");
        let _ = crate::privileged_helper::clear_block("session");
        let _ = crate::macos_hosts::clear_blocks();

        // 1. Query all network services
        let services_out = std::process::Command::new("/usr/sbin/networksetup")
            .arg("-listallnetworkservices")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        for line in services_out.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("An asterisk") {
                continue;
            }
            let service = trimmed.trim_start_matches('*').trim();
            if service.is_empty() {
                continue;
            }

            let current_dns = std::process::Command::new("/usr/sbin/networksetup")
                .args(["-getdnsservers", service])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();

            if current_dns.contains("127.0.0.1") || current_dns.to_lowercase().contains("localhost")
            {
                log::warn!(
                    "[SAFETY_NET] Stale 127.0.0.1 DNS redirect detected on service '{}'. Restoring system DNS to Empty.",
                    service
                );
                let _ = std::process::Command::new("/usr/sbin/networksetup")
                    .args(["-setdnsservers", service, "Empty"])
                    .status();
            }
        }

        // 2. Remove stale control sockets, policy files, and DNS capture files
        let _ = std::fs::remove_file(DEFAULT_POLICY_PATH);
        let _ = std::fs::remove_file(control_socket_path());
        let _ = std::fs::remove_file(DEFAULT_SOCKET_PATH);
        let _ = std::fs::remove_file(ORIGINAL_DNS_PATH);
        let _ = std::fs::remove_file(UPSTREAM_DNS_PATH);

        if let Ok(entries) = std::fs::read_dir("/tmp") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("preelock-dns-control") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }

        // 3. Flush macOS DNS cache
        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();
        let _ = std::process::Command::new("/usr/bin/killall")
            .args(["-HUP", "mDNSResponder"])
            .status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = crate::windows_hosts::clear_blocks();
        let _ = crate::windows_firewall::clear_firewall_rules();
    }
}

/// Set to true once the one-time native admin dialog has been offered in this
/// app run, so a cancelled setup is not re-offered on every engine tick.
/// Only used on Windows (the macOS engine never shows the admin dialog).
#[cfg(target_os = "windows")]
static GUI_SETUP_ATTEMPTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Spawns the proxy via passwordless sudo and waits up to ~2s for the
/// control socket to answer. Returns the child pid when the proxy is
/// responsive, or None (killing the child) when it never answers.
#[cfg(target_os = "macos")]
fn spawn_proxy_via_sudo(binary: &Path) -> Option<u32> {
    let sock_path = control_socket_path();
    let sock_str = sock_path.to_str().unwrap_or(DEFAULT_SOCKET_PATH);
    let mut child = std::process::Command::new("sudo")
        .args([
            "-n",
            binary.to_str().unwrap_or(""),
            "--port",
            "53",
            "--policy",
            DEFAULT_POLICY_PATH,
            "--socket",
            sock_str,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(80));
        if send_control_command("status").is_ok() {
            // The control socket answering is not enough: the proxy
            // must also be listening on port 53, or the DNS redirect
            // would point at a dead port and no site would be blocked.
            // Poll for a real DNS answer before reporting success.
            for _ in 0..10 {
                if verify_dns_resolves() {
                    return Some(child.id());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            log::warn!("[DNS_PROXY] Proxy answered the control socket but never served DNS on port 53; killing it");
            let _ = child.kill();
            return None;
        }
    }
    let _ = child.kill();
    None
}

/// Starts the DNS proxy and applies system DNS redirection for an active website-blocking session.
pub fn start_for_session(
    domains: &[String],
    is_allowlist: bool,
    block_adult: bool,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // System DNS redirection can be disabled by the user in Settings.
        if has_user_consent() == Some(false) {
            log::info!("[DNS_PROXY] Skipping native DNS proxy: user has disabled DNS redirection in Settings");
            return Err("User has disabled system DNS redirection".into());
        }
        if has_user_consent().is_none() {
            let _ = set_user_consent(true);
        }

        let service = detect_active_network_service();

        // 1. Prepare policy file before spawning so sidecar starts with valid policy
        let _ = write_policy_file(domains, is_allowlist, block_adult);

        // 2. Check if sidecar is already answering ON PORT 53.
        let sock_path = control_socket_path();
        let sock_str = sock_path.to_str().unwrap_or(DEFAULT_SOCKET_PATH);
        with_state(|s| {
            s.socket_path = sock_str.to_string();
        });

        let is_running = send_control_command("status").is_ok() && verify_dns_resolves();
        let mut ready = is_running;
        let mut child_pid = None;

        if !is_running {
            // Drop a stale control socket so the fresh proxy can bind it cleanly.
            let _ = std::fs::remove_file(&sock_path);
            if let Some(binary) = find_binary() {
                log::info!("[DNS_PROXY] Spawning DNS proxy sidecar from {:?}", binary);

                // Attempt direct spawn on port 53
                let spawned_child = std::process::Command::new(&binary)
                    .args([
                        "--port",
                        "53",
                        "--policy",
                        DEFAULT_POLICY_PATH,
                        "--socket",
                        sock_str,
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                if spawned_child.is_ok() {
                    for _ in 0..25 {
                        std::thread::sleep(Duration::from_millis(60));
                        if send_control_command("status").is_ok() && verify_dns_resolves() {
                            ready = true;
                            child_pid = spawned_child.as_ref().ok().map(|c| c.id());
                            break;
                        }
                    }
                }

                // If direct spawn failed to respond, try non-interactive sudo (sudo -n)
                if !ready {
                    if let Ok(mut c) = spawned_child {
                        let _ = c.kill();
                    }
                    child_pid = spawn_proxy_via_sudo(&binary);
                    if child_pid.is_some() {
                        ready = true;
                    }
                }
            }

            if !ready {
                // The proxy could not be started, so there is no DNS
                // enforcement. In blocklist mode the hosts file still
                // blocks (applied in step 1); in allowlist mode the
                // hosts file was cleared, so nothing blocks until the
                // proxy starts. Do NOT mark the session active: leaving
                // is_active false makes the engine retry starting the
                // proxy on a later tick (throttled in
                // handle_policy_update) instead of silently running a
                // session with no blocking.
                if is_allowlist {
                    log::warn!("[DNS_PROXY] Port 53 DNS proxy sidecar unavailable; allowlist session has NO website blocking until the proxy starts");
                } else {
                    log::warn!("[DNS_PROXY] Port 53 DNS proxy sidecar unavailable; OS hosts blocking is active");
                }
                with_state(|s| {
                    s.last_start_attempt_ms = now_ms();
                    s.active_service = Some(service.clone());
                });
                return Ok(());
            }

            with_state(|s| {
                s.child_pid = child_pid;
                s.active_service = Some(service.clone());
            });
        }

        // 4. Synchronize policy with running proxy
        let _ = sync_policy(domains, is_allowlist, block_adult);

        // 5. RULE 1: Never redirect system DNS. Preelock exclusively relies
        // on SelfControl-grade /etc/hosts + PF firewall via privileged helper.
        log::info!("[DNS_PROXY] System DNS redirect skipped (RULE 1: system DNS untouched)");

        with_state(|s| {
            s.is_active = true;
            s.active_service = Some(service.clone());
            s.last_policy_domains = domains.to_vec();
            s.last_is_allowlist = is_allowlist;
            s.last_block_adult = block_adult;
        });

        // 6. Ensure watchdog monitor is active
        ensure_watchdog_started();

        log::info!(
            "[DNS_PROXY] Active session DNS blocking enabled on {}",
            service
        );
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        if has_user_consent() == Some(false) {
            log::info!("[WINDOWS_HOSTS] Skipping native blocking: user has disabled DNS/hosts redirection in Settings");
            return Err("User has disabled system website blocking".into());
        }
        if has_user_consent().is_none() {
            let _ = set_user_consent(true);
        }

        if is_allowlist {
            log::info!("[WINDOWS_HOSTS] Allowlist mode is active; hosts-file blocking cannot invert the full web, deferring to browser extension");
            let _ = crate::windows_hosts::clear_blocks();
            return Ok(());
        }

        if crate::windows_hosts::is_admin_setup_required()
            && !GUI_SETUP_ATTEMPTED.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let _ = crate::windows_hosts::run_admin_setup();
        }

        crate::windows_hosts::apply_blocks(domains)?;
        let _ = crate::windows_firewall::apply_firewall_rules(domains);

        with_state(|s| {
            s.is_active = true;
            s.last_policy_domains = domains.to_vec();
            s.last_is_allowlist = is_allowlist;
            s.last_block_adult = block_adult;
        });

        log::info!(
            "[WINDOWS_HOSTS] Native hosts-file blocking enabled for {} domains",
            domains.len()
        );
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (domains, is_allowlist, block_adult);
        Ok(())
    }
}

/// Stops the DNS proxy and restores original system DNS.
pub fn stop_session_blocking() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = crate::privileged_helper::clear_block("engine");
        let _ = crate::macos_hosts::clear_blocks();
        let service =
            with_state(|s| s.active_service.clone()).unwrap_or_else(detect_active_network_service);

        // 1. Restore system DNS first
        let _ = restore_system_dns(&service);

        // 2. Request clean shutdown via control socket (works for both user and sudo-spawned proxy)
        let _ = send_control_command("stop");

        // 3. Give proxy up to 500ms to exit cleanly
        let mut stopped = false;
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(50));
            if send_control_command("status").is_err() {
                stopped = true;
                break;
            }
        }

        // 4. Fallback termination if still alive
        if !stopped {
            let pid_opt = with_state(|s| s.child_pid.take());
            if let Some(pid) = pid_opt {
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            if let Some(bin) = find_binary() {
                let _ = std::process::Command::new("sudo")
                    .args(["-n", bin.to_str().unwrap_or(""), "--kill"])
                    .status();
            }
        }

        // Clean up temporary policy and socket files
        let _ = std::fs::remove_file(DEFAULT_POLICY_PATH);
        let _ = std::fs::remove_file(control_socket_path());
        let _ = std::fs::remove_file(DEFAULT_SOCKET_PATH);

        with_state(|s| {
            s.is_active = false;
            s.last_policy_domains.clear();
            s.last_is_allowlist = false;
            s.last_block_adult = false;
        });

        log::info!(
            "[DNS_PROXY] Session DNS blocking stopped; normal DNS restored on {}",
            service
        );
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // Ending a session must restore the whole machine, not just the hosts
        // file. The outbound netsh rules for this session's resolved IPs would
        // otherwise keep those sites blocked until the next session applies
        // new rules or the app exits.
        let hosts_result = crate::windows_hosts::clear_blocks();
        let firewall_result = crate::windows_firewall::clear_firewall_rules();
        with_state(|s| {
            s.is_active = false;
            s.last_policy_domains.clear();
            s.last_is_allowlist = false;
            s.last_block_adult = false;
        });
        hosts_result?;
        firewall_result?;
        log::info!(
            "[WINDOWS_HOSTS] Native hosts-file blocking stopped; normal hosts and firewall restored"
        );
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(())
    }
}

/// Central policy update handler called by the engine during session lifecycle transitions.
///
/// If `Some((domains, is_allowlist, block_adult))` is given, starts or updates the DNS proxy with the given rules.
/// If `None` is given, stops DNS proxy blocking and restores default DNS.
pub fn handle_policy_update(policy: Option<(&[String], bool, bool)>) -> Result<(), String> {
    match policy {
        Some(_) => {
            // RULE 1: the sidecar can only enforce anything by pointing
            // system DNS at 127.0.0.1, which is permanently forbidden.
            // Starting it therefore did nothing except spawn a process
            // every ~19 s that failed its port-53 probe (and its Swift
            // NetworkWatcher rewrote Wi-Fi DNS back to 127.0.0.1 every
            // 3 s). Sessions are enforced by /etc/hosts + PF + the
            // extension, so there is nothing to start here.
            Ok(())
        }
        None => {
            let is_already_active = with_state(|s| s.is_active);
            if is_already_active {
                stop_session_blocking()
            } else {
                Ok(())
            }
        }
    }
}

/// Queries current status of the DNS proxy for UI / diagnostics.
pub fn get_status() -> serde_json::Value {
    let running = send_control_command("status").is_ok();
    let service = detect_active_network_service();
    let active = with_state(|s| s.is_active);
    let rules_count = with_state(|s| s.last_policy_domains.len());
    let admin_required = is_admin_setup_required();

    serde_json::json!({
        "supported": cfg!(any(target_os = "macos", target_os = "windows")),
        "running": running,
        "session_active": active,
        "active_service": service,
        "rule_count": rules_count,
        "admin_setup_required": admin_required
    })
}

/// Cleanup hook called at application shutdown.
pub fn cleanup_at_exit() {
    #[cfg(target_os = "macos")]
    {
        let _ = crate::privileged_helper::clear_block("engine");
        let _ = crate::privileged_helper::clear_block("session");
        let _ = crate::macos_hosts::clear_blocks();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = crate::windows_hosts::clear_blocks();
        let _ = crate::windows_firewall::clear_firewall_rules();
    }
    let _ = stop_session_blocking();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_domain() {
        assert_eq!(
            normalize_domain("https://instagram.com/p/123"),
            "instagram.com"
        );
        assert_eq!(normalize_domain("http://www.facebook.com"), "facebook.com");
        assert_eq!(normalize_domain("reddit.com:8080"), "reddit.com");
        assert_eq!(normalize_domain("  .tiktok.com.  "), "tiktok.com");
        assert_eq!(
            normalize_domain("sub.example.co.uk/path?q=1"),
            "sub.example.co.uk"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_find_binary_or_fallback() {
        let binary = find_binary();
        assert!(
            binary.is_some(),
            "Binary should be found in dev or build tree"
        );
    }

    #[test]
    fn test_get_status_structure() {
        let status = get_status();
        assert!(status.get("supported").is_some());
        assert!(status.get("running").is_some());
        assert!(status.get("session_active").is_some());
        assert!(status.get("active_service").is_some());
        assert!(status.get("rule_count").is_some());
    }

    #[test]
    fn test_policy_v2_json_generation() {
        let domains = vec!["instagram.com".to_string(), "reddit.com".to_string()];
        let json = serde_json::json!({
            "version": 2,
            "name": "Preelock Session Policy",
            "mode": "blocklist",
            "domains": domains,
            "block_doh": true,
            "block_adult": true
        });

        assert_eq!(json["version"], 2);
        assert_eq!(json["mode"], "blocklist");
        assert_eq!(json["domains"].as_array().unwrap().len(), 2);
        assert_eq!(json["block_doh"], true);
        assert_eq!(json["block_adult"], true);
    }

    #[test]
    fn test_build_sudoers_entry_scoped_line() {
        let user = "testuser";
        let path = "/Applications/Preelock.app/Contents/Resources/binaries/preelock-dns-proxy";
        let entry = build_sudoers_entry(user, path);
        assert_eq!(
            entry,
            "testuser ALL=(ALL) NOPASSWD: /Applications/Preelock.app/Contents/Resources/binaries/preelock-dns-proxy\n"
        );
        assert!(!entry.contains('*'));
        assert!(!entry.contains("ALL:ALL"));
        assert!(!entry.contains("NOPASSWD: ALL"));
    }

    #[cfg(unix)]
    #[test]
    fn test_write_policy_file_injects_canary_domains() {
        let domains = vec!["instagram.com".to_string()];
        let res = write_policy_file(&domains, false, false);
        assert!(res.is_ok());

        let content = std::fs::read_to_string(DEFAULT_POLICY_PATH).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let doms = parsed["domains"].as_array().unwrap();
        let doms_str: Vec<String> = doms
            .iter()
            .map(|d| d.as_str().unwrap().to_string())
            .collect();

        assert!(doms_str.contains(&"instagram.com".to_string()));
        assert!(doms_str.contains(&"mask.icloud.com".to_string()));
        assert!(doms_str.contains(&"mask-h2.icloud.com".to_string()));
        assert!(doms_str.contains(&"use-application-dns.net".to_string()));
    }
}
