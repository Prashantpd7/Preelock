# VYSTRIQ 0.4.1 validation record

## Local Apple Silicon test — 20 October 2026

Source: `ecd79bb` (the free public beta release changes).
Machine: Apple Silicon, macOS 26.6.2.

- Built production app and DMG successfully.
- Mounted the final DMG read-only. Its checksum passed.
- Strict code-signature verification passed on the contained app, including
  its resource seal. Architecture, app identifier/version, camera usage string,
  camera entitlement, and all bundled extension resources passed inspection.
- Copied the contained app into a temporary installation folder outside the
  repository, verified it again, and ejected the image.
- Launched that copy using macOS application launching. Confirmed the process
  ran from the test installation, not the development bundle or old installed app.
- First-run Permissions & setup dialog and dashboard rendered successfully.
  The new OS profile data started with no tasks/sessions.
- Created an explicitly named release-verification task through the UI.
- Changed appearance from Dark to Light and saved. Quit normally, reopened,
  and confirmed the task and Light preference persisted. Restored Dark and saved.
- Quit normally. Existing installed app and user files were not deleted or replaced.
  The test created app data and one clearly named test task; those were retained.

This local build was not a browser-downloaded app. Gatekeeper's read-only
`spctl --assess --type execute` assessment rejected it, as expected for this
unnotarized ad-hoc build. No Gatekeeper settings or quarantine metadata were
changed. Successful local launch is **not** evidence that all downloaded-app
approval policies will permit it.

## Automated release checks — 21 October 2026

Release source: `2a923f9bab0f6ae75770f197b178b009dce5d470`.
[Successful GitHub Actions run](https://github.com/Prashantpd7/VYSTRIQ/actions/runs/39554443355).

- 60 frontend tests, type checking, lint, build and formatting passed.
- 45 Rust tests passed on each of Apple Silicon, Intel Mac, and Windows runners;
  Clippy and Rust formatting passed on all platforms.
- Both Mac DMGs passed checksum, contained bundle/architecture/permission and
  resource verification, copy/install outside the checkout, startup, and
  persisted-preference restart checks on native macOS 15 runners.
- Windows x64 NSIS installed on a Windows Server 2022 runner. Installed binary
  integrity (including Tauri's documented NSIS marker patch), bundled resources,
  process startup, fresh SQLite integrity/schema, and uninstall checks passed.
  Uninstallation preserved the test profile's database.
- All three uploaded release installers were downloaded and their SHA-256
  checksums verified before the beta was made public.
- All three public installers were also downloaded on the maintainer's Mac and
  compared with the published checksum file successfully.

The first release attempt correctly withheld publication when its Windows
comparison rejected Tauri's expected bundle-type marker change. The check was
corrected to allow exactly that patch and reject every other byte difference.
No installer integrity gate was skipped.

## Manual testing still needed

- Browser-downloaded Gatekeeper approval on Apple Silicon and Intel hardware.
- Interactive Windows SmartScreen/Smart App Control behavior on real Windows PCs.
- Minimum OS versions and a Windows machine without WebView2 already installed.
- Camera/notification consent, browser extension pairing, private-window filtering,
  focus-session enforcement/recovery, and upgrade testing with existing data.

This is a public beta, not a claim of complete device/feature certification.
