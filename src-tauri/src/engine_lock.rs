//! M1: one cross-process engine lock (`engine.lock`).
//!
//! The GUI engine (driven by the tray thread) and the background engine
//! (`--background`) both hold a whole-`Snapshot` in memory and both
//! `db.save()` it. Until now nothing stopped both ticking at once: if the
//! GUI tray thread stalled for more than the 3 s desktop-heartbeat
//! staleness, the background engine opened and ticked concurrently, and a
//! stale writer could delete rows the other engine had just created (a
//! whole-snapshot `save()` deletes rows absent from the writer's
//! snapshot; tombstones only stop resurrection of *deleted* sessions).
//!
//! The lock is an exclusive non-blocking flock on `engine.lock` in the
//! app-data dir, the same pattern as the existing `background.lock` (and
//! `attention.lock`). Whoever holds it is the one authoritative engine:
//! only the holder may tick, commit, or `db.save()`. The other engine
//! opens read-only (observer): it reloads the snapshot so the UI keeps
//! showing the owner's state, but never ticks, never saves, and never
//! publishes a browser policy. On every tick / write attempt the observer
//! retries the lock, so handover is automatic within about one tick of
//! the holder releasing it — and the OS releases a flock when the
//! holding process dies, so a force-quit hands over with no cleanup
//! step. The lock file itself is never deleted (same as
//! `background.lock`): the flock, not the file's existence, is the
//! mutex.
use fs2::FileExt;
use std::{fs::File, path::Path};

/// The held cross-process engine lock. Dropping it releases the flock.
pub struct EngineLock {
    _file: File,
}

impl Drop for EngineLock {
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

const LOCK_FILE: &str = "engine.lock";

impl EngineLock {
    /// Try to take the engine lock in `data_dir`. Returns `None` when
    /// another engine (GUI or background, on this machine) already holds
    /// it. Never blocks.
    pub fn try_acquire(data_dir: &Path) -> Option<Self> {
        let file = File::create(data_dir.join(LOCK_FILE)).ok()?;
        file.try_lock_exclusive().ok()?;
        Some(Self { _file: file })
    }

    /// Test/diagnostic probe: true when another engine currently holds
    /// the lock. Same pattern as the background lock: try the lock,
    /// release at once. An uncreatable lock file is reported as held:
    /// fail closed.
    #[cfg(test)]
    pub fn held(data_dir: &Path) -> bool {
        let Ok(file) = File::create(data_dir.join(LOCK_FILE)) else {
            return true;
        };
        if file.try_lock_exclusive().is_ok() {
            let _ = file.unlock();
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_while_first_held() {
        let dir = tempfile::tempdir().unwrap();
        let first = EngineLock::try_acquire(dir.path()).unwrap();
        assert!(EngineLock::try_acquire(dir.path()).is_none());
        assert!(EngineLock::held(dir.path()));
        drop(first);
    }

    #[test]
    fn lock_is_released_when_holder_drops() {
        let dir = tempfile::tempdir().unwrap();
        let holder = EngineLock::try_acquire(dir.path()).unwrap();
        assert!(EngineLock::held(dir.path()));
        drop(holder);
        assert!(!EngineLock::held(dir.path()));
        assert!(EngineLock::try_acquire(dir.path()).is_some());
    }

    #[test]
    fn probe_reports_free_when_no_holder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!EngineLock::held(dir.path()));
    }
}
