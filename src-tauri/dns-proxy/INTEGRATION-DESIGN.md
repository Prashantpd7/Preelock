# Preelock Native DNS Proxy Integration Design (Phase 8)

## 1. Executive Summary

This document specifies the integration of the user-space local DNS proxy into the main Preelock application (`src-tauri/`). The proxy acts as an operating-system-level website enforcement layer that complements the existing Manifest V3 browser extension without altering core session, database, or UI architectures.

---

## 2. Architecture & Components

```
┌────────────────────────────────────────────────────────┐
│ Preelock Main Application (Rust / Tauri 2)             │
│                                                        │
│  ┌──────────────────┐        ┌───────────────────────┐ │
│  │ Session Engine   │───────>│ DNS Proxy Manager     │ │
│  │ (engine.rs)      │        │ (dns_proxy.rs)        │ │
│  └──────────────────┘        └──────────┬────────────┘ │
│                                         │ (IPC)        │
└─────────────────────────────────────────┼──────────────┘
                                          │
                  ┌───────────────────────┴───────────────────────┐
                  │ Unix Domain Socket (0600)                     │
                  │ /tmp/preelock-dns-control.sock                │
                  └───────────────────────┬───────────────────────┘
                                          │
┌─────────────────────────────────────────┼──────────────────────────────┐
│ DNS Proxy Sidecar Process (Swift CLI)   │                              │
│                                         ▼                              │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ Local DNS Engine (preelock-dns-proxy)                            │  │
│  │ - 127.0.0.1:53 listener (UDP & TCP, RFC 1035 / RFC 7766)         │  │
│  │ - Blocklist / Allowlist policy enforcement                       │  │
│  │ - DoH / DoT resolver blocking (17 endpoints)                     │  │
│  │ - Resilient upstream forwarding & failover                       │  │
│  └──────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.1 Sidecar Binary (`preelock-dns-proxy`)

- Built directly from Swift source inside the repo (`src-tauri/dns-proxy/`) using `swift build`.
- Binds strictly to `127.0.0.1:53` (loopback only, never an open resolver).
- Drops root privileges to the console user (UID 501) immediately after binding port 53.
- Controlled via local Unix domain socket (`/tmp/preelock-dns-control.sock` with `0600` permissions).
- Packaged as a resource in `tauri.conf.json`.

### 2.2 Rust Manager Module (`src-tauri/src/dns_proxy.rs`)

- Implemented using pure standard library (`std::process::Command`, `std::os::unix::net::UnixStream`).
- Zero external Cargo crate dependencies.
- Responsibilities:
  1. Spawns and supervises the proxy sidecar process.
  2. Saves current system DNS configuration before redirection.
  3. Applies `127.0.0.1` redirect to active network service (e.g. `Wi-Fi`).
  4. Generates and pushes session-specific policy JSON files via `set-policy <path>`.
  5. Restores original system DNS (or DHCP) on session completion, pause, cancellation, or application quit.
  6. Implements graceful degradation: if DNS redirection fails, logs warning and allows session to proceed with extension-only blocking.

---

## 3. Session Lifecycle Hooks & Policy Sync

### 3.1 Activation Points

- **Session Start / Resume**:
  When a focus session starts with website blocking enabled (`WebsiteBlocking::Blocklist` or `Allowlist`):
  1. Manager checks if proxy is running; if not, starts it.
  2. Generates dynamic policy file from `session.focus.domains` + `policies/doh_endpoints.json`.
  3. Sends `set-policy <path>` to control socket.
  4. Redirects active service DNS to `127.0.0.1`.
- **Session End / Pause / Cancel**:
  1. Reverts active network service DNS to original saved state (or DHCP Empty).
  2. Stops proxy sidecar process if no other active blocking session requires it.
  3. Flushes macOS resolver cache (`dscacheutil -flushcache`).
- **App Exit & Crash Cleanup**:
  - `dns_proxy::cleanup()` is registered in application shutdown paths (`ExitRequested`, `RunEvent`) to guarantee no system DNS redirect is left pointing to a terminated proxy.

### 3.2 UI-Closed Behavior

- Preelock's background engine (`--background` / `--tray`) runs independently of the main GUI window.
- The DNS proxy sidecar lifecycle is tied to the **background engine session state**, not the frontend window.
- Closing or hiding the main GUI window while a session is active keeps the DNS proxy running.

---

## 4. Fallback & Graceful Degradation

- **Primary Blocking Layer**: The browser extension remains the primary in-browser blocking mechanism with custom blocked page UI.
- **Secondary OS Layer**: The DNS proxy provides native OS-level blocking across non-paired browsers and background processes.
- **Fail-Safe Invariant**: If the sidecar binary is unavailable, fails to bind port 53, or network setup fails, the engine logs an informational message and allows the session to proceed normally using extension-based enforcement. No session is ever aborted due to DNS proxy initialization failure.
