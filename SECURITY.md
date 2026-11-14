# Preelock Security Policy & Threat Model

Preelock is a local-first productivity, focus, and screen-time management application for macOS and Windows. This document defines the security architecture, threat model, trust boundaries, and vulnerability reporting process.

---

## 1. Security Architecture & Principles

### 1.1 Local-First Architecture

- **No Cloud Accounts or External Analytics:** Preelock has no remote backend server, accounts, tracking, or cloud telemetries. All sessions, tasks, screen time data, and settings remain on the user's local disk.
- **Storage Location:**
  - macOS: `~/Library/Application Support/com.preelock.desktop/`
  - Windows: `%APPDATA%\com.preelock.desktop\` (or MSIX isolated package directory)
- **Local Database:** SQLite with Write-Ahead Logging (WAL) and synchronous full writes (`preelock.sqlite`). Database migrations are strictly atomic and transactional.

### 1.2 Dual-Process Architecture & Privilege Separation

- **Background Engine (`--background`):**
  - Authoritative owner of `Engine`, SQLite persistence, timer countdowns, browser bridge, and application locks.
  - Runs as the standard user (no root or administrator elevation).
  - Supervised by macOS LaunchAgent (`com.preelock.background`) or Windows Scheduled Task (`PreelockBackground`).
- **Thin GUI Process:**
  - Runs Tauri 2.x and React 19.
  - Connects to `--background` via authenticated loopback IPC.
  - Unprivileged: closing the GUI or restarting it never disrupts active focus sessions or blocking enforcement.

### 1.3 Local IPC Security Model

- **Loopback Only:** Binds strictly to `127.0.0.1:0` (ephemeral port per user data directory). Never listens on `0.0.0.0` or external network interfaces.
- **Bearer Authentication:** Requires a 256-bit bearer token generated on engine startup and written to `bg-ipc.json` with `0600` permissions (Unix) so other OS users cannot read it.
- **Anti-DNS Rebinding & Anti-CSRF:**
  - Rejects any request carrying an `Origin` header (preventing browser scripts from issuing cross-origin fetches).
  - Validates that the `Host` header strictly matches `127.0.0.1:{port}`.
- **Payload Bound:** Maximum request body is capped at 32 MB with read timeouts to prevent denial-of-service memory exhaustion.

### 1.4 Browser Extension Security (MV3)

- **Principle of Least Privilege:**
  - Permissions: `declarativeNetRequest`, `declarativeNetRequestFeedback`, `storage`, `alarms`, `tabs`.
  - Host Permissions: `["http://127.0.0.1:47832/*"]` only.
  - The extension **does not** request `<all_urls>` permission. It cannot read the user's browsing traffic or web page contents.
- **Origin & ID Verification:**
  - The loopback bridge (`127.0.0.1:47832`) verifies that requests originate only from the paired browser extension's internal extension ID (`chrome-extension://...`).
  - Web pages cannot access or trigger bridge endpoints.

### 1.5 Tauri & Webview Sandboxing

- **Strict Content Security Policy (CSP):**
  ```text
  default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' asset: data: https:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-src 'none'
  ```
- **Scoped Capabilities:**
  - Tauri commands are locked down via `capabilities/main.json`.
  - Arbitrary shell execution and unrestricted filesystem access are disabled.
  - Commands are restricted strictly to `["main", "blocked-app"]` window labels.

---

## 2. Threat Model

| Threat / Attacker                         | Entry Point                          | Mitigation / Defense                                                                                                                                        |
| :---------------------------------------- | :----------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Malicious Web Page**                    | Local network probe (fetch/XHR)      | IPC rejects any request with an `Origin` header; enforces matching `Host` header; requires 256-bit bearer token.                                            |
| **Local Other User (Multi-user OS)**      | Reading IPC socket / DB              | User-specific data directory; `bg-ipc.json` written with `0600` mode; SQLite database protected by OS user file permissions.                                |
| **Focus Bypass via Force Quit**           | Killing GUI (`Cmd+Q` / Task Manager) | Background engine runs independently under LaunchAgent / Task Scheduler; re-launches GUI within 1-5s if strict mode is active.                              |
| **Focus Bypass via Extension Tampering**  | Disabling or removing extension      | Dual-layer enforcement: native macOS Accessibility (AX) / Windows UI Automation detects foreground browser domain and redirects tab even without extension. |
| **Data Corruption via Sudden Power Loss** | SQLite file corruption               | SQLite Write-Ahead Logging (WAL); `PRAGMA synchronous=FULL`; automatic `quick_check(1)` on open; automatic pre-migration backups before schema upgrades.    |
| **SQL Injection**                         | User input in forms / locks          | All queries use parameterized statements (`rusqlite::params!`). No dynamic SQL string interpolation.                                                        |
| **Remote Code Execution (RCE)**           | Tauri frontend injection             | Strict CSP disables external scripts; Tauri capabilities permit only strongly typed Rust commands.                                                          |

---

## 3. Security Boundaries & Intended Limitations

Preelock is a focus and productivity application designed to help users commit to deep work and avoid distractions. It is **not** an enterprise mobile device management (MDM) or kernel-level DRM lock:

1. **Operating System Superuser:** A user with full root/administrator privileges can kill daemons or edit system files. Preelock does not install kernel extensions or root daemons.
2. **Third-party Browser Downloads:** If a user installs an obscure unsupported browser not recognized by system app tracking, Preelock will track it under general app screen time rather than domain-level web blocking until mapped.
3. **Hardware Clock Manipulation:** Preelock measures continuous monotonic time (`mach_continuous_time` on macOS, `GetTickCount64` on Windows) to prevent local clock rollbacks from bypassing timed focus commitments.

---

## 4. Reporting a Security Vulnerability

If you discover a security vulnerability in Preelock:

1. **Do not create a public GitHub issue.**
2. Report the vulnerability privately via GitHub Security Advisories at:
   `https://github.com/Prashantpd7/Preelock/security/advisories/new`
   or email the maintainer directly.
3. Provide a clear description, reproduction steps, and impact assessment.
4. Security reports will be acknowledged within 48 hours, and patches will be released in an expedited maintenance update.
