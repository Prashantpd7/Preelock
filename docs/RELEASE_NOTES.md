# Preelock 0.5.2 — Free public beta

Preelock brings the new product name to the desktop app, browser extension,
installers, notifications and documentation. The existing V-shaped artwork is
retained. Focus sessions, tasks, analytics and local-only privacy behavior remain.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. No data migration or deletion is performed.
The Store package is version 1.5.2.0, with the existing identity and publisher.
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
