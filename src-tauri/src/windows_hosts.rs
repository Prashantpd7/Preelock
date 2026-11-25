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

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(target_os = "windows")]
use std::path::Path;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const DEFAULT_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";
pub const BACKUP_HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts.preelock.bak";
pub const BLOCK_START_MARKER: &str = "# === PREELOCK BLOCKLIST START ===";
pub const BLOCK_END_MARKER: &str = "# === PREELOCK BLOCKLIST END ===";

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

/// Checks whether write access to the hosts file is available without further elevation.
pub fn is_admin_setup_required() -> bool {
    #[cfg(target_os = "windows")]
    {
        let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
        if !hosts_path.exists() {
            return true;
        }
        // Test write access
        std::fs::OpenOptions::new()
            .append(true)
            .open(hosts_path)
            .is_err()
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
             & icacls $hosts /grant '*S-1-5-32-545:(M)' '*S-1-5-11:(M)'\r\n\
             if ($LASTEXITCODE -ne 0) {{\r\n\
                 & takeown /F $hosts /A 2>$null\r\n\
                 & icacls $hosts /grant '*S-1-5-32-545:(M)' '*S-1-5-11:(M)'\r\n\
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

        if let Err(e) = std::fs::OpenOptions::new()
            .append(true)
            .open(Path::new(DEFAULT_HOSTS_PATH))
        {
            log::error!("[WINDOWS_HOSTS] Hosts file still not writable after UAC setup: {e}");
            return Err(format!(
                "Write access to hosts file was not granted by UAC setup ({e})"
            ));
        }

        log::info!("[WINDOWS_HOSTS] One-time UAC setup completed successfully");
        Ok(())
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

        let current_content = std::fs::read_to_string(hosts_path)
            .map_err(|e| format!("Failed to read {DEFAULT_HOSTS_PATH}: {e}"))?;

        // Create backup of pristine hosts file before first Preelock edit
        if !backup_path.exists() {
            let clean_content = strip_hosts_rules_from_content(&current_content);
            let _ = std::fs::write(backup_path, clean_content);
        }

        let updated_content = apply_hosts_rules_to_content(&current_content, domains);
        std::fs::write(hosts_path, updated_content)
            .map_err(|e| format!("Failed to write to {DEFAULT_HOSTS_PATH}: {e}"))?;

        // Flush DNS resolver cache so changes take effect immediately
        let _ = std::process::Command::new("ipconfig")
            .arg("/flushdns")
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

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

        if let Ok(content) = std::fs::read_to_string(hosts_path) {
            if content.contains(BLOCK_START_MARKER) {
                let cleaned = strip_hosts_rules_from_content(&content);
                let _ = std::fs::write(hosts_path, cleaned);
                let _ = std::process::Command::new("ipconfig")
                    .arg("/flushdns")
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                log::info!("[WINDOWS_HOSTS] Preelock blocks cleared from hosts file");
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
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
}
