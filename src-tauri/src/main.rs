#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // Step 15 CLI: a known CLI flag runs one command and exits
    // without launching the GUI; anything else launches the GUI
    // exactly as before (see `cli::is_cli_invocation` — unknown
    // flags are treated as launcher arguments, not CLI commands).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if preelock_lib::cli::is_cli_invocation(&args) {
        std::process::exit(preelock_lib::cli::run(&args));
    }
    preelock_lib::run();
}
