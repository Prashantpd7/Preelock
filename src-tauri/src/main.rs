#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // Step 15 CLI: a known CLI flag runs one command and exits
    // without launching the GUI; anything else launches the GUI
    // exactly as before (see `cli::is_cli_invocation` — unknown
    // flags are treated as launcher arguments, not CLI commands).
    // M7: the mode decision lives in `launch::launch_mode` (one
    // table, unit-tested). Explicit arguments always win over the
    // executable name, so `PreelockBackground.exe --tray` is a tray
    // process, not a second background — the MSIX background copy
    // only runs the background when launched bare, which is exactly
    // what its startupTask does.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe_stem = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        })
        .unwrap_or_default();
    match preelock_lib::launch::launch_mode(&exe_stem, &args) {
        preelock_lib::launch::LaunchMode::Attention => {
            std::process::exit(preelock_lib::attention_agent::run())
        }
        preelock_lib::launch::LaunchMode::Background => {
            std::process::exit(preelock_lib::background::run())
        }
        // M2: persistent menu-bar / system-tray process. Owns the tray
        // icon while the GUI is down; exits when the GUI returns.
        preelock_lib::launch::LaunchMode::Tray => std::process::exit(preelock_lib::tray::run()),
        preelock_lib::launch::LaunchMode::Cli => std::process::exit(preelock_lib::cli::run(&args)),
        preelock_lib::launch::LaunchMode::Gui => preelock_lib::run(),
    }
}
