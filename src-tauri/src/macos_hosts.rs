//! Preelock Native Hosts-File Website Blocking for macOS.
//!
//! Provides unbreakable, OS-level domain blocking on macOS by managing
//! scoped entries in `/etc/hosts` alongside the local DNS proxy.
//!
//! ## Architecture (SelfControl-Grade Multi-Layer Blocking)
//! - Maps each blocked domain and its `www.` variant to IPv4 null route (`0.0.0.0`)
//!   and IPv6 loopback (`::1`).
//! - Standard POSIX resolution (`getaddrinfo` / `gethostbyname`) inspects `/etc/hosts`
//!   before any network socket or DNS query is created, preventing browsers (including
//!   Safari Private Browsing and Chrome Incognito) from resolving the domain.
//! - Scoped entries: All Preelock rules are bounded by `# === PREELOCK BLOCKLIST START ===`
//!   and `# === PREELOCK BLOCKLIST END ===`. Outside entries are never modified.
//! - Pristine backup: `/etc/hosts.preelock.bak` is created before the first modification.
//! - Instant restoration: When the active session or app lock ends, the Preelock section
//!   is stripped and `/usr/bin/dscacheutil -flushcache` is executed.

use std::path::Path;

pub const DEFAULT_HOSTS_PATH: &str = "/etc/hosts";
pub const BACKUP_HOSTS_PATH: &str = "/etc/hosts.preelock.bak";
pub const BLOCK_START_MARKER: &str = "# === PREELOCK BLOCKLIST START ===";
pub const BLOCK_END_MARKER: &str = "# === PREELOCK BLOCKLIST END ===";

/// Formats the Preelock block section for `/etc/hosts`.
///
/// Maps each domain (and its `www.` variant if not already prefixed) to both
/// IPv4 null route (`0.0.0.0`) and IPv6 loopback (`::1`).
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
        lines.push(format!("0.0.0.0 {norm}"));
        lines.push(format!("::1 {norm}"));

        if !norm.starts_with("www.") && !norm.contains('/') {
            lines.push(format!("0.0.0.0 www.{norm}"));
            lines.push(format!("::1 www.{norm}"));
        }
    }

    lines.push(BLOCK_END_MARKER.to_string());
    lines.join("\n") + "\n"
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

    let mut out = result.join("\n");
    if !out.is_empty() && content.ends_with('\n') {
        out.push('\n');
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
    if stripped.ends_with('\n') {
        format!("{stripped}{block_section}")
    } else if stripped.is_empty() {
        block_section
    } else {
        format!("{stripped}\n{block_section}")
    }
}

/// Checks whether write access to `/etc/hosts` is available without further elevation.
pub fn is_admin_setup_required() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .open(DEFAULT_HOSTS_PATH)
            .is_err()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Ensures `/etc/hosts` permissions allow the local admin group to manage blocks.
pub fn ensure_hosts_writable() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if !is_admin_setup_required() {
            return Ok(());
        }

        // Try fast helper elevation via preelock-dns-proxy --setup-hosts
        if let Some(bin) = crate::dns_proxy::find_binary() {
            let status = std::process::Command::new("sudo")
                .args(["-n", bin.to_str().unwrap_or(""), "--setup-hosts"])
                .status();
            if let Ok(s) = status {
                if s.success() && !is_admin_setup_required() {
                    return Ok(());
                }
            }
        }

        // Otherwise prompt via standard macOS administrator dialog
        let script = "do shell script \"chown root:admin /etc/hosts && chmod 664 /etc/hosts\" with administrator privileges";
        let out = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", script])
            .output()
            .map_err(|e| format!("Failed to run elevation script: {e}"))?;

        if !out.status.success() {
            return Err("Administrator approval was cancelled or declined".into());
        }

        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

/// Applies domain blocks to `/etc/hosts` and flushes the macOS resolution cache.
pub fn apply_blocks(domains: &[String]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
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

        // Flush macOS DirectoryService & mDNS resolution cache immediately
        let _ = std::process::Command::new("/usr/bin/dscacheutil")
            .arg("-flushcache")
            .status();

        log::info!(
            "[MACOS_HOSTS] Successfully applied {} blocked domains to /etc/hosts",
            domains.len()
        );
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = domains;
        Ok(())
    }
}

/// Strips all Preelock blocks from `/etc/hosts` and flushes macOS DNS.
pub fn clear_blocks() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let hosts_path = Path::new(DEFAULT_HOSTS_PATH);
        if !hosts_path.exists() {
            return Ok(());
        }

        if let Ok(content) = std::fs::read_to_string(hosts_path) {
            if content.contains(BLOCK_START_MARKER) {
                let cleaned = strip_hosts_rules_from_content(&content);
                let _ = std::fs::write(hosts_path, cleaned);
                let _ = std::process::Command::new("/usr/bin/dscacheutil")
                    .arg("-flushcache")
                    .status();
                log::info!("[MACOS_HOSTS] Successfully removed Preelock blocks from /etc/hosts");
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_hosts_block_section() {
        let domains = vec!["instagram.com".to_string(), "reddit.com".to_string()];
        let section = generate_hosts_block_section(&domains);

        assert!(section.contains(BLOCK_START_MARKER));
        assert!(section.contains(BLOCK_END_MARKER));
        assert!(section.contains("0.0.0.0 instagram.com"));
        assert!(section.contains("0.0.0.0 www.instagram.com"));
        assert!(section.contains("::1 instagram.com"));
        assert!(section.contains("::1 www.instagram.com"));
        assert!(section.contains("0.0.0.0 reddit.com"));
    }

    #[test]
    fn test_apply_and_strip_hosts_rules_preserves_external_entries() {
        let original = "127.0.0.1 localhost\n::1 localhost\n192.168.1.50 custom.local\n";
        let domains = vec!["twitter.com".to_string()];

        let applied = apply_hosts_rules_to_content(original, &domains);
        assert!(applied.contains("192.168.1.50 custom.local"));
        assert!(applied.contains("0.0.0.0 twitter.com"));

        let stripped = strip_hosts_rules_from_content(&applied);
        assert_eq!(stripped, original);
    }

    #[test]
    fn test_empty_domains_clears_section() {
        let original = "127.0.0.1 localhost\n";
        let domains = vec!["x.com".to_string()];

        let applied = apply_hosts_rules_to_content(original, &domains);
        assert!(applied.contains(BLOCK_START_MARKER));

        let cleared = apply_hosts_rules_to_content(&applied, &[]);
        assert!(!cleared.contains(BLOCK_START_MARKER));
        assert_eq!(cleared, original);
    }
}
