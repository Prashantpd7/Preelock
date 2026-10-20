# VYSTRIQ 0.4.1 — Free public beta

This release repairs the invalid macOS bundle signature in v0.4.0 and supersedes
that defective Mac download. Existing user data is preserved. No paid signing
services, accounts, subscriptions, or development tools are needed by users.

## Downloads

Choose `_aarch64.dmg` for Apple Silicon, `_x64.dmg` for Intel Macs, or
`_x64-setup.exe` for Windows x64. Installers and SHA256SUMS.txt are attached below.
Read the attached INSTALLATION.md or the
[installation and permissions guide](https://github.com/Prashantpd7/VYSTRIQ/blob/main/docs/INSTALLATION.md).

## Security and installation

Mac bundles are correctly **ad-hoc signed**, but are **not Developer ID signed
or notarized**. Windows installers are **unsigned**. OS security warnings are
expected. Where offered, use macOS Privacy & Security → Open Anyway or Windows
SmartScreen → More info → Run anyway only if you trust the download. Stricter
policies may disallow these builds. Do not disable system protections.

## Validation and limits

Publication requires frontend tests/lint/type checks, Rust tests/Clippy, both
Mac architecture builds, DMG integrity and contained bundle/resource checks,
Mac runner startup/database persistence, and Windows NSIS installation,
resource checks, process startup, database initialization and uninstallation.
The workflow downloads the uploaded assets and verifies their SHA-256 checksums
before publishing this beta.

These are automated runner checks, not complete real-device testing. Browser
download approval, camera/notification permissions, extension pairing, and
end-to-end focus sessions require manual device testing. macOS 12+ and Windows
10/11 x64 are compatibility targets; not every OS/hardware version was tested.
Camera presence is Mac-only. Browser filtering requires manually loading and
pairing the included extension. No system-wide, administrator-proof blocking
or screen recording is provided.

Please test the installer for your device and report the exact version, OS,
and error through [Issues](https://github.com/Prashantpd7/VYSTRIQ/issues).
