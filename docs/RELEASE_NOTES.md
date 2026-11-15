# Preelock 0.5.7 — Free public beta

Preelock 0.5.7 delivers critical production stability, lifecycle hardening, and cross-platform bug fixes:

- Single-instance desktop lifecycle: robust lock-file singleton guards on macOS and Windows prevent ghost Dock icons, unresponsive windows, and duplicate application processes. Subsequent launch attempts immediately activate and unhide the primary running window and exit without creating secondary Dock icons.
- Robust Accessibility permission management: centralized cached permission checks with TTL and proactive OS authorization polling eliminate recurring, spurious accessibility prompts on macOS and Windows while protecting active focus sessions.
- Strict-session GUI recovery: instant ~1-second background supervisor relaunch upon GUI force-quit during active strict focus sessions, fully restoring original session parameters and countdown timer.
- Production adult website blocking hardening: complete active tab redirection/closure in Chromium browsers and Safari via native automation events and hide signals, chunked MV3 Declarative Net Request rules across all 47,664 adult domains, and polished, transparent blocked popup UX with accurate domain attribution and escape/close triggers.
- Retains full support for focus sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.7.0, with the existing identity and publisher.
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
