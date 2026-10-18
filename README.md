# VYSTRIQ

A local-first desktop foundation for an AI-powered focus, study and productivity application on Windows and macOS. Version 0.3 adds productive-time accounting, task-aware app/domain checks, reviewable corrections and a visible background controller, alongside voluntary native application redirection, a paired Chromium request-filtering extension, consented activity sampling and optional local macOS camera presence analysis. Managed Deep Focus and system-wide network control remain unavailable.

## Run

Install the [platform prerequisites](DEVELOPMENT.md), then from this repository root:

```sh
npm ci
npm run desktop
```

`npm run dev` opens only the browser preview; saving is deliberately disabled there. The actual application runs in Tauri 2 with a Rust backend.

## Included

- Dashboard, Focus Sessions, Tasks, Activity, Analytics and Settings.
- Create/start/pause/resume/end sessions; choose flexible or no-pause policy.
- Rust-owned accepted productive-time targets; distraction/absence/unknown intervals earn no time, while restrictions remain active.
- No credit across sleep/restarts; explicit camera fallback or self-reported checkpoints; indexed SQLite interval history.
- Close-to-background behavior with visible menu bar/system tray reopen and recovery.
- Create, rename and complete tasks; persist dark/light/system themes and default duration.
- SQLite transactions, versioned migrations, actual session statistics, clear empty states.
- Standard blocklists and Strict approved-app/browser-domain sessions, with deliberate emergency recovery.
- Native app hiding on macOS and foreground minimization on Windows (cooperative, not OS launch prevention).
- Chromium browser-extension allowlists/blocklists/offline profiles, activation acknowledgement and short-lived recovery leases.
- Opt-in app/idle summaries, interruption history, local macOS camera presence estimates and a permissions dashboard.
- Clear capability limitations: no universal administrator-proof locking, system-wide internet blocking or phone/learning verification.
- Strict TypeScript, ESLint, Prettier, Vitest, Rust tests, Rustfmt, Clippy and desktop CI.

## Project map

```text
src/
  components/   reusable controls, session cards and task editing
  layouts/      navigation and application shell
  pages/        six application pages
  hooks/        native polling and theme lifecycle
  services/     typed IPC and productivity analytics
  state/        serialized Zustand state updates
  types/        frontend domain contracts
  tests/        UI workflows and analytics tests
  config.ts     shared frontend metadata
src-tauri/
  src/
    lib.rs         Tauri commands and application lifecycle
    engine.rs      persistence, enforcement lifecycle and recovery
    browser.rs     authenticated loopback bridge
    focus.rs       mode policy and validated configuration
    clock.rs       suspend-aware platform monotonic clocks
    sessions.rs    session state machine and tests
    models.rs      sessions, tasks, preferences, commands
    database.rs    SQLite repository
    capabilities.rs  truthful platform capability declarations
    platform/      Windows and macOS adapter boundaries
  native/       macOS AppKit, AVFoundation and Vision adapter/probe
  migrations/   versioned schema
  capabilities/ local-window access policy
  permissions/  application command permissions
  icons/        native application icons and SVG source
browser-extension/ Chromium MV3 filtering, pairing and recovery
docs/FOCUS_MODE.md setup, bypass limits, privacy and recovery
.github/workflows/ci.yml
```

See [development](DEVELOPMENT.md), [architecture](ARCHITECTURE.md), [contribution guidelines](CONTRIBUTING.md) and [roadmap](ROADMAP.md). No cloud account, API key, telemetry or administrator privileges are required.

## Enable focus restrictions

See [Focus Mode setup and recovery](docs/FOCUS_MODE.md). Start VYSTRIQ, open Settings, load `browser-extension/` as an unpacked extension in a supported Chromium browser, and pair that profile. Use the extension's installed-matcher self-test before relying on browser restrictions. The app does not install extensions or alter OS policies silently.

[Validation results](VALIDATION.md) distinguish automated checks from unverified live enforcement. Camera capture and application redirection are activated only by an explicitly configured session.
