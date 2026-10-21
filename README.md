# Preelock — Free Focus & Productivity App for Windows and macOS

Preelock is a local-first focus and productivity app for Mac and Windows. Create
focus sessions, manage tasks, track productive time, and optionally block
distracting browser sites while you work.

There is no account and no cloud storage. Your sessions, tasks, settings, and
activity stay on your own computer. Every new installation on another computer
starts with empty data.

## Download Preelock — Free for Windows and macOS

Download the [Preelock v0.5.0 release](https://github.com/Prashantpd7/Preelock/releases/tag/v0.5.0):

- [Apple Silicon DMG](https://github.com/Prashantpd7/Preelock/releases/download/v0.5.0/Preelock_0.5.0_aarch64.dmg)
- [Intel DMG](https://github.com/Prashantpd7/Preelock/releases/download/v0.5.0/Preelock_0.5.0_x64.dmg)
- [Windows installer](https://github.com/Prashantpd7/Preelock/releases/download/v0.5.0/Preelock_0.5.0_x64-setup.exe)
- [Microsoft Store MSIX](https://github.com/Prashantpd7/Preelock/releases/download/v0.5.0/Preelock_0.5.0_x64.msix)
- [SHA-256 checksums](https://github.com/Prashantpd7/Preelock/releases/download/v0.5.0/SHA256SUMS.txt)

[Earlier releases](https://github.com/Prashantpd7/Preelock/releases) retain their
historical product names.

Read the [installation and troubleshooting guide](docs/INSTALLATION.md), including
[permission setup](docs/INSTALLATION.md#permissions-and-first-run) and
[browser extension setup](docs/INSTALLATION.md#browser-extension-setup).

Requirements: macOS 12+ (separate Apple Silicon/Intel builds) or Windows 10/11 x64
with WebView2 (installed automatically when needed; internet required).
No development tools, account, or subscription are needed.

Free Mac builds are ad-hoc signed, **not notarized**; Windows builds are
**unsigned**. Security warnings are expected. Use the documented OS approval
controls only when available and you trust the download. Some policies prohibit
these builds. Never disable system protections.

## Your privacy

Preelock does not require an account, API key, or subscription. It stores data
locally in the operating-system application-data folder. The download does not
contain the developer's sessions, tasks, or settings.

Browser blocking and optional camera features require the permissions explained
by the app during setup. You can use the core focus and task features without
creating an online account.
Browser blocking currently also requires loading and pairing the bundled
extension through the setup wizard.

## Known limitations

This is a public beta. Automated build/install checks do not replace real-device
security approval, permissions, and focus-session testing. Camera presence is
Mac-only. Browser filtering requires manually loading and pairing the bundled
Chromium extension; Safari and Firefox are unsupported. Restrictions are not
system-wide or administrator-proof. No screen recording is performed.

Maintainers: [release procedure](docs/RELEASING.md) and
[Microsoft Store MSIX preparation](docs/STORE_SUBMISSION.md).

See the [0.4.1 validation record](docs/VALIDATION_0.4.1.md) for tested and untested flows.
