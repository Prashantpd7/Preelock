//! Preelock Windows Firewall (Outbound Packet Filter Equivalent) Module.
//!
//! Manages outbound block rules for Windows Defender Firewall using `netsh advfirewall`.
//! Safely isolated behind conditional compilation for Windows targets.

#[cfg(target_os = "windows")]
use std::process::Command;

pub const PREELOCK_FIREWALL_GROUP: &str = "PreelockFocusBlock";

/// Applies outbound TCP/UDP firewall block rules for resolved IP addresses.
#[allow(unused_variables)]
pub fn apply_firewall_rules(domains: &[String]) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // 1. Clear any prior Preelock firewall rules
        clear_firewall_rules()?;

        if domains.is_empty() {
            return Ok(());
        }

        // 2. Resolve domain IPs (Windows system resolver)
        let mut ips = Vec::new();
        for domain in domains {
            let bare = domain.trim().trim_start_matches("www.");
            if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&(format!("{bare}:80")[..]))
            {
                for addr in addrs {
                    let ip_str = addr.ip().to_string();
                    if !ips.contains(&ip_str) {
                        ips.push(ip_str);
                    }
                }
            }
        }

        if ips.is_empty() {
            return Ok(());
        }

        let ip_list = ips.join(",");

        // 3. Add outbound TCP block rule
        let tcp_status = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={PREELOCK_FIREWALL_GROUP}_TCP"),
                "dir=out",
                "action=block",
                &format!("remoteip={ip_list}"),
                "protocol=TCP",
                "remoteport=80,443",
            ])
            .status()
            .map_err(|e| format!("Failed to add Windows Firewall TCP rule: {e}"))?;

        // 4. Add outbound UDP block rule (QUIC / HTTP3)
        let udp_status = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={PREELOCK_FIREWALL_GROUP}_UDP"),
                "dir=out",
                "action=block",
                &format!("remoteip={ip_list}"),
                "protocol=UDP",
                "remoteport=80,443,853",
            ])
            .status()
            .map_err(|e| format!("Failed to add Windows Firewall UDP rule: {e}"))?;

        if !tcp_status.success() || !udp_status.success() {
            log::warn!("[WINDOWS_FIREWALL] Netsh rule creation returned non-zero exit code");
        }

        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// Clears all Preelock outbound firewall block rules.
pub fn clear_firewall_rules() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={PREELOCK_FIREWALL_GROUP}_TCP"),
            ])
            .status();

        let _ = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={PREELOCK_FIREWALL_GROUP}_UDP"),
            ])
            .status();

        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}
