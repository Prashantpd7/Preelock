//! Suspend-aware monotonic clocks on supported desktops; wall clock is only for restart recovery.
#[cfg(target_os = "macos")]
pub fn continuous_ms() -> u64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    static SCALE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let (n, d) = *SCALE.get_or_init(|| {
        let mut t = Timebase { numer: 0, denom: 0 };
        unsafe {
            mach_timebase_info(&mut t);
        }
        (t.numer, t.denom.max(1))
    });
    ((unsafe { mach_continuous_time() } as u128 * n as u128) / d as u128 / 1_000_000) as u64
}
#[cfg(target_os = "windows")]
pub fn continuous_ms() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn continuous_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
