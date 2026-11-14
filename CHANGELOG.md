# Preelock Changelog

All notable changes to the Preelock project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
