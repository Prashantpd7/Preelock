# Architecture

React routes render an authoritative Rust snapshot. Zustand serializes IPC reads and writes to prevent stale polling responses from replacing mutations. The frontend refreshes once per second; React never owns session timing. React Router uses hash navigation for packaged assets. Tailwind's Vite integration and semantic CSS provide the theme and responsive shell.

## Native domain and persistence

Tauri exposes three commands: snapshot, execute and capabilities. The main local window has narrowly scoped custom permissions. No shell, filesystem, HTTP, camera, screen or notification plugins are exposed. The CSP limits scripts to bundled resources and connections to local IPC. Remote pages are not authorized.

A mutex serializes the Rust engine. Mutations clone the snapshot, validate a typed action, commit all affected records in a SQLite transaction and publish the new state only after success. Tables hold versioned JSON domain records, keyed by UUID, with a separate preferences record. This keeps Phase 1 migrations small; indexed columns and incremental queries should replace full snapshot writes before large histories are supported. User version protects against opening a newer schema. Startup errors preserve the database rather than silently resetting it.

SQLite uses WAL and full synchronous commits. The database is inside Tauri's application data directory. Do not copy a live database without its WAL; quit before backing up. Data is local but not encrypted at rest.

## Session engine

`ready → running ⇄ paused → completed/ended`. Only one running or paused session is allowed. A no-pause policy rejects pause in Rust. Ending is always available; no-pause is not an OS lock. Completion caps focused duration at the planned duration. A Rust worker advances the engine independently of webview rendering. Closing the app leaves persisted state for restoration.

While running, `Instant` supplies monotonic elapsed milliseconds. Mutations checkpoint remaining time and a wall-clock deadline. On ordinary restart, the deadline reconciles elapsed time, including time while the application was closed. A paused session stays paused. This is timer time, not proof of attention. Idle detection is not implemented.

System-clock changes while the process is running do not change measured focus duration, but can affect timestamps. Clock changes while the process is closed can shorten or extend inferred elapsed time; restoration clamps remaining time to the saved amount. Suspension behavior of monotonic clocks varies across operating systems; sleeping may pause the in-process countdown. These timers are not tamper-resistant. Changing the clock between persisted checkpoints and restart can change recovered elapsed time. Sub-millisecond truncation on ticks is possible. Phase 2 must define and test sleep/wake and clock discontinuity policy before strict enforcement.

Analytics include actual recorded elapsed time in ended, completed and active sessions. Focus is assigned to the local calendar date the session started, including sessions spanning midnight. Tasks are independent of sessions; verification is manual.

## Future subsystem boundaries

`capabilities.rs` defines PermissionProvider, FocusRestrictions, ActivityMonitor, CameraMonitor, AttentionAnalyzer and TaskVerifier. Windows and macOS adapters are compile-time modules. The capability list reports unavailable features; it never requests permission or starts monitoring. Future task verification covers lecture comprehension and coding evidence. Network policy and reminders require separate implementations in later phases.

All monitoring must be explicit opt-in, visible during operation and separately revocable. AI should consume only consented inputs and default to local processing. Raw camera/screen data must not be retained by default. API secrets must stay outside frontend bundles.

## Emergency recovery contract

Phase 1: choose End session or quit the application; no OS setting needs recovery. Future restriction adapters must implement idempotent emergency_release, keep a durable journal of changes, provide a visible emergency release control and independent OS-accessible recovery path, and restore settings after crash/reboot. Never make irreversible device locks or suppress OS emergency controls. Test recovery before enabling any enforcement capability.
