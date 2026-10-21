# Partner Center submission sheet

Field-by-field companion to [STORE_SUBMISSION.md](STORE_SUBMISSION.md). Copy text
from the sections named here; do not retype it, so the listing stays consistent
with the verified feature set. Fields marked **owner** are values only you can
provide.

## Package

| Field           | Value                                                                              |
| --------------- | ---------------------------------------------------------------------------------- |
| Package file    | `Preelock_0.5.0_x64.msix`                                                          |
| SHA-256         | `e9385c23dc5b45f4b48f5cd1d26a7e2853f7e531b411cd85b3cdb49407e9c6e8`                 |
| Package version | `1.5.0.0` (the fourth segment is reserved by the Store and left at 0)              |
| Architecture    | x64 only                                                                           |
| Signature       | Unsigned. The Store re-signs accepted packages; do not upload a self-signed build. |
| Identity        | `Prashantpd7.VYSTRIQ` / `CN=2A3BE073-2A0A-47AA-958D-9543D60D234A`                  |
| Store ID        | `9NCZGZM843N5`                                                                     |

## Store listing fields

| Partner Center field | What to enter                                                                                                                                         |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Display name         | `Preelock`                                                                                                                                            |
| Short description    | Copy the "Short description" block in [§9](STORE_SUBMISSION.md#9-store-listing-copy-only-verified-functionality).                                     |
| Description          | Copy the "Description" block in §9.                                                                                                                   |
| Product features     | Copy the feature bullets in §9. Do not add camera-on-Windows, system-wide blocking, or automatic-update claims.                                       |
| Search terms         | `focus timer`, `deep work`, `productivity`, `pomodoro`, `website blocker`, `task tracker`                                                             |
| Category             | Productivity                                                                                                                                          |
| Pricing              | Free                                                                                                                                                  |
| Markets              | **owner**: choose all markets or a subset.                                                                                                            |
| Copyright            | **owner**: `© <year> <name>`                                                                                                                          |
| Support contact      | **owner**: a monitored email address or support URL. Required.                                                                                        |
| Privacy policy URL   | **owner**: publish [PRIVACY_POLICY.md](PRIVACY_POLICY.md) and paste the public URL. Required.                                                         |
| Website              | Optional. Use only a URL you control.                                                                                                                 |
| System requirements  | Windows 10 version 1809 or newer / Windows 11, x64, Microsoft Edge WebView2 Runtime (included with Windows 11 and installed with Edge on Windows 10). |

## Restricted capability

When Partner Center asks for the `runFullTrust` justification, paste the text in
[§8](STORE_SUBMISSION.md#8-capability-justification-paste-into-partner-center).

## Notes for certification

Paste the reviewer notes in
[§9](STORE_SUBMISSION.md#9-store-listing-copy-only-verified-functionality).
They explain that browser filtering is optional, requires loading the bundled
unpacked Chromium extension, and that no elevation or background service is
installed.

## Age rating questionnaire

Answer from actual behaviour; do not guess at intent.

- No user-generated content, no chat, no social features, no online multiplayer.
- No purchases, ads, or gambling.
- No location, contacts, or microphone access.
- Camera: optional presence checking exists on macOS only; Windows builds report
  it unavailable and do not open the camera. Declare this only if the listing
  covers macOS.
- Data: stored locally, no collection by the developer. See the privacy policy.
- The app can restrict access to websites the user selects inside their own
  browser profile. It is a self-imposed focus tool, not parental control or a
  security product.

## Screenshots

At least one genuine Windows screenshot is required (1366×768 or larger; 16:9
recommended). Capture these from the installed MSIX, not from a mock-up:

1. Dashboard with a task and a scheduled focus session.
2. A running session with remaining time and the focus controls.
3. Analytics or Activity showing completed local history.
4. Settings → Connections → Permissions & setup (camera shown as Mac-only).
5. Optional: Focus setup with browser/application rules.

Do not reuse macOS screenshots in a Windows listing and do not fabricate UI.

## Before you upload

- [ ] Downloaded the artifact from the CI run and matched the SHA-256 above.
- [ ] Ran `scripts/verify-msix.ps1` against the downloaded file.
- [ ] Installed the package on Windows and completed the test plan in
      [STORE_SUBMISSION.md §6](STORE_SUBMISSION.md#6-manual-windows-test-plan-packaged-install).
- [ ] Ran the Windows App Certification Kit and kept the report.
- [ ] Privacy policy published; support contact ready.
- [ ] Screenshots captured.
- [ ] Confirmed the Package Family Name in Partner Center (the supplied value has
      a 14-character suffix; see
      [§2](STORE_SUBMISSION.md#still-needed-from-the-owner)).
