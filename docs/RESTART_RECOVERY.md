# Restart and camera recovery

Local development build, 2026-10-21 (based on 0.4.1; not a published release).

- A lifecycle checkpoint records the OS boot identity. A confirmed OS shutdown/logoff also writes a durable marker without waiting for the engine lock on the UI thread.
- macOS uses its boot-session UUID and AppKit quit reason. Windows uses LastBootUpTime plus confirmed WM_ENDSESSION; cancelled shutdown queries do not mark a restart. The Windows marker also covers Fast Startup.
- On an OS restart, running/preparing sessions recover their saved remaining time. Browser rules are republished and must be acknowledged before productive time resumes. User-paused sessions stay paused. Offline time receives no focus credit. SQLite checkpoints occur every five seconds, so a sudden interruption can lose up to one checkpoint of recent progress.
- Emergency-exit controls have been removed; the old IPC command rejects requests. Historical records remain readable. Strict sessions cannot be ended early through the app; flexible-session controls keep their existing policy.
- Reopening after same-boot Force Quit still ends the session with the existing warning. A force quit followed by a reboot before reopening cannot reliably be distinguished from abrupt power loss; recovery takes precedence when a new boot is detected. Missing/unreadable boot evidence falls back to the existing interruption behavior.
- Camera device failures no longer revoke session consent. Capture retries every five seconds, and checks existing OS authorization without prompting. Revoked permission stops capture. Retry/start/settings controls remain available in active-session details, including records disabled by older versions.
- Capture stops when the session finishes, is user-paused, or camera consent is withdrawn. App-only sessions never start capture. While a camera session is active, capture continues checking for the person's return; an empty frame does not permanently switch off the camera. No video is saved.
- The first-run setup now offers Open at login alongside camera and notification permissions. OS permission decisions and login registration remain OS-owned. The upgraded guide is shown once to existing users too.
- Windows camera capture remains unsupported; this change does not claim to add it. macOS can require permission again after an unsigned/ad-hoc application rebuild.

## Verification

Rust regression coverage: same-boot Force Quit, boot change, consumed shutdown marker, preserved time/consent/user pause, browser-policy reactivation, camera outage/stall retries and revoked/restored permission. Frontend coverage: startup opt-in, no camera reprompt when granted, active-session retry and permission recovery controls. macOS ARM release bundle built and signature/resources checked. Windows runtime and a real physical reboot remain manual checks.

## Manual test

1. Open the installed local build; enable Open at login and allow camera in Permissions & setup. Enable/approve the login item in macOS settings if requested.
2. Start a short camera session and use a selected work app. Restart normally, then sign in. The unfinished session should recover; return to the selected app to earn time. For browser sessions, open the paired browser so its extension reconnects.
3. If camera access is unavailable, expand session details: retry, start camera, or open camera settings. Remaining time must not be credited without the selected verification; after recovery it must progress and complete.
4. Verify app-only sessions leave the camera off, and completed sessions release it.
5. Separately force-quit during a session and reopen without rebooting: the original Force Quit warning must remain.
