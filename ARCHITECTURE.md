# Preelock Architecture & Component Map

This document describes the complete architecture of Preelock, its components, process supervision model, data storage, and security boundaries.

---

## 1. High-Level Architecture Overview

Preelock is structured as a local-first, dual-process desktop application built with **Tauri 2.x**, a **Rust backend**, a **React 19 / TypeScript frontend**, an **embedded SQLite database**, and an **MV3 Chromium browser extension**.

```mermaid
flowchart TD
    subgraph UI ["Thin GUI Client (Tauri / React 19)"]
        ReactApp["React UI (Dashboard, Screen Time, Sessions)"]
        ZustandStore["Zustand Global State"]
        TauriIPC["Tauri IPC Commands (src-tauri/src/ipc.rs)"]
        ReactApp --> ZustandStore --> TauriIPC
    end

    subgraph Daemon ["Authoritative Background Daemon (--background)"]
        Engine["Engine Loop (1-sec tick, Continuous Monotonic Clock)"]
        IPCServer["Loopback IPC Server (127.0.0.1:0 + Bearer Token)"]
        DB["SQLite Database (preelock.sqlite, WAL Mode)"]
        NativeMac["Native macOS Adapter (macos.m, AppKit, Vision, AX)"]
        NativeWin["Native Windows Adapter (windows.rs, UI Automation)"]
        Bridge["Browser Extension Bridge (127.0.0.1:47832)"]

        IPCServer <--> Engine
        Engine <--> DB
        Engine --> NativeMac
        Engine --> NativeWin
        Engine <--> Bridge
    end

    subgraph OS ["Operating System Supervisors"]
        LaunchAgent["macOS LaunchAgent (com.preelock.background)"]
        WinTask["Windows Task Scheduler (PreelockBackground)"]
    end

    subgraph Browsers ["Browser Integrations"]
        SafariNative["Safari (Direct AppleScript & Accessibility)"]
        ExtChromium["Chromium Browsers (Chrome, Brave, Edge, Comet, Arc via MV3)"]
    end

    TauriIPC <== "Loopback IPC (JSON-RPC)" ==> IPCServer
    LaunchAgent -.-> Daemon
    WinTask -.-> Daemon
    NativeMac --> SafariNative
    Bridge <== "HTTP Loopback" ==> ExtChromium
```

---

## 2. Process Separation & Mutex Model

### 2.1 The Two Processes

1. **Background Engine (`preelock --background`):**
   - Single authoritative owner of `Engine`, SQLite persistence, schedules, app locks, adult website blocking, and continuous timer countdowns.
   - Runs continuously in the background supervised by macOS LaunchAgent (`~/Library/LaunchAgents/com.preelock.background.plist`) or Windows Task Scheduler.
   - Protected by `engine.lock` (flock).
2. **Thin Desktop GUI (`preelock`):**
   - Renders the window, system tray menu, and modals.
   - Protected by `gui.lock` (single GUI instance). If launched when a GUI is already running, brings the existing window to focus.
   - Can be closed (`Cmd+W`, `Cmd+Q`, or minimized to tray) at any time without terminating or interrupting background focus sessions or blocking enforcement.

### 2.2 Force-Quit Recovery & Re-Launch Supervisor

- During an active strict focus session or active strict app/website block, if the GUI is terminated (macOS Force Quit or Windows Task Manager), the background daemon detects GUI termination and automatically relaunches Preelock GUI within 1-5 seconds.
- Outside of strict mode, quitting the GUI leaves the background engine running quietly in the background.

---

## 3. Communication & IPC Architecture

### 3.1 Desktop GUI to Background Engine

- **Protocol:** HTTP POST to `/rpc` on `127.0.0.1:{ephemeral_port}`.
- **Authentication:** 256-bit bearer token written to `bg-ipc.json` with `0600` permissions on startup.
- **Security:**
  - Rejects any request with an `Origin` header (prevents browser script CSRF).
  - Enforces exact `Host` matching (`127.0.0.1:{port}`) to prevent DNS rebinding.
  - Maximum payload bounded to 32 MB.

### 3.2 Browser Extension to Background Engine

- **Bridge Endpoint:** `127.0.0.1:47832` (`src-tauri/src/browser.rs`).
- **Origin Verification:** Verifies Chrome Web Store extension origin (`chrome-extension://...`).
- **Lease Model:** Blocking rules receive leases of 65 seconds refreshed on every heartbeat. If the engine stops, the extension automatically clears rules after lease expiration.

---

## 4. Website & Adult Content Blocking

### 4.1 Safari Direct Integration (macOS)

- Requires no browser extension.
- Uses native macOS Accessibility APIs (`AXUIElementCopyAttributeValue`) to read active URL and AppleScript to navigate blocked domains to `about:blank`.

### 4.2 Chromium Browsers (Chrome, Brave, Edge, Comet, Arc)

- **Primary Layer:** Declarative Net Request (DNR) session rules in MV3 extension.
- **Secondary Native Layer:** Native macOS Accessibility / Windows UI Automation monitors foreground address bars and redirects tabs directly if extension is absent or tampered with.

### 4.3 Adult Website Blocking

- Bundles an embedded list of ~50,000 adult domains compiled directly into the binary (`include_str!("../resources/adult-domains.txt")`), requiring no runtime file lookup or internet connection.
- Operates system-wide across all supported browsers.

---

## 5. Storage & Database Architecture

- **Database Engine:** Embedded SQLite with `rusqlite` bundled.
- **Path:** `com.preelock.desktop/preelock.sqlite`
- **Concurrency:** Write-Ahead Logging (`PRAGMA journal_mode=WAL;`), synchronous disk writes (`PRAGMA synchronous=FULL;`), and 5-second busy timeout.
- **Migrations:** Additive migrations `001_initial.sql` through `012_strict_blocks.sql`.
- **Reliability:** Automated pre-migration backups and `PRAGMA quick_check(1)` integrity checking on database initialization.
