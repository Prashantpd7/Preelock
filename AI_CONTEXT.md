# VYSTRIQ project context for AI agents

This document is the repository-level handoff for an AI agent working on VYSTRIQ.
Read it before changing code. Prefer the existing architecture and patterns over
introducing new frameworks or storage systems.

## Product purpose

VYSTRIQ is a local-first focus and productivity desktop application for macOS
and Windows. Users create focus sessions, tasks, and optional focus policies.
During a session the app can track productive time, monitor attention signals,
hide or minimize distracting applications, and pair with a Chromium extension
to filter browser sites.

The product has no account system, cloud backend, telemetry service, API key, or
remote database. Sessions, tasks, preferences, activity history, and pairing
state belong to the local operating-system user. A new computer/user profile
starts with an empty local database.

The app is designed to make a deliberate focus commitment strong, but it is
not an administrator-proof lock. Users can use emergency exit controls, force
quit the process, disable permissions, or bypass browser restrictions outside
the supported extension path. Do not describe enforcement as technically
impossible to stop.

## Current release and downloads

**Free distribution policy (20 October 2026):** the owner accepts OS warnings
and does not want paid signing services. v0.4.0's Mac installer has an invalid
bundle signature. v0.4.1 repairs the bundle seal using ad-hoc signing; it is not
Apple-notarized. Windows remains unsigned. See [RELEASING.md](docs/RELEASING.md)
and [INSTALLATION.md](docs/INSTALLATION.md). Do not require paid credentials or
claim that every security policy permits manual approval.

- Product version: `0.4.1`
- Extension manifest version: `0.4.1`
- Extension/bridge behavior protocol: `3`
- Repository: `Prashantpd7/VYSTRIQ`
- Release: https://github.com/Prashantpd7/VYSTRIQ/releases/tag/v0.4.0
- Apple Silicon macOS DMG:
  https://github.com/Prashantpd7/VYSTRIQ/releases/download/v0.4.0/VYSTRIQ_0.4.0_aarch64.dmg
- Windows x64 installer:
  https://github.com/Prashantpd7/VYSTRIQ/releases/download/v0.4.0/VYSTRIQ_0.4.0_x64-setup.exe

The macOS artifact is for Apple Silicon Macs (M1/M2/M3/M4 and newer). The
Windows artifact is a 64-bit NSIS installer. The release assets contain the
application and bundled extension program files, not the developer's database.

## Technology stack

### Frontend

- React `19.3.1`
- TypeScript `6.0.3`
- Vite `8.3.4`
- React Router `7.18.4`
- Zustand `5.0.15`
- Lucide React icons
- Tailwind CSS `4.3.3` through the Vite plugin
- Vitest, Testing Library, jsdom
- ESLint `10` and Prettier `3`

Frontend source is under `src/`:

- `src/components/`: reusable UI, session cards, task controls, break controls
- `src/pages/`: Dashboard, Sessions, Tasks, Activity, Analytics, Settings
- `src/layouts/`: application shell/navigation
- `src/services/`: typed Tauri IPC and settings/productivity services
- `src/state/`: serialized Zustand state updates
- `src/types/`: frontend domain contracts
- `src/tests/`: UI and analytics tests

### Desktop and backend

- Tauri `2.12.x`
- Rust edition `2021`
- Minimum Rust version `1.90`
- SQLite through `rusqlite` with the bundled SQLite library
- Serde and Serde JSON for IPC and persisted payloads
- `tiny_http` for the authenticated local browser bridge
- UUID v4 for client/session identifiers
- `env_logger` and `log`

Rust source is under `src-tauri/src/`:

- `lib.rs`: Tauri commands, startup, tray, close/quit handling
- `engine.rs`: session engine, persistence, lifecycle, bridge and native work
- `database.rs`: SQLite repository and migrations
- `models.rs`: session, task, preferences, snapshot, and action types
- `sessions.rs`: session state machine and validation
- `focus.rs`: focus configuration, policy modes, domains, and browser selection
- `browser.rs`: authenticated loopback bridge and browser client state
- `attention.rs`: attention signals and focused/away/distracted accounting
- `platform/`: macOS and Windows platform adapters
- `desktop_settings.rs`: native notifications, sound, and system settings
- `settings.rs`: preferences, backup/restore, CSV export and settings actions
- `setup.rs`: extension deployment and platform setup links
- `capabilities.rs`: truthful platform capability declarations
- `clock.rs`: wall-clock and suspend-aware monotonic timing

### Native platform code

- `src-tauri/native/macos.m`
- `src-tauri/native/probe.m`

macOS native code uses AppKit, AVFoundation, Vision, CoreGraphics, CoreMedia,
CoreVideo, UserNotifications, and ServiceManagement where available.

Windows uses `windows-sys`, `winreg`, `rfd`, and Tauri notifications. Windows
platform code handles application enumeration, foreground/idle sampling,
minimization, camera/permission capability checks, and browser discovery.

### Browser extension

`browser-extension/` is a Chromium Manifest V3 extension:

- `worker.js`: service worker, pairing, heartbeats, policy application
- `identity.js`: browser identity detection and stored browser choice
- `rules.js`: declarative network filtering rules
- `popup.*`: extension popup UI
- `blocked.*`: blocked-page UI
- `self-test.js`: extension installation/matcher self-test
- `manifest.json`: MV3 permissions, service worker, popup, and host access

The extension uses `declarativeNetRequest`, `declarativeNetRequestFeedback`,
`storage`, `alarms`, and `tabs`. It communicates only with the authenticated
loopback bridge at `127.0.0.1:47832`.

## Runtime architecture

### Startup

1. Tauri starts and resolves the per-user application-data directory.
2. The app opens `vystriq.sqlite`.
3. Database migrations are applied up to schema version 4.
4. Historical live sessions are recovered safely; offline elapsed time is never
   credited.
5. The native browser bridge starts on `127.0.0.1:47832`.
6. The current extension files are copied from bundled resources into the
   user's application-data extension directory.
7. The frontend reads a `Snapshot` through Tauri IPC.

### Frontend/backend IPC

The frontend uses `@tauri-apps/api/core`:

- `snapshot`: read current sessions/tasks/preferences
- `execute`: apply a typed `Action`
- `capabilities`: read platform capability declarations
- `focus_setup`: inspect browsers, extension path, pairing and bridge status
- `open_setup`: open extension/settings locations
- `request_camera`: request camera permission
- `reset_browser_pairing`: rotate the local browser pairing token
- `settings_action`: backup, restore, CSV export, and settings operations

The browser bridge is not a general web server. It authenticates requests with
the local `browser-token`, validates origins and payloads, and tracks each
extension client by UUID.

### Focus/session behavior

Session statuses are:

`ready`, `preparing`, `running`, `paused`, `completed`, `ended`, and
`interrupted`.

The engine owns accepted productive-time accounting. Distracted, absent, and
unknown intervals do not earn productive credit. A session can include:

- standard or strict focus mode
- allowlist/blocklist/automatic website policy
- selected browsers
- selected applications
- optional attention tracking
- optional local camera presence checks
- pause policy
- deliberate emergency exit

When a live session is active, closing the window and application quit requests
are guarded. The tray shows the live timer instead of a normal Quit item.
When idle, the window can hide to the tray and the tray exposes Quit.

Hide/minimize failures for an individual application are tolerated so one
uncooperative application does not crash or block the complete session.

### Attention and camera

`attention.rs` classifies signals such as focused, distracted, absent, unknown,
and recovery. Away time is accumulated from absent intervals. Camera analysis
is optional and local; camera data is not uploaded. A restarted camera session
cannot silently assume the camera is available and exposes a warning when it is
not.

## Data and privacy

The app stores data per operating-system user:

- macOS: `~/Library/Application Support/com.vystriq.desktop/`
- Windows: `%APPDATA%\com.vystriq.desktop\`

Important files include:

- `vystriq.sqlite`: sessions, tasks, preferences, attention intervals, audits
- `browser-token`: local bridge authentication secret
- `browser-extension/`: deployed extension files for the paired browser

The SQLite schema uses versioned migrations:

- `001_initial.sql`: sessions, tasks, preferences
- `002_focus_audit.sql`: focus audit events
- `003_attention.sql`: attention intervals
- `004_deleted_tombstones.sql`: permanent deleted-session tombstones

Deleted sessions are filtered through tombstones so stale writers or app
restarts cannot resurrect them. The database is not packaged into the DMG or
Windows installer.

Removing only the application does not necessarily remove the per-user data
directory. For a completely fresh local test, quit VYSTRIQ and delete only
`com.vystriq.desktop` in the platform-specific application-data directory.
This permanently deletes that user's local sessions, tasks, settings, pairing,
and history.

## Extension pairing and browser support

The extension is paired to the desktop app using the local token. The desktop
app deploys bundled extension files into the app-data directory and the user
loads/pairs the supported Chromium profile through the setup flow.

Supported browser IDs and resolution logic are in `focus.rs` and `browser.rs`.
Comet can report a Chrome-like identity in some service-worker contexts, so
browser resolution contains explicit handling for installed browsers.

The extension sends heartbeats containing:

- client UUID
- raw browser identity
- applied policy ID
- protocol version (`v: 3`)
- block/error/activity state

Do not reintroduce a hard session-start failure based only on
`extension_stale`. Some Comet service workers have reported `ext_v=0` while
being authenticated, connected, correctly identified, and using the current
extension files. Connection, authentication, and browser identity checks must
remain enforced.

## Backup and restore

Backups are explicit user actions handled by `settings.rs`. Backup payloads are
validated, versioned, and marked with:

```text
format: vystriq-backup
version: 1
```

Restored live sessions are converted to interrupted sessions; offline time is
not credited. Restored data does not restore a browser pairing token.

## Development commands

Run from the repository root:

```sh
npm ci
npm run dev
npm run desktop
npm run build
npm run check
npm run format:check
npm run rust:check
npm run rust:test
npm run desktop:build
```

`npm run dev` is only a frontend preview and deliberately does not persist
desktop data. Use `npm run desktop` for the real Tauri app.

The correct Rust test command is:

```sh
cargo test --manifest-path src-tauri/Cargo.toml
```

There is no root-level `Cargo.toml`.

## Validation and release workflow

GitHub Actions is defined in `.github/workflows/ci.yml` and validates:

- frontend type checking, linting, tests, and formatting
- Rust formatting, Clippy with `-D warnings`, and Rust tests
- macOS ARM64 app build
- macOS x86_64 app build
- Windows x86_64 NSIS installer build

Build artifacts are uploaded under the target-specific artifact names. A
successful Windows CI build can be attached to a GitHub release as the `.exe`.
The public `v0.4.0` release already contains both the Mac DMG and Windows
installer.

Presence on a release is not distribution verification. CI checks the Mac
bundle seal. `.github/workflows/release.yml` builds free installers, verifies
final packaging and runner install/startup behavior, downloads and checks the
uploaded hashes, and publishes all platforms together as a public beta. No
signing secrets are required. Real-device security/permission/extension testing
must be reported separately. Never overwrite old releases or delete user data.

## Important constraints for future changes

- Do not add an account service, cloud database, telemetry, or network sync
  without an explicit product decision.
- Do not package personal SQLite files, browser tokens, logs, or local backups.
- Preserve per-user app-data paths and fresh-install isolation.
- Do not silently delete user data during ordinary upgrades.
- Keep browser communication authenticated and loopback-only.
- Keep extension protocol changes synchronized between `worker.js`,
  `browser.rs`, and extension tests.
- Preserve truthful capability reporting; do not claim universal OS-level
  enforcement.
- Preserve deleted-session tombstones and migration compatibility.
- Test both macOS and Windows conditional code when changing platform logic.
- Use targeted tests first, then the full checks before release.
- Do not modify the extension version check as a shortcut for unrelated issues.

## Known manual tests

Automated CI does not replace these manual checks:

1. Start a live session and verify window close, `Cmd+Q`/application Quit, and
   tray Quit are blocked until the session ends.
2. In a supported Chromium browser, enable extension access in Incognito and
   verify the configured distracting sites are blocked in an Incognito window.
3. On Windows, test installer installation, permissions, browser pairing,
   camera behavior, application minimization, and uninstall/reinstall.
