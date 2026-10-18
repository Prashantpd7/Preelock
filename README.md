# VYSTRIQ

A local-first desktop foundation for an AI-powered focus, study and productivity application on Windows and macOS. Phase 1 provides real focus sessions, tasks, SQLite storage and analytics. AI and OS-level enforcement are future capabilities.

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
- A Rust-owned monotonic timer, automatic completion and restart restoration.
- Create, rename and complete tasks; persist dark/light/system themes and default duration.
- SQLite transactions, versioned migrations, actual session statistics, clear empty states.
- Capability contracts for future permission-based integrations, currently unavailable.
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
    engine.rs      persistence, monotonic clock and restart handling
    sessions.rs    session state machine and tests
    models.rs      sessions, tasks, preferences, commands
    database.rs    SQLite repository
    capabilities.rs  future subsystem interfaces and availability
    platform/      Windows and macOS adapter boundaries
  migrations/   versioned schema
  capabilities/ local-window access policy
  permissions/  application command permissions
  icons/        native application icons and SVG source
.github/workflows/ci.yml
```

See [development](DEVELOPMENT.md), [architecture](ARCHITECTURE.md), [contribution guidelines](CONTRIBUTING.md) and [roadmap](ROADMAP.md). No cloud account, API key, telemetry or administrator privileges are required.
