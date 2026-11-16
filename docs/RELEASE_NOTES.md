# Preelock 0.5.9 — Free public beta

Preelock 0.5.9 delivers target-tab website blocking, address-bar autocomplete protection, adult website blocking parity, and browser window stability:

- Target-tab website blocking: Committed navigations to blocked websites (such as Instagram timed locks) redirect only the originating tab to Preelock's existing blocked page (`blocked.html`) with "Stay with your task" and clear site attribution. The browser window remains fully open, other tabs (e.g. YouTube media playback, GitHub, work tabs) remain completely undisturbed, and no desktop application popups or "Back to work" overlays appear.
- Address-bar autocomplete & typing protection: macOS AX and Windows UI Automation samplers check keyboard focus on the address bar edit element. Inline autocomplete suggestions and partially typed search terms are never treated as committed navigations and never trigger false-positive blocks.
- Adult website blocking parity: Navigating to adult domains redirects directly in-tab to the existing Preelock blocked page displaying "Adult websites are blocked by Preelock." Native window minimization, desktop application foregrounding, and external native dialogs are completely removed.
- Native enforcement separation: Desktop application locks and browser website locks are strictly separated in the engine. Website locks are enforced solely by the browser extension inside the tab; native window hiding (`platform::hide`), AppleScript tab-closing, and `Cmd+W` keystroke synthesis for websites have been eliminated.
- YouTube responsive layout & churn resilience: Route-scoped container selectors and lifecycle listeners (`resize`, `focus`, `pageshow`) maintain home feed, recommendation sidebar, and Shorts restrictions across viewport changes and sidebar activation.
- Chrome Web Store release package: Every release now automatically generates and attaches a version-matched Chrome Web Store upload archive (`preelock-focus-extension-v0.5.9-chrome-web-store.zip`) for streamlined extension updates.
- Retains full support for focus sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.9.0, with the existing identity and publisher.
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
