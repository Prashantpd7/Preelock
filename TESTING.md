# Preelock Testing Guide & Quality Gates

This document outlines the testing architecture, quality gates, and verification commands required for any code changes in Preelock.

---

## 1. Quality Gates Overview

Before any code is committed, merged, or packaged for release, it **must** satisfy all quality gates:

1. **Frontend Type Checking:** `npm run typecheck` (`tsc -b`)
2. **Frontend Linting:** `npm run lint` (`eslint . --max-warnings 0`)
3. **Frontend Code Style:** `npm run format:check` (`prettier --check .`)
4. **Frontend Unit & Integration Tests:** `npm test` (`vitest run` across 54 test files)
5. **Rust Formatting:** `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
6. **Rust Clippy (Zero Warnings):** `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
7. **Rust Unit & Integration Tests:** `cargo test --manifest-path src-tauri/Cargo.toml` (395+ tests)
8. **Native Probe (macOS):** `npm run native:probe`
9. **Production Desktop Build:** `npm run desktop:build -- --bundles app` (or Windows MSIX / NSIS)

A single failure in any of these gates invalidates the change.

---

## 2. Test Suites

### 2.1 Frontend Tests (Vitest & React Testing Library)

Run all frontend tests:

```bash
npm test
```

Run an individual test file:

```bash
npx vitest run src/tests/appLocks.test.tsx
```

Key test coverage areas:

- `src/tests/appLocks.test.tsx`: App locks, website locks, adult website blocking, strict mode 20-second countdown persistence, bypass prevention.
- `src/tests/activeSession.test.tsx`: Focus session lifecycle, pause limits, deep focus, countdowns, emergency challenges.
- `src/tests/screenTime.test.tsx`: Usage aggregation, categorization, day boundaries, opt-in privacy controls.
- `src/tests/schedules.test.tsx`: Scheduled focus sessions, recurring window enforcement, lock rules.
- `src/tests/allowances.test.tsx`: Budget limits, accrual tracking, exhaustion triggers.
- `src/tests/safariRestriction.test.tsx`: Safari-specific website blocking and domain normalization.
- `browser-extension/*.test.js`: Extension worker routing, declarativeNetRequest rules, YouTube controls, pairing tokens.

### 2.2 Rust Backend Tests

Run all backend tests:

```bash
npm run rust:test
# Or directly with cargo:
cargo test --manifest-path src-tauri/Cargo.toml
```

Key test modules:

- `database.rs`: SQLite migrations v1 through v12, WAL mode, transaction atomicity, corruption detection (`quick_check`), automated pre-migration backups, legacy session deserialization.
- `sessions.rs`: State machine, strict mode commitments, password hashing, grace periods, emergency unlocks.
- `engine.rs`: 1-second tick cadence, continuous monotonic time tracking, sleep/wake gap handling, background engine recovery, UI relaunch supervision.
- `adult_block.rs`: Embedded adult domain lookup, strict countdown transitions, duration caps.
- `browser.rs`: Loopback bridge authentication, origin verification, heartbeat timeout, client liveness.
- `ipc.rs`: Loopback authentication, bearer token verification, anti-DNS rebinding, anti-CSRF.
- `applocks.rs`: Timed locks, daily limits, wildcard domain matching, midnight resets.
- `platform/`: Native macOS and Windows sampling, window minimization, accessibility URL detection.

### 2.3 Native Probe Test (macOS)

Tests Objective-C native frameworks (`AppKit`, `AVFoundation`, `Vision`, `CoreGraphics`, `UserNotifications`, `ServiceManagement`):

```bash
npm run native:probe
```

### 2.4 End-to-End Local Testing Script

Run the automated end-to-end verification script:

```bash
bash scripts/test-local-env.sh
```

This script verifies:

1. Production bundle integrity (`/Applications/Preelock.app`).
2. Single-instance GUI mutex (`gui.lock`).
3. Single-instance background mutex (`engine.lock`).
4. LaunchAgent configuration (`com.preelock.background.plist`).
5. Process supervision under force-quit (`kill -9`).
6. Loopback IPC authentication.
7. Local SQLite database migrations and schema version.

---

## 3. Continuous Integration (CI)

The GitHub Actions workflow (`.github/workflows/ci.yml`) runs on every push and pull request across:

- **Ubuntu 24.04:** Frontend typecheck, lint, formatting, unit tests, web build.
- **macOS 15 (Apple Silicon - aarch64):** Rust checks, native probe, full test suite, `.app` bundle packaging, bundle verification.
- **macOS 15 (Intel - x86_64):** Cross-compilation and packaging verification.
- **Windows 2022 (x86_64):** Windows manifest embedding, Windows library & binary tests, NSIS package build.
