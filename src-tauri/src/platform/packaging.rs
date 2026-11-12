//! MSIX (Store) packaging behaviour that is not platform-specific.
//!
//! Packaged builds behave differently from the direct NSIS download: the
//! package installs under `%ProgramFiles%\WindowsApps`, HKCU writes are
//! virtualised into a per-package hive, and files created under AppData are
//! private to the package. These helpers are pure so they can be tested on any
//! development machine; the Windows adapter supplies the real inputs.

/// TaskId declared by `desktop:StartupTask` in the MSIX manifest.
pub const STARTUP_TASK_ID: &str = "Preelock";
/// Legacy TaskId from prior versions (before standardization on Preelock),
/// retained strictly for migration / cleanup on upgrade.
pub const LEGACY_STARTUP_TASK_ID: &str = "VYSTRIQ";
pub const BACKGROUND_STARTUP_TASK_ID: &str = "PreelockBackground";

/// File stem of the dedicated MSIX background executable. M7: this
/// is the stem `main.rs` (via `launch::launch_mode`) treats as the
/// background copy, and the stem `launch::gui_executable` resolves
/// away from when a spawn must open the GUI. It must stay identical
/// to the file stem of [`BACKGROUND_EXE_FILE_NAME`], which is the
/// `Executable` of the `PreelockBackground` startupTask in
/// `windows/msix/AppxManifest.xml.template` — `scripts/verify-msix.ps1`
/// checks the manifest side of that coupling, the `launch` tests
/// check the dispatch side.
pub const BACKGROUND_EXE_STEM: &str = "preelockbackground";
/// The dedicated MSIX background executable, staged next to the GUI
/// executable by `scripts/package-msix.ps1` (a copy of the same
/// binary under a different name; the name is what selects the mode).
pub const BACKGROUND_EXE_FILE_NAME: &str = "PreelockBackground.exe";
/// The GUI executable, next to the background copy in the package.
pub const GUI_EXE_FILE_NAME: &str = "Preelock.exe";

/// `State` values reported by `Windows.ApplicationModel.StartupTask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupState {
    /// Disabled and the user may enable it.
    Off,
    /// Enabled by the user.
    On,
    /// Enabled by OS policy; the user cannot disable it in the app.
    PolicyOn,
    /// Disabled by OS policy; the user cannot enable it.
    PolicyOff,
    /// The state could not be read.
    Unknown,
}

impl StartupState {
    pub fn from_winrt(value: i32) -> Self {
        // StartupTaskState: Disabled=0, DisabledByUser=1, Enabled=2,
        // DisabledByPolicy=3, EnabledByPolicy=4.
        match value {
            0 | 1 => Self::Off,
            2 => Self::On,
            3 => Self::PolicyOff,
            4 => Self::PolicyOn,
            _ => Self::Unknown,
        }
    }

    /// Existing IPC contract: -1 unavailable, 0 off, 1 on, 2 needs approval.
    /// Windows packaged builds never need the macOS "approve in Login Items"
    /// state, so a policy-disabled task is reported as unavailable instead of
    /// showing a switch the user cannot actually change.
    pub fn login_value(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::On | Self::PolicyOn => 1,
            Self::PolicyOff | Self::Unknown => -1,
        }
    }

    pub fn is_on(self) -> bool {
        matches!(self, Self::On | Self::PolicyOn)
    }
}

/// A Store package always installs into a `WindowsApps` package volume. The
/// package's HKCU writes are private, so the unpackaged `Run` registration is
/// not a valid startup mechanism there.
pub fn is_packaged_exe(path: &str) -> bool {
    path.to_ascii_lowercase().contains(r"\windowsapps\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_executable_name_and_task_id_agree() {
        // The MSIX manifest's background startupTask uses
        // PreelockBackground.exe and `launch` dispatches on the
        // stem: if either side renames one without the other, the
        // packaged background stops starting (or the tray stops
        // dispatching). The manifest side is checked by
        // scripts/verify-msix.ps1; this pins the Rust side.
        assert_eq!(
            BACKGROUND_EXE_FILE_NAME
                .strip_suffix(".exe")
                .unwrap()
                .to_ascii_lowercase(),
            BACKGROUND_EXE_STEM
        );
        assert_eq!(BACKGROUND_STARTUP_TASK_ID, "PreelockBackground");
        assert_eq!(STARTUP_TASK_ID, "Preelock");
        assert_eq!(LEGACY_STARTUP_TASK_ID, "VYSTRIQ");
        assert_eq!(GUI_EXE_FILE_NAME, "Preelock.exe");
    }

    #[test]
    fn winrt_states_map_to_the_existing_login_contract() {
        assert_eq!(StartupState::from_winrt(0), StartupState::Off);
        assert_eq!(StartupState::from_winrt(1), StartupState::Off);
        assert_eq!(StartupState::from_winrt(2), StartupState::On);
        assert_eq!(StartupState::from_winrt(3), StartupState::PolicyOff);
        assert_eq!(StartupState::from_winrt(4), StartupState::PolicyOn);
        assert_eq!(StartupState::from_winrt(99), StartupState::Unknown);

        assert_eq!(StartupState::Off.login_value(), 0);
        assert_eq!(StartupState::On.login_value(), 1);
        assert_eq!(StartupState::PolicyOn.login_value(), 1);
        assert_eq!(StartupState::PolicyOff.login_value(), -1);
        assert_eq!(StartupState::Unknown.login_value(), -1);
        assert!(StartupState::On.is_on());
        assert!(!StartupState::Off.is_on());
    }

    #[test]
    fn only_windowsapps_installations_are_treated_as_packaged() {
        assert!(is_packaged_exe(
            r"C:\Program Files\WindowsApps\Preelock_1.0.0.0_x64__abc\Preelock.exe"
        ));
        assert!(is_packaged_exe(
            r"d:\WINDOWSAPPS\Preelock_1.0.0.0_x64__abc\Preelock.exe"
        ));
        assert!(!is_packaged_exe(
            r"C:\Users\me\AppData\Local\Preelock\preelock.exe"
        ));
        assert!(!is_packaged_exe(r"C:\Program Files\Preelock\preelock.exe"));
    }
}
