# v0.3.1 usability and connection fixes

- Replaced technical session controls with short questions and radio choices; removed the sidebar privacy card; reduced Settings to General and Connections. Advanced details remain expandable.
- Fixed Comet detection using the native browser list, added stable exported extension files and fixed-action setup buttons, and stopped unpaired browser sessions in the wizard. Strict desktop-only sessions no longer require an extension.
- Fixed ignored HTTP acknowledgement failures in the extension; failed connections clear rules and show a short recovery instruction.
- 21 source frontend/extension tests and 27 Rust tests pass, including consent validation, Comet selection, app-only Strict rules, failed browser acknowledgements, hostname-only reporting, timer recovery and emergency cleanup. TypeScript, ESLint, format checks, Clippy, the native read-only probe and ARM64 bundle build pass.
- Launched v0.3.1 in the real Tauri app. Checked the rendered setup screen, empty sidebar area, short Settings page and native Comet detection. Exported extension files match the bundle exactly.
- Test discovery now excludes generated app bundles; earlier frontend totals included duplicate bundled tests.
- Live Comet installation/blocking remains pending user setup: computer control of Comet was denied by the tool. Camera permission and real presence accuracy also need the user. Instructions are shown in Settings. No privileged OS policy or Windows runtime test is claimed.

# Productive-time implementation — v0.3

Current implementation adds Rust attention accounting, conservative recovery, app/domain/camera signal integration, explicit fallback, corrections, SQLite interval journaling and close-to-background lifecycle. Legacy elapsed records remain clearly labeled. Final checks: 28 Vitest tests; 26 Rust tests; strict TypeScript, ESLint, Prettier, Rustfmt and Clippy pass. Production frontend and ARM64 Tauri v0.3.0 bundle built. Read-only native app enumeration, foreground/idle and camera authorization probe passed. No camera permission was granted.

The actual rebuilt app was launched from `src-tauri/target/release/bundle/macos/VYSTRIQ.app`; its running process path, bundle version 0.3.0 and native accessibility tree (`tauri://localhost`) were verified. A real one-minute manual-policy session stayed at 01:00 while UNKNOWN, accepted time only after explicit confirmation, continued after window close/reopen, and ended through typed emergency recovery. The labeled validation session remains in local history. This establishes packaged frontend-to-Rust/SQLite behavior for manual policy, not camera or installed browser enforcement.

Still manual: load/reload and pair the Chromium extension (including new tabs permission), run its popup matcher self-test, test allow/blocked sites, and grant opt-in camera permission to evaluate actual capture/presence accuracy. Foreground APIs were probed, but hiding the user's apps and camera accuracy were not exercised. Windows runtime, phone/head-direction detection, learning/playback/file-progress verification, system-wide network enforcement, managed Deep Focus and signing/notarization remain unverified or unavailable. No public application release was created.

## Historical Phase 2 validation

18 October 2026

Validated on Apple Silicon macOS using Node 24.13.1 and Rust 1.99.1. The existing repository was clean on main tracking origin/main at the start. No commit, push, remote change, destructive OS-policy test or camera permission request was performed.

| Check                                                      | Result                                                                                             |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Strict TypeScript, ESLint                                  | Passed                                                                                             |
| Frontend + extension rule tests                            | 11 passed across 3 files                                                                           |
| Rust domain + real loopback transport tests                | 19 passed                                                                                          |
| Rustfmt and Clippy (warnings denied)                       | Passed                                                                                             |
| Prettier                                                   | Passed                                                                                             |
| Frontend production build                                  | Passed                                                                                             |
| Native macOS read-only probe                               | Passed: running-app enumeration, foreground identity, idle-time API and camera authorization query |
| Tauri macOS ARM64 app bundle                               | Built successfully; includes camera usage metadata and browser extension resources                 |
| Windows adapter cross-target type checking                 | Passed against windows-sys 0.61.2 for x86_64-pc-windows-msvc in an isolated harness                |
| Full Windows/Intel macOS build and real-device enforcement | Not run here; CI configured                                                                        |
| Live app hiding/minimizing                                 | Not exercised against the user's applications                                                      |
| Installed browser extension enforcement/matcher            | Not exercised in a live browser; extension contains an installed-matcher self-test                 |
| Actual camera capture/presence accuracy                    | Not exercised; permission not requested                                                            |
| Native visual smoke test                                   | Not repeated after the Phase 1 screen-capture service failure                                      |

Rust tests cover configuration/domain validation, unsupported capabilities, legacy data, strict commitment and emergency exit, countdown completion, SQLite persistence/restart, browser preparation/acknowledgement/timeout/loss, cleanup, a simulated database write failure, real authenticated loopback requests, token rotation and short-lived network profiles. Frontend tests cover existing navigation/tasks, the new session wizard and pure browser rule compilation. Unit-test acknowledgement helpers are test-only; production requires a real extension acknowledgement.

The native probe is read-only and does not establish that every application's window can be hidden or that a camera model performs reliably. The Windows harness checks Win32 API types, not linking the entire Tauri application or running it. Browser filtering still requires loading/pairing the extension; it is not active merely because the build passes. Run the explicit manual checks in docs/FOCUS_MODE.md before relying on restrictions.

CI now includes the read-only Mac probe alongside the existing frontend, Rust and platform bundle jobs. No CI run is claimed. The final app is at `src-tauri/target/release/bundle/macos/VYSTRIQ.app`. Release signing/notarization and production distribution remain pending.

Phase 3 priorities: live platform/browser/camera testing, managed-policy design with administrator recovery, bridge hardening, retention/backup and signed releases. Deep Focus, system-wide network controls, Windows camera analysis, phone detection and learning/task verification are unavailable.

---

## Historical Phase 1 validation — 18 October 2026

Validated in the existing project root on Apple Silicon macOS using Node 24.13.1, npm 11.8.0 and Rust 1.99.1 (isolated temporary installation).

| Check                                  | Result                                                                                         |
| -------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Dependency installation                | Passed; npm reported 0 vulnerabilities                                                         |
| Strict TypeScript                      | Passed                                                                                         |
| ESLint, zero warnings                  | Passed                                                                                         |
| Prettier                               | Passed                                                                                         |
| Vitest                                 | 6 tests passed across 2 files                                                                  |
| Rustfmt                                | Passed                                                                                         |
| Clippy, warnings denied                | Passed                                                                                         |
| Rust tests                             | 8 passed                                                                                       |
| Frontend production build              | Passed                                                                                         |
| Tauri macOS ARM64 release build        | Passed; 13.21 MiB app bundle                                                                   |
| Windows / Intel macOS builds           | Configured in CI, not run locally                                                              |
| Native visual / interactive smoke test | Blocked by the computer-use service's macOS screen-capture failure (SCStreamErrorDomain -3811) |

Frontend tests cover six-route navigation, task creation through the IPC boundary, chosen session policy, persistence-error reporting, actual analytics and countdown formatting. Rust tests cover pause enforcement, completion capping, concurrent-session rejection, invalid input, wall-clock-independent elapsed time, deadline restoration, paused-state SQLite round-trip and rejected mutation behavior. Frontend tests use a mocked native boundary; they do not establish end-to-end IPC behavior in a running packaged app.

Build output: `src-tauri/target/release/bundle/macos/VYSTRIQ.app`. Its executable was verified as Mach-O ARM64. The final bundle includes the corrected page breadcrumb. Signing/notarization, DMG creation, minimum-OS runtime testing and native interactive workflows remain unverified.

The supplied folder was empty and had no Git metadata or remote. Initialized `main`; all work remains uncommitted, with no remote configured and no push performed. Existing GitHub history must be reconciled before connecting this local initialization to a populated remote.

Remaining limitations: system-clock changes across restarts and OS sleep behavior (see ARCHITECTURE.md), full-snapshot writes for growing histories, no persistent production log file, and a temporary Rust installation. Advanced AI, monitoring and OS restrictions are explicitly unavailable. The next phase should prioritize native lifecycle reliability and real-device end-to-end tests.
