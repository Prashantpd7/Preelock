//! Process-mode dispatch for the Preelock executable (M7).
//!
//! One binary runs in five modes: the GUI, `--background`, `--tray`,
//! `--attention`, and the step-15 CLI. Which mode runs is decided by
//! the arguments — except in the MSIX (Store) build, where the
//! background also runs as a second copy of the same binary named
//! `PreelockBackground.exe` (see `platform::packaging` and
//! `windows/msix/AppxManifest.xml.template`): its `startupTask`
//! launches that copy with no arguments, and `main.rs` used to decide
//! the mode from the executable name *before* looking at the
//! arguments. That order broke the tray in the packaged build: the
//! background process spawns the tray as `<current exe> --tray`, so
//! under MSIX that was `PreelockBackground.exe --tray`, which the old
//! dispatch sent to `background::run()` — where it exited silently
//! on the already-held `background.lock` and no tray icon ever
//! appeared. The tray's timer click had the same fault in reverse:
//! it relaunched `<current exe>` with no arguments, which under MSIX
//! is the background executable, not the GUI.
//!
//! The rule here, applied by `main.rs` and nowhere else, is:
//! explicit arguments always win over the executable name. `--tray`
//! (and `--attention`) are honoured no matter which copy of the
//! binary runs, and only a bare launch of the background copy —
//! exactly what the MSIX `startupTask` does — runs the background.
//! `gui_executable` resolves the GUI copy for spawns that must open
//! the GUI (tray spawn / tray timer click), so those paths work from
//! either copy.

use crate::platform::packaging;
use std::path::{Path, PathBuf};

/// Which mode this process should run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    /// `--attention`: the camera presence agent.
    Attention,
    /// `--background`, or a bare launch of `PreelockBackground.exe`.
    Background,
    /// `--tray`: the persistent tray process.
    Tray,
    /// A step-15 CLI invocation (`--status`, `--start`, ...).
    Cli,
    /// Anything else: the GUI.
    Gui,
}

/// Decide the mode from the executable's file stem (lowercase, no
/// extension) and the arguments after the program name.
///
/// Precedence is deliberate and pinned by the tests below:
/// `--attention` first (the agent must never be mistaken for
/// anything else), then `--tray`, then the CLI, then the background
/// (explicit `--background` flag, or a bare launch of the dedicated
/// background copy), then the GUI. In particular every explicit
/// argument beats the background executable name:
/// `PreelockBackground.exe --tray` is a tray process and
/// `PreelockBackground.exe --status` is a CLI invocation, not a
/// second background — the copy only runs the background when
/// launched bare, which is exactly what the MSIX `startupTask` does.
pub fn launch_mode(exe_stem: &str, args: &[String]) -> LaunchMode {
    if args.iter().any(|arg| arg == "--attention") {
        return LaunchMode::Attention;
    }
    if crate::tray::is_invocation(args) {
        return LaunchMode::Tray;
    }
    if crate::cli::is_cli_invocation(args) {
        return LaunchMode::Cli;
    }
    if crate::background::is_invocation(args)
        || exe_stem.eq_ignore_ascii_case(packaging::BACKGROUND_EXE_STEM)
    {
        return LaunchMode::Background;
    }
    LaunchMode::Gui
}

/// The executable that opens the GUI, for spawns that must show the
/// GUI (the tray process itself, and the tray timer click that
/// reopens the GUI).
///
/// The MSIX background copy (`PreelockBackground.exe`) sits next to
/// the GUI copy (`Preelock.exe`) in the package; launching it bare
/// runs the background, never the GUI — so from that copy the GUI
/// copy is used when it exists. Everywhere else (macOS, NSIS, dev)
/// the current executable already is the GUI copy and is returned
/// unchanged. A missing sibling falls back to the current executable
/// rather than failing the spawn.
pub fn gui_executable(current_exe: &Path) -> PathBuf {
    let stem = current_exe
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem == packaging::BACKGROUND_EXE_STEM {
        let sibling = current_exe.with_file_name(packaging::GUI_EXE_FILE_NAME);
        if sibling.is_file() {
            return sibling;
        }
    }
    current_exe.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn dispatch_matrix() {
        // Plain GUI launch, either copy.
        assert_eq!(launch_mode("preelock", &args(&[])), LaunchMode::Gui);
        // The MSIX background startupTask launches the background
        // copy with no arguments.
        assert_eq!(
            launch_mode("preelockbackground", &args(&[])),
            LaunchMode::Background
        );
        // Explicit flags work from either copy.
        assert_eq!(
            launch_mode("preelock", &args(&["--background"])),
            LaunchMode::Background
        );
        assert_eq!(
            launch_mode("preelockbackground", &args(&["--background"])),
            LaunchMode::Background
        );
        assert_eq!(
            launch_mode("preelock", &args(&["--tray"])),
            LaunchMode::Tray
        );
        // M7: --tray beats the background executable name. This is
        // the case that used to send the MSIX tray spawn into
        // background::run(), where it died silently on the lock.
        assert_eq!(
            launch_mode("preelockbackground", &args(&["--tray"])),
            LaunchMode::Tray
        );
        // --attention beats everything, including --tray.
        assert_eq!(
            launch_mode("preelockbackground", &args(&["--attention", "--tray"])),
            LaunchMode::Attention
        );
        assert_eq!(
            launch_mode("preelock", &args(&["--attention"])),
            LaunchMode::Attention
        );
        // CLI from either copy; unknown flags are launcher
        // arguments, not CLI commands, so they open the GUI.
        assert_eq!(
            launch_mode("preelock", &args(&["--status"])),
            LaunchMode::Cli
        );
        assert_eq!(
            launch_mode("preelockbackground", &args(&["--status"])),
            LaunchMode::Cli
        );
        assert_eq!(
            launch_mode("preelock", &args(&["--unknown-flag"])),
            LaunchMode::Gui
        );
        // The stem comparison is case-insensitive: Windows file
        // names are not case-sensitive.
        assert_eq!(
            launch_mode("PreelockBackground", &args(&[])),
            LaunchMode::Background
        );
    }

    #[test]
    fn gui_executable_resolves_the_sibling_for_the_background_copy() {
        let dir = tempfile::tempdir().unwrap();
        let background = dir.path().join("PreelockBackground.exe");
        let gui = dir.path().join("Preelock.exe");
        std::fs::write(&background, b"background").unwrap();
        // No GUI sibling yet: fall back to the current executable.
        assert_eq!(gui_executable(&background), background);
        std::fs::write(&gui, b"gui").unwrap();
        assert_eq!(gui_executable(&background), gui);
        // The GUI copy resolves to itself, sibling or not.
        assert_eq!(gui_executable(&gui), gui);
        // Any other executable is returned unchanged.
        let other = dir.path().join("something-else.exe");
        std::fs::write(&other, b"x").unwrap();
        assert_eq!(gui_executable(&other), other);
    }
}
