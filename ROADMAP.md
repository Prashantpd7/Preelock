# Roadmap

## Phase 1 — foundation

Desktop shell, modular Rust engine, persisted sessions/tasks/preferences, actual analytics, capability boundaries, tests and native CI. Initial validation results are recorded in VALIDATION.md.

## Phase 2 — reliability and native lifecycle

Validate Windows and both Mac architectures on real hardware. Add sleep/wake reconciliation, clock-discontinuity tests, larger-history pagination, incremental database writes, persistent diagnostic logs, a startup recovery dialog and backup/export. Add desktop end-to-end automation. Test accessibility and supported minimum OS versions. Establish signing and reproducible release builds.

## Phase 3 — optional focus restrictions

Implement per-platform permission checks, app allowlisting and website restrictions. Respect OS policies for VPN/proxy controls. Add opt-in onboarding, a durable restriction journal, emergency release and crash/reboot recovery. Ship strict sessions only after recovery tests pass.

## Phase 4 — consent-based attention and verification

Optional active-window/screen signals, camera monitoring, local-first attention analysis, distraction detection, reminders and interventions. Add manually reviewable task, lecture and coding verification. Describe evidence and uncertainty; never pretend AI judgments are proof.

## Phase 5 — production distribution

Signed/notarized installers, update signatures, release security review, performance and endurance testing, backup migration validation and explicit privacy documentation. Cloud services remain optional and require separate consent and secret management.
