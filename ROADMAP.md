# Roadmap

## Phase 1 — existing foundation

Desktop shell, Rust session engine, SQLite persistence, tasks/preferences, actual analytics, tests and CI.

## Phase 2 — voluntary focus controls

Implemented app redirection, Chromium browser request rules, preparation/acknowledgement, commitment/emergency transitions, consented activity summaries, local macOS camera presence, recovery leases and setup UI. Deep Focus and system-wide network control remain unavailable. VALIDATION.md records exactly what was exercised.

## Phase 3 — prove reliability before stronger enforcement

1. Run the installed extension matcher self-test and full session smoke tests in isolated Chrome/Edge profiles on Windows and macOS. Test app hiding/minimizing with disposable windows, actual camera hardware and permission revocation, sleep/wake, offline/network switching, extension removal, disk-full and process crashes.
2. Validate sustained camera and CPU behavior, Vision false positives/negatives and supported minimum OS versions. Build Windows camera integration only with equivalent visible consent and local processing. Do not add phone-use claims without a validated model.
3. Harden the local bridge against malicious same-user clients and slow connections, bind pairing to a specific browser instance, distribute signed extensions and test all lifecycle paths. Improve startup recovery UI and persistent diagnostic logging.
4. Design optional managed deployments with administrator-owned AppLocker/App Control or supported Apple policies, signed/entitled native network integrations, restricted accounts and independent recovery. Audit-only deployment and rollback tests must precede enforcement. Do not expose arbitrary privileged policy commands to the frontend.
5. Add paginated history, per-app retention controls, export/backup and durable user-controlled deletion. Sign/notarize desktop packages and configure signed updates before distribution.

Task, lecture and coding verification remain a separate, optional research phase. Keep evidence reviewable and distinguish probabilistic estimates from verified outcomes.
