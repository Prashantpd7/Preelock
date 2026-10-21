# Preelock 0.5.0 compatibility and branding audit

Public product spelling: **Preelock**, one word. Browser extension: **Preelock Focus**.
The existing V-shaped artwork is intentionally retained; no new artwork was invented.
Repository: https://github.com/Prashantpd7/Preelock (renamed in place).

## Stable identities and user data

| Retained value                                                             | Reason                                                                                |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `Prashantpd7.VYSTRIQ`                                                      | Existing Microsoft Store package identity                                             |
| `CN=2A3BE073-2A0A-47AA-958D-9543D60D234A`                                  | Existing Store publisher                                                              |
| Store ID `9NCZGZM843N5`, publisher display name `Prashantpd7`              | Existing listing and publisher                                                        |
| MSIX Application Id and startup TaskId `VYSTRIQ`                           | Preserve app registration and startup consent                                         |
| `com.vystriq.desktop` and `vystriq.sqlite`                                 | Reuse existing SQLite data and OS permissions without moving files                    |
| `vystriq-backup`, version 1                                                | Existing backups continue to import                                                   |
| `vystriq-permissions-v2`                                                   | Preserve first-run consent/setup state                                                |
| Existing `%USERPROFILE%\VYSTRIQ\browser-extension`                         | Chromium's registered extension path and pairing must not change                      |
| `browser-token`, bridge port 47832, protocol 3                             | Existing browser authentication and communication                                     |
| Rust crate/library/native symbols and executable `vystriq` / `VYSTRIQ.exe` | Internal build and process paths; bundle and executable product metadata use Preelock |
| Legacy Windows registry keys and shortcuts in upgrade code                 | Locate the previous installation and remove obsolete registration after replacement   |

Fresh Store installations export their extension under `%USERPROFILE%\Preelock`.
Existing extension paths are reused. No SQLite, pairing, session, task or backup
files are moved or deleted. No schema, network, permissions or privacy change is
part of this rebrand. MSIX version **1.5.0.0** exceeds **1.4.1.0**; its fourth
segment remains zero. Package family identity continues to derive from the
unchanged name and publisher. The supplied informational PFN in identity.json
has a previously documented length discrepancy; verify the OS-derived PFN on a
Windows device rather than changing package identity to match that string.

## Windows direct installer upgrade

Tauri's NSIS registry keys include the product name. The installer hook reads
the old per-user uninstall record, verifies the old executable exists and
reuses that installation directory. After successful copying and new registry
registration, it removes only the legacy uninstall record and matching old
shortcuts. New installations use the new product folder. Data directories are
untouched. The old installation folder name may remain as a compatibility path.

On first unpackaged launch, an existing startup Run value is copied to the new
label, including any Windows StartupApproved state, before its old value is
removed. The operation is idempotent and never creates an opt-in for a user who
had no entry. Packaged installations keep their declarative startup TaskId and
`Enabled="false"`. Failure to rename a startup value is logged and leaves the
existing registration available for retry.

The release runner installs the actual 0.4.2 NSIS release, seeds a saved setting
and disabled startup state, upgrades in place, then verifies the preference,
pairing token, display name, registration and resource integrity before uninstall.
This does not establish real-device Store upgrade or interactive security behavior.

## macOS upgrades

The application is `Preelock.app`; bundle identifier and data directory stay
unchanged. Install the new bundle, quit the old app and remove only the old
application bundle after verifying the new app. Do not run both simultaneously.
No automatic removal of a user's old application or data is performed. Regrant
permissions through normal OS controls if requested. Builds use ad-hoc signatures,
not notarization; Gatekeeper is not disabled or bypassed.

## Remaining old-name occurrences

A case-insensitive tracked-source search is reviewed before release. Remaining
occurrences fall into the table above, test fixtures/internal temporary paths,
original author credit, historical validation records, or genuine legacy asset
URLs. `VALIDATION.md`, `docs/VALIDATION_0.4.1.md`, old Git commits and old GitHub
release assets remain historical evidence. The Store guide's old build record
and agent context history remain explicitly historical. Installer scripts also
retain test/placeholder identity names that are not public production branding.
No old visible branding is intended in current runtime copy or new asset names.

## Validation scope

Local checks: frontend type checking, ESLint, 63 frontend tests, production web
build, Rust formatting/Clippy and 53 Rust tests passed. The Rust loopback tests
required execution outside the network-restricted sandbox.

Release gates: both Mac architectures, codesign seal, DMG verification, bundle
labels, resource hashes, disposable-profile startup/persistence; Windows Rust
checks, NSIS upgrade/start/uninstall and MSIX manifest/identity/version/assets.
Uploaded packages are downloaded again and SHA-256 verified before publication.
Final workflow results and checksums are recorded in the release.

Unverified until separately exercised: real-device Store update/install/uninstall,
WACK certification, browser UI pairing/blocking, interactive camera/notification
permissions and OS download approval prompts. Windows camera presence remains
unavailable. No package is submitted to Partner Center automatically.
