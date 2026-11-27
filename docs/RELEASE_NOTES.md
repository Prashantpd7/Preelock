# Preelock 0.5.19 — Free public beta

Preelock 0.5.19 hardens Windows hosts-file blocking and clears the firewall rules on session stop:

- Robust Windows hosts-file writes: Open the system hosts file with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` so the Windows Defender real-time scanner and DNS client can keep their legitimate handles without blocking Preelock. Retries `ERROR_SHARING_VIOLATION` and `ERROR_LOCK_VIOLATION` with a bounded backoff (~1.1 s total) before giving up, so a transient scanner lock no longer surfaces as a permission failure.
- Verified hosts-file writes: After every hosts-file write the Preelock block markers (`# === PREELOCK BLOCKLIST START ===` / `=== END ===`) are re-read and checked; a block that didn't land is no longer reported as applied.
- Cleaner Windows session stop: Ending or unblocking a session now also clears the Preelock firewall ruleset, and unblocking verifies that _both_ the hosts-file markers and the firewall rules are gone — preventing the "Enable System Blocking" UAC loop from reappearing on the next launch.
- Cross-platform parity: The macOS path is unchanged and the three reload-related CI gates stay green on Apple Silicon, Intel, and Windows MSVC.
- Retains full support for Focus Sessions, tasks, analytics, screen time, allowances, schedules, and local-first privacy.

Existing database paths, backup format, browser pairing and Microsoft Store
registration identity are retained. Database migration is additive and backward-compatible.
The Store package is version 1.5.19.0, with the existing identity and publisher.
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
