#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    // Step 15 CLI: a known CLI flag runs one command and exits
    // without launching the GUI; anything else launches the GUI
    // exactly as before (see `cli::is_cli_invocation` — unknown
    // flags are treated as launcher arguments, not CLI commands).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let background_executable = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        })
        .is_some_and(|stem| stem == "preelockbackground");
    if args.iter().any(|arg| arg == "--attention") {
        std::process::exit(preelock_lib::attention_agent::run());
    }
    if background_executable || preelock_lib::background::is_invocation(&args) {
        std::process::exit(preelock_lib::background::run());
    }
    if preelock_lib::cli::is_cli_invocation(&args) {
        std::process::exit(preelock_lib::cli::run(&args));
    }
    preelock_lib::run();
}
