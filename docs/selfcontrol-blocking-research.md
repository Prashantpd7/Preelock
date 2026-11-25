# SelfControl-Style Blocking Research & Technical Architecture

This document synthesizes findings from reverse-engineering SelfControl, Harbour Control, macOS Packet Filter (`pf`), launchd daemon privilege models, and Windows equivalents to inform Preelock's unbreakable, kernel-level website blocking engine.

---

## 1. SelfControl Source Code (`SelfControlApp/selfcontrol`)

SelfControl is an open-source (GPL) macOS focus application that pioneered kernel-level packet filter and hosts-file domain blocking. Its core enforcement architecture splits across three layers:

- **Daemon (`selfcontrold`):** A root-privileged helper located in `/Library/PrivilegedHelperTools/` that runs independently of the GUI via launchd (`/Library/LaunchDaemons/org.eyebeam.selfcontrold.plist`). It holds the block timer and active configuration, ensuring blocks survive app quits, user logouts, and system reboots.
- **Block Management (`BlockManager.m`):** On block initiation, SelfControl resolves blocked domains to IP addresses using POSIX DNS APIs, creates an anchor configuration file at `/etc/pf.anchors/org.eyebeam`, injects an `anchor "org.eyebeam"` reference into `/etc/pf.conf`, loads it using `pfctl -E -f /etc/pf.conf -F states`, and appends entries to `/etc/hosts` pointing domains to loopback/null sinks. On expiration, it flushes the PF anchor (`pfctl -a org.eyebeam -F rules`), cleans `/etc/pf.conf`, strips `/etc/hosts`, and flushes DirectoryService DNS caches (`dscacheutil -flushcache`).
- **Helpers (`SCKillerHelper`, `LaunchctlHelper`):** SCKillerHelper safely tears down legacy block states under strict security constraints (preventing unauthorized manual invocation via execution keys), while LaunchctlHelper orchestrates daemon reinstallation and upgrades.
- **Known Limitations:** SelfControl historically resolves target domains only once at block start (missing subsequent CDN IP rotation), warns in its FAQ that VPN connections can weaken enforcement if tunnels bypass local PF rules, and notes that browsers using encrypted DNS (Firefox DoH) may require restart unless DoH endpoints are explicitly blocked.

---

## 2. Harbour Control (`garrrikkotua/harbour-app`)

Harbour Control is a modern, MIT-licensed macOS focus app built on the concepts of SelfControl that specifically refines three key production concerns:

1. **Name-Level Hosts Routing:** Blocked domains and their `www` variants are mapped to `0.0.0.0` (IPv4) and `::` (IPv6) between explicit `# HARBOUR_HOSTS_START` and `# HARBOUR_HOSTS_END` markers, completely isolated from user entries and reliably cleanly removable.
2. **Periodic PF Re-resolution & State Killing:** IP addresses are populated into a dedicated PF anchor (`org.harbour`), and a background thread re-resolves domain IPs every 5 minutes. Newly discovered IPs are unioned into an accumulated set and added via `pfctl -a org.harbour -f ...`, followed by `pfctl -k ...` to terminate existing active TCP states for those IPs.
3. **Shared Hosting Range Exemption:** Addresses belonging to major shared CDN/cloud providers (Cloudflare, Fastly, Vercel, AWS CloudFront, AWS Global Accelerator) are checked against known CIDR blocks and skipped from raw IP blocking. This ensures that blocking a single service (e.g., an individual site on Cloudflare or Vercel) does not accidentally blackhole thousands of unrelated innocent websites sharing the same anycast IP.
4. **Persistent Daemon (`harbour-daemon`):** Supervised by a LaunchDaemon (`com.harbour.daemon`), it persists block state in `/var/db/harbour/state.json`, resumes filtering immediately upon boot, and tears down rules only when monotonic time expires.

---

## 3. macOS Packet Filter (PF) Architecture

macOS integrates the OpenBSD Packet Filter (`pf`) at the kernel networking layer below user-space VPN interfaces and browser sockets:

- **Anchors & Isolation:** Anchors allow modular sub-rulesets without overwriting system firewall rules. Loading rules into an anchor is executed via `pfctl -a <anchor_name> -f <rule_file>`, and unloading is executed via `pfctl -a <anchor_name> -F rules`.
- **System Anchor Hook:** For macOS PF to evaluate rules inside a custom anchor at runtime, `/etc/pf.conf` must declare the anchor reference (`anchor "com.preelock"` and `load anchor "com.preelock" from "/etc/pf.anchors/com.preelock"`). Preelock encapsulates this in idempotent comment markers (`# PREELOCK_PF_START` / `# PREELOCK_PF_END`).
- **Enabling & Reference Counting:** PF is enabled using `pfctl -E`, which increments an OS reference count and outputs a token (`Token : <id>`). Preelock saves this token so that upon block termination, `pfctl -X <token>` releases only Preelock's reference without disabling the firewall if other tools (or macOS Internet Sharing) rely on it.
- **Reboot Resilience:** PF in-memory states and anchors do not persist across reboots. A root LaunchDaemon (`RunAtLoad=true`) is essential to re-apply anchor rules and enable PF upon system boot.

---

## 4. macOS Privileged Helper Architecture: SMJobBless vs. LaunchDaemon + Sudoers

Standard macOS applications install privileged root helpers through one of two mechanisms:

- **`SMJobBless` (Apple Blessed API):** Requires an official, paid Apple Developer ID certificate ($99/yr) embedded with matching designated requirements and bundle identifiers in both the main app binary and helper binary. If the app is ad-hoc signed, unsigned, or compiled locally during development, `SMJobBless` consistently errors out with `kSMErrorInvalidSignature`.
- **`launchd` Daemon Plist + Sudoers (Preelock's Proven Pattern):** The app triggers standard macOS administrator elevation once (`osascript ... with administrator privileges`) from an explicit UI button click. It copies the helper binary to `/Library/PrivilegedHelperTools/preelock-helper`, installs `/Library/LaunchDaemons/com.preelock.helper.plist`, and writes a scoped `/etc/sudoers.d/preelock-helper` entry granting `NOPASSWD` execution of the helper binary to `%admin`.
- **Decision for Preelock:** Preelock will use the **LaunchDaemon Plist + Sudoers** pattern. It works flawlessly on both ad-hoc local builds and signed releases, requires the administrator password exactly once during setup, and enables the unprivileged desktop GUI and background engine to communicate with the helper over a secure local Unix domain socket (or passwordless `sudo -n`) without ever prompting the user during session start or background operation.

---

## 5. Windows Equivalent Architecture

The SelfControl blocking paradigm translates directly to Windows architectures:

- **Hosts File:** Located at `C:\Windows\System32\drivers\etc\hosts`. Requires elevated write privileges. Domains map to `0.0.0.0` and `::1`. DNS cache is flushed with `ipconfig /flushdns`.
- **Windows Filtering Platform / Windows Defender Firewall:** Outbound TCP/UDP connection blocking is achieved via Windows Firewall outbound rules:
  ```cmd
  netsh advfirewall firewall add rule name="Preelock_Block_<domain>" dir=out action=block remoteip=<IP1>,<IP2> protocol=TCP remoteport=80,443
  ```
  Or programmatically via PowerShell `New-NetFirewallRule` / direct Windows Filtering Platform (WFP) callout drivers.
- **Privileged Helper:** Runs as a Windows Service under `NT AUTHORITY\SYSTEM` or a high-privilege scheduled task (`RunLevel=HighestAvailable`).
- **Rust Modular Separation:** The codebase isolates platform-specific code behind `#[cfg(target_os = "windows")]` (e.g. `src-tauri/src/windows_firewall.rs` and `src-tauri/src/windows_helper.rs`), ensuring zero impact or breakage to the macOS build.

---

## 6. Synthesis & Contradiction Resolution

| Feature                     | SelfControl                     | Harbour Control                                | Preelock Implementation Choice & Rationale                                                                                                                                              |
| :-------------------------- | :------------------------------ | :--------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **IP Resolution & Refresh** | Resolves once at block start    | Re-resolves every 5 minutes in background      | **Follow Harbour Control**: Re-resolve every 5 minutes and accumulate IPs to defeat CDN IP-rotation bypasses.                                                                           |
| **Shared Hosting (CDNs)**   | Blocks all resolved IPs         | Skips Cloudflare, Vercel, Fastly, AWS          | **Follow Harbour Control**: Skip shared hosting CIDR ranges in PF rules so blocking one domain does not take down unrelated neighbor sites. /etc/hosts still blocks the domain by name. |
| **PF Anchor Integration**   | Appends to `/etc/pf.conf`       | Appends marker-bounded lines to `/etc/pf.conf` | **Follow Harbour Control**: Use strict marker bounds (`# PREELOCK_PF_START` / `# PREELOCK_PF_END`) with rollback safeguards.                                                            |
| **PF Reference Token**      | Reads `Token : ...`             | Saves token to disk, releases via `pfctl -X`   | **Follow Harbour Control**: Preserves existing system firewall state by releasing only Preelock's reference.                                                                            |
| **Helper Elevation**        | SMJobBless (requires paid cert) | osascript helper install script                | **Follow Preelock / Harbour Pattern**: One-time GUI authorization installing LaunchDaemon + sudoers entry. Zero password prompts thereafter.                                            |
| **Allowlist Mode**          | Blocklist only                  | Blocklist only                                 | **Preelock Hybrid Innovation**: Hosts and PF block known distractions and on-demand non-allowed IPs visited by the user; local DNS proxy returns NXDOMAIN for non-allowlisted domains.  |
