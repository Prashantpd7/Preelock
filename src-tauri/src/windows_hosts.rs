//! Preelock Native Hosts-File Website Blocking for Windows (Phase 10).
//!
//! Provides system-wide, OS-level domain blocking on Windows by managing
//! scoped entries in `C:\Windows\System32\drivers\etc\hosts`.
//!
//! ## Security & Durability Design
//! - Uses standard Windows hosts file resolution: `127.0.0.1 <domain>` and `::1 <domain>`.
//! - One-time setup: uses standard Windows UAC elevation (`Start-Process icacls ... -Verb RunAs`)
//!   to grant Modify permission on `hosts` to the `Users` group. After this one-time approval,
//!   sessions apply and remove blocks without prompting on every block change.
//! - Delimited: All Preelock entries are bounded by `# === PREELOCK BLOCKLIST START ===` and
//!   `# === PREELOCK BLOCKLIST END ===`. Outside content is never touched or modified.
//! - Backed up: Creates `hosts.preelock.bak` before the first modification.
//! - Reversible: Cleans up on session end, session pause, and app exit.
//! - Non-disruptive: Only listed domains are blocked; all other internet traffic flows untouched.
//!
//! ## Sharing & Retry (Windows `os error 32`)
//! `C:\Windows\System32\drivers\etc\hosts` is a shared system file. The DNS
//! client, Windows Defender's real-time scanner and other filter drivers open
//! it with their own share modes. A scanner that grabs the file immediately
//! after the UAC elevation changes its ACL holds it without sharing write, so
//! a one-shot `OpenOptions` open can fail with `ERROR_SHARING_VIOLATION`
//! (os error 32) even though the user *does* have Modify permission. That is a
//! transient lock, not a permission problem, and must never be reported as a
//! declined UAC setup.
//!
//! Every hosts-file open/write here therefore:
//! 1. requests `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`, so
//!    Preelock never blocks the scanner/reader holding the file, and
//! 2. retries `ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION` with a bounded
//!    backoff (~1s total) before giving up, and
//! 3. classifies failures: only `ERROR_ACCESS_DENIED` means "UAC setup is
//!    required"; a lock that survives the retries is treated as "permission is
//!    fine, retry later" so the Enable System Blocking modal cannot loop.

#[cfg(target_os = "windows")]
use std::io::{Read, Write};
#[cfg(target_os = "windows")]
use std::path::Path;
#[cfg(any(target_os = "windows", test))]
use std::time::Duration;

#[cfg(target_os = "windows")]
use std::os::windows::fs::OpenOptionsExt;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const DEFAULT_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub const BACKUP_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts.preelock.bak";
pub const BLOCK_START_MARKER: &str = "# === PREELOCK BLOCKLIST START ===";
pub const BLOCK_END_MARKER: &str = "# === PREELOCK BLOCKLIST END ===";

/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
///
/// Matches what Rust's `std` uses by default and, more importantly, keeps
/// Defender/Dnscache handles on `hosts` from turning our open into
/// `ERROR_SHARING_VIOLATION`.
#[cfg(target_os = "windows")]
const FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;

/// Windows `ERROR_ACCESS_DENIED`: the real "needs UAC" signal.
#[cfg(any(target_os = "windows", test))]
const ERROR_ACCESS_DENIED: i32 = 5;
/// Windows `ERROR_SHARING_VIOLATION`: another process holds `hosts` without sharing.
#[cfg(any(target_os = "windows", test))]
const ERROR_SHARING_VIOLATION: i32 = 32;
/// Windows `ERROR_LOCK_VIOLATION`: a byte-range lock is held on `hosts`.
#[cfg(any(target_os = "windows", test))]
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Bounded retry budget for transient hosts-file locks (~1.1s in total).
#[cfg(target_os = "windows")]
const HOSTS_OPEN_ATTEMPTS: u32 = 8;
#[cfg(target_os = "windows")]
const HOSTS_RETRY_BASE_DELAY: Duration = Duration::from_millis(25);
#[cfg(target_os = "windows")]
const HOSTS_RETRY_MAX_DELAY: Duration = Duration::from_millis(250);

/// Result of probing whether this process can write the Windows hosts file.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostsWriteAccess {
    /// The file can be opened and written right now.
    Writable,
    /// The ACL denies writes: the one-time UAC setup is genuinely required.
    PermissionDenied,
    /// Another process holds the file without write sharing (scanner, DNS client).
    /// Permission is fine; the caller should retry rather than ask for UAC again.
    TransientLock,
}

/// Maps a raw Win32 error code from a hosts-file operation to an actionable state.
///
/// Only `ERROR_ACCESS_DENIED` asks the user for elevation. Sharing/lock
/// violations and unknown errors are treated as transient, because sending the
/// user to a UAC prompt cannot fix a handle held by another process — doing so
/// is what produced the endless "Enable System Blocking" modal.
#[cfg(any(target_os = "windows", test))]
pub fn classify_hosts_error(raw: Option<i32>) -> HostsWriteAccess {
    match raw {
        Some(ERROR_ACCESS_DENIED) => HostsWriteAccess::PermissionDenied,
        Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION) => {
            HostsWriteAccess::TransientLock
        }
        _ => HostsWriteAccess::TransientLock,
    }
}

/// Whether an error is a transient handle conflict worth retrying.
#[cfg(any(target_os = "windows", test))]
fn is_transient_lock(raw: Option<i32>) -> bool {
    matches!(
        raw,
        Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION)
    )
}

/// Runs `op`, retrying only transient sharing/lock violations with bounded backoff.
///
/// Permission errors (`ERROR_ACCESS_DENIED`) are returned immediately: retrying
/// them wastes time and would mask the need for UAC.
#[cfg(any(target_os = "windows", test))]
pub fn retry_transient<T>(
    attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
    mut op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let attempts = attempts.max(1);
    let mut delay = base_delay;
    let mut last: Option<std::io::Error> = None;

    for attempt in 0..attempts {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) => {
                let transient = is_transient_lock(err.raw_os_error());
                log::warn!(
                    "[WINDOWS_HOSTS] hosts file operation attempt {}/{} failed: {err} (transient={transient})",
                    attempt + 1,
                    attempts
                );
                last = Some(err);
                if !transient || attempt + 1 == attempts {
                    break;
                }
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
                delay = (delay * 2).min(max_delay);
            }
        }
    }

    Err(last.unwrap_or_else(|| std::io::Error::other("hosts file retry made no attempt")))
}

/// Opens the hosts file for append with every share flag set.
#[cfg(target_os = "windows")]
fn open_hosts_append(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .append(true)
        .share_mode(FILE_SHARE_ALL)
        .open(path)
}

/// Opens the hosts file for reading with every share flag set.
#[cfg(target_os = "windows")]
fn open_hosts_read(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_ALL)
        .open(path)
}

/// Reads a file with transient-lock retries.
#[cfg(target_os = "windows")]
fn read_file_with_retry(path: &Path) -> std::io::Result<String> {
    retry_transient(
        HOSTS_OPEN_ATTEMPTS,
        HOSTS_RETRY_BASE_DELAY,
        HOSTS_RETRY_MAX_DELAY,
        || {
            let mut content = String::new();
            open_hosts_read(path)?.read_to_string(&mut content)?;
            Ok(content)
        },
    )
}

/// Truncates and rewrites a file in place with transient-lock retries.
///
/// In-place rewriting (never create/rename) keeps the file's identity, ACL and
/// hard links intact in `System32\drivers\etc`, where normal users may modify
/// `hosts` but cannot create new files.
#[cfg(target_os = "windows")]
fn write_file_with_retry(path: &Path, content: &str) -> std::io::Result<()> {
    retry_transient(
        HOSTS_OPEN_ATTEMPTS,
        HOSTS_RETRY_BASE_DELAY,
        HOSTS_RETRY_MAX_DELAY,
        || {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .create(true)
                .share_mode(FILE_SHARE_ALL)
                .open(path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            file.sync_all()
        },
    )
}

/// Probes whether the current (non-elevated) process can write the hosts file.
///
/// Returns [`HostsWriteAccess::TransientLock`] if the probe never gets past a
/// foreign handle, and [`HostsWriteAccess::PermissionDenied`] only when Windows
/// reports an actual access denial.
#[cfg(target_os = "windows")]
pub fn probe_hosts_write_access() -> HostsWriteAccess {
    let path = Path::new(DEFAULT_HOSTS_PATH);
    if !path.exists() {
        return HostsWriteAccess::PermissionDenied;
    }
    let result = retry_transient(
        HOSTS_OPEN_ATTEMPTS,
        HOSTS_RETRY_BASE_DELAY,
        HOSTS_RETRY_MAX_DELAY,
        || open_hosts_append(path).map(drop),
    );
    match result {
        Ok(()) => HostsWriteAccess::Writable,
        Err(err) => {
            let access = classify_hosts_error(err.raw_os_error());
            log::warn!(
                "[WINDOWS_HOSTS] hosts file write probe: {err} -> {access:?} (raw={:?})",
                err.raw_os_error()
            );
            access
        }
    }
}

/// Formats the Preelock block section for the Windows hosts file.
///
/// Maps each domain (and its `www.` variant if not already prefixed) to both
/// IPv4 loopback (`127.0.0.1`) and IPv6 loopback (`::1`).
pub fn generate_hosts_block_section(domains: &[String]) -> String {
    let mut lines = Vec::new();
    lines.push(BLOCK_START_MARKER.to_string());
    lines.push("# Managed by Preelock. Do not edit this section manually.".to_string());

    let mut sorted = domains.to_vec();
    sorted.sort();
    sorted.dedup();

    for domain in sorted {
        let norm = domain.trim().to_ascii_lowercase();
        if norm.is_empty() || norm.contains(' ') || norm.starts_with('#') {
            continue;
        }
        lines.push(format!("127.0.0.1 {norm}"));
        lines.push(format!("::1 {norm}"));

        if !norm.starts_with("www.") && !norm.contains('/') {
            lines.push(format!("127.0.0.1 www.{norm}"));
            lines.push(format!("::1 www.{norm}"));
        }
    }

    lines.push(BLOCK_END_MARKER.to_string());
    lines.join("\r\n") + "\r\n"
}

/// Removes any existing Preelock block section from hosts file content.
pub fn strip_hosts_rules_from_content(content: &str) -> String {
    let mut result = Vec::new();
    let mut in_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_START_MARKER {
            in_block = true;
            continue;
        }
        if trimmed == BLOCK_END_MARKER {
            in_block = false;
            continue;
        }
        if !in_block {
            result.push(line);
        }
    }

    let mut out = result.join("\r\n");
    if !out.is_empty() && (content.ends_with("\r\n") || content.ends_with('\n')) {
        out.push_str("\r\n");
    }
    out
}

/// Injects the Preelock block section into existing hosts file content,
/// replacing any previous block section while preserving all external entries.
pub fn apply_hosts_rules_to_content(original_content: &str, domains: &[String]) -> String {
    let stripped = strip_hosts_rules_from_content(original_content);
    if domains.is_empty() {
        return stripped;
    }
    let block_section = generate_hosts_block_section(domains);
    if stripped.ends_with("\r\n") || stripped.ends_with('\n') {
        format!("{stripped}{block_section}")
    } else if stripped.is_empty() {
        block_section
    } else {
        format!("{stripped}\r\n{block_section}")
    }
}

/// Checks whether one-time UAC elevation is still required to write the hosts file.
///
/// A hosts file that is momentarily held by a scanner is *not* a reason to ask
/// for UAC again: only a real access denial reports `true`. This keeps the
/// Enable System Blocking modal from reappearing after the user already granted
/// permission.
pub fn is_admin_setup_required() -> bool {
    #[cfg(target_os = "windows")]
    {
        probe_hosts_write_access() == HostsWriteAccess::PermissionDenied
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Executes the one-time Windows UAC administrator elevation to grant Modify
/// permission on the hosts file to `BUILTIN\Users` and `Authenticated Users`.
pub fn run_admin_setup() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // Idempotent: never re-prompt UAC when the hosts file is already writable.
        match probe_hosts_write_access() {
            HostsWriteAccess::Writable => {
                log::info!("[WINDOWS_HOSTS] hosts file already writable; UAC setup skipped");
                return Ok(());
            }
            HostsWriteAccess::TransientLock => {
                log::info!(
                    "[WINDOWS_HOSTS] hosts file briefly locked by another process; UAC setup skipped"
                );
                return Ok(());
            }
            HostsWriteAccess::PermissionDenied => {}
        }

        log::info!(
            "[WINDOWS_HOSTS] Requesting one-time UAC elevation to grant hosts file write permissions"
        );

        let temp_dir = std::env::temp_dir();
        let ps1_path = temp_dir.join(format!("preelock_hosts_setup_{}.ps1", std::process::id()));
        let ps1_script = format!(
            "$ErrorActionPreference = 'Stop'\r\n\
             $hosts = '{hosts}'\r\n\
             $etcDir = Split-Path -Parent $hosts\r\n\
             if (-not (Test-Path $etcDir)) {{ New-Item -ItemType Directory -Path $etcDir -Force | Out-Null }}\r\n\
             if (-not (Test-Path $hosts)) {{ New-Item -ItemType File -Path $hosts -Force | Out-Null }}\r\n\
             & attrib -R -S -H $hosts 2>$null\r\n\
             $granted = $false\r\n\
             for ($i = 0; $i -lt 4; $i++) {{\r\n\
                 & icacls $hosts /grant '*S-1-5-32-545:(M)' '*S-1-5-11:(M)' | Out-Null\r\n\
                 if ($LASTEXITCODE -eq 0) {{ $granted = $true; break }}\r\n\
                 Start-Sleep -Milliseconds 250\r\n\
             }}\r\n\
             if (-not $granted) {{\r\n\
                 & takeown /F $hosts /A 2>$null | Out-Null\r\n\
                 & icacls $hosts /grant '*S-1-5-32-545:(M)' '*S-1-5-11:(M)' | Out-Null\r\n\
             }}\r\n\
             exit $LASTEXITCODE\r\n",
            hosts = DEFAULT_HOSTS_PATH.replace('\'', "''"),
        );

        std::fs::write(&ps1_path, &ps1_script)
            .map_err(|e| format!("Failed to write temporary UAC setup script: {e}"))?;

        let ps1_str = ps1_path.to_string_lossy().replace('\'', "''");
        let launcher = format!(
            "$p = Start-Process powershell.exe -ArgumentList @('-NoProfile','-NonInteractive','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File','{ps1_str}') -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode"
        );

        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &launcher,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        let _ = std::fs::remove_file(&ps1_path);

        let output = output.map_err(|e| format!("Failed to launch elevated UAC setup: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!(
                "[WINDOWS_HOSTS] Elevated UAC setup exited with {:?}: {}",
                output.status.code(),
                stderr.trim()
            );
            return Err("Windows UAC administrator elevation was cancelled or failed".into());
        }

        // Verify with the retrying probe: a scanner holding `hosts` for a few
        // hundred milliseconds after the ACL change is not a failed setup.
        match probe_hosts_write_access() {
            HostsWriteAccess::Writable => {
                log::info!("[WINDOWS_HOSTS] One-time UAC setup completed successfully");
                Ok(())
            }
            HostsWriteAccess::TransientLock => {
                log::info!(
                    "[WINDOWS_HOSTS] UAC setup completed; hosts file momentarily held by another process, writes will retry"
                );
                Ok(())
            }
            HostsWriteAccess::PermissionDenied => {
                log::error!(
                    "[WINDOWS_HOSTS] hosts file still not writable after UAC setup (access denied)"
                );
                Err(
                    "Windows administrator permission is still required to write the hosts file. Approve the Windows UAC prompt to enable system blocking."
                        .into(),
                )
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// Applies domain blocks to the Windows hosts file and flushes the DNS resolver cache.
pub fn apply_blocks(domains: &[String]) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
        let backup_path = Path::new(BACKUP_HOSTS_PATH);

        if !hosts_path.exists() {
            return Err(format!("Hosts file not found at {DEFAULT_HOSTS_PATH}"));
        }

        let current_content = read_file_with_retry(hosts_path)
            .map_err(|e| format!("Failed to read {DEFAULT_HOSTS_PATH}: {e}"))?;

        // Create backup of pristine hosts file before first Preelock edit.
        // Creating a new file in System32\drivers\etc is not permitted for a
        // standard user, so a failure here is logged, not fatal.
        if !backup_path.exists() {
            let clean_content = strip_hosts_rules_from_content(&current_content);
            if let Err(e) = write_file_with_retry(backup_path, &clean_content) {
                log::warn!("[WINDOWS_HOSTS] Could not write hosts backup: {e}");
            }
        }

        let updated_content = apply_hosts_rules_to_content(&current_content, domains);
        write_file_with_retry(hosts_path, &updated_content)
            .map_err(|e| format!("Failed to write to {DEFAULT_HOSTS_PATH}: {e}"))?;

        // Verify by reading back: a write that silently did not land must not
        // report success and leave the user unprotected.
        let verify = read_file_with_retry(hosts_path)
            .map_err(|e| format!("Failed to verify {DEFAULT_HOSTS_PATH}: {e}"))?;
        if !domains.is_empty() && !verify.contains(BLOCK_START_MARKER) {
            return Err(format!(
                "Hosts file write did not persist at {DEFAULT_HOSTS_PATH}"
            ));
        }
        if domains.is_empty() && verify.contains(BLOCK_START_MARKER) {
            return Err(format!(
                "Hosts file block section was not removed from {DEFAULT_HOSTS_PATH}"
            ));
        }

        flush_dns_cache();

        log::info!(
            "[WINDOWS_HOSTS] Successfully applied {} blocked domains to hosts file",
            domains.len()
        );
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = domains;
        Ok(())
    }
}

/// Strips all Preelock blocks from the Windows hosts file and flushes DNS.
pub fn clear_blocks() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
        if !hosts_path.exists() {
            return Ok(());
        }

        let content = read_file_with_retry(hosts_path)
            .map_err(|e| format!("Failed to read {DEFAULT_HOSTS_PATH}: {e}"))?;
        if !content.contains(BLOCK_START_MARKER) {
            return Ok(());
        }

        let cleaned = strip_hosts_rules_from_content(&content);
        write_file_with_retry(hosts_path, &cleaned)
            .map_err(|e| format!("Failed to write to {DEFAULT_HOSTS_PATH}: {e}"))?;

        // Verify by reading back, exactly like `apply_blocks`: a removal that
        // did not land (rollback by a scanner, partially applied write) must
        // not be reported as a successful cleanup and leave the user blocked.
        let verify = read_file_with_retry(hosts_path)
            .map_err(|e| format!("Failed to verify {DEFAULT_HOSTS_PATH}: {e}"))?;
        if verify.contains(BLOCK_START_MARKER) || verify.contains(BLOCK_END_MARKER) {
            return Err(format!(
                "Hosts file block section was not removed from {DEFAULT_HOSTS_PATH}"
            ));
        }

        flush_dns_cache();
        log::info!("[WINDOWS_HOSTS] Preelock blocks cleared from hosts file");
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// Flushes the Windows DNS resolver cache so hosts changes take effect immediately.
///
/// Only called after a hosts write actually landed; failures are non-fatal
/// because the resolver also honours the hosts TTL on its own.
#[cfg(target_os = "windows")]
fn flush_dns_cache() {
    let _ = std::process::Command::new("ipconfig")
        .arg("/flushdns")
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_hosts_block_section() {
        let domains = vec!["reddit.com".to_string(), "instagram.com".to_string()];
        let section = generate_hosts_block_section(&domains);

        assert!(section.contains(BLOCK_START_MARKER));
        assert!(section.contains(BLOCK_END_MARKER));
        assert!(section.contains("127.0.0.1 reddit.com"));
        assert!(section.contains("::1 reddit.com"));
        assert!(section.contains("127.0.0.1 www.reddit.com"));
        assert!(section.contains("::1 www.reddit.com"));
        assert!(section.contains("127.0.0.1 instagram.com"));
        assert!(section.contains("::1 instagram.com"));

        // Non-blocked domain must NOT be present
        assert!(!section.contains("wikipedia.org"));
        assert!(!section.contains("google.com"));
    }

    #[test]
    fn test_apply_and_strip_hosts_rules_preserves_external_entries() {
        let original =
            "# Custom hosts entry\r\n192.168.1.10 myrouter.local\r\n127.0.0.1 localhost\r\n";
        let domains = vec!["youtube.com".to_string()];

        let applied = apply_hosts_rules_to_content(original, &domains);
        assert!(applied.contains("192.168.1.10 myrouter.local"));
        assert!(applied.contains("127.0.0.1 localhost"));
        assert!(applied.contains("127.0.0.1 youtube.com"));
        assert!(applied.contains(BLOCK_START_MARKER));
        assert!(applied.contains(BLOCK_END_MARKER));

        let stripped = strip_hosts_rules_from_content(&applied);
        assert_eq!(stripped, original);
        assert!(!stripped.contains("youtube.com"));
        assert!(!stripped.contains(BLOCK_START_MARKER));
    }

    #[test]
    fn test_empty_domains_clears_section() {
        let original = "127.0.0.1 localhost\r\n# === PREELOCK BLOCKLIST START ===\r\n127.0.0.1 bad.com\r\n# === PREELOCK BLOCKLIST END ===\r\n";
        let updated = apply_hosts_rules_to_content(original, &[]);
        assert_eq!(updated, "127.0.0.1 localhost\r\n");
    }

    #[test]
    fn test_classify_hosts_error_distinguishes_permission_from_lock() {
        // Only a genuine access denial may send the user back to UAC.
        assert_eq!(
            classify_hosts_error(Some(ERROR_ACCESS_DENIED)),
            HostsWriteAccess::PermissionDenied
        );
        assert_eq!(
            classify_hosts_error(Some(ERROR_SHARING_VIOLATION)),
            HostsWriteAccess::TransientLock
        );
        assert_eq!(
            classify_hosts_error(Some(ERROR_LOCK_VIOLATION)),
            HostsWriteAccess::TransientLock
        );
        // Unknown error codes must never be reported as "UAC not granted".
        assert_eq!(
            classify_hosts_error(Some(1234)),
            HostsWriteAccess::TransientLock
        );
        assert_eq!(classify_hosts_error(None), HostsWriteAccess::TransientLock);
    }

    #[test]
    fn test_retry_transient_recovers_from_sharing_violation() {
        let mut attempts = 0;
        let result: std::io::Result<()> =
            retry_transient(8, Duration::ZERO, Duration::ZERO, || {
                attempts += 1;
                if attempts < 3 {
                    Err(std::io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION))
                } else {
                    Ok(())
                }
            });
        assert!(result.is_ok());
        assert_eq!(attempts, 3);
    }

    #[test]
    fn test_retry_transient_does_not_retry_access_denied() {
        let mut attempts = 0;
        let result: std::io::Result<()> =
            retry_transient(8, Duration::ZERO, Duration::ZERO, || {
                attempts += 1;
                Err(std::io::Error::from_raw_os_error(ERROR_ACCESS_DENIED))
            });
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().raw_os_error(),
            Some(ERROR_ACCESS_DENIED),
            "permission failure must surface immediately, not be retried"
        );
        assert_eq!(attempts, 1);
    }

    #[test]
    fn test_retry_transient_gives_up_with_last_error() {
        let mut attempts = 0;
        let result: std::io::Result<()> =
            retry_transient(3, Duration::ZERO, Duration::ZERO, || {
                attempts += 1;
                Err(std::io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION))
            });
        assert!(result.is_err());
        assert_eq!(attempts, 3);
        assert_eq!(
            result.unwrap_err().raw_os_error(),
            Some(ERROR_SHARING_VIOLATION)
        );
    }

    /// A handle held without write sharing must be survived by the retry loop
    /// once the holder releases it (Defender releasing a scan handle).
    #[cfg(target_os = "windows")]
    #[test]
    fn test_write_file_with_retry_survives_transient_exclusive_handle() {
        let dir = std::env::temp_dir().join(format!("preelock-hosts-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("hosts");
        std::fs::write(&path, "127.0.0.1 localhost\r\n").expect("seed file");

        let holder_path = path.clone();
        let holder = std::thread::spawn(move || {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0) // exclusive: exactly what a scanner can do
                .open(&holder_path)
                .expect("holder open");
            std::thread::sleep(Duration::from_millis(120));
            drop(file);
        });

        // Give the holder time to take the exclusive handle first.
        std::thread::sleep(Duration::from_millis(30));

        let content = "127.0.0.1 localhost\r\n127.0.0.1 blocked.example\r\n";
        let result = write_file_with_retry(&path, content);
        holder.join().expect("holder thread");

        assert!(
            result.is_ok(),
            "retry must survive a transient exclusive handle"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read + write round trip through the share-flag path on a real file.
    #[cfg(target_os = "windows")]
    #[test]
    fn test_read_write_round_trip_with_share_flags() {
        let dir = std::env::temp_dir().join(format!("preelock-hosts-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("hosts");
        let content = "127.0.0.1 localhost\r\n# === PREELOCK BLOCKLIST START ===\r\n127.0.0.1 x.example\r\n# === PREELOCK BLOCKLIST END ===\r\n";

        write_file_with_retry(&path, content).expect("write");
        assert_eq!(read_file_with_retry(&path).expect("read"), content);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live check against the real hosts file: probe, apply, verify, clear.
    ///
    /// Excluded from the default suite because it edits a system file. Run it
    /// on a real Windows machine with:
    /// `cargo test --lib -- --ignored hosts_file_round_trip_live`
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore]
    fn hosts_file_round_trip_live() {
        let before = std::fs::read_to_string(DEFAULT_HOSTS_PATH).expect("read hosts");
        assert_eq!(
            probe_hosts_write_access(),
            HostsWriteAccess::Writable,
            "hosts file must be writable before the live round trip"
        );

        let domains = vec!["preelock-selftest.invalid".to_string()];
        apply_blocks(&domains).expect("apply blocks to the real hosts file");
        let applied = std::fs::read_to_string(DEFAULT_HOSTS_PATH).expect("read after apply");
        assert!(applied.contains("127.0.0.1 preelock-selftest.invalid"));
        assert_eq!(
            strip_hosts_rules_from_content(&applied),
            strip_hosts_rules_from_content(&before),
            "entries outside the Preelock section must be preserved"
        );

        clear_blocks().expect("clear blocks from the real hosts file");
        let cleared = std::fs::read_to_string(DEFAULT_HOSTS_PATH).expect("read after clear");
        assert!(!cleared.contains(BLOCK_START_MARKER));
        assert_eq!(cleared, strip_hosts_rules_from_content(&before));
    }
}
