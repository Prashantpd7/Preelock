# Microsoft Store (MSIX) submission preparation

Prepared 21 October 2026 against the 0.4.1 source tree. This document covers the
free MSIX route: a packaged full-trust Win32 app, submitted to Partner Center,
signed by Microsoft after certification. Nothing here has been submitted,
uploaded, or published.

**Status in one line:** packaging automation, the manifest, verification tooling,
the free CI build, and this documentation exist; the package has **not** been
built, installed, or exercised on Windows from this repository yet, so the
product is **not ready to submit** until the Windows steps below are run and
recorded.

## 1. What is in the repository now

| Piece              | Path                                     | Purpose                                                                                                                                    |
| ------------------ | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Manifest template  | `windows/msix/AppxManifest.xml.template` | Package identity placeholders, full-trust entry point, startup task, `runFullTrust`                                                        |
| Packager           | `scripts/package-msix.ps1`               | Stages the built app, renders the manifest, runs `makeappx.exe`, verifies, writes SHA-256 and build info                                   |
| Verifier           | `scripts/verify-msix.ps1`                | Unpacks and checks identity, entry point, startup task, capabilities, Store asset sizes, bundled extension bytes, and absence of user data |
| Local test signing | `scripts/sign-msix-dev.ps1`              | Free self-signed certificate for sideload testing only                                                                                     |
| CI build           | `.github/workflows/msix.yml`             | Free `windows-2022` runner: Windows compile, Rust tests, MSIX build, structural validation, artifact upload                                |
| Privacy draft      | `docs/PRIVACY_POLICY.md`                 | Text to publish at a URL you own                                                                                                           |

The existing macOS DMG and Windows NSIS release pipeline is untouched.

## 2. What only you can supply

### Identity supplied by the owner

The developer account is verified and the product name is reserved. These values
were supplied on 21 October 2026 and are stored in
`windows/msix/identity.json`, which `scripts/package-msix.ps1` uses by default so
local and CI builds cannot drift. Explicit script parameters still override it.

| Value                   | Supplied                                       | Used as                                      |
| ----------------------- | ---------------------------------------------- | -------------------------------------------- |
| Product name            | VYSTRIQ                                        | Display name, listing                        |
| Identity Name           | `Prashantpd7.VYSTRIQ`                          | `Identity/@Name`                             |
| Publisher               | `CN=2A3BE073-2A0A-47AA-958D-9543D60D234A`      | `Identity/@Publisher`                        |
| Publisher display name  | `Prashantpd7`                                  | `Properties/PublisherDisplayName`            |
| Package Family Name     | `Prashantpd7.VYSTRIQ_jkzj39eszd2zgp`           | Informational; printed by `Get-AppxPackage`  |
| Store ID                | `9NCZGZM843N5`                                 | Store listing link                           |

The Identity Name and Publisher must match Partner Center **character for
character**, including case and punctuation. The packager writes both into the
manifest and the verifier checks them; an upload with a different value is
rejected by the Store, not silently accepted.

**Check the Package Family Name before using it in commands.** Microsoft's
publisher hash is 13 base32 characters (`..._8wekyb3d8bbwe`); the supplied value
has 14. The PFN is not part of the package and does not affect this build, but
confirm it after the first install:
`Get-AppxPackage Prashantpd7.VYSTRIQ | Select-Object PackageFamilyName`.

### Still needed from the owner

| Value                            | Used as       |
| -------------------------------- | ------------- |
| Support contact (email or URL)   | Store listing |
| Privacy policy URL               | Store listing |
| Windows screenshots              | Store listing |
| Age rating questionnaire answers | Store listing |
| Pricing and markets              | Store listing |

## 3. Requirements confirmed from Microsoft documentation

- **Registration is free.** The individual registration fee has been waived and
  company onboarding no longer charges the former $99 fee:
  [individual](https://learn.microsoft.com/en-us/windows/apps/publish/whats-new-individual-developer),
  [company](https://learn.microsoft.com/en-us/windows/apps/publish/whats-new-company-developer),
  [open a developer account](https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account).
- **No code-signing certificate purchase is needed** for MSIX Store
  submissions. Microsoft re-signs the package during publishing, after
  certification. Unsigned `.msix` uploads are accepted; MSI/EXE installers are
  not re-signed.
  ([App package requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements))
- **Version rules:** four segments, the fourth reserved by the Store and must be
  `0`; segments are integers 0–65535 and the **first cannot be 0**. The app
  version `0.4.1` therefore cannot be used verbatim; the script maps it to
  `1.4.1.0` and prints the mapping. The package version is what decides Store
  updates, so keep it increasing: use `-PackageVersion` once the mapping would
  go backwards (for example `2.0.0` after `1.9.x`). File names keep the app
  version; `msix-build-info.json` records both.
- **Package formats:** `.msix` (single architecture) or `.msixbundle`. The
  script produces a single `.msix`; only x64 is built today.
- **Windows App Certification Kit (WACK)** is recommended before submission.
  It needs an installed package, so run it on a Windows machine after local
  signing (see §5), not on the unsigned CI artifact.
- **WebView2 runtime:** Windows 11 includes it; most Windows 10 devices have it,
  but it is not guaranteed. See §7.
- **Full-trust packaging** is a supported Store scenario (Desktop Bridge
  "packaged classic app"). `runFullTrust` is a restricted capability that
  requires a justification in Partner Center; text is in §8.

## 4. Packaged vs unpackaged behaviour

These differences are why the code changed, not cosmetic details.

### Startup

An MSIX package installs under `%ProgramFiles%\WindowsApps` and its HKCU writes
are virtualised into a private per-package hive
([packaged desktop app behaviour](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes),
[flexible virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization)).
The old unpackaged `HKCU\...\CurrentVersion\Run` entry therefore cannot work
inside a package: Windows at sign-in never sees it.

- The manifest declares `desktop:StartupTask` with `Enabled="false"`.
  Windows registers the task but starts nothing until the user opts in
  ([desktop:StartupTask](https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop-startuptask)).
- In a packaged build, the app reads and changes that task through
  `Windows.ApplicationModel.StartupTask`, and reports the state Windows actually
  stored. It never enables startup on its own, and a state disabled by the user
  or by policy is shown truthfully rather than overwritten.
- Unpackaged (NSIS) builds keep the existing registry `Run` behaviour, and macOS
  keeps its login item behaviour. Unchanged.

### Extension folder

A package's AppData writes are private: "Files in the virtualized location are
visible only to the app." The bundled extension must be readable by Chrome/Edge
(a separate process) and by Explorer, so a packaged build deploys it to
`%USERPROFILE%\VYSTRIQ\browser-extension` instead of AppData. Unpackaged builds
and macOS keep the existing per-user AppData path. The app shows the real folder
path in **Permissions & setup**.

Consequence: after switching from the NSIS build to the Store build, re-add the
unpacked extension from the new folder. The old extension keeps working for
pairing but stops receiving file updates.

### Data and uninstall

- Existing `%APPDATA%\com.vystriq.desktop` data written by the NSIS build is read
  and updated in place by the packaged app (Windows falls back to the real
  AppData file and writes to it), so upgrading keeps sessions, tasks, settings
  and pairing.
- On a **fresh** packaged install, the database and pairing token are new files
  inside the package's private AppData store and are removed when the app is
  uninstalled. The NSIS uninstaller keeps them. Document this difference to
  users and recommend **Back up** before uninstalling a Store-installed app.
- Upgrades keep both kinds of data; nothing is migrated or deleted by this
  change.

### Everything else

- The browser bridge still binds `127.0.0.1:47832`. Full-trust packaged apps
  need no network capability for loopback or outbound access; only
  `runFullTrust` is declared.
- The extension workload is unchanged; protocol version 3 still applies.
- Notifications: the per-app switch for a packaged app lives under its package
  family name, which the app cannot read reliably, so a packaged build reports
  the global Windows notification setting instead of claiming a per-app state.
- Windows camera presence remains **not implemented** and is reported as
  unavailable by capability declarations, the setup guide, and the IPC command.
  This is unrelated to Store readiness; see §10.

## 5. Build and validate on Windows

Requires Windows 10 1809+ or Windows 11, Node 24.13.1, Rust 1.99.0, the Windows
SDK (for `makeappx.exe`/`signtool.exe`), and WebView2.

```powershell
npm ci
npm run rust:test

# Submittable package: identity comes from windows/msix/identity.json
powershell -ExecutionPolicy Bypass -File scripts/package-msix.ps1 -Build

# Optional: build for a different identity (for example a local test package)
powershell -ExecutionPolicy Bypass -File scripts/package-msix.ps1 `
  -Build -AllowPlaceholderIdentity
```

Outputs land in `src-tauri/target/msix/`: the `.msix`, `SHA256SUMS.txt`, and
`msix-build-info.json` (identity, version, source commit, executable hash).

To re-check an artifact later, including one downloaded from CI:

```powershell
powershell -File scripts/verify-msix.ps1 -Package <path> `
  -ExpectedIdentity "Prashantpd7.VYSTRIQ" `
  -ExpectedPublisher "CN=2A3BE073-2A0A-47AA-958D-9543D60D234A" `
  -ExpectedVersion 1.4.1.0 -ExpectedArchitecture x64
```

### Install and test the packaged app without paying for a certificate

A self-signed certificate is free and adequate for local testing. The
certificate subject must equal the package `Publisher`, so use a validation
build:

```powershell
powershell -File scripts/package-msix.ps1 -AllowPlaceholderIdentity `
  -IdentityName "VYSTRIQ.LocalTest" -Publisher "CN=VYSTRIQ Local Test"
powershell -File scripts/sign-msix-dev.ps1 `
  -Package src-tauri/target/msix/VYSTRIQ_0.4.1_x64-validation.msix `
  -TrustCertificate
Add-AppxPackage -Path src-tauri/target/msix/VYSTRIQ_0.4.1_x64-validation.msix
Get-AppxPackage VYSTRIQ.LocalTest | Select-Object PackageFullName, InstallLocation
```

Launch with Start, or `explorer.exe shell:AppsFolder\<PackageFamilyName>!VYSTRIQ`.
Uninstall with `Remove-AppxPackage -Package <PackageFullName>`.

Signing is for local testing only. Never distribute or upload a self-signed
package, and never add the test certificate to a machine-wide trust store.

### CI

`.github/workflows/msix.yml` (workflow_dispatch) runs on the free `windows-2022`
runner. With the three identity inputs it produces a submittable package; with
them empty it produces a validation-only package. It runs `rust:check` and
`rust:test` so the Windows-only code is actually compiled, then builds, verifies
and uploads the package and checksums. It never signs, uploads to Partner
Center, or publishes.

## 6. Manual Windows test plan (packaged install)

Record actual results; do not infer them from CI.

1. Clean Windows 10 1809+ and Windows 11 x64 machines, no VYSTRIQ data. Install
   the MSIX. Confirm no administrator prompt and no UAC elevation.
2. Launch from Start. Confirm the window renders (WebView2 present) and a fresh
   `vystriq.sqlite` is created with schema version 4.
3. Create a task, a session, change a preference. Quit from the tray when idle.
   Reopen: data persists.
4. Start a session, close the window (X / Alt+F4): the session must not end.
   Try tray Quit and the app's Quit while live: blocked with the existing
   warning. Finish or pause the session normally.
5. **Startup:** Settings → Open at login off by default. Turn on; check
   `Settings > Apps > Startup` shows VYSTRIQ enabled. Sign out/in: the app opens.
   Turn off; sign out/in: it must not open. Disable it in Windows Settings
   instead; the app must show it as off and must not silently re-enable it.
6. **Restart recovery:** during a running session, restart Windows normally.
   After sign-in the session resumes with saved remaining time, no offline
   credit, and browser rules must be re-acknowledged before time accrues.
   Repeat with Shut down (Fast Startup), with a cancelled shutdown (session must
   continue), and with a user-paused session (must stay paused).
7. **Same-boot termination:** during a session, End task in Task Manager, then
   reopen without rebooting. The session must end with the existing Force Quit
   warning. Documented ambiguity: force quit _then_ reboot before reopening is
   indistinguishable from an OS shutdown, so recovery takes precedence.
8. **Browser:** complete extension setup from the folder shown by the app, pair,
   start a browser session, confirm blocking, close/reopen the browser and
   confirm reconnection, end the session and confirm rules are cleared.
9. **Camera:** verify the app never claims Windows camera support, never opens
   the camera, and the camera controls are absent/unavailable on Windows.
10. **Update:** install a higher `Version` package over the existing one
    (same identity). Confirm data survives and the startup-task state is kept.
11. **Uninstall:** confirm the app is removed, no leftover Run key claims
    startup, and record whether local data was removed (see §4).
12. Run the Windows App Certification Kit against the installed package and
    keep the report.

## 7. WebView2 handling

The MSIX declares no bundled runtime and no external dependency element, because
`win32dependencies:ExternalDependency` is only honoured by App Installer and is
ignored by Store deployment. Consequences:

- Windows 11: runtime present by default.
- Windows 10: installed with Microsoft Edge on the vast majority of devices, but
  a machine without it shows no app window. The NSIS installer installs it; the
  Store package has no equivalent hook.
- Mitigation options if you want certainty: keep the NSIS download as the
  documented fallback for old Windows 10 machines, or add a fixed-version
  WebView2 runtime to the package (large, and a separate decision).

Test step 1 above on a clean Windows 10 image with the runtime removed if you
need to know the failure mode.

## 8. Capability justification (paste into Partner Center)

> VYSTRIQ is a Tauri-based desktop productivity app. The packaged app runs as a
> full-trust Win32 process because it needs to read the foreground application
> and idle time through Win32 APIs during a focus session the user starts, show a
> notification-area icon, open Explorer and the user's browser for extension
> setup, and host its own UI in Microsoft Edge WebView2. It installs no service,
> driver, or OS policy, does not require administrator rights, and only reads
> which application is in the foreground, never window titles, keystrokes, or
> screen contents. Browser site filtering is performed by a separately loaded
> Chromium extension the user pairs locally.

Declared capabilities: `runFullTrust` only.

## 9. Store listing copy (only verified functionality)

Short description:

> Local-first focus sessions, tasks, and productive-time tracking for Windows.

Description:

> VYSTRIQ helps you protect deep work. Create a focus session, choose how long
> you want to work, and let VYSTRIQ track the time you actually spend on the
> task. Tasks, sessions and settings stay on your own PC. There is no account,
> no subscription and no cloud storage.
>
> Optional focus rules can minimise distracting desktop applications during a
> session, and the bundled Chromium extension can block distracting websites in
> the browser profile you pair. Attention signals (foreground application and
> idle time) are used only while a session is running, with your consent.
>
> You stay in control: sessions can be paused or ended where the session type
> allows it, the app never hides that it is starting, and quitting during a
> session is guarded with a clear warning. If Windows restarts during a session,
> VYSTRIQ reopens the unfinished session without crediting the time you were
> away.

Feature list (verified in this source tree):

- Focus sessions with pause policy and productive-time accounting.
- Tasks and daily goal.
- Activity and analytics history stored locally.
- Session types with optional application minimisation (Windows foreground
  minimisation; not a security boundary).
- Browser site filtering through the paired Chromium extension (Chrome, Edge,
  Brave, Vivaldi, Comet; profile-scoped, not system-wide).
- Restart recovery that preserves saved remaining time and never credits
  offline time.
- Optional local backup/restore and CSV export you initiate.

Do **not** claim: camera presence on Windows, system-wide/VPN/firewall blocking,
administrator-proof enforcement, phone or task verification, cloud sync,
automatic updates, or notarised/signed distribution outside the Store.

Reviewer notes to include in the submission:

> Browser filtering requires the user to load the bundled unpacked Chromium
> extension (Developer mode) and paste a local pairing code; it is optional and
> not needed for sessions, tasks or time tracking. All focus features in the
> listing work without it. The app writes to `%APPDATA%\com.vystriq.desktop`
> (or the package's private AppData when freshly installed) and to
> `%USERPROFILE%\VYSTRIQ\browser-extension`; it installs no service and requests
> no elevation.

## 10. Validation record (this checkout)

Run on macOS 21 October 2026. Nothing below was executed on Windows.

| Check                                                                                                   | Result                                                                                               |
| ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `npm run check` (typecheck, lint, 63 frontend tests in 10 files, production build)                      | Pass                                                                                                 |
| `npm run format:check`                                                                                  | Pass                                                                                                 |
| `cargo fmt --check` + `cargo clippy --all-targets -D warnings`                                          | Pass                                                                                                 |
| `cargo test`                                                                                            | Pass, 53 tests (51 existing + 2 new packaging tests)                                                 |
| macOS app bundle build (`tauri build --bundles app`)                                                    | Pass                                                                                                 |
| `scripts/verify-macos-bundle.sh` on the built bundle                                                    | Pass: signature valid on disk, designated requirement satisfied, camera usage string present         |
| Manifest template render + XML parse + referenced asset sizes                                           | Pass (all five Store assets exist at the declared sizes)                                             |
| `Windows.ApplicationModel.StartupTask` calls type-checked for `x86_64-pc-windows-msvc` (isolated crate) | Pass                                                                                                 |
| Full `cargo check --target x86_64-pc-windows-msvc` from macOS                                           | Failed at bundled SQLite C compilation (no MSVC toolchain); CI runs this on Windows instead          |
| Packaging scripts parsed by PowerShell 7.6                                                              | Pass                                                                                                 |
| Packaging + verification flow end-to-end with a stand-in `makeappx` (zip) on macOS                      | Pass: staging, identity/token rendering, resources, assets, forbidden-file check, hashes, build info |
| Verifier negative tests (smuggled `browser-token`, wrong publisher)                                     | Pass: both rejected with exit code 1                                                                 |
| Real `makeappx.exe` / MSIX install / launch / update / uninstall / WACK                                 | **Not run.** No Windows host or Windows CI run available here.                                       |

The stand-in packer proves script logic, not MSIX validity. The package bytes
themselves must be produced by the Windows SDK and validated on Windows, which
is what the workflow is for. No package artifact or checksum is claimed in this
repository; the script writes them next to the build output when it runs.

## 11. Known limitations and open items

| Item                                      | State                                                                                                                                                                      |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Package built/installed/tested on Windows | **Not done.** No Windows host or Windows CI run is available from this environment.                                                                                        |
| Windows App Certification Kit             | Not run. Needs an installed package on Windows.                                                                                                                            |
| Screenshots                               | Not captured. Take genuine Windows screenshots; do not fabricate.                                                                                                          |
| Partner Center identity                   | Not available. Placeholders are enforced by the script.                                                                                                                    |
| Privacy policy URL and support contact    | Owner must supply; the draft text is in `docs/PRIVACY_POLICY.md`.                                                                                                          |
| Windows camera presence                   | Not implemented, intentionally. Reported unavailable everywhere.                                                                                                           |
| Single instance at sign-in                | Pre-existing gap: if the app is already running and the startup task launches it again, Windows starts a second process. Not changed by this work; test before submission. |
| Automatic updates                         | None. Store updates replace the package; update compatibility relies on migrations and unchanged data paths.                                                               |
| ARM64                                     | Not built. Only x64 is produced and tested.                                                                                                                                |
| WebView2 on clean Windows 10              | See §7.                                                                                                                                                                    |

## 12. Steps to submit (once §5 and §6 pass)

1. Create the free Partner Center developer account and verify your identity.
2. Reserve the product name; copy Identity Name, Publisher and Publisher display
   name.
3. Run the MSIX workflow with those three values (or build locally), download the
   artifact, and compare its SHA-256 with `SHA256SUMS.txt`.
4. Run the Windows test plan in §6 and the WACK; fix anything that fails.
5. Publish `docs/PRIVACY_POLICY.md` at a URL you control; add the support contact.
6. Create the submission: packages, listing copy from §9, screenshots, category
   Productivity, pricing Free, markets, age rating, privacy policy URL, and the
   `runFullTrust` justification from §8.
7. Submit for certification. Microsoft re-signs the package and publishes it
   after review. Uploading is not the same as approval.
8. Only after publication, update README/`docs/INSTALLATION.md` download links to
   point at the Store listing. Keep the DMG/NSIS downloads working.
