# Packaging and startup (M7)

How the background engine, the tray, and the main window start at
login on each installer form — and which mechanism owns what. The
runtime pieces (engine, tray, recovery) come from M1–M6; this file
is the wiring map for them.

## Process modes (one binary, `src/launch.rs`)

| Mode           | How it is started                                                                                                       |
| -------------- | ----------------------------------------------------------------------------------------------------------------------- |
| GUI            | Plain launch of `Preelock` / `Preelock.exe`                                                                             |
| `--background` | `Preelock --background`; macOS LaunchAgent; Windows Run key (NSIS); bare launch of `PreelockBackground.exe` (MSIX)      |
| `--tray`       | Spawned by the background process while a session is live; exits when the GUI returns (`tray.lock` guarantees one icon) |
| `--attention`  | Spawned by the background/GUI when a camera session needs it                                                            |
| CLI            | Step-15 flags (`--status`, `--start`, …)                                                                                |

Explicit arguments always win over the executable name: the MSIX
background copy launched as `PreelockBackground.exe --tray` is a
tray process, not a second background. The tray is always spawned
from, and reopens, the GUI executable (`launch::gui_executable`).

## macOS (DMG / `.app`)

- **Background:** `~/Library/LaunchAgents/com.preelock.background.plist`,
  written by the GUI on launch (`background::register_startup`) and
  loaded by launchd at login. `RunAtLoad` + `KeepAlive` with
  `SuccessfulExit=false` (restart on crash/kill, not on a clean
  exit). It is a user-session agent, never a LaunchDaemon — the
  engine needs the login session, the camera, and the browser
  bridge. Stdout/stderr go to
  `~/Library/Logs/com.preelock.background.{stdout,stderr}.log`.
- **Main window:** `SMAppService` login item, registered when the
  user enabled "Open at login" themselves (Settings → General,
  recorded in the `login-user-enabled` marker file) OR while a
  session is live OR a schedule is enabled
  (`lib.rs::login_registration_decision`); the M6 login-window rule
  additionally hides the window at a post-shutdown login when idle
  and the user never enabled "Open at login". Background
  tracking/scheduling always runs regardless, and turning the
  toggle OFF never unregisters the login item while a session is
  live, so mandatory recovery after a shutdown still works.
- **Tray:** spawned by the background when a session is live; the
  GUI waits for the tray lock on relaunch so there is never a
  duplicate icon.
- **Signing:** the DMG build is ad-hoc signed, not notarized
  (`signingIdentity: "-"` in `tauri.conf.json`). That does not
  establish Gatekeeper trust; a Developer ID identity and
  notarization are still owed and need an Apple Developer
  certificate. `scripts/verify-macos-bundle.sh` checks the bundle,
  the camera entitlement, and the bundled extension resources.
- **Permissions:** camera entitlement
  (`com.apple.security.device.camera`) + `NSCameraUsageDescription`.
  TCC grants are per bundle identifier, so the LaunchAgent job, the
  tray, and the attention agent (all the same binary in the same
  bundle) share them. No microphone is used anywhere in the app.

## Windows, NSIS (`Preelock_…-setup.exe`, per-user)

- **Background:** HKCU `Run` value `"Preelock Background"` →
  `"…\preelock.exe" --background`, written by the GUI on launch;
  `RegisterApplicationRestart` re-registers it for the next login.
- **Main window:** HKCU `Run` value `"Preelock"`, same rule as
  macOS — the user's "Open at login" preference, a live session,
  or an enabled schedule; the M6 marker file governs the
  login-window decision.
- **Tray:** spawned by the background when a session is live.
- **Uninstall:** the `NSIS_HOOK_PREUNINSTALL` hook deletes the
  `Preelock`, `Preelock Background`, and legacy `VYSTRIQ` Run
  values, so uninstall leaves no sign-in launch of a deleted exe.
  User data (SQLite, pairing token, settings) is never removed.

## Windows, MSIX (Microsoft Store)

- **No registry Run entries.** HKCU writes are virtualised into a
  per-package hive and invisible at sign-in, so the packaged build
  must not — and does not — use them (`platform::packaging`).
- **Startup:** two `desktop:startupTask` extensions in
  `windows/msix/AppxManifest.xml.template`, both `Enabled="false"`
  (opt-in only): `Preelock` → `Preelock.exe` (main window) and
  `PreelockBackground` → `PreelockBackground.exe` (background).
  The background copy is the same binary staged under a different
  name by `scripts/package-msix.ps1`; `scripts/verify-msix.ps1`
  checks both tasks, that `PreelockBackground.exe` is packaged, and
  that it is byte-identical to `Preelock.exe`.
- The app enables the background task via
  `StartupTask.RequestEnableAsync` (`enable_background_startup`,
  packaged builds only) and the main task only through the user's
  own "Open at login" choice or the session/schedule rule
  (`platform::settings_windows`).
- **No Windows Service.** There is no privileged-enforcement
  service in this build: everything needing the interactive
  desktop, the camera, or the browser runs as the per-user
  background helper above. A service would be the wrong shape for
  all three and is deliberately not added.

## What is NOT verified here (owed, physical device)

Installer, Gatekeeper, MSIX `startupTask` firing, and reboot/login
behaviour are not verifiable on a Linux VM. See `M7-REPORT.md` §6
for the exact owed scenario matrix (login with/without a live
session, schedule firing at login, tray after force-quit, NSIS
uninstall Run-key cleanup, MSIX install → sign-in).
