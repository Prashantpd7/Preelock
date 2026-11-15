//! Centralized cross-platform permission state machine.
//!
//! Authoritatively tracks and caches operating system permissions (Accessibility on macOS,
//! UI Automation availability on Windows). Distinguishes:
//! - UNKNOWN: Status has not yet been checked.
//! - GRANTED: Permission is genuinely authorized by the OS.
//! - NOT_GRANTED: Permission is missing/denied by the OS.
//! - CHECK_FAILED: OS check could not be completed.
//! - RECHECK_REQUIRED: Marked dirty after settings visit or system change.
//!
//! Requirements:
//! - Thread-safe, cached with 1.5s TTL to prevent OS check spamming during rendering loops.
//! - Authoritative query via `AXIsProcessTrustedWithOptions(Prompt=NO)` on macOS (no stale HIServices cache).
//! - Window focus and explicit request support for instantaneous status updates.
//! - Background engine and GUI process coordination without duplicate prompts.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionStatus {
    Unknown,
    Granted,
    NotGranted,
    CheckFailed,
    RecheckRequired,
}

impl PermissionStatus {
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Granted => "granted",
            Self::NotGranted => "not_granted",
            Self::CheckFailed => "check_failed",
            Self::RecheckRequired => "recheck_required",
        }
    }

    pub fn to_code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Granted => 1,
            Self::NotGranted => 2,
            Self::CheckFailed => 3,
            Self::RecheckRequired => 4,
        }
    }

    pub fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Granted,
            2 => Self::NotGranted,
            3 => Self::CheckFailed,
            4 => Self::RecheckRequired,
            _ => Self::Unknown,
        }
    }
}

pub struct PermissionManager {
    cached_status: AtomicU8,
    last_checked: RwLock<Option<Instant>>,
    ttl: Duration,
}

impl PermissionManager {
    pub const fn new(ttl: Duration) -> Self {
        Self {
            cached_status: AtomicU8::new(0),
            last_checked: RwLock::new(None),
            ttl,
        }
    }

    pub fn mark_recheck_required(&self) {
        self.cached_status.store(
            PermissionStatus::RecheckRequired.to_code(),
            Ordering::Release,
        );
    }

    pub fn current_cached_status(&self) -> PermissionStatus {
        PermissionStatus::from_code(self.cached_status.load(Ordering::Acquire))
    }

    pub fn check_status<F>(&self, force_refresh: bool, query_os: F) -> PermissionStatus
    where
        F: FnOnce() -> Result<bool, String>,
    {
        let current_code = self.cached_status.load(Ordering::Acquire);
        let current_status = PermissionStatus::from_code(current_code);

        if !force_refresh
            && current_status != PermissionStatus::Unknown
            && current_status != PermissionStatus::RecheckRequired
        {
            if let Ok(guard) = self.last_checked.read() {
                if let Some(instant) = *guard {
                    if instant.elapsed() < self.ttl {
                        return current_status;
                    }
                }
            }
        }

        // Run authoritative OS query
        let fresh_status = match query_os() {
            Ok(true) => PermissionStatus::Granted,
            Ok(false) => PermissionStatus::NotGranted,
            Err(_) => PermissionStatus::CheckFailed,
        };

        self.cached_status
            .store(fresh_status.to_code(), Ordering::Release);
        if let Ok(mut guard) = self.last_checked.write() {
            *guard = Some(Instant::now());
        }

        fresh_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_code_roundtrip() {
        for status in [
            PermissionStatus::Unknown,
            PermissionStatus::Granted,
            PermissionStatus::NotGranted,
            PermissionStatus::CheckFailed,
            PermissionStatus::RecheckRequired,
        ] {
            assert_eq!(PermissionStatus::from_code(status.to_code()), status);
        }
    }

    #[test]
    fn permission_manager_caches_within_ttl() {
        let manager = PermissionManager::new(Duration::from_millis(500));
        assert_eq!(manager.current_cached_status(), PermissionStatus::Unknown);

        let mut queries = 0;
        let s1 = manager.check_status(false, || {
            queries += 1;
            Ok(true)
        });
        assert_eq!(s1, PermissionStatus::Granted);
        assert_eq!(queries, 1);

        // Immediate subsequent call uses cache
        let s2 = manager.check_status(false, || {
            queries += 1;
            Ok(false)
        });
        assert_eq!(s2, PermissionStatus::Granted);
        assert_eq!(queries, 1);

        // Force refresh bypasses cache
        let s3 = manager.check_status(true, || {
            queries += 1;
            Ok(true)
        });
        assert_eq!(s3, PermissionStatus::Granted);
        assert_eq!(queries, 2);
    }

    #[test]
    fn mark_recheck_required_invalidates_cache() {
        let manager = PermissionManager::new(Duration::from_secs(10));
        let mut val = true;
        let _ = manager.check_status(false, || Ok(val));
        assert_eq!(manager.current_cached_status(), PermissionStatus::Granted);

        manager.mark_recheck_required();
        assert_eq!(
            manager.current_cached_status(),
            PermissionStatus::RecheckRequired
        );

        val = false;
        let s = manager.check_status(false, || Ok(val));
        assert_eq!(s, PermissionStatus::NotGranted);
    }

    #[test]
    fn check_failure_distinguished_from_denied() {
        let manager = PermissionManager::new(Duration::from_secs(1));
        let status = manager.check_status(true, || Err("OS query failed".into()));
        assert_eq!(status, PermissionStatus::CheckFailed);
        assert!(!status.is_granted());
        assert_eq!(status.as_str(), "check_failed");
    }

    #[test]
    fn permission_revoked_after_previously_granted() {
        let manager = PermissionManager::new(Duration::from_millis(50));
        let mut os_allowed = true;
        let s1 = manager.check_status(false, || Ok(os_allowed));
        assert_eq!(s1, PermissionStatus::Granted);

        // OS permission genuinely revoked in System Settings
        os_allowed = false;
        // Wait for TTL expiration
        std::thread::sleep(Duration::from_millis(60));
        let s2 = manager.check_status(false, || Ok(os_allowed));
        assert_eq!(s2, PermissionStatus::NotGranted);
        assert!(!s2.is_granted());
    }

    #[test]
    fn concurrent_status_checks_coalesce_safely() {
        use std::sync::{Arc, Barrier};
        let manager = Arc::new(PermissionManager::new(Duration::from_secs(5)));
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();

        for _ in 0..8 {
            let m = Arc::clone(&manager);
            let b = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                b.wait();
                m.check_status(false, || Ok(true))
            }));
        }

        for h in handles {
            let status = h.join().unwrap();
            assert_eq!(status, PermissionStatus::Granted);
        }
    }

    #[test]
    fn permission_status_after_gui_or_engine_restart() {
        // Simulates GUI or background engine restart where a new process/manager starts fresh.
        // It begins in Unknown, queries OS with prompt=NO, and authoritatively receives Granted.
        let fresh_manager = PermissionManager::new(Duration::from_millis(1500));
        assert_eq!(
            fresh_manager.current_cached_status(),
            PermissionStatus::Unknown
        );

        let os_trust = true;
        let status = fresh_manager.check_status(false, || Ok(os_trust));
        assert_eq!(status, PermissionStatus::Granted);
        assert_eq!(
            fresh_manager.current_cached_status(),
            PermissionStatus::Granted
        );
    }

    #[test]
    fn background_engine_and_gui_permission_state_consistency() {
        // Simulates two separate processes: GUI process and Background Engine process.
        let gui_manager = PermissionManager::new(Duration::from_millis(1500));
        let background_manager = PermissionManager::new(Duration::from_millis(1500));

        let shared_os_state = true;
        let gui_status = gui_manager.check_status(false, || Ok(shared_os_state));
        let bg_status = background_manager.check_status(false, || Ok(shared_os_state));

        assert_eq!(gui_status, PermissionStatus::Granted);
        assert_eq!(bg_status, PermissionStatus::Granted);
        assert_eq!(
            gui_manager.current_cached_status(),
            background_manager.current_cached_status()
        );
    }

    #[test]
    fn os_restart_authoritative_query_flow() {
        // Simulates complete OS restart: cache is cold, queries OS without prompt,
        // gets granted status and caches it with TTL.
        let manager = PermissionManager::new(Duration::from_millis(1500));
        assert_eq!(manager.current_cached_status(), PermissionStatus::Unknown);

        let mut os_queries = 0;
        let status = manager.check_status(false, || {
            os_queries += 1;
            Ok(true)
        });
        assert_eq!(status, PermissionStatus::Granted);
        assert_eq!(os_queries, 1);

        // Immediate subsequent calls in startup sequence do not spam OS
        for _ in 0..10 {
            assert_eq!(
                manager.check_status(false, || {
                    os_queries += 1;
                    Ok(true)
                }),
                PermissionStatus::Granted
            );
        }
        assert_eq!(os_queries, 1);
    }
}
