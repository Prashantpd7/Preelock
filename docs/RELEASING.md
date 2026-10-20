# Installer release and verification

## Confirmed v0.4.0 problem

On 20 October 2026, the downloaded Apple Silicon DMG passed `hdiutil verify`
and matched the GitHub asset SHA-256:

```text
9fa329ec62bffc1a7e5dafb94895e7ec47e92f52b34cd6ca6492cc138b0c4477
```

The app inside had only a linker-generated ad-hoc executable signature:
`Info.plist=not bound`, `Sealed Resources=none`, `TeamIdentifier=not set`.
`codesign --verify --deep --strict` rejected it with
`code has no resources but signature indicates they must be present`.
This is a packaging/signing failure, not a download-integrity failure.

The Windows installer matched the release SHA-256:

```text
c94efc948dee940e0f6526007d35dcc93810deb57072719532e883b6525184d1
```

Its PE certificate table was empty (no Authenticode signature). This establishes
that it is unsigned, not that it cannot install. Windows runtime behavior must
be tested on Windows.

The old CI built apps/installers and uploaded artifacts without distribution
signature verification. HTTP 200 and compilation success do not establish that
an internet-downloaded installer will launch on a customer's computer.

## Free public beta pipeline

The owner explicitly chose free distribution without paid certificates.
The macOS signing identity `-` creates a consistent ad-hoc bundle signature.
It does not establish publisher trust or notarization. Windows NSIS installers
are unsigned. No signing secrets are required. Optional paid signing can be
added in a future separate workflow; it is not a prerequisite for this beta.

Run `.github/workflows/release.yml` (Free public beta release) with workflow_dispatch
from the intended source revision. It pins action commits, Node 24.13.1, Rust
1.99.0, and standard runner OS versions. This public repository uses standard
GitHub-hosted runners; no larger/paid runners or external services are requested.

The workflow validates versions and app tests, builds native Apple Silicon and
Intel DMGs and a Windows x64 NSIS installer, then verifies the final artifacts.
Mac checks cover the bundle seal, architecture, identifier, version, camera
usage declaration/entitlement, and byte-for-byte extension resources. The app
is copied outside the checkout and started twice on each disposable runner;
a freshly created profile is checked for database integrity and persistence of
a seeded preference. This is not a UI interaction or Gatekeeper approval test.

Windows checks install the actual NSIS executable silently into a temporary
folder, compare the installed app and resources, start the process, check the
fresh SQLite database, uninstall, and ensure user data is preserved. The runner's
existing WebView2 runtime does not prove installation on a machine missing it.
Interactive installer/security dialogs remain a manual check.

Only after every job succeeds does the workflow stage all three installers in
a draft, generate SHA256SUMS.txt, attach the installation guide, download the
uploaded files again, check their hashes, and publish a prerelease. Existing
tags/releases are never overwritten. A failed upload/check leaves a draft for
inspection, not a partly validated public release. Investigate before retrying;
use a new version if an existing release/tag has already been created.

## Release procedure

1. Synchronize package.json, package-lock.json, src-tauri/tauri.conf.json,
   Cargo.toml, and the root package entry in Cargo.lock. Run
   `node scripts/release-version.mjs` to check them.
2. Update RELEASE_NOTES.md, commit, push without force, then dispatch the release
   workflow from the intended branch/revision.
3. Inspect every job result. Do not bypass failed integrity or installation checks.
4. After publication, download the public files and compare SHA256SUMS.txt.
5. Update README direct links only to assets confirmed to exist. Mark the old
   release as superseded without removing historical artifacts.
6. Test the browser-downloaded installers on real devices. Keep beta status
   while security approval, permissions, extension pairing and focus flows have
   incomplete real-device coverage. Record actual results separately from CI.

## Manual validation checklist

- Download through a browser, mount/install outside the repository, and launch
  normally. Record exact Gatekeeper/SmartScreen behavior without removing
  quarantine or changing global security policies.
- On Mac, use Privacy & Security → Open Anyway only where offered and appropriate.
  A missing override is a limitation, not permission to bypass policy.
- On Windows, test the interactive installer, missing-WebView2 setup, supported
  SmartScreen approval (if offered), Start menu launch, and uninstall/reinstall.
- Create a task, change a preference, quit/reopen, confirm persistence, and run
  a short focus session. Preserve existing user data and end the test session.
- Test camera consent on Mac, notifications, browser pairing, filtering and
  cleanup. Windows camera presence is not implemented. The current adapters do
  not request Accessibility, Screen Recording, or Input Monitoring permission.

See [INSTALLATION.md](INSTALLATION.md) for beginner-facing instructions and
[RELEASE_NOTES.md](RELEASE_NOTES.md) for the public beta's verification limits.
