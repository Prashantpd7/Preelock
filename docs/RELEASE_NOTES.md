# Preelock 0.5.20 — Free public beta

Preelock 0.5.20 makes a blocked page on macOS actually go away, with or without the browser extension:

- Blocked-tab takeover without the extension: When the Apple Events sweep cannot replace a tab (no Automation consent, or a browser that refuses scripting), Preelock closes the one frontmost tab that is still showing the blocked host. A service-worker-cached page such as YouTube's kept playing from its cached shell after the network was already cut, so the user only ever saw a stream of "blocked" notifications.
- Guarded by re-verification: The native layer re-checks Accessibility trust, that the browser is frontmost, that the focused page really is the blocked host, and that the keyboard focus is not in a text field, then posts a single Cmd+W — the same action Windows already takes. Nothing is read, typed, or copied.
- Diagnosable outcomes: Tab-takeover results are logged once a minute per app/domain, and each reason (missing Accessibility, browser no longer frontmost, user typing, unconfirmed address) is named in the lifecycle log at `~/Library/Logs/Preelock/lifecycle.log`.
- Automation consent is asked for, not silently skipped: A denied macOS Automation grant used to disable blocked-tab replacement without a trace. Preelock now raises the standard per-browser consent prompt once, skips the AppleScript sweep only when macOS explicitly refused it, and re-reads a denial every 30 seconds so enabling System Settings → Privacy & Security → Automation works without relaunching. A one-time notification explains what to enable.
- Loud `osascript` failures: A browser that refuses the tab redirect, or cannot be scripted at all, is now reported in the lifecycle log instead of failing silently.
- Retains full support for Focus Sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.20.0, with the existing identity and publisher.
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
