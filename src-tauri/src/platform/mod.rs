//! Compile-time adapter boundaries; OS permissions are not requested in Phase 1.
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;
pub fn name() -> &'static str {
    std::env::consts::OS
}
