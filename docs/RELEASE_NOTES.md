# Preelock 0.5.11 — Free public beta

Preelock 0.5.11 refines macOS GUI activation lifecycle and supervisor handovers:

- macOS Clean Direct GUI Spawning: Spawning the bundled executable directly rather than routing through LaunchServices eliminates phantom bouncing Dock tiles when multiple activation requests race for the singleton GUI lock.
- Robust UI Supervisor Handover: Background daemon supervisor falls through to check `gui.lock` upon previous GUI PID exit, seamlessly acknowledging an already starting replacement GUI and preventing duplicate instance launches.
- Chrome Web Store release package: Version-matched Chrome Web Store upload archive (`preelock-focus-extension-v0.5.11-chrome-web-store.zip`) for streamlined extension distribution.
- Retains full support for Focus Sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.11.0, with the existing identity and publisher.
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
