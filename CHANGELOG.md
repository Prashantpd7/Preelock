# Preelock Changelog

All notable changes to the Preelock project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.5.20] - 2026-11-27

### Added

- **Blocked pages are taken over on macOS even without the browser extension.** When the Apple Events sweep cannot replace a tab — no Automation consent, or a browser that refuses scripting — Preelock now closes the one frontmost tab that is still showing the blocked host. A service-worker-cached page such as YouTube's kept running from its cached shell after `/etc/hosts` and the packet filter had already cut its network, so the user only ever saw a stream of "blocked" notifications. The native layer re-verifies everything before acting (Accessibility trust, the browser is frontmost, the focused page really is the blocked host, and the keyboard focus is not in a text field) and posts a single Cmd+W — the same action Windows already takes. Nothing is read, typed, or copied.
- Tab-takeover outcomes are written to the lifecycle log once a minute per app/domain instead of on every one-second tick, and each reason (missing Accessibility, browser no longer frontmost, user typing, unconfirmed address) is named so a block that only notifies can be diagnosed.

### Fixed

- **A denied macOS Automation grant silently disabled blocked-tab replacement**, leaving an already-open blocked page running with only notifications arriving. Preelock now asks for Automation per browser once (the standard system consent prompt), skips the AppleScript sweep only when macOS explicitly refused it (`errAEEventNotPermitted` / `errAEEventWouldRequireUserConsent`), and re-reads a denial every 30 seconds so turning the grant on in System Settings → Privacy & Security → Automation takes effect without relaunching the app. A one-time notification says what to enable.
- Failures from `osascript` are no longer discarded: a browser that refuses the tab redirect, or cannot be scripted at all, is reported in the lifecycle log instead of failing silently.

---

## [0.5.19] - 2026-11-27

### Fixed

- **Windows hosts-file writes sometimes failed with `os error 32` (sharing/lock violation)** even though the Preelock one-time UAC setup had already granted Modify permission. `C:\Windows\System32\drivers\etc\hosts` is a shared system file and the Windows Defender real-time scanner or DNS client can momentarily hold it without sharing write. The hosts file is now opened with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`, and `ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION` are retried with bounded backoff (~1.1 s total) before the open is reported failed. Only `ERROR_ACCESS_DENIED` is treated as a real "needs UAC" signal.
- **Windows hosts-file writes could silently succeed without the block landing.** After every write the Preelock block markers (`# === PREELOCK BLOCKLIST START ===` / `=== END ===`) are now read back from disk and verified; a block that did not make it into the file is reported as failed instead of reported as applied.
- **Ending or unblocking a Windows session left Preelock firewall rules behind, which could resurface the "Enable System Blocking" UAC loop on the next launch.** Session stop now also clears firewall rules, and unblocking verifies that _both_ the hosts-file markers and the firewall rules are gone before declaring the system unblocked.

### Build / CI

- macOS-only retry constants in the Windows hosts-file helper (`HOSTS_OPEN_ATTEMPTS`, `HOSTS_RETRY_BASE_DELAY`, `HOSTS_RETRY_MAX_DELAY`) are now strictly gated with `cfg(target_os = "windows")` so they no longer appear as dead code during the macOS test compilation. The CI matrix (frontend, macOS Apple Silicon, macOS Intel, Windows MSVC) is now green.

---

## [Unreleased]

### Fixed

- **macOS notification click after Command+Q showed "The application 'Preelock' is not open anymore" and left a ghost Dock icon.** The `--background` engine initialized `NSApplication` and set a Prohibited activation policy, which registered a second `com.preelock.desktop` application instance with LaunchServices (`lsappinfo`: `BackgroundOnly`) beside the GUI. Once the GUI quit, macOS resolved "Preelock" to that windowless daemon: notification activation failed, `open -a Preelock.app` promoted the daemon to a frontmost app with no window, and the only way out was Force Quit. Headless modes no longer touch AppKit at all, so the bundle has exactly one application identity — the GUI.
- Every GUI activation request (notification click, second instance, tray timer, supervisor relaunch, new `preelock --open-gui`) now goes through one coordinator that activates a _verified_ live GUI or launches the bundle through LaunchServices. A `gui.lock` PID is no longer trusted on its own: bundle identity, bundled executable and activation policy are validated first.
- Notification responses are handled with a correlation id and a non-sensitive kind (`reminder`, `warning`, `general`) that survives a cold launch through `notification-click.json`, consumed exactly once. Notification identifiers are stable, so one reminder replaces itself instead of stacking. The response completion handler always runs.
- A GUI launch loser now exits without creating an `NSApplication`, so a duplicate launch request can no longer leave a ghost Dock icon.

### Added

- Structured lifecycle diagnostics (`src/lifecycle_log.rs`) written to `~/Library/Logs/Preelock/lifecycle.log` and stderr, plus `scripts/collect-macos-diagnostics.sh` for collecting them with LaunchServices, process and signature state.
- `scripts/acceptance-macos-notification.py`: production acceptance harness for the installed signed app, including the invariant that LaunchServices has no entry for the bundle while the GUI is closed.
- `preelock --open-gui`: activate or launch the GUI through the real coordinator and report the outcome.

---

## [0.5.5] - 2026-11-14

### Added

- **System-Wide Adult Website Blocking:**
  - Integrated high-performance embedded domain filter containing ~50,000 top adult domains with zero runtime external file dependency.
  - Safari native blocking using AppleScript and macOS Accessibility APIs without requiring any browser extension.
  - Native browser blocking across Chrome, Brave, Comet, Edge, and Arc.
  - Dedicated Toggle control in Screen Time > App lock > Block adult websites.
- **Strict Block Mode with Durable 20-Second Countdown:**
  - Persistent 20-second countdown backed directly by SQLite (`activating_until_ms`).
  - Survives collapsing accordion dropdowns, switching navigation tabs, and quitting the app completely (`Cmd+Q`).
  - Background daemon (`preelock --background`) ticks down and commits strict mode transition to database even if GUI is closed.
  - User can cancel via "Turn back off" button during the 20-second window.
  - Unified Strict Block UI and controls across all three categories: **Block apps**, **Block websites**, and **Block adult websites**.
  - Preset duration chips: `1 day`, `2 days`, `3 days`, `5 days`, `7 days`.
- **Strict Bypass Prevention:**
  - Deletion of active app locks and website locks is rejected by the backend when strict mode is active.
  - Frontend replaces remove `(X)` buttons with `Strictly locked` indicators.
  - Active timed locks are automatically extended to the duration of the strict block.
- **Database Reliability & Corruption Safeguards:**
  - Database schema version 12 with dedicated `strict_blocks` table.
  - Automatic database integrity check (`PRAGMA quick_check(1)`) on startup with automatic forensic preservation of corrupted databases.
  - Automatic pre-migration backup (`preelock.pre_v12_v{version}.bak`) created before upgrading existing databases.
  - Seamless backward compatibility for older session JSON payloads (`#[serde(default, alias = "minutes")]`).
- **Comprehensive Engineering Documentation:**
  - Created `SECURITY.md`, `TESTING.md`, `DATABASE_MIGRATIONS.md`, `COMPATIBILITY.md`, `RELEASE.md`, and `AGENTS.md`.

### Fixed

- Fixed single-instance GUI relaunch behavior using `gui.lock` file mutex.
- Fixed LaunchAgent background supervisor to prevent orphaned or duplicate processes.
- Fixed inactivity attribution and continuous monotonic sampling during system sleep and wake cycles.

---

## [0.5.4] - 2026-11-09

- Enhanced background engine LaunchAgent supervision on macOS and Scheduled Task supervision on Windows.
- Improved browser pairing reliability with Chromium MV3 extensions.

---

## [0.5.0] - 2026-10-22

- Added Screen Time tracking module with local-first persistent SQLite daily buckets.
- Added Application and Website locks (timed locks and daily limit rules).
