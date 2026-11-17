# Preelock 0.5.10 — Free public beta

Preelock 0.5.10 introduces item-level strict block protection, extension protocol v14 for desktop-driven adult blocking, and background daemon freshness supervision:

- Individual Strict App & Website Locks: Every app lock and website lock can now be strictly locked individually with an explicit confirmation dialog. Active strict blocks cannot be cancelled, removed, or bypassed until their duration has expired.
- Extension Protocol v14: Synchronisation response includes desktop-driven adult blocking status, ensuring clear communication between the desktop engine and the browser extension.
- Background Binary Freshness Detection: Added executable modification-time (`exe_mtime_ms`) tracking so background process supervisors cleanly detect and relaunch freshly rebuilt binaries without stale daemon conflicts.
- Streamlined Planning Sections: Simplified schedule, timer, and allowance controls for focused session management.
- Chrome Web Store release package: Version-matched Chrome Web Store upload archive (`preelock-focus-extension-v0.5.10-chrome-web-store.zip`) for seamless extension distribution.
- Retains full support for focus sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.10.0, with the existing identity and publisher.
It is supplied for owner upload and is not claimed to be Store-certified.

Mac builds are ad-hoc signed, not notarized. Windows direct installers are unsigned.
Windows camera presence monitoring remains unavailable. See the
[installation guide](https://github.com/Prashantpd7/Preelock/blob/main/docs/INSTALLATION.md)
and [compatibility audit](https://github.com/Prashantpd7/Preelock/blob/main/docs/REBRANDING.md).

Release publication requires automated tests, Mac bundle and DMG checks, Windows
NSIS install/start/uninstall checks, MSIX structural checks and uploaded checksums.
Real-device permission, browser pairing, Gatekeeper/SmartScreen and Store upgrade
and certification tests require separate verification. Historical releases remain
available under their original asset names.
