# Preelock 0.5.6 — Free public beta

Preelock 0.5.6 introduces System-Wide Adult Website Blocking with an embedded
47,663-domain hash set and 7-day non-cancellable strict mode, strict mode for
application and website locks with persistent countdowns, background LaunchAgent
and Windows supervisor persistence with automatic recovery after OS restart,
SQLite database corruption protection and automatic schema backups, and hardened
screen-time tracking with dedicated reading/coding and video playback thresholds.
Focus sessions, tasks, analytics and local-only privacy behavior remain unchanged.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.6.0, with the existing identity and publisher.
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
