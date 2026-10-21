# Development

## Prerequisites

Use Node.js 24 LTS, npm and current stable Rust with Rustfmt and Clippy. Dependency lockfiles belong in Git. Native toolchain requirements follow the [official Tauri guide](https://v2.tauri.app/start/prerequisites/); Tailwind uses its [official Vite integration](https://tailwindcss.com/docs/installation/using-vite).

### macOS (Intel and Apple Silicon)

Target macOS 12 or newer. Install Apple's command-line development tools:

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
rustup component add rustfmt clippy
```

Install Node 24 from nodejs.org, or `brew install node@24` if you use Homebrew, and ensure its bin directory is in PATH. Full Xcode may be needed for distribution workflows. Test on the oldest supported OS before release.

### Windows 10/11

In PowerShell, install Node and Rust:

```powershell
winget install OpenJS.NodeJS.LTS
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install Microsoft.EdgeWebView2Runtime
rustup default stable-msvc
rustup component add rustfmt clippy
```

Ensure the Visual Studio installer includes Desktop development with C++ and a Windows SDK. Restart your terminal after installation. Windows builds use the MSVC toolchain; WebView2 is required to run the UI. The NSIS installer runs per user.

## Commands (repository root)

```sh
npm ci
npm run desktop                 # real native app and Vite development server
npm run dev                     # read-only browser preview at 127.0.0.1:1420
npm run check                   # TypeScript, ESLint, frontend tests and build
npm run format:check
npm run rust:check              # Rustfmt and Clippy
npm run rust:test
npm run desktop:build           # native production bundle
npm run desktop:build -- --bundles app   # macOS app only, skip DMG packaging
```

Format using `npm run format` and `cargo fmt --manifest-path src-tauri/Cargo.toml`. `.env.example` intentionally contains no required values. `RUST_LOG=info npm run desktop` enables native diagnostic logs. Logs avoid task titles and monitoring data; operational failures appear in the UI. Startup failures report to stderr in development. Packaged Windows builds do not expose a console; persistent log files and a startup recovery dialog are follow-up work.

The initialization environment lacked Rust; validation installed it temporarily in `/tmp/vystriq-cargo` and `/tmp/vystriq-rustup` without changing shell profiles. While those directories exist, use:

```sh
export CARGO_HOME=/tmp/vystriq-cargo
export RUSTUP_HOME=/tmp/vystriq-rustup
export PATH="$CARGO_HOME/bin:$PATH"
npm run desktop
```

Install Rust normally using the prerequisite commands for a durable setup; temporary directories can be cleaned by the operating system.

## Data and recovery

macOS: `~/Library/Application Support/com.vystriq.desktop/vystriq.sqlite`.
Windows: `%APPDATA%\com.vystriq.desktop\vystriq.sqlite`.

Quit Preelock before backing up the database and any `-wal`/`-shm` sidecars. If startup reports a corrupted or newer database, preserve those files, verify the matching application version and restore a known-good backup. Do not delete user data automatically. To deliberately start fresh, move the closed application's entire data directory to a backup location and relaunch. Ordinary restarts restore session timing; browser-dependent sessions re-enter preparation and must receive fresh extension acknowledgement. Paused sessions remain paused. Phase 2 app redirection stops when Preelock quits. Browser recovery is independently available in the extension; see docs/FOCUS_MODE.md. Reopen hidden/minimized apps normally.

## Builds and CI

CI validates frontend and native Rust, builds Windows x64 NSIS and macOS ARM64/Intel app bundles, and uploads build artifacts. Intel macOS is cross-compiled on the macOS runner; Rust tests run on the runner's host architecture. CI configuration is not proof of successful execution. Use native hardware for release smoke tests. Unsigned builds are development artifacts; signing, notarization, Windows publisher identity, update signing and release distribution are not configured. Gatekeeper may flag unsigned builds.

The bundle identifier is `com.vystriq.desktop`; verify ownership before a public release. Phase 2 began on a clean main branch tracking origin/main. The existing remote and history were preserved; no commits or pushes were made during Phase 2.

## Phase 2 native checks

`npm run native:probe` on macOS builds a read-only native API probe. It verifies application enumeration, foreground identity, idle sampling and the authorization query without requesting camera access or hiding anything. `npm test` also includes extension rule-compiler tests. Rust integration tests bind ephemeral loopback ports; sandboxed environments must permit localhost test servers.

The macOS build compiles `native/macos.m` with Apple's Clang and links AppKit, AVFoundation, Vision, CoreGraphics, CoreMedia and CoreVideo. The bundle includes a camera usage description and camera entitlement; neither grants camera consent. Windows uses supported Win32 APIs and needs no administrator rights for same-user foreground minimization. Elevated/protected windows can refuse access, which interrupts the session visibly.

The browser bridge binds only `127.0.0.1:47832`. A port conflict disables browser setup; the app reports it rather than killing another process. Do not expose this port beyond loopback. Pairing tokens reside in the private application data directory and should not be shared. Reset pairing from Settings when no session is active.
