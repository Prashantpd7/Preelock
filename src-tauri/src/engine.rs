use crate::{
    attention::{self, Signals, State},
    browser::{self, Bridge, Policy},
    clock,
    database::Database,
    focus::{rule_matches, FocusConfig, Mode, WebsiteBlocking, ADULT_DOMAINS},
    models::{Action, Snapshot, Status},
    platform, sessions,
};
use std::path::Path;

pub struct Engine {
    db: Database,
    data_dir: std::path::PathBuf,
    lifecycle_path: std::path::PathBuf,
    data: Snapshot,
    anchor: u64,
    sample_anchor: u64,
    screen_time_anchor: u64,
    wall: i64,
    /// Wall-clock time (epoch ms) of the last tick that ran. A gap
    /// far larger than the 1-second tick interval means ticks paused
    /// (sleep / long suspension); see `TICK_GAP_THRESHOLD_MS`.
    last_tick_wall_ms: Option<i64>,
    checkpoint: u64,
    /// Continuous-ms time of the last allowance-usage flush (step
    /// 8): tick accrual is written to SQLite at most every 5 s, so
    /// a crash loses at most ~5 s of budget usage.
    allowance_checkpoint: u64,
    bridge: Option<Bridge>,
    bridge_error: String,
    /// M4: continuous-ms time before which `ensure_bridge` must not
    /// retry a failed bridge start (throttle, not a tight loop — a
    /// start attempt while the port is still bound by the dying
    /// holder is wasted work). `0` means a retry is due now.
    bridge_retry_at: u64,
    /// M1: cross-process engine lock. `Some` while this engine is the
    /// one authoritative engine allowed to tick and save; `None` while
    /// it is an observer (the other process owns the lock). The flock
    /// is released when this engine drops.
    engine_lock: Option<crate::engine_lock::EngineLock>,
    native: bool,
    /// M6: whether this engine was opened after an OS shutdown,
    /// logout, or restart (`lifecycle::initialize` resume flag).
    /// True only when the previous run ended via the OS event path
    /// or the boot id changed — never after an ordinary quit — so
    /// the GUI can tell a login launch (start hidden when idle)
    /// from a manual launch (always show the window). See
    /// `quit::login_window_action`.
    resumed_after_os_shutdown: bool,
    camera_enabled: bool,
    attention_camera_status: String,
    camera: crate::camera::Capture,
    camera_test_until: u64,
    failed: Option<String>,
    /// Step 13: live app-block transition + throttle state. Never
    /// persisted: after a restart the sampler re-detects the blocked
    /// app on the first tick (still subject to the 60 s throttle).
    attempt_tracker: crate::stats::AttemptTracker,
    /// "Why am I blocked" app page: last epoch-ms the page was
    /// (re-)triggered per app id. Tracked separately from the
    /// attempt-log throttle so the two never consume each other;
    /// never persisted.
    overlay_last_shown: std::collections::HashMap<String, i64>,
}
/// Whether the browser's active domain counts as relevant work for the
/// focus timer. `domains` is interpreted according to the session's
/// website-blocking mode: in blocklist modes a domain is relevant when it
/// is NOT blocked, in allowlist modes only when it is listed. Matching
/// uses the shared `rule_matches()` (plain, wildcard and path rules;
/// path rules are evaluated with an empty path here, since the engine
/// only sees a host — see focus.rs for the exact consequences).
/// `block_adult` is the session's "Block adult content" toggle: when
/// any website blocking is on, an adult-category host is never
/// relevant, even if it is explicitly allowlisted (the toggle is a
/// block overlay, mirroring the extension policy). Keyword rules are
/// not evaluated here at all: the extension reports a matching search
/// page via `search_distracted`, and the tick code turns that into
/// `Some(false)` before calling this function.
/// Per-block exception precedence (identical to browser-extension/
/// rules.js):
/// 1. Offline mode is handled by the caller/policy, not here — the
///    engine never sees an offline session's browser.
/// 2. The adult overlay wins over exceptions: an adult host is never
///    relevant, even if it matches an exception.
/// 3. In blocklist mode (including Auto in Standard, which resolves
///    to blocklist), a domain matching an exception rule counts as
///    relevant, overriding the block rule.
/// 4. Otherwise the block rules decide. In allowlist mode (including
///    Auto outside Standard) exceptions are meaningless — the allowed
///    list already defines what is open — so they are ignored here,
///    exactly as in the extension policy.
#[cfg(test)]
fn browser_domain_relevant(
    mode: &Mode,
    website_blocking: &WebsiteBlocking,
    domains: &[String],
    active_domain: &str,
    block_adult: bool,
) -> bool {
    browser_domain_relevant_with_exceptions(
        mode,
        website_blocking,
        domains,
        &[],
        active_domain,
        block_adult,
    )
}
fn browser_domain_relevant_with_exceptions(
    mode: &Mode,
    website_blocking: &WebsiteBlocking,
    domains: &[String],
    exceptions: &[String],
    active_domain: &str,
    block_adult: bool,
) -> bool {
    if block_adult
        && !matches!(website_blocking, WebsiteBlocking::None)
        && ADULT_DOMAINS
            .iter()
            .any(|d| rule_matches(d, active_domain, ""))
    {
        return false;
    }
    let effective_blocklist = matches!(website_blocking, WebsiteBlocking::Blocklist)
        || (matches!(website_blocking, WebsiteBlocking::Auto) && matches!(mode, Mode::Standard));
    if effective_blocklist
        && exceptions
            .iter()
            .any(|d| rule_matches(d, active_domain, ""))
    {
        return true;
    }
    let in_domains = domains.iter().any(|d| rule_matches(d, active_domain, ""));
    match website_blocking {
        WebsiteBlocking::Blocklist => !in_domains,
        WebsiteBlocking::Allowlist => in_domains,
        // No website blocking: any site is fine.
        WebsiteBlocking::None => true,
        // Sessions saved before the choice existed follow the extension
        // policy: blocklist in Standard, allowlist otherwise.
        WebsiteBlocking::Auto => match mode {
            Mode::Standard => !in_domains,
            _ => in_domains,
        },
    }
}

/// Tick-gap threshold (step 6b): ticks run every second, so a
/// wall-clock gap over 120 seconds cannot be ordinary scheduling
/// jitter — ticks paused, which on both macOS and Windows means the
/// machine slept (or the process was suspended). No platform power
/// APIs are needed: the gap itself is the signal. 120 seconds is
/// deliberately generous so a slow or loaded machine never triggers
/// a false wake; the cost of a missed wake is nil, because the
/// ordinary tick already re-evaluates schedules every second.
pub const TICK_GAP_THRESHOLD_MS: i64 = 120_000;

/// M4: minimum gap between bridge (re)start attempts by a lock
/// holder whose bridge failed to start (port still bound by the
/// dying holder, lock still held by a holder mid-exit). Five seconds
/// is far inside the 65 s policy lease the extension already holds,
/// so retrying on this cadence can never open a policy gap, and a
/// tick never stalls more than a single fast attempt (see
/// `Bridge::try_start`).
pub const BRIDGE_RETRY_MS: u64 = 5_000;

/// True when the wall-clock gap since the previous tick exceeds the
/// threshold. `None` (no previous tick yet) and a clock moving
/// backwards are never a gap.
pub fn tick_gap_exceeded(previous_wall_ms: Option<i64>, now_ms: i64) -> bool {
    previous_wall_ms.is_some_and(|previous| now_ms.saturating_sub(previous) > TICK_GAP_THRESHOLD_MS)
}

/// Drop one-shot timers whose fire time has already passed (step 6c):
/// a time that passed while the app was closed is skipped, never
/// fired retroactively. Returns whether anything was removed. Used
/// at launch and after a backup restore; the same rule on a wake
/// tick is handled by `one_shot_action` with the gap flag.
fn purge_expired_one_shots(data: &mut Snapshot, now_ms: i64) -> bool {
    let before = data.one_shots.len();
    data.one_shots.retain(|shot| shot.fire_at_ms > now_ms);
    data.one_shots.len() != before
}
/// M5: screen-time recording ownership. There is exactly one
/// recording path — the always-on sampler in `sample_screen_time`,
/// reached only from `tick()`, which only the engine-lock holder
/// ever runs (M1: an observer's tick returns before sampling).
/// Recording therefore follows the lock, never the heartbeat
/// files: the old heartbeat gates (`desktop-alive` vs
/// `tracker-alive`) left a hole where NO path recorded — GUI gone,
/// background tracker alive, no session running — so idle
/// background screen time was silently dropped, and a Paused or
/// Preparing session's time with it. The session tick path used
/// to record as the mirror image of those gates; it no longer
/// records at all, so one interval can never be written twice and
/// the foreground browser's site attribution (via
/// `browser_status_for_app`) is the only attribution used.
/// Heartbeat files are still used where they belong: liveness
/// (background takeover, tray exit), never accounting.
/// "Why am I blocked" app page trigger: record the blocked app +
/// its true reason so the `blocked-app` overlay window can show it
/// instead of the app just silently vanishing. A free function (not
/// a method) because the session-path call site holds `s: &mut
/// Session` borrowed from `self.data` — only disjoint field borrows
/// are allowed there. Throttled to one trigger per app per 60 s,
/// tracked separately from the attempt-log throttle so the two
/// never consume each other. The page is purely informational: the
/// block itself is unchanged.
fn record_blocked_app_page(
    notice: &mut Option<crate::models::BlockedAppNotice>,
    last_shown: &mut std::collections::HashMap<String, i64>,
    sample: &crate::platform::Sample,
    cause: &crate::blockreasons::BlockCause,
    now: i64,
) {
    let last = last_shown.get(&sample.id).copied().unwrap_or(i64::MIN);
    if now.saturating_sub(last) < 60_000 {
        return;
    }
    last_shown.insert(sample.id.clone(), now);
    let app_name = if sample.name.is_empty() {
        sample.id.clone()
    } else {
        sample.name.clone()
    };
    let reason = crate::blockreasons::cause_text(cause, &app_name, now);
    let ends_at_ms = match cause.kind.as_str() {
        "timed_lock" => cause.ends_at_ms,
        _ => None,
    };
    *notice = Some(crate::models::BlockedAppNotice {
        app_name,
        reason,
        ends_at_ms,
        at_ms: now,
    });
}
impl Engine {
    #[cfg(test)]
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, false, false)
    }
    pub fn open_native(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, true, false)
    }
    pub fn open_background(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, true, false)
    }
    fn open_inner(path: &Path, native: bool, camera_enabled: bool) -> Result<Self, String> {
        let resume = if native {
            crate::lifecycle::initialize(path)?
        } else {
            false
        };
        Self::open_recovery(path, native, resume, camera_enabled)
    }
    fn open_recovery(
        path: &Path,
        native: bool,
        resume: bool,
        camera_enabled: bool,
    ) -> Result<Self, String> {
        let data_dir = path
            .parent()
            .ok_or("Invalid database location.")?
            .to_path_buf();
        // The data dir must exist for the lock file; Database::open
        // below would fail without it anyway.
        let _ = std::fs::create_dir_all(&data_dir);
        // M1: only the engine that holds the cross-process lock may run
        // recovery and save. An observer still loads the snapshot (so
        // the UI can show the owner's state) and still applies the
        // recovery transforms in memory, but it must not persist them:
        // a whole-snapshot save from a stale observer could delete rows
        // the owner just created. The observer re-checks the lock on
        // every tick / write (see `try_acquire_engine_lock`) and
        // reloads from the database the moment it becomes the owner.
        let engine_lock = crate::engine_lock::EngineLock::try_acquire(&data_dir);
        let owns_lock = engine_lock.is_some();
        let mut db = Database::open(path)?;
        let mut data = db.load()?;
        // One-shot timers (step 6c): anything whose time passed while
        // the app was closed is skipped and removed here, before the
        // first tick, so it can never fire retroactively at launch.
        // (The save at the end of this function persists the purge.)
        purge_expired_one_shots(&mut data, clock::now());
        // Manual check-ins were removed; sessions saved with that
        // verification now use app-based checking.
        for s in &mut data.sessions {
            if s.focus.attention.verification == "manual_checkpoints" {
                s.focus.attention.verification = "activity_only".into();
            }
        }
        // M6: live plain-timer sessions (attention disabled — one-shot
        // sessions, backup restores, and legacy rows can all be plain)
        // resume exactly like attention sessions below, with their
        // checkpointed `remaining_ms` intact. An earlier "upgrade"
        // loop ended every live plain timer here as Interrupted on
        // every reopen, which silently lifted its block across any
        // restart and dropped its remaining time. That is wrong for
        // shutdown recovery: `remaining_ms` is authoritative, the
        // tick anchors are reset at open, and the first tick after a
        // long gap credits nothing (M5 wake clamp), so offline time
        // can never be credited or drain the session. Sessions that
        // were already Interrupted stay Interrupted; nothing here
        // resurrects a finished session.
        for s in &mut data.sessions {
            // Step 12 Frozen Turkey, recovery: a frozen session
            // whose wall-clock timer already passed while the app
            // was closed is completed, never resumed — the lock
            // lifts on expiry even across a kill/restart, and the
            // overlay must not re-appear for it. A frozen session
            // with no remaining productive time is likewise done.
            // Anything else frozen resumes below, like any live
            // session, and the overlay re-appears on the first
            // sync tick (see `lib.rs`).
            if s.focus.frozen.enabled
                && matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                )
                && (s.remaining_ms == 0
                    || crate::frozen::wall_expired(s.started_at, s.duration_minutes, clock::now()))
            {
                s.status = Status::Completed;
                s.ended_at = Some(clock::now());
                s.deadline_ms = None;
                s.grace_until_ms = None;
                s.attention.state = State::Idle;
                s.attention.reason = "Frozen lock expired".into();
                s.report.active_app.clear();
                s.report.camera_status = "off".into();
                s.report.enforcement = "Frozen lock expired; monitoring stopped".into();
                s.report.event(
                    clock::now(),
                    "frozen_expired",
                    "Frozen lock lifted on expiry during restart recovery; the timer had already run out.",
                );
                continue;
            }
            // Only a confirmed OS shutdown or a different boot uses the
            // OS-restart wording. Every other live-session reopen is a
            // continuation, never a force quit (see the fallback below).
            if matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                if resume {
                    attention::recover(s, clock::now());
                    if s.status != Status::Paused {
                        // On resume after system restart, go directly to Running.
                        // The user expects blocking to resume immediately when the
                        // PC starts, without having to open the browser first.
                        // Browser website rules are picked up by the extension
                        // when the browser is opened; native app blocking starts
                        // at once via the backend.
                        s.status = Status::Running;
                    }
                    s.ended_at = None;
                    s.deadline_ms = None;
                    s.report.camera_status = "off".into();
                    s.durability.recovery_status = Some("recovered_after_os_shutdown".into());
                    s.durability.last_checkpoint_at_ms = Some(clock::now());
                    if s.durability.intended_end_at_ms.is_none() {
                        s.durability.intended_end_at_ms = Some(
                            clock::now().saturating_add(s.remaining_ms.min(i64::MAX as u64) as i64),
                        );
                    }
                    s.report.event(clock::now(), "system_restart", "Session restored after system shutdown/restart; offline time does not count.");
                    continue;
                }
                // Step 10 restart lock: a session that opted into
                // surviving restarts resumes locked here too, even
                // without a confirmed OS shutdown — killing the app
                // must not silently drop a restart-locked block.
                // Sessions without this lock keep the existing
                // Force Quit behaviour below. The lock itself is
                // unchanged (its gates still apply to early exit),
                // and the timer still ends the block on expiry.
                if s.lock.restart_lock {
                    attention::recover(s, clock::now());
                    if s.status != Status::Paused {
                        s.status = Status::Running;
                    }
                    s.ended_at = None;
                    s.deadline_ms = None;
                    s.report.camera_status = "off".into();
                    s.durability.recovery_status = Some("lock_restored".into());
                    s.durability.last_checkpoint_at_ms = Some(clock::now());
                    if s.durability.intended_end_at_ms.is_none() {
                        s.durability.intended_end_at_ms = Some(
                            clock::now().saturating_add(s.remaining_ms.min(i64::MAX as u64) as i64),
                        );
                    }
                    s.report.event(
                        clock::now(),
                        "lock_restored",
                        "Locked session restored after restart; its lock still applies.",
                    );
                    continue;
                }
                // Same-boot recovery is a continuation, not a force
                // quit. The desktop UI is only a view of a session the
                // engine owns: closing its window, quitting it normally,
                // a UI process crash, or a handover to the independent
                // background engine all resume the same session with its
                // remaining time, restrictions and scheduled end intact
                // (publish() at the end of this function republishes its
                // policy before any tick counts time, exactly as the
                // OS-restart path does). Nothing here is recorded as the
                // user forcing Preelock closed, so the obsolete
                // `force_quit` flag is no longer set — it stays only on
                // historical records. Only a session whose state cannot
                // be loaded at all (legacy upgrade, storage failure)
                // still ends as Interrupted; every live session here is
                // recoverable from its SQLite checkpoint.
                attention::recover(s, clock::now());
                s.status = crate::tamper::resume_status(&s.status);
                s.ended_at = None;
                s.deadline_ms = None;
                s.report.camera_status = "off".into();
                s.durability.recovery_status = Some("recovered_after_restart".into());
                s.durability.last_checkpoint_at_ms = Some(clock::now());
                if s.durability.intended_end_at_ms.is_none() {
                    s.durability.intended_end_at_ms = Some(
                        clock::now().saturating_add(s.remaining_ms.min(i64::MAX as u64) as i64),
                    );
                }
                s.report.event(
                    clock::now(),
                    "session_resumed",
                    "Preelock had no running engine for a moment (the desktop app closed, restarted or handed over to the background engine); this session resumed with its remaining time, restrictions and scheduled end intact. Time while no engine ran was not counted.",
                );
                continue;
            }
            attention::recover(s, clock::now());
        }
        // M1: persist recovery only as the lock holder (see above).
        if owns_lock {
            let now = clock::now();
            for s in &mut data.sessions {
                if matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                ) {
                    s.durability.last_checkpoint_at_ms = Some(now);
                }
            }
            db.save(&data)?;
            let summary = Self::active_session_checkpoint(&data, now);
            let _ = crate::lifecycle::write_session_checkpoint(
                &path.with_extension("lifecycle.json"),
                crate::lifecycle::current_boot_id(),
                crate::lifecycle::ending(),
                summary,
            );
        }
        // M1: only the lock holder starts a bridge. An observer that
        // grabbed the bridge would publish a possibly stale policy
        // while the owner ticks — the dual-owner hazard this lock
        // exists to prevent. M4: an observer's bridge is the owner's
        // bridge (same port, same token), so the error says so
        // instead of the domain-test message — the extension stays
        // connected to the owner's bridge while this engine observes.
        let (bridge, bridge_error) = if native && owns_lock {
            match Bridge::start(path.parent().ok_or("Invalid database location.")?) {
                Ok(b) => (Some(b), String::new()),
                Err(e) => (None, e),
            }
        } else if native {
            (
                None,
                "Another Preelock process currently owns the browser bridge.".into(),
            )
        } else {
            (None, "Native runtime disabled in domain tests.".into())
        };
        let now = clock::continuous_ms();
        // M4: computed before the literal below moves `bridge` into
        // the engine — see the `bridge_retry_at` field docs.
        let bridge_retry_at = if bridge.is_none() && native && owns_lock {
            now + BRIDGE_RETRY_MS
        } else {
            0
        };
        let lifecycle_path = path.with_extension("lifecycle.json");
        let engine = Self {
            db,
            data_dir: path
                .parent()
                .ok_or("Invalid database location.")?
                .to_path_buf(),
            lifecycle_path: lifecycle_path.clone(),
            data,
            anchor: now,
            sample_anchor: now,
            screen_time_anchor: now,
            wall: clock::now(),
            last_tick_wall_ms: Some(clock::now()),
            checkpoint: now,
            allowance_checkpoint: now,
            bridge,
            bridge_error,
            // M4: a holder whose open-time start failed retries on
            // the tick (see `ensure_bridge`); an observer never does.
            bridge_retry_at,
            engine_lock,
            native,
            resumed_after_os_shutdown: resume,
            camera_enabled,
            attention_camera_status: "camera_unavailable".into(),
            camera: Default::default(),
            camera_test_until: 0,
            failed: None,
            attempt_tracker: crate::stats::AttemptTracker::default(),
            overlay_last_shown: Default::default(),
        };
        engine.publish();
        // Screen Time sampling is always on: the observe-only lease
        // starts at launch so usage is collected whenever the app runs.
        if let Some(bridge) = &engine.bridge {
            bridge.set_screen_time_tracking(true);
        }
        Ok(engine)
    }
    /// M1: returned when a write is refused because the other engine
    /// (GUI or background) currently owns the cross-process engine
    /// lock. The handover is automatic within about a second, so the
    /// caller just retries.
    const ENGINE_LOCK_HELD: &str =
        "Another Preelock process currently owns the session engine; try again in a moment.";
    /// M1: whether this engine currently holds the cross-process engine
    /// lock — the only engine allowed to tick, commit, or save.
    pub fn owns_engine_lock(&self) -> bool {
        self.engine_lock.is_some()
    }

    /// M6: whether this engine opened after an OS shutdown, logout,
    /// or restart (see the `resumed_after_os_shutdown` field docs).
    pub fn recovered_after_os_shutdown(&self) -> bool {
        self.resumed_after_os_shutdown
    }

    /// M6: best-effort shutdown checkpoint — persist the current
    /// snapshot exactly as it is, with no status changes. Unlike
    /// `shutdown()` (idle quit, which releases restrictions), this
    /// keeps a live session live with its remaining time intact, so
    /// recovery after an OS shutdown resumes it instead of ending
    /// it. Holder-only: an observer refuses, so a stale snapshot
    /// can never overwrite the owner's checkpoint. Callers on
    /// shutdown paths ignore the error and log it — the periodic
    /// ~5 s commit checkpoint already bounds the loss, and a
    /// failing save must never block the OS from shutting down.
    fn active_session_checkpoint(
        snapshot: &Snapshot,
        now: i64,
    ) -> Option<crate::lifecycle::SessionCheckpoint> {
        let s = snapshot.sessions.iter().find(|s| {
            matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
        })?;
        let schedule = s
            .schedule_id
            .as_deref()
            .and_then(|id| snapshot.schedules.iter().find(|sc| sc.id == id));
        let kind = match s.kind {
            crate::models::SessionKind::Focus => "focus",
            crate::models::SessionKind::Scheduled => "scheduled",
            crate::models::SessionKind::OneShot => "one_shot",
        };
        let status = match s.status {
            Status::Ready => "ready",
            Status::Preparing => "preparing",
            Status::Running => "running",
            Status::Paused => "paused",
            Status::Completed => "completed",
            Status::Ended => "ended",
            Status::Interrupted => "interrupted",
        };
        let mode = match s.focus.mode {
            Mode::Standard => "standard",
            Mode::Strict => "strict",
            Mode::Deep => "deep",
        };
        let is_allowlist_apps = !matches!(s.focus.mode, Mode::Standard);
        let (blocked_apps, allowed_apps) = if is_allowlist_apps {
            (Vec::new(), s.focus.apps.clone())
        } else {
            (s.focus.apps.clone(), Vec::new())
        };
        let is_allowlist_sites = matches!(s.focus.website_blocking, WebsiteBlocking::Allowlist)
            || (matches!(s.focus.website_blocking, WebsiteBlocking::Auto)
                && !matches!(s.focus.mode, Mode::Standard));
        let (blocked_sites, allowed_sites) =
            if matches!(s.focus.website_blocking, WebsiteBlocking::None) {
                (Vec::new(), Vec::new())
            } else if is_allowlist_sites {
                (Vec::new(), s.focus.domains.clone())
            } else {
                (s.focus.domains.clone(), s.focus.exceptions.clone())
            };
        Some(crate::lifecycle::SessionCheckpoint {
            session_id: s.id.clone(),
            kind: kind.into(),
            status: status.into(),
            mode: mode.into(),
            schedule_id: s.schedule_id.clone(),
            schedule_title: schedule.map(|sc| sc.title.clone()),
            weekdays: schedule.map(|sc| sc.days.clone()).unwrap_or_default(),
            start_time: schedule.map(|sc| sc.start_time.clone()),
            end_time: schedule.map(|sc| sc.end_time.clone()),
            started_at_ms: s.started_at,
            intended_end_at_ms: s.durability.intended_end_at_ms,
            remaining_ms: s.remaining_ms,
            blocked_apps,
            allowed_apps,
            blocked_sites,
            allowed_sites,
            recovery_status: s
                .durability
                .recovery_status
                .clone()
                .unwrap_or_else(|| "clean".into()),
            last_checkpoint_ms: s.durability.last_checkpoint_at_ms.unwrap_or(now),
            last_enforcement_at_ms: s.durability.last_enforcement_at_ms,
        })
    }

    pub fn save_checkpoint(&mut self) -> Result<(), String> {
        if !self.owns_engine_lock() {
            return Err(Self::ENGINE_LOCK_HELD.into());
        }
        let now = clock::now();
        for s in &mut self.data.sessions {
            if matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                s.durability.last_checkpoint_at_ms = Some(now);
            }
        }
        self.db.save(&self.data)?;
        let summary = Self::active_session_checkpoint(&self.data, now);
        crate::lifecycle::write_session_checkpoint(
            &self.lifecycle_path,
            crate::lifecycle::current_boot_id(),
            crate::lifecycle::ending(),
            summary,
        )
    }

    /// M1: try to become the authoritative engine. A no-op when this
    /// engine already holds the lock. On a fresh acquisition the snapshot
    /// is reloaded from the database — the observer's in-memory copy may
    /// predate everything the previous owner saved — and the tick anchors
    /// are reset so no time is credited for the observer period. Expired
    /// one-shots that lapsed while this engine observed are purged (the
    /// step-6c launch rule), never fired late. If the bridge is free now
    /// (the previous owner died while bridge-less, or never had it), it
    /// is started here and the current policy republished, so website
    /// enforcement resumes with the takeover instead of waiting for a
    /// restart. A takeover-time start that still fails (the old
    /// holder's port not yet released) is not the end of the story
    /// any more: the holder's tick retries it via `ensure_bridge`
    /// (M4), so a bridge-less holder recovers on its own.
    fn try_acquire_engine_lock(&mut self) -> bool {
        if self.engine_lock.is_some() {
            return true;
        }
        let Some(lock) = crate::engine_lock::EngineLock::try_acquire(&self.data_dir) else {
            return false;
        };
        // Reload first: if the load fails, give the lock straight back
        // rather than ticking from a stale snapshot.
        let Ok(data) = self.db.load() else {
            drop(lock);
            return false;
        };
        self.engine_lock = Some(lock);
        self.data = data;
        let now_continuous = clock::continuous_ms();
        let now_wall = clock::now();
        self.anchor = now_continuous;
        self.sample_anchor = now_continuous;
        self.screen_time_anchor = now_continuous;
        self.checkpoint = now_continuous;
        self.allowance_checkpoint = now_continuous;
        self.wall = now_wall;
        self.last_tick_wall_ms = Some(now_wall);
        if purge_expired_one_shots(&mut self.data, now_wall) {
            // Best-effort: the purge is already applied in memory, so
            // the timer cannot fire from this engine, and the next
            // commit save persists it.
            if self.db.save(&self.data).is_err() {
                log::warn!("Could not persist expired one-shot purge on engine takeover.");
            }
        }
        self.start_bridge_patient();
        self.publish();
        true
    }

    /// M4: patient bridge start, used at open and on engine-lock
    /// takeover, where waiting ~2 s for the previous holder's bridge
    /// to release is correct. On failure the error is kept and the
    /// tick retry is armed — the holder recovers via `ensure_bridge`
    /// instead of staying bridge-less until a restart.
    fn start_bridge_patient(&mut self) {
        if self.bridge.is_some() || !self.native {
            return;
        }
        match Bridge::start(&self.data_dir) {
            Ok(bridge) => {
                bridge.set_screen_time_tracking(true);
                bridge.set_connection_locked(self.has_active_strict());
                self.bridge = Some(bridge);
                self.bridge_error = String::new();
                self.bridge_retry_at = 0;
            }
            Err(error) => {
                self.bridge_error = error;
                self.bridge_retry_at = clock::continuous_ms() + BRIDGE_RETRY_MS;
            }
        }
    }

    /// M4: keep the bridge owned while this engine holds the lock.
    /// Called on every holder tick, and only there — never by an
    /// observer, which must not start a second bridge while the
    /// owner serves the extension. Three cases:
    /// - No bridge yet (the open/takeover start failed, e.g. the
    ///   port was still bound by the dying holder): retry with one
    ///   fast attempt, at most once per `BRIDGE_RETRY_MS`, until it
    ///   starts. A holder that opened bridge-less no longer
    ///   enforces nothing forever.
    /// - Bridge present but its server thread dead: drop the husk
    ///   (releases `bridge.lock` and the port) and restart below,
    ///   so a dead server can never strand enforcement while this
    ///   process lives.
    /// - Bridge live: nothing to do; the tick's lease refresh keeps
    ///   the observe-only policy fresh.
    ///
    /// A successful (re)start republishes the current policy at
    /// once, so the extension picks it up on its next 30 s sync —
    /// inside the 65 s lease the policy it already holds carries.
    fn ensure_bridge(&mut self) {
        if !self.native || !self.owns_engine_lock() {
            return;
        }
        if let Some(bridge) = &self.bridge {
            if bridge.server_alive() {
                return;
            }
            log::warn!(
                "Browser bridge server thread is no longer answering; restarting the bridge."
            );
            self.bridge = None;
        }
        let now = clock::continuous_ms();
        if now < self.bridge_retry_at {
            return;
        }
        match Bridge::try_start(&self.data_dir) {
            Ok(bridge) => {
                bridge.set_screen_time_tracking(true);
                self.bridge = Some(bridge);
                self.bridge_error = String::new();
                self.bridge_retry_at = 0;
                self.publish();
            }
            Err(error) => {
                self.bridge_error = error;
                self.bridge_retry_at = now + BRIDGE_RETRY_MS;
            }
        }
    }

    /// Test helper: flip the native flag after `open()` (which always
    /// opens non-native in tests). Lets bridge-ownership tests drive
    /// `ensure_bridge` without `open_native`, whose lifecycle
    /// one-shot cannot run twice in one test process. Also refreshes
    /// the bridge error so an observer reports the bridge as owned
    /// elsewhere, exactly as `open_recovery` would.
    #[cfg(test)]
    fn set_native_for_test(&mut self, native: bool) {
        self.native = native;
        if native && self.bridge.is_none() && !self.owns_engine_lock() {
            self.bridge_error =
                "Another Preelock process currently owns the browser bridge.".into();
        }
    }
    pub fn set_attention_camera_status(&mut self, signal: crate::attention_agent::Signal) {
        let now = clock::continuous_ms();
        let fresh = signal.fresh(now);
        let valid = matches!(
            signal.status.as_str(),
            "person_detected"
                | "person_not_detected"
                | "extended_absence"
                | "permission_required"
                | "camera_unavailable"
                | "camera_interrupted"
                | "camera_stalled"
                | "analysis_error"
                | "no_camera"
                | "camera_error"
                | "starting"
                | "off"
        );
        self.attention_camera_status = if fresh && valid {
            signal.status
        } else {
            "camera_unavailable".into()
        };
    }
    pub fn needs_attention_agent(&self) -> bool {
        self.data.sessions.iter().any(|session| {
            matches!(session.status, Status::Running | Status::Preparing)
                && session.focus.camera
                && session.focus.attention.enabled
                && session.focus.attention.verification == "camera_and_activity"
        })
    }
    /// M3: the state-dependent quit decision (`crate::quit`).
    /// Gathers the authoritative inputs, applies the one decision
    /// table, and performs the shutdown side effect only for the
    /// owner-idle outcome — an observer never shuts the engine down.
    /// - Owner (lock held or just acquired; the snapshot was reloaded
    ///   by `try_acquire_engine_lock`): live session → close the UI
    ///   only; idle → `shutdown()` and exit.
    /// - Observer (another engine owns the lock): a fresh read-only
    ///   database load is the authoritative live-session check — the
    ///   in-memory snapshot may predate the owner's latest save by a
    ///   tick. When the database cannot be read, the in-memory
    ///   snapshot is the fallback. Live → close the UI only; idle →
    ///   exit the UI only. This is the fix for the M1 transient:
    ///   an idle observer no longer reports "not idle" (and no
    ///   longer shows the "Finish your focus session" notice for a
    ///   session that is not running).
    pub fn notification_prefs(&self) -> (bool, bool, u32) {
        (
            self.data.preferences.notifications_enabled,
            self.data.preferences.notifications_sound,
            self.data.preferences.volume,
        )
    }
    fn notify_general(&self, title: &str, body: &str) {
        crate::desktop_settings::notify(title, body);
        if self.data.preferences.notifications_sound {
            crate::desktop_settings::sound(self.data.preferences.volume);
        }
    }
    pub fn quit_decision(&mut self) -> crate::quit::QuitDecision {
        let owns_lock = self.try_acquire_engine_lock();
        let has_active = if owns_lock {
            self.has_active()
        } else {
            // Read-only: `Database::open` on an existing file only.
            // A missing file must not be created here — fall back to
            // the in-memory snapshot instead.
            let db_path = self.data_dir.join("preelock.sqlite");
            if db_path.exists() {
                Database::open(&db_path)
                    .and_then(|db| db.load())
                    .map(|data| {
                        data.sessions.iter().any(|s| {
                            matches!(
                                s.status,
                                Status::Running | Status::Paused | Status::Preparing
                            )
                        })
                    })
                    .unwrap_or_else(|_| self.has_active())
            } else {
                self.has_active()
            }
        };
        let decision = crate::quit::decide(has_active, owns_lock);
        if decision == crate::quit::QuitDecision::ShutdownAndExit {
            self.shutdown();
        }
        decision
    }
    pub fn shutdown(&mut self) {
        self.stop_test();
        self.stop_camera();
        if let Some(b) = &self.bridge {
            b.publish(None);
        }
        for s in &mut self.data.sessions {
            if matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                s.status = Status::Interrupted;
                s.ended_at = Some(clock::now());
                s.deadline_ms = None;
                s.attention.state = State::EmergencyExit;
                s.attention.reason = "User quit and released restrictions".into();
                s.report.enforcement = "Released on quit".into();
                s.report.camera_status = "off".into();
                s.report.active_app.clear();
                s.report.event(
                    clock::now(),
                    "user_quit",
                    "User quit Preelock; temporary restrictions released.",
                );
            }
        }
        // If storage fails, retain the older checkpoint for conservative restart recovery.
        if self.db.save(&self.data).is_err() {
            log::error!("Could not save quit checkpoint; restrictions released.");
        }
        self.failed = Some("Application is shutting down".into());
    }
    pub fn browser_status(&self) -> browser::BrowserStatus {
        if let Some(session) = self.data.sessions.iter().find(|s| {
            matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            )
        }) {
            if !session.focus.browser_app.is_empty() {
                let status = self.browser_status_for(&session.focus.browser_app);
                if status.connected {
                    return status;
                }
            }
        }
        self.bridge
            .as_ref()
            .map(Bridge::status)
            .unwrap_or_else(|| self.bridge_unavailable())
    }
    /// Connection status for one specific browser (platform browser id).
    pub fn browser_status_for(&self, browser_id: &str) -> browser::BrowserStatus {
        self.bridge
            .as_ref()
            .map(|b| b.status_for(browser_id))
            .unwrap_or_else(|| self.bridge_unavailable())
    }
    /// True when every browser this session's website rules cover is
    /// properly paired: connected, no bridge error, and the extension
    /// reports living in that browser. Mirrors the browser loop of
    /// the manual start preflight, so the 5-minute warning and the
    /// manual start agree on what "connected" means. A session with
    /// no browsers to pair (no website rules, or a frozen session
    /// with no browser picked) is ready by definition.
    pub fn session_browsers_ready(&self, focus: &FocusConfig) -> bool {
        let ids = focus.session_browsers();
        if ids.is_empty() {
            return true;
        }
        ids.iter().all(|browser_id| {
            let status = self.browser_status_for(browser_id);
            status.connected
                && status.error.is_empty()
                && !status.client_browser.is_empty()
                && crate::browser::same_browser(browser_id, &status.client_browser)
        })
    }
    /// Status of the connected browser matching the foreground app sample
    /// id, if any. Site attribution uses this so another paired browser's
    /// report can never be credited to the foreground app.
    pub fn browser_status_for_app(&self, app_id: &str) -> browser::BrowserStatus {
        self.bridge
            .as_ref()
            .map(|b| b.status_for_app(app_id))
            .unwrap_or_else(|| self.bridge_unavailable())
    }
    /// M4: status when this engine has no bridge of its own. If
    /// another engine's bridge is answering on the loopback port,
    /// the extension is still served — just not by this process —
    /// so report the bridge as owned elsewhere instead of
    /// "unavailable". The probe is a bare TCP connect (see
    /// `Bridge::probe`): no HTTP, no auth, no client state is read,
    /// so nothing about the owner's extension connections leaks.
    fn bridge_unavailable(&self) -> browser::BrowserStatus {
        if self.native && Bridge::probe(&self.data_dir, browser::PORT).port_listening {
            return browser::unavailable(
                "Another Preelock process currently owns the browser bridge.",
            );
        }
        browser::unavailable(&self.bridge_error)
    }
    /// Browser ids with a live extension connection.
    pub fn connected_browsers(&self) -> Vec<String> {
        self.bridge
            .as_ref()
            .map(|b| b.connected_browsers())
            .unwrap_or_default()
    }
    pub fn test_browser(&self) -> Result<(), String> {
        if self.has_active() {
            return Err("Finish your focus session first.".into());
        }
        self.bridge
            .as_ref()
            .ok_or("Browser unavailable.")?
            .request_test()
    }
    pub fn browser_test_result(&self) -> String {
        self.bridge
            .as_ref()
            .map(|b| b.test_result())
            .unwrap_or_default()
    }
    pub fn ensure_bridge_ready(&mut self) {
        if !self.native || !self.owns_engine_lock() {
            return;
        }
        if let Some(bridge) = &self.bridge {
            if bridge.server_alive() {
                return;
            }
            log::warn!(
                "Browser bridge server thread is no longer answering; restarting bridge for setup request."
            );
            self.bridge = None;
        }
        self.bridge_retry_at = 0;
        self.ensure_bridge();
    }
    pub fn pairing_token(&self) -> String {
        if let Some(b) = &self.bridge {
            return b.token.clone();
        }
        match browser::read_or_create_token(&self.data_dir) {
            Ok(token) => token,
            Err(err) => {
                log::warn!("Pairing token read failed ({err}); generating fresh token");
                let _ = std::fs::remove_file(self.data_dir.join("browser-token"));
                browser::read_or_create_token(&self.data_dir).unwrap_or_default()
            }
        }
    }
    pub fn reset_pairing(&mut self, path: &Path) -> Result<(), String> {
        // M1: pairing reset can start a bridge — holder only, so an
        // observer can never steal the bridge from the owner.
        if !self.try_acquire_engine_lock() {
            return Err(Self::ENGINE_LOCK_HELD.into());
        }
        if self.data.sessions.iter().any(|s| {
            matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
        }) {
            return Err("End the session before resetting browser pairing.".into());
        }
        if let Some(bridge) = self.bridge.as_mut() {
            bridge.rotate_token(path)?;
        } else {
            let mut bridge = Bridge::start(path)?;
            bridge.rotate_token(path)?;
            bridge.set_screen_time_tracking(true);
            self.bridge = Some(bridge);
            self.bridge_error = String::new();
            self.bridge_retry_at = 0;
        }
        self.publish();

        Ok(())
    }
    fn preflight(&mut self, id: &str) -> Result<(), String> {
        // Friendly browser name for user-facing errors, falling back to
        // the raw id when the browser isn't installed anymore.
        fn browser_display_name(id: &str) -> String {
            platform::installed_browsers()
                .into_iter()
                .find(|b| b.id.eq_ignore_ascii_case(id))
                .map(|b| b.name)
                .unwrap_or_else(|| id.to_string())
        }
        let s = self
            .data
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or("Session not found.")?;
        if !s.focus.attention.enabled {
            return Err("This is a legacy timer. Create a new productive-time session.".into());
        }
        if s.focus.mode == Mode::Deep {
            return Err("Deep Focus is unavailable: managed OS policy, a restricted account, signed deployment and administrator recovery must be configured separately.".into());
        }
        if s.focus.activity_consent && (!self.native || !platform::supported()) {
            return Err("Native application restrictions are unavailable on this platform.".into());
        }
        if s.focus.camera && (!self.native || platform::camera_permission() != "granted") {
            return Err("Grant camera permission in setup before starting this session.".into());
        }
        if s.focus.attention.verification != "manual_checkpoints" && !s.focus.activity_consent {
            return Err(
                "Activity verification requires your foreground-app sampling consent.".into(),
            );
        }
        if s.focus.attention.verification == "camera_and_activity"
            && !s.focus.camera
            && s.focus.attention.camera_fallback != "activity_only"
        {
            return Err("This session checks you're at your desk with the camera, but camera access isn't allowed. Allow it in setup, or start a session without camera checks.".into());
        }
        if s.focus.network_required() {
            // Session-specific Safari restriction, re-checked here because
            // a stored session can predate it (or be restored from a
            // backup): Safari has no extension, so it can never enforce
            // this session's website rules.
            if s.focus.safari_approved() {
                return Err("This session includes Safari, which can't enforce website rules. Edit the session and choose a supported browser. No session has started.".into());
            }
            // The website rules are enforced in every selected browser, so
            // each one must be paired before the session starts. A selected
            // browser without the extension would browse unfiltered.
            for browser_id in s.focus.session_browsers() {
                let b = self.browser_status_for(&browser_id);
                let wrong_browser = b.client_browser.is_empty()
                    || !browser::same_browser(&browser_id, &b.client_browser);
                if !b.connected || !b.error.is_empty() || wrong_browser {
                    return Err(format!(
                        "Pair the Preelock extension in {} before starting. No session has started.",
                        browser_display_name(&browser_id)
                    ));
                }
            }
        }
        if s.focus.activity_consent {
            platform::sample().map_err(|e| format!("Native activity API unavailable: {e}"))?;
        }
        Ok(())
    }
    pub fn current(&self) -> Snapshot {
        self.data.clone()
    }
    pub fn has_active(&self) -> bool {
        self.data.sessions.iter().any(|s| {
            matches!(
                s.status,
                Status::Running | Status::Paused | Status::Preparing
            )
        })
    }
    /// True while a strict focus session is live (Running, Paused or
    /// Preparing). Browser connection management — extension
    /// disconnect, pairing reset, connecting another browser — is
    /// locked then, and the lock lifts as soon as no strict session
    /// is live anymore.
    pub fn has_active_strict(&self) -> bool {
        self.data.sessions.iter().any(|s| {
            s.focus.mode == Mode::Strict
                && matches!(
                    s.status,
                    Status::Running | Status::Paused | Status::Preparing
                )
        })
    }
    /// Step 12: record a frozen-overlay creation failure as a
    /// report event on the live frozen session, once per run of
    /// consecutive failures (the sync in `lib.rs` retries every
    /// second, so recording every attempt would spam the report).
    /// Enforcement continues regardless: app and website blocking
    /// are engine-side and never depend on the overlay window.
    pub fn note_frozen_overlay_failure(&mut self) {
        // M1: this records a report event and saves it — holder only.
        // An observer skips it (no acquire attempt: it is a
        // best-effort diagnostic, and the owner is the one enforcing).
        if !self.owns_engine_lock() {
            return;
        }
        let now = clock::now();
        let mut changed = false;
        for s in &mut self.data.sessions {
            if s.focus.frozen.enabled && s.status == Status::Running {
                let already = s
                    .report
                    .events
                    .last()
                    .is_some_and(|e| e.kind == "frozen_overlay_failed");
                if !already {
                    s.report.event(
                        now,
                        "frozen_overlay_failed",
                        "The frozen lock overlay window could not be opened; app and website blocking continue, and the overlay will be retried.",
                    );
                    changed = true;
                }
            }
        }
        if changed {
            let next = self.data.clone();
            if self.db.save(&next).is_ok() {
                self.data = next;
            }
        }
    }

    pub fn test_camera(&mut self) -> Result<(), String> {
        if self.has_active() {
            return Err("Finish your session before testing the camera.".into());
        }
        if platform::camera_permission() != "granted" {
            return Err("Allow camera access first.".into());
        }
        if !platform::camera_start() {
            return Err("Camera unavailable. Close other camera apps and try again.".into());
        }
        self.camera_test_until = clock::continuous_ms() + 30_000;
        Ok(())
    }
    pub fn stop_test(&mut self) {
        if self.camera_test_until > 0 {
            platform::camera_stop();
            self.camera_test_until = 0;
        }
    }
    pub fn backup(&mut self) -> Result<Snapshot, String> {
        self.snapshot()?;
        self.db.backup()
    }
    pub fn replace_data(&mut self, data: Snapshot) -> Result<(), String> {
        // M1: a backup restore is a whole-database replace — holder only.
        if !self.try_acquire_engine_lock() {
            return Err(Self::ENGINE_LOCK_HELD.into());
        }
        if self.has_active() {
            return Err("Finish your session before changing history.".into());
        }
        // Replace in one SQLite transaction, including records with matching IDs.
        self.db.replace(&data)?;
        // Re-read so tombstoned (deleted) sessions can never linger in memory.
        self.data = self.db.load()?;
        // A backup restore bypasses `sessions::apply` validation, so
        // a restored stale timer must not fire retroactively on the
        // next tick: purge it here, like at launch.
        if purge_expired_one_shots(&mut self.data, clock::now()) {
            self.db.save(&self.data)?;
        }
        self.stop_test();
        self.publish();
        Ok(())
    }
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        // M1: reading state never needs the lock — but the read must be
        // fresh, and the observer must not save (commit() checkpoints
        // every 5 s). When this engine does not own the lock, reload and
        // return the owner's snapshot without writing anything.
        if !self.owns_engine_lock() && !self.try_acquire_engine_lock() {
            self.data = self.db.load()?;
            self.data.browser = Some(self.browser_status());
            return Ok(self.data.clone());
        }
        self.commit(None, false)
    }

    /// Step 13: recent block attempts, newest first (SQLite rows
    /// only; website blocking has no per-domain rows — see
    /// `stats.rs` — so this list is app attempts).
    pub fn block_attempts(&self) -> Result<Vec<crate::stats::BlockAttempt>, String> {
        self.db.list_attempts(crate::stats::RECENT_LIMIT)
    }
    /// Step 13: JSON export of statistics, as a string for a
    /// frontend Blob download (no file plugin needed). Sessions
    /// summary (productive/distracted totals), per-app attempt
    /// counts, the recent attempt list, the website-block aggregate
    /// (sum of per-session `browser_blocks`, the only website
    /// signal the extension reports), and allowance usage in the
    /// current period. Nothing leaves the device: the string is
    /// handed to the frontend, which downloads it locally.
    pub fn export_statistics(&self) -> Result<String, String> {
        let now = clock::now();
        let attempts = self.db.list_attempts(crate::stats::RECENT_LIMIT)?;
        let (productive_ms, distracted_ms, session_count, completed_count) =
            crate::stats::sessions_summary(&self.data.sessions);
        let website_blocks_total: u64 = self
            .data
            .sessions
            .iter()
            .map(|s| s.report.browser_blocks)
            .sum();
        let allowances: Vec<serde_json::Value> = self
            .data
            .allowances
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "title": a.title,
                    "kind": a.kind,
                    "target": a.target,
                    "budget_minutes": a.budget_minutes,
                    "period": a.period,
                    "mode": a.mode,
                    "enabled": a.enabled,
                    "used_ms": crate::allowances::used_ms(a, now),
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "preelock-statistics",
            "version": 1,
            "exported_at_ms": now,
            "sessions": {
                "productive_ms": productive_ms,
                "distracted_ms": distracted_ms,
                "total": session_count,
                "completed": completed_count,
            },
            "website_blocks_total": website_blocks_total,
            "website_blocks_note": "Aggregate only: the extension reports a single blocks counter, not per-domain counts, so website attempts have no per-site rows.",
            "app_attempt_counts": crate::stats::counts_by_target(&attempts)
                .into_iter()
                .map(|(target, kind, count)| serde_json::json!({
                    "target": target, "kind": kind, "count": count,
                }))
                .collect::<Vec<_>>(),
            "recent_attempts": attempts,
            "allowances": allowances,
        }))
        .map_err(|e| e.to_string())
    }
    /// Step 13: delete statistics data — block-attempt rows only.
    /// Sessions, schedules, tasks, settings, allowances and lock
    /// configurations are untouched, and live sessions are never
    /// affected (this only deletes rows from the attempts table).
    /// Returns the number of rows deleted.
    pub fn delete_statistics(&mut self) -> Result<u64, String> {
        self.db.delete_attempts()
    }
    pub fn execute(&mut self, action: Action) -> Result<Snapshot, String> {
        // M1: writes go through the lock holder only (see commit()).
        // Fail fast, before preflight, so no action runs on a stale
        // observer snapshot.
        if !self.try_acquire_engine_lock() {
            return Err(Self::ENGINE_LOCK_HELD.into());
        }
        if let Action::StartSession { id } | Action::ResumeSession { id } = &action {
            self.stop_test();
            self.preflight(id)?;
        }
        if matches!(&action, Action::EnableCamera { .. }) {
            self.stop_camera();
        }
        let result = self.commit(Some(action), false);
        // Consent revocation or terminal transitions stop camera synchronously.
        if !self
            .data
            .sessions
            .iter()
            .any(|s| s.status == Status::Running && s.focus.camera)
        {
            self.stop_camera();
        }
        result
    }
    fn publish(&self) {
        if let Some(b) = &self.bridge {
            let policy = if self.failed.is_some() {
                None
            } else {
                // Step 14: a grace session publishes no policy, so
                // the extension removes its rules until grace ends
                // — website blocking is not enforced during grace.
                let now = clock::now();
                self.data
                    .sessions
                    .iter()
                    .find(|s| {
                        matches!(s.status, Status::Running | Status::Preparing)
                            && !crate::notifications::grace_active(s.grace_until_ms, now)
                            // Screen Time sampling is always on, so a live session
                            // always publishes a policy: the extension needs it to
                            // report the active domain for usage collection, and a
                            // session with no website rules simply carries none.
                            // Breaks (step 7 Pomodoro, step 9 reward):
                            // on a break the engine publishes no
                            // policy, so the extension removes its
                            // rules until focus republishes them. No
                            // other session can be active at the same
                            // time, so skipping this one publishes
                            // nothing.
                            && !crate::rewards::on_any_break(s)
                    })
                    .map(|s| {
                        // Step 8: exhausted site rules ride the
                        // policy's extra_blocked overlay (the step-1
                        // adult mechanism), outranking allowlist
                        // rules and exceptions in the extension.
                        // Exhaustion is recomputed from current
                        // usage on every publish, so a period roll,
                        // disable, or delete lifts the block on the
                        // next publish with no cleanup step.
                        // App locks: enforced site locks ride the
                        // same overlay — the extension protocol is
                        // unchanged (version stays 6), since the
                        // extension already treats extra_blocked as
                        // a block that outranks everything else.
                        let mut exhausted = crate::allowances::exhausted_site_rules(
                            &self.data.allowances,
                            clock::now(),
                        );
                        exhausted.extend(crate::applocks::locked_site_rules(
                            &self.data.app_locks,
                            &self.data.screen_time_usage,
                            crate::screen_time::local_day_start_ms(now),
                            now,
                        ));
                        exhausted.sort();
                        exhausted.dedup();
                        // "Why am I blocked": the true cause for the
                        // session's own rules, plus per-rule causes for
                        // the extra_blocked overlay (allowances, app
                        // locks), so the extension's blocked page can
                        // explain itself.
                        let rule_causes = self.extra_blocked_causes(now);
                        let mut p = Policy::new(
                            &s.id,
                            &s.focus,
                            if s.status == Status::Preparing || s.focus.attention.enabled {
                                60_000
                            } else {
                                s.remaining_ms
                            },
                        );
                        // If specific target browsers were picked but none are connected while
                        // another paired browser IS connected, clear p.browsers so that the
                        // connected browser can enforce website blocking instead of leaving it unenforced.
                        if !p.browsers().is_empty() {
                            let target_connected =
                                p.browsers().iter().any(|id| b.status_for(id).connected);
                            if !target_connected {
                                p.clear_browsers();
                            }
                        }
                        p.with_exhausted_rules(&exhausted)
                            .with_session_cause(crate::blockreasons::session_cause(
                                s,
                                &self.data.schedules,
                            ))
                            .with_rule_causes(&rule_causes)
                            .with_activity_tracking(true)
                    })
            };
            b.publish(policy);
            // Browser-connection lock, derived from the authoritative
            // session state above: while a strict session is live the
            // bridge refuses explicit disconnects, and the desktop
            // pairing reset already refuses while any session is live
            // (see `reset_pairing`). Publish runs on every tick and on
            // every session transition, so the lock lifts on the same
            // publish that ends the session — no restart needed.
            b.set_connection_locked(self.has_active_strict());
        }
    }
    /// Manual refresh of the browser bridge and enforcement state.
    pub fn refresh_browser(&mut self) {
        if let Some(b) = &self.bridge {
            b.refresh();
        }
        self.publish();
    }
    /// Per-rule block causes for the currently enforced
    /// `extra_blocked` site rules (allowance exhaustion and app
    /// locks). A rule blocked by several causes keeps the most
    /// specific one: timed/daily lock > allowance. The extension's
    /// blocked page looks the matched rule up here.
    fn extra_blocked_causes(&self, now: i64) -> Vec<crate::blockreasons::RuleCause> {
        use crate::blockreasons::{cause_rank, BlockCause, RuleCause};
        let mut causes: Vec<RuleCause> = Vec::new();
        for (rule, title) in crate::allowances::exhausted_site_causes(&self.data.allowances, now) {
            causes.push(RuleCause {
                rule,
                cause: BlockCause::allowance(&title),
            });
        }
        let day_ms = crate::screen_time::local_day_start_ms(now);
        for lock in &self.data.app_locks {
            if lock.kind != crate::allowances::TargetKind::Site {
                continue;
            }
            if !crate::applocks::is_enforced(lock, &self.data.screen_time_usage, day_ms, now) {
                continue;
            }
            let cause = match lock.rule {
                crate::applocks::LockRule::Timed { ends_at_ms } => {
                    BlockCause::timed_lock(&lock.title, ends_at_ms)
                }
                crate::applocks::LockRule::DailyLimit { .. } => {
                    BlockCause::daily_limit(&lock.title)
                }
            };
            causes.push(RuleCause {
                rule: lock.target.clone(),
                cause,
            });
        }
        // Most specific cause wins per rule.
        let mut by_rule: std::collections::HashMap<String, (u8, RuleCause)> =
            std::collections::HashMap::new();
        for rc in causes {
            let rank = cause_rank(&rc.cause);
            by_rule
                .entry(rc.rule.clone())
                .and_modify(|e| {
                    if rank < e.0 {
                        *e = (rank, rc.clone());
                    }
                })
                .or_insert((rank, rc));
        }
        let mut out: Vec<RuleCause> = by_rule.into_values().map(|(_, rc)| rc).collect();
        out.sort_by(|a, b| a.rule.cmp(&b.rule));
        out
    }
    /// "Why am I blocked" for apps: tell the user the true cause
    /// when a blocked app is hidden. Gated by the notifications
    /// preference and throttled to one per app per 60 s (the
    /// block-attempt throttle), so a long blocked stretch never
    /// spams. Never fails a tick.
    fn notify_block_cause(
        &mut self,
        sample: &crate::platform::Sample,
        cause: &crate::blockreasons::BlockCause,
        now: i64,
    ) {
        if !self.data.preferences.notifications_enabled {
            return;
        }
        if !self.attempt_tracker.should_log(&sample.id, now) {
            return;
        }
        let name = if sample.name.is_empty() {
            sample.id.as_str()
        } else {
            sample.name.as_str()
        };
        self.notify_general(
            "Preelock",
            &crate::blockreasons::cause_text(cause, name, now),
        );
    }
    /// Grace-start notification (step 14 parity): tell the user
    /// once, when an auto-started session enters its grace period,
    /// that blocking begins when the countdown ends. The grace
    /// itself never depended on camera — schedule and one-shot
    /// sessions get it unconditionally — but until now it was
    /// silent: only the in-app banner and tray countdown showed
    /// it, so a session that auto-started while the user was not
    /// looking seemed to start instantly, while a camera session
    /// (camera light, readiness popup) drew the eye to the
    /// countdown. This notification makes the 1-minute pre-start
    /// visible for both, with the true remaining seconds.
    /// Returns whether the marker flipped (the caller persists it:
    /// this runs on the tick's early-return path, past the normal
    /// save). Gated on the master notifications toggle and on a
    /// deliverable channel — an undeliverable tick (Windows
    /// background engine, W-1) leaves the marker clear so a later
    /// tick retries while grace is still active. The persisted
    /// flag means a later engine (GUI takeover, restart) never
    /// re-fires. Ending during grace stays free (sessions::apply);
    /// the notification never touches enforcement or the timer.
    fn notify_grace_start(&mut self, s: &mut crate::models::Session, now: i64) -> bool {
        if s.grace_notified || !self.data.preferences.notifications_enabled {
            return false;
        }
        if !crate::desktop_settings::can_notify() {
            return false;
        }
        let secs = crate::notifications::grace_remaining_secs(s.grace_until_ms, now);
        self.notify_general(
            "Session starting",
            &crate::notifications::grace_body(&s.title, secs),
        );
        s.grace_notified = true;
        true
    }
    /// Dismiss the "why am I blocked" app page. Clears the notice so
    /// the tick sync destroys the window; the block itself is
    /// untouched. Called by the `dismiss_blocked_app_page` command
    /// (the page's "Back to work" button and the Escape key).
    pub fn dismiss_blocked_app_page(&mut self) {
        self.data.blocked_app_notice = None;
    }
    fn stop_camera(&mut self) {
        self.camera.stop();
    }
    fn fail_open(&mut self, message: &str) {
        self.failed = Some(message.into());
        self.stop_camera();
        if let Some(b) = &self.bridge {
            b.publish(None);
        }
        for s in &mut self.data.sessions {
            if matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                s.status = Status::Interrupted;
                s.ended_at = Some(clock::now());
                s.deadline_ms = None;
                s.report.enforcement = "Released after error".into();
                s.attention.state = State::Recovering;
                s.attention.reason = message.into();
                s.report.event(clock::now(), "storage_failure", message);
            }
        }
    }
    /// M5: `wake_gap` is true only for the first tick after a long
    /// tick gap (sleep / long suspension, see `tick()`). The
    /// continuous clock keeps running while the machine sleeps on
    /// both macOS (`mach_continuous_time`) and Windows
    /// (`GetTickCount64`), so `elapsed` then spans the whole sleep
    /// — and `sessions::advance` would credit it as focused time
    /// and drain the session's remaining time. A wake tick credits
    /// none of it: the gap is skipped, the session resumes with its
    /// remaining time intact, and later ticks accrue normally.
    /// The true `elapsed` is still used for the wall-jump math
    /// below (wall and continuous agree across sleep, so the jump
    /// stays ~0 and no false clock-jump event fires), and
    /// wall-clock rules — schedule windows, one-shot expiry,
    /// frozen wall expiry — still evaluate on wall time, so a
    /// window that ended during sleep still ends on this tick.
    fn commit(&mut self, action: Option<Action>, wake_gap: bool) -> Result<Snapshot, String> {
        // M1: commit is the single writer behind execute / snapshot /
        // tick, so the lock is enforced here as well as at each caller.
        // An observer refuses outright — its snapshot may be stale, and
        // a whole-snapshot save from it could delete rows the owner
        // just created.
        if !self.try_acquire_engine_lock() {
            return Err(Self::ENGINE_LOCK_HELD.into());
        }
        if let Some(error) = &self.failed {
            return Err(format!("Restrictions released after a storage error. Restart Preelock after resolving: {error}"));
        }
        let tick = clock::continuous_ms();
        let now = clock::now();
        let elapsed = tick.saturating_sub(self.anchor);
        // M5: on a wake tick the sleep span is not focus time (see
        // the doc comment above); every other commit credits the
        // full elapsed interval.
        let credit_elapsed = if wake_gap { 0 } else { elapsed };
        let mut next = self.data.clone();
        // Step 11 clock-tamper detection: compare the wall clock
        // against the monotonic anchor. Productive time is only
        // ever credited from the monotonic `elapsed` below (and
        // from monotonic tick deltas in `tick()`), so a forward
        // wall jump can never be credited and never shortens the
        // remaining time — the remaining time is the authoritative
        // value, and deadlines are recomputed from it. A backward
        // jump likewise never extends the session. The one thing
        // a wall jump COULD shortcut is a step-10 delay lock
        // (its deadline is a wall-clock timestamp), so a forward
        // jump pushes every pending delay deadline forward by
        // exactly the jump BEFORE any action is applied in this
        // same commit — the full delay must still be waited out.
        let jump = crate::tamper::wall_jump_ms(self.wall, now, elapsed);
        let jump_forward = crate::tamper::is_forward_jump(jump);
        let jump_backward = crate::tamper::is_backward_jump(jump);
        if jump_forward {
            for s in &mut next.sessions {
                if matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                ) {
                    s.lock_state.delay_until_ms =
                        crate::tamper::shift_delay_until(s.lock_state.delay_until_ms, jump);
                }
            }
            for schedule in &mut next.schedules {
                schedule.lock_state.delay_until_ms =
                    crate::tamper::shift_delay_until(schedule.lock_state.delay_until_ms, jump);
            }
        }
        for s in &mut next.sessions {
            if (jump_forward || jump_backward)
                && matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                )
            {
                let seconds = jump.abs() / 1000;
                let detail = if jump_forward {
                    format!(
                        "Wall clock jumped forward by {seconds} seconds; the jumped time was not credited, remaining time is unchanged, and any pending delay lock was pushed forward by the same jump."
                    )
                } else {
                    format!(
                        "Wall clock moved backward by {seconds} seconds; remaining time is unchanged and the session was not extended."
                    )
                };
                s.report.event(now, "clock_jump", &detail);
            }
            // Step 14: grace sessions never accrue in commit
            // either — not just in tick. A schedule session is
            // attention-enabled (advance skips it anyway), but a
            // one-shot session need not be, so the guard belongs
            // here, where every advance runs.
            if !s.focus.attention.enabled
                && !crate::notifications::grace_active(s.grace_until_ms, now)
            {
                sessions::advance(s, credit_elapsed, now);
            }
        }
        let changed = action.is_some();
        let start = match &action {
            Some(Action::StartSession { id } | Action::ResumeSession { id }) => Some(id.clone()),
            _ => None,
        };
        if let Some(action) = action {
            sessions::apply(&mut next, action, now)?;
        }
        if let Some(id) = start {
            self.sample_anchor = tick;
            let s = next.sessions.iter_mut().find(|s| s.id == id).unwrap();
            s.attention.checkpoint_at = Some(now);
            s.attention.candidate_ms = 0;
            s.attention.state = State::Unknown;
            s.attention.reason = "Waiting for fresh observations".into();
            if !s.focus.activity_consent && !s.focus.network_required() {
                s.report.enforcement = "No application or browser restrictions requested".into();
            }
            if s.focus.attention.enabled {
                s.deadline_ms = None;
            }
            if s.durability.intended_end_at_ms.is_none() {
                s.durability.intended_end_at_ms =
                    Some(now.saturating_add(s.remaining_ms.min(i64::MAX as u64) as i64));
            }
            if s.durability.recovery_status.is_none() {
                s.durability.recovery_status = Some("clean".into());
            }
            if s.focus.network_required() {
                s.started_at = self
                    .data
                    .sessions
                    .iter()
                    .find(|old| old.id == id)
                    .and_then(|old| old.started_at);
                s.status = Status::Preparing;
                s.attention.state = State::Preparing;
                s.deadline_ms = None;
                s.report.enforcement = "Waiting for browser rule acknowledgement".into();
            }
            s.report.event(
                now,
                "activation_requested",
                "Session policy committed; activating supported restrictions.",
            );
        }
        let transitioned = next
            .sessions
            .iter()
            .zip(&self.data.sessions)
            .any(|(a, b)| a.status != b.status);
        for s in &mut next.sessions {
            if let Some(previous) = self.data.sessions.iter().find(|old| old.id == s.id) {
                if previous.status != s.status
                    && matches!(s.status, Status::Completed | Status::Ended | Status::Paused)
                {
                    s.report.event(now,"restriction_cleanup",if s.status==Status::Paused {"Paused: native monitoring stops; browser cleanup requested."}else{"Session finished: native monitoring stops; browser cleanup requested where applicable."});
                }
            }
            if matches!(
                s.status,
                Status::Completed | Status::Ended | Status::Interrupted | Status::Paused
            ) {
                s.report.enforcement = if s.focus.network_required() {
                    "Native monitoring stopped; browser cleanup requested"
                } else {
                    "Native monitoring stopped"
                }
                .into();
                s.report.active_app.clear();
                s.report.camera_status = "off".into();
                if matches!(s.status, Status::Paused | Status::Ended) {
                    s.attention.state = State::Idle;
                    s.attention.reason =
                        "Session paused or ended by you; monitoring stopped".into();
                }
            }
        }
        if changed || transitioned || tick.saturating_sub(self.checkpoint) >= 5000 {
            for s in &mut next.sessions {
                if matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                ) {
                    s.durability.last_checkpoint_at_ms = Some(now);
                }
            }
            if let Err(e) = self.db.save(&next) {
                self.fail_open(&e);
                return Err(e);
            }
            let _ = crate::lifecycle::write_session_checkpoint(
                &self.lifecycle_path,
                crate::lifecycle::current_boot_id(),
                crate::lifecycle::ending(),
                Self::active_session_checkpoint(&next, now),
            );
            self.checkpoint = tick;
        }
        self.data = next;
        self.anchor = tick;
        self.wall = now;
        self.publish();
        self.data.browser = Some(self.browser_status());
        Ok(self.data.clone())
    }
    /// Step 14 advance warnings, evaluated on every tick from the
    /// in-memory snapshot (fired markers persist in the schedule /
    /// one-shot rows, so a restart cannot re-fire a warning):
    /// - Skipped entirely when the master `notifications_enabled`
    ///   toggle is OFF: nothing fires and nothing is marked, so
    ///   re-enabling before the block starts still warns.
    /// - Skipped when the warning list is empty (no warnings
    ///   configured) or unparseable (corrupt preference data must
    ///   never break a tick; validation on save normally prevents
    ///   this).
    /// - Schedules: only enabled ones, only when no session from
    ///   that schedule is already live, and only for the NEXT
    ///   window start. Stale fired keys for past instances are
    ///   pruned first. One-shots: only timers still present (a
    ///   fired or expired timer is already removed).
    /// - Each due point fires once via the existing
    ///   `desktop_settings::notify` path (no-op on Linux test
    ///   builds, where only the fired markers are observable).
    ///   Returns whether anything changed (and was saved).
    /// - A due point is only recorded as fired when the
    ///   notification can actually be shown from this process
    ///   (`desktop_settings::can_notify`): on Windows the
    ///   background engine has no Tauri app handle, so its
    ///   notifications would silently vanish — the point is left
    ///   unfired instead of consumed, and the crossing due rule
    ///   means it still fires on a later tick once the GUI is
    ///   open. A warning whose block already started is never
    ///   fired late.
    fn evaluate_warnings(&mut self, now: i64) -> Result<bool, String> {
        self.evaluate_warnings_deliverable(now, crate::desktop_settings::can_notify())
    }

    /// The real warning evaluation. `deliverable` is whether an OS
    /// notification can be shown from this process right now; see
    /// the `evaluate_warnings` docs above.
    fn evaluate_warnings_deliverable(
        &mut self,
        now: i64,
        deliverable: bool,
    ) -> Result<bool, String> {
        let preferences = self.data.preferences.clone();
        if !preferences.notifications_enabled {
            return Ok(false);
        }
        let points = crate::notifications::parse_warning_minutes(&preferences.warning_minutes)
            .unwrap_or_default();
        if points.is_empty() {
            return Ok(false);
        }
        let mut next = self.data.clone();
        let mut changed = false;
        let schedules = next.schedules.clone();
        for schedule in &schedules {
            if !schedule.enabled {
                continue;
            }
            let already_started = next.sessions.iter().any(|s| {
                s.schedule_id.as_deref() == Some(schedule.id.as_str())
                    && matches!(
                        s.status,
                        Status::Running | Status::Paused | Status::Preparing
                    )
            });
            if already_started {
                continue;
            }
            let Some(start) = crate::schedules::next_window_start_ms(schedule, now) else {
                continue;
            };
            let slot = next
                .schedules
                .iter_mut()
                .find(|s| s.id == schedule.id)
                .expect("schedule cloned from this list");
            crate::notifications::prune_schedule_warnings(&mut slot.warning_fired, start);
            let due = crate::notifications::due_schedule_warnings(
                start,
                &slot.warning_fired,
                &points,
                now,
            );
            for minutes in due {
                // No channel to show this on (Windows background
                // engine): leave the point unfired so it stays due
                // for a later tick instead of being consumed unseen.
                if !deliverable {
                    continue;
                }
                // Readiness rides on the warning: if the session
                // needs a browser or camera it does not have yet,
                // the notification says so, so the user can fix it
                // before the block starts instead of hitting the
                // fail-closed browser block at start time.
                let missing = crate::notifications::missing_prerequisites(
                    &schedule.focus,
                    self.session_browsers_ready(&schedule.focus),
                    platform::camera_permission() == "granted",
                    cfg!(target_os = "macos"),
                );
                self.notify_general(
                    "Block starting soon",
                    &crate::notifications::warning_body_with_readiness(
                        &schedule.title,
                        start,
                        now,
                        &missing,
                    ),
                );
                slot.warning_fired
                    .push(crate::notifications::schedule_warning_key(start, minutes));
                changed = true;
            }
            // Pruning alone (stale keys dropped) is also a change
            // worth persisting, so the stored list never grows.
            if slot.warning_fired.len() != schedule.warning_fired.len() {
                changed = true;
            }
        }
        let shots = next.one_shots.clone();
        for shot in &shots {
            let due = crate::notifications::due_one_shot_warnings(
                shot.fire_at_ms,
                &shot.warning_fired,
                &points,
                now,
            );
            if due.is_empty() {
                continue;
            }
            for minutes in due {
                // No channel to show this on (Windows background
                // engine): leave the point unfired so it stays due
                // for a later tick instead of being consumed unseen.
                if !deliverable {
                    continue;
                }
                let missing = crate::notifications::missing_prerequisites(
                    &shot.focus,
                    self.session_browsers_ready(&shot.focus),
                    platform::camera_permission() == "granted",
                    cfg!(target_os = "macos"),
                );
                self.notify_general(
                    "Block starting soon",
                    &crate::notifications::warning_body_with_readiness(
                        &shot.title,
                        shot.fire_at_ms,
                        now,
                        &missing,
                    ),
                );
                let slot = next
                    .one_shots
                    .iter_mut()
                    .find(|s| s.id == shot.id)
                    .expect("one-shot cloned from this list");
                slot.warning_fired.push(minutes);
                changed = true;
            }
        }
        if changed {
            self.db.save(&next)?;
            self.data = next;
        }
        Ok(changed)
    }
    /// Scheduled blocks (step 5), evaluated on every tick from the
    /// in-memory snapshot (no separate map, so a manual end or
    /// delete can never leave stale "active schedule" state behind):
    /// - Window EXIT ends the schedule session, always — even when
    ///   the tick was delayed past the boundary, because the check
    ///   uses the current local time, not the session timer. A
    ///   schedule session whose schedule was deleted is ended too,
    ///   so a deleted schedule can never leave an unending block.
    /// - Window ENTER auto-creates and starts a session from the
    ///   schedule's focus snapshot, but only when no session at all
    ///   is active: a manual session is never interrupted, and the
    ///   schedule waits — the next tick after the manual session
    ///   ends starts it, if still inside the window. A window that
    ///   already expired while the app was closed is never started
    ///   retroactively; a window still active at launch starts on
    ///   the first tick after launch.
    /// - The new session spans the whole window (its duration is the
    ///   window length) and runs the exact same tick/cleanup path as
    ///   a normal session from here on. It starts directly Running
    ///   (skipping the browser-acknowledgement gate) so app blocking
    ///   is in force at window entry even before a browser opens;
    ///   website rules are published to connected browsers as usual.
    ///   It is never pausable: pausing would suspend the block.
    ///
    /// Returns whether anything changed (and was saved).
    fn evaluate_schedules(&mut self, now: i64) -> Result<bool, String> {
        let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
        let schedules = self.data.schedules.clone();
        let mut next = self.data.clone();
        let mut changed = false;
        // Pass 1: end every schedule session whose window has closed.
        for s in &mut next.sessions {
            let Some(schedule_id) = s.schedule_id.clone() else {
                continue;
            };
            if !matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                continue;
            }
            let schedule = schedules.iter().find(|s| s.id == schedule_id);
            let window_over = match schedule {
                None => true,
                Some(schedule) => !crate::schedules::window_active(schedule, weekday, minutes),
            };
            if window_over {
                s.status = Status::Completed;
                s.ended_at = Some(now);
                s.deadline_ms = None;
                s.grace_until_ms = None;
                // Step 9: the window ending ends any break and
                // discards unspent credit.
                crate::rewards::clear(s);
                crate::locks::clear_state(&mut s.lock_state);
                s.attention.state = State::Idle;
                s.attention.reason = "Schedule window ended".into();
                s.report.active_app.clear();
                s.report.camera_status = "off".into();
                s.report.enforcement = "Schedule window ended; monitoring stopped".into();
                s.report.event(
                    now,
                    if schedule.is_none() {
                        "schedule_removed"
                    } else {
                        "schedule_window_ended"
                    },
                    "Scheduled block ended.",
                );
                changed = true;
            }
        }
        // Pass 2: start at most one entered window, and only when
        // nothing is active (a manual session waits out the window).
        let any_active = next.sessions.iter().any(|s| {
            matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            )
        });
        if !any_active {
            for schedule in &schedules {
                let has_schedule_session = next.sessions.iter().any(|s| {
                    s.schedule_id.as_deref() == Some(schedule.id.as_str())
                        && matches!(
                            s.status,
                            Status::Running | Status::Preparing | Status::Paused
                        )
                });
                if crate::schedules::next_action(
                    schedule,
                    weekday,
                    minutes,
                    has_schedule_session,
                    any_active,
                ) != crate::schedules::ScheduleAction::Start
                {
                    continue;
                }
                let occurrence_key = crate::schedules::window_occurrence_key(schedule, now);
                if occurrence_key.as_deref().is_some_and(|key| {
                    next.sessions
                        .iter()
                        .any(|s| s.durability.schedule_window_key.as_deref() == Some(key))
                }) {
                    continue;
                }
                let mut focus = match crate::focus::validate(schedule.focus.clone()) {
                    Ok(focus) => focus,
                    Err(e) => {
                        log::error!(
                            "Scheduled block '{}' skipped: invalid focus snapshot: {e}",
                            schedule.title
                        );
                        continue;
                    }
                };
                focus.attention.enabled = true;
                let duration = crate::schedules::window_minutes(schedule);
                let remaining_ms = u64::from(duration) * 60_000;
                // Pomodoro config rides the schedule's focus snapshot
                // (step 7): a schedule session starts in Focus like
                // any other session; its window end still ends it.
                let pomodoro = crate::pomodoro::initial_state(&focus.pomodoro);
                let mut session = crate::models::Session {
                    id: uuid::Uuid::new_v4().to_string(),
                    title: schedule.title.clone(),
                    duration_minutes: duration,
                    remaining_ms,
                    focused_ms: 0,
                    allow_pause: false,
                    status: Status::Running,
                    started_at: Some(now),
                    ended_at: None,
                    deadline_ms: if focus.attention.enabled {
                        None
                    } else {
                        Some(now + remaining_ms as i64)
                    },
                    focus,
                    report: Default::default(),
                    attention: Default::default(),
                    force_quit: false,
                    schedule_id: Some(schedule.id.clone()),
                    kind: crate::models::SessionKind::Scheduled,
                    pomodoro,
                    reward: Default::default(),
                    // Step 10: a schedule session inherits the
                    // schedule's lock, with a fresh challenge for
                    // this session (the schedule keeps its own
                    // challenge for its own changes).
                    lock: schedule.lock.clone(),
                    lock_state: crate::locks::new_state(
                        &schedule.lock,
                        &format!("{}:{now}", schedule.id),
                    ),
                    policy_applied: false,
                    browser_fail_closed: false,
                    // Step 14: auto-started sessions get the global
                    // grace period; manual sessions never do.
                    grace_until_ms: crate::notifications::grace_until_for(
                        self.data.preferences.grace_seconds,
                        now,
                    ),
                    // Step 14 grace-start notification: not sent
                    // yet — the tick notifies once when grace is
                    // entered (see notify_grace_start).
                    grace_notified: false,
                    durability: crate::models::SessionDurability {
                        intended_end_at_ms: Some(
                            now.saturating_add(remaining_ms.min(i64::MAX as u64) as i64),
                        ),
                        schedule_window_key: occurrence_key,
                        last_checkpoint_at_ms: Some(now),
                        last_enforcement_at_ms: Some(now),
                        recovery_status: Some("clean".into()),
                    },
                    start_notified: false,
                    complete_notified: false,
                    inactivity_notified: false,
                };
                session.report.event(
                    now,
                    "schedule_started",
                    "Scheduled block started automatically.",
                );
                if let Some(until) = session.grace_until_ms {
                    let secs = crate::notifications::grace_remaining_secs(Some(until), now);
                    session.report.event(
                        now,
                        "grace_started",
                        &format!(
                            "Grace period: blocking begins in {secs} seconds; finish up while it runs."
                        ),
                    );
                }
                session.report.enforcement = "Scheduled block active".into();
                next.sessions.push(session);
                changed = true;
                break;
            }
        }
        if changed {
            self.db.save(&next)?;
            self.data = next;
            self.publish();
        }
        Ok(changed)
    }
    /// One-shot start timers (step 6c), evaluated on every tick
    /// AFTER schedules, so a schedule that starts on the same tick
    /// wins and the one-shot is skipped, not queued:
    /// - Expired timers are removed without starting anything: a
    ///   time that passed during a tick gap (sleep) or arrived
    ///   while another session was running never fires late.
    /// - At most one timer fires per tick, and only when nothing is
    ///   active. The new session copies the timer's snapshot
    ///   (duration, pause setting, focus) and starts directly
    ///   Running, like a schedule session; unlike a schedule
    ///   session it has no schedule_id, so it is an ordinary
    ///   duration-bound session from here on — pausable when its
    ///   template was, and never auto-ended by a window end.
    ///
    /// Returns whether anything changed (and was saved).
    fn evaluate_one_shots(&mut self, now: i64, tick_gap_exceeded: bool) -> Result<bool, String> {
        let mut next = self.data.clone();
        let mut changed = false;
        // Pass 1: remove every timer that must not fire. Active
        // sessions are read before any removal, since removals
        // never change which sessions are active.
        let active_before = next.sessions.iter().any(|s| {
            matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            )
        });
        next.one_shots.retain(|shot| {
            let action = crate::schedules::one_shot_action(
                shot,
                now,
                tick_gap_exceeded,
                active_before,
            );
            if action == crate::schedules::OneShotAction::Expire {
                log::info!(
                    "One-shot timer '{}' skipped: its time passed while the app was closed, asleep, or busy.",
                    shot.title
                );
                changed = true;
                false
            } else {
                true
            }
        });
        // Pass 2: fire at most one due timer, only when idle. A
        // timer whose focus snapshot no longer validates is removed
        // and the scan continues to the next due timer.
        let mut fired = false;
        let mut index = 0;
        while !fired && index < next.one_shots.len() {
            let shot = next.one_shots[index].clone();
            let any_active = next.sessions.iter().any(|s| {
                matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                )
            });
            if crate::schedules::one_shot_action(&shot, now, tick_gap_exceeded, any_active)
                != crate::schedules::OneShotAction::Fire
            {
                index += 1;
                continue;
            }
            let focus = match crate::focus::validate(shot.focus.clone()) {
                Ok(focus) => focus,
                Err(e) => {
                    log::error!(
                        "One-shot timer '{}' skipped: invalid focus snapshot: {e}",
                        shot.title
                    );
                    next.one_shots.remove(index);
                    changed = true;
                    continue;
                }
            };
            let remaining_ms = u64::from(shot.minutes) * 60_000;
            // Pomodoro config rides the one-shot's focus snapshot
            // (step 7), same as a schedule session. Step 12: a
            // frozen one-shot is never pausable, mirroring the
            // CreateSession forcing in sessions.rs.
            let pomodoro = crate::pomodoro::initial_state(&focus.pomodoro);
            let mut session = crate::models::Session {
                id: uuid::Uuid::new_v4().to_string(),
                title: shot.title.clone(),
                duration_minutes: shot.minutes,
                remaining_ms,
                focused_ms: 0,
                allow_pause: shot.allow_pause && !focus.frozen.enabled,
                status: Status::Running,
                started_at: Some(now),
                ended_at: None,
                deadline_ms: if focus.attention.enabled {
                    None
                } else {
                    Some(now + remaining_ms as i64)
                },
                focus,
                report: Default::default(),
                attention: Default::default(),
                force_quit: false,
                schedule_id: None,
                // The one-shot record is purged when it fires, so
                // the kind is recorded here: without it the session
                // would be indistinguishable from an instant focus
                // session from here on.
                kind: crate::models::SessionKind::OneShot,
                pomodoro,
                reward: Default::default(),
                // One-shot timers carry no step 10 lock: they are
                // ordinary duration-bound sessions once started.
                lock: Default::default(),
                lock_state: Default::default(),
                policy_applied: false,
                browser_fail_closed: false,
                // Step 14: same global grace as schedule sessions.
                grace_until_ms: crate::notifications::grace_until_for(
                    self.data.preferences.grace_seconds,
                    now,
                ),
                // Step 14 grace-start notification: not sent yet —
                // the tick notifies once when grace is entered
                // (see notify_grace_start).
                grace_notified: false,
                durability: crate::models::SessionDurability {
                    intended_end_at_ms: Some(
                        now.saturating_add(remaining_ms.min(i64::MAX as u64) as i64),
                    ),
                    schedule_window_key: None,
                    last_checkpoint_at_ms: Some(now),
                    last_enforcement_at_ms: Some(now),
                    recovery_status: Some("clean".into()),
                },
                start_notified: false,
                complete_notified: false,
                inactivity_notified: false,
            };
            session.report.event(
                now,
                "one_shot_started",
                "One-shot block started automatically.",
            );
            if let Some(until) = session.grace_until_ms {
                let secs = crate::notifications::grace_remaining_secs(Some(until), now);
                session.report.event(
                    now,
                    "grace_started",
                    &format!(
                        "Grace period: blocking begins in {secs} seconds; finish up while it runs."
                    ),
                );
            }
            session.report.enforcement = "One-shot block active".into();
            next.sessions.push(session);
            next.one_shots.remove(index);
            changed = true;
            fired = true;
        }
        if changed {
            self.db.save(&next)?;
            self.data = next;
            self.publish();
        }
        Ok(changed)
    }
    /// M5: the single screen-time recording path. Attributes one
    /// sampled interval (at most one bucket — a website replaces
    /// the browser app, never adds to it, and the extension and the
    /// native adapter never both contribute) and writes it once: an
    /// incrementing row in the `screen_time_usage` table plus the
    /// same amount in the in-memory usage the snapshot later saves
    /// with overwrite semantics, so a save after a record can never
    /// double the total (see the M5 round-trip test). Called only
    /// from the always-on sampler, which only the engine-lock
    /// holder runs — no heartbeat or tray state is consulted, so
    /// recording works identically with the GUI alive, the GUI
    /// closed, the GUI force-quit, or no tray process at all, and
    /// with or without a live session. Returns whether anything
    /// was recorded.
    fn record_screen_time_buckets(
        &mut self,
        now: i64,
        delta: u64,
        sample: &crate::platform::Sample,
        browser_for_site: &crate::browser::BrowserStatus,
        native_site: Option<&crate::websites::NativeSite>,
    ) -> Result<bool, String> {
        let buckets = crate::screen_time::sample_buckets_with_native(
            now,
            delta,
            sample,
            browser_for_site,
            native_site,
        );
        if buckets.is_empty() {
            return Ok(false);
        }
        self.db.record_screen_time(&buckets)?;
        for bucket in &buckets {
            crate::screen_time::add_usage(
                &mut self.data.screen_time_usage,
                bucket.day_ms,
                &bucket.kind,
                &bucket.target,
                &bucket.name,
                bucket.ms,
            );
        }
        Ok(true)
    }
    fn sample_screen_time(&mut self, now: i64, delta: u64) -> Result<(), String> {
        if delta == 0 || delta > 5000 || !self.native {
            return Ok(());
        }
        let sample = platform::sample()?;
        // Site attribution is scoped to the connected browser that
        // matches the foreground app, so another paired browser's report
        // can never be credited to this interval. The allowance accrual
        // below keeps using the most-recently-seen connection exactly as
        // before (its own frontmost check handles apps).
        let browser_for_site = self.browser_status_for_app(&sample.id);
        let browser = self.browser_status();
        // Native website identification (extension-free), sampled only for
        // a browser this platform has an adapter for, and skipped when the
        // extension already reports a fresh, focused domain for it — the
        // credited result is identical and no OS query is spent.
        let native_site = if browser_for_site.activity_fresh
            && browser_for_site.browser_focused
            && !browser_for_site.active_domain.is_empty()
        {
            None
        } else {
            crate::websites::native_site(&sample.id)
        };
        let has_usage = self.record_screen_time_buckets(
            now,
            delta,
            &sample,
            &browser_for_site,
            native_site.as_ref(),
        )?;
        let day_ms = crate::screen_time::local_day_start_ms(now);
        if self.has_active() {
            return Ok(());
        }
        // Screen Time limits remain active outside focus sessions. The app
        // budget is enforced by hiding the frontmost app; site limits are
        // pushed to the browser's observe-only policy below.
        let usage = crate::allowances::TickUsage {
            sample_id: &sample.id,
            sample_idle_ms: sample.idle_ms,
            sample_available: true,
            browser_domain: &browser.active_domain,
            browser_focused: browser.browser_focused,
            browser_activity_fresh: browser.activity_fresh,
            browser_client_id: &browser.client_browser,
            session_browser_app: "",
        };
        let allowance_changed = crate::allowances::accrue_for_tick(
            &mut self.data.allowances,
            now,
            delta,
            false,
            &usage,
        );
        if sample.idle_ms < 180_000
            && !crate::focus::protected(&sample.id)
            && crate::allowances::app_exhausted(&self.data.allowances, now, &sample.id, "")
        {
            let _ = platform::hide(&sample);
            // "Why am I blocked": the exhausted allowance's title,
            // throttled per app inside notify_block_cause.
            if let Some(a) = crate::allowances::exhausted_allowance_for_app(
                &self.data.allowances,
                now,
                &sample.id,
                "",
            ) {
                let cause = crate::blockreasons::BlockCause::allowance(&a.title);
                self.notify_block_cause(&sample, &cause, now);
                // "Why am I blocked" app page (see record_blocked_app_page).
                record_blocked_app_page(
                    &mut self.data.blocked_app_notice,
                    &mut self.overlay_last_shown,
                    &sample,
                    &cause,
                    now,
                );
            }
        }
        // App locks: independent of sessions. Expired timed locks
        // are pruned here (the save below persists the deletion);
        // a locked app is hidden best-effort, exactly like the
        // allowance path above. Unlike allowances there is no
        // browser exemption: the user targeted this app explicitly,
        // and hiding keeps its process and the paired extension
        // alive.
        let mut strict_changed = false;
        if crate::adult_block::check_expiry(&mut self.data.adult_block, now) {
            strict_changed = true;
        }
        if crate::adult_block::check_strict_transition(
            &mut self.data.strict_apps.enabled,
            &mut self.data.strict_apps.strict,
            &mut self.data.strict_apps.ends_at_ms,
            &mut self.data.strict_apps.activating_until_ms,
            now,
        ) {
            strict_changed = true;
            if self.data.strict_apps.strict {
                if let Some(strict_end) = self.data.strict_apps.ends_at_ms {
                    for lock in &mut self.data.app_locks {
                        if lock.kind == crate::allowances::TargetKind::App {
                            if let crate::applocks::LockRule::Timed { ends_at_ms } = &mut lock.rule
                            {
                                if *ends_at_ms < strict_end {
                                    *ends_at_ms = strict_end;
                                }
                            }
                        }
                    }
                }
            }
        }
        if crate::adult_block::check_strict_transition(
            &mut self.data.strict_sites.enabled,
            &mut self.data.strict_sites.strict,
            &mut self.data.strict_sites.ends_at_ms,
            &mut self.data.strict_sites.activating_until_ms,
            now,
        ) {
            strict_changed = true;
            if self.data.strict_sites.strict {
                if let Some(strict_end) = self.data.strict_sites.ends_at_ms {
                    for lock in &mut self.data.app_locks {
                        if lock.kind == crate::allowances::TargetKind::Site {
                            if let crate::applocks::LockRule::Timed { ends_at_ms } = &mut lock.rule
                            {
                                if *ends_at_ms < strict_end {
                                    *ends_at_ms = strict_end;
                                }
                            }
                        }
                    }
                }
            }
        }
        if strict_changed {
            let _ = self.db.save(&self.data);
        }

        let locks_pruned = if self.data.strict_apps.strict || self.data.strict_sites.strict {
            false
        } else {
            crate::applocks::prune_expired(&mut self.data.app_locks, now)
        };
        if sample.idle_ms < 180_000
            && !crate::focus::protected(&sample.id)
            && crate::applocks::app_locked(
                &self.data.app_locks,
                &self.data.screen_time_usage,
                day_ms,
                now,
                &sample.id,
            )
        {
            let _ = platform::hide(&sample);
            // "Why am I blocked": the enforced lock's own kind
            // (timed with remaining time, or daily limit).
            if let Some(lock) = crate::applocks::enforced_lock_for_app(
                &self.data.app_locks,
                &self.data.screen_time_usage,
                day_ms,
                now,
                &sample.id,
            ) {
                let cause = match lock.rule {
                    crate::applocks::LockRule::Timed { ends_at_ms } => {
                        crate::blockreasons::BlockCause::timed_lock(&lock.title, ends_at_ms)
                    }
                    crate::applocks::LockRule::DailyLimit { .. } => {
                        crate::blockreasons::BlockCause::daily_limit(&lock.title)
                    }
                };
                self.notify_block_cause(&sample, &cause, now);
                // "Why am I blocked" app page (see record_blocked_app_page).
                record_blocked_app_page(
                    &mut self.data.blocked_app_notice,
                    &mut self.overlay_last_shown,
                    &sample,
                    &cause,
                    now,
                );
            }
        }
        let domain_to_check = if browser_for_site.activity_fresh
            && browser_for_site.browser_focused
            && !browser_for_site.active_domain.is_empty()
        {
            Some(browser_for_site.active_domain.as_str())
        } else {
            native_site.as_ref().map(|s| s.domain.as_str())
        };
        if let Some(domain) = domain_to_check {
            if self.data.adult_block.enabled && crate::adult_block::is_adult_domain(domain) {
                platform::redirect_browser_blocked(&sample.name);
                if crate::focus::safari(&sample.id) {
                    platform::redirect_safari_blocked();
                }
                let cause = crate::blockreasons::BlockCause::adult();
                self.notify_block_cause(&sample, &cause, now);
                record_blocked_app_page(
                    &mut self.data.blocked_app_notice,
                    &mut self.overlay_last_shown,
                    &sample,
                    &cause,
                    now,
                );
                let _ = self.db.record_attempt(&crate::stats::BlockAttempt {
                    at_ms: now,
                    kind: crate::stats::AttemptKind::Site,
                    target: domain.into(),
                    session_id: None,
                    reason: "adult_block".into(),
                });
            } else if sample.idle_ms < 600_000
                && crate::applocks::site_locked(
                    &self.data.app_locks,
                    &self.data.screen_time_usage,
                    day_ms,
                    now,
                    domain,
                )
            {
                if crate::focus::safari(&sample.id) {
                    platform::redirect_safari_blocked();
                }
                let _ = platform::hide(&sample);
                if let Some(lock) = crate::applocks::enforced_lock_for_site(
                    &self.data.app_locks,
                    &self.data.screen_time_usage,
                    day_ms,
                    now,
                    domain,
                ) {
                    let cause = match lock.rule {
                        crate::applocks::LockRule::Timed { ends_at_ms } => {
                            crate::blockreasons::BlockCause::timed_lock(&lock.title, ends_at_ms)
                        }
                        crate::applocks::LockRule::DailyLimit { .. } => {
                            crate::blockreasons::BlockCause::daily_limit(&lock.title)
                        }
                    };
                    self.notify_block_cause(&sample, &cause, now);
                    record_blocked_app_page(
                        &mut self.data.blocked_app_notice,
                        &mut self.overlay_last_shown,
                        &sample,
                        &cause,
                        now,
                    );
                }
            }
        }
        if let Some(bridge) = &self.bridge {
            let mut site_rules =
                crate::allowances::exhausted_site_rules(&self.data.allowances, now);
            site_rules.extend(crate::applocks::locked_site_rules(
                &self.data.app_locks,
                &self.data.screen_time_usage,
                day_ms,
                now,
            ));
            site_rules.sort();
            site_rules.dedup();
            let rule_causes = self.extra_blocked_causes(now);
            bridge.set_screen_time_blocked_rules(&site_rules, &rule_causes);
        }
        // A pruned timed lock must persist its deletion at once
        // (one save, then the vec stays empty); the allowance
        // 5-second flush checkpoint is otherwise untouched.
        let allowance_due = allowance_changed
            && clock::continuous_ms().saturating_sub(self.allowance_checkpoint) >= 5000;
        if allowance_due || locks_pruned || strict_changed {
            self.db.save(&self.data)?;
            if allowance_due {
                self.allowance_checkpoint = clock::continuous_ms();
            }
        }
        if has_usage || allowance_changed {
            self.publish();
        }
        Ok(())
    }
    /// Advance engine state by one tick and evaluate durable background notifications.
    pub fn tick(&mut self) -> Result<(), String> {
        let result = self.tick_inner();
        if result.is_ok() && self.owns_engine_lock() {
            self.evaluate_notifications();
        }
        result
    }

    fn evaluate_notifications(&mut self) {
        if !crate::desktop_settings::can_notify() {
            return;
        }
        let now = clock::now();
        let p = self.data.preferences.clone();
        let mut session_changed = false;

        for s in &mut self.data.sessions {
            // 1. Session completed alert
            if s.status == Status::Completed && !s.complete_notified {
                if p.notifications_enabled && p.session_alert {
                    crate::desktop_settings::notify("Focus complete", "You reached your target.");
                    if p.sound {
                        crate::desktop_settings::sound(p.volume);
                    }
                }
                s.complete_notified = true;
                session_changed = true;
            }

            // 2. Session started alert (only once grace has finished or if no grace)
            if s.status == Status::Running
                && !crate::notifications::grace_active(s.grace_until_ms, now)
                && !s.start_notified
            {
                if p.notifications_enabled && p.session_start_alert {
                    crate::desktop_settings::notify(
                        "Focus started",
                        "Your session timer is running.",
                    );
                    if p.session_start_sound {
                        crate::desktop_settings::sound(p.volume);
                    }
                }
                s.start_notified = true;
                session_changed = true;
            }

            // 3. Inactivity reminder: absent from desk during an active session
            if matches!(s.status, Status::Running | Status::Preparing) {
                if s.attention.state == crate::attention::State::Absent {
                    if !s.inactivity_notified {
                        if p.notifications_enabled {
                            crate::desktop_settings::notify(
                                "Focus paused",
                                "No activity detected for 5 minutes. Move your mouse or type to resume.",
                            );
                            if p.notifications_sound {
                                crate::desktop_settings::sound(p.volume);
                            }
                        }
                        s.inactivity_notified = true;
                        session_changed = true;
                    }
                } else if s.attention.state == crate::attention::State::Focused
                    && s.inactivity_notified
                {
                    s.inactivity_notified = false;
                    session_changed = true;
                }
            }
        }

        if session_changed {
            let _ = self.db.save(&self.data);
        }

        // 4. User-configured daily focus reminder
        self.evaluate_daily_reminders();
    }

    fn evaluate_daily_reminders(&mut self) {
        let p = self.data.preferences.clone();
        if !p.reminder || !p.notifications_enabled || self.has_active() {
            return;
        }
        if let Ok(info) = crate::desktop_settings::info() {
            let (Some(day), Some(time_str), Some(date_str)) = (
                info["day"].as_u64(),
                info["time"].as_str(),
                info["date"].as_str(),
            ) else {
                return;
            };
            let key = format!("{}:{}", date_str, p.reminder_time);
            if time_str == p.reminder_time
                && p.reminder_days.contains(&(day as u32))
                && self.data.preferences.last_reminder_date != key
            {
                self.data.preferences.last_reminder_date = key;
                crate::desktop_settings::notify(
                    "Time to focus",
                    "Choose a task when you’re ready.",
                );
                if p.notifications_sound {
                    crate::desktop_settings::sound(p.volume);
                }
                let _ = self.db.save(&self.data);
            }
        }
    }
    fn tick_inner(&mut self) -> Result<(), String> {
        // M1: only the engine holding the cross-process lock may tick.
        // An observer reloads the snapshot so the UI keeps showing the
        // owner's state, then returns without ticking, sampling,
        // publishing, or saving anything.
        if !self.try_acquire_engine_lock() {
            if let Ok(data) = self.db.load() {
                self.data = data;
            }
            return Ok(());
        }
        // M4: a holder that opened or took over bridge-less retries
        // the bridge here (throttled, one fast attempt), and a holder
        // whose bridge server thread died restarts it — before any
        // tick work publishes or samples, so enforcement never runs
        // a tick with a bridge it could already have back.
        self.ensure_bridge();
        // Step 6b: wall-clock gap detection. Ticks pause during sleep
        // on both OSes, so a gap over the threshold means wake from
        // sleep (or a long suspension). The tick below runs commit()
        // with the gap flag (M5: the sleep span is never credited
        // as focus time — the continuous clock runs through sleep)
        // and then schedule re-evaluation on current local time, so
        // a window that ended during sleep ends on this same tick
        // and is never extended. The gap flag additionally expires
        // due one-shot timers (step 6c) instead of firing them
        // late. The timestamp is captured once so schedule and
        // one-shot decisions share one moment.
        let tick_wall = clock::now();
        let gap_exceeded = tick_gap_exceeded(self.last_tick_wall_ms, tick_wall);
        self.last_tick_wall_ms = Some(tick_wall);
        if gap_exceeded {
            log::warn!(
                "Long tick gap detected; treating as wake from sleep and re-evaluating schedules."
            );
        }
        if self.camera_test_until > 0 && clock::continuous_ms() >= self.camera_test_until {
            self.stop_test();
        }
        // M5: the gap flag travels into commit so the sleep span is
        // never credited as focus time (see `commit`).
        self.commit(None, gap_exceeded)?;
        if crate::background::tracker_alive(&self.data_dir) {
            self.data.screen_time_usage = self.db.load_screen_time_usage()?;
        }
        let running = self
            .data
            .sessions
            .iter()
            .any(|s| s.status == Status::Running);
        if self.native {
            let sample_tick = clock::continuous_ms();
            let sample_delta = sample_tick.saturating_sub(self.screen_time_anchor);
            self.screen_time_anchor = sample_tick;
            if sample_delta > 0 {
                self.sample_screen_time(tick_wall, sample_delta)?;
            }
        } else {
            self.screen_time_anchor = clock::continuous_ms();
        }
        // App locks: prune expired timed locks on every tick — the
        // session and the no-session path alike. Enforcement is
        // wall-clock computed, so an expired lock can never block
        // past its end; the prune deletes it and the save persists
        // the deletion at once.
        if crate::applocks::prune_expired(&mut self.data.app_locks, tick_wall) {
            self.db.save(&self.data)?;
        }
        // Screen Time is always on: keep the observe-only lease fresh
        // and push exhausted site rules while no session runs.
        if let Some(bridge) = &self.bridge {
            bridge.refresh_screen_time_tracking();
            if !running {
                // App locks ride the same observe-only rules as
                // exhausted allowances: merged, sorted, deduped.
                let mut site_rules =
                    crate::allowances::exhausted_site_rules(&self.data.allowances, tick_wall);
                site_rules.extend(crate::applocks::locked_site_rules(
                    &self.data.app_locks,
                    &self.data.screen_time_usage,
                    crate::screen_time::local_day_start_ms(tick_wall),
                    tick_wall,
                ));
                site_rules.sort();
                site_rules.dedup();
                let rule_causes = self.extra_blocked_causes(tick_wall);
                bridge.set_screen_time_blocked_rules(&site_rules, &rule_causes);
            }
        }
        // Step 14: advance warnings, evaluated on every tick before
        // schedules/one-shots start anything: a warning is due only
        // while its block has not started (see notifications.rs),
        // so evaluating first can never warn for a block that
        // starts on this same tick. A warning failure is logged
        // only: warnings must never break a tick.
        if let Err(e) = self.evaluate_warnings(tick_wall) {
            log::error!("Warning evaluation failed: {e}");
        }
        // Schedules are evaluated on every tick, after the normal
        // commit, so a schedule session joins the same tick pipeline
        // (attention, browser policy) as any other live session.
        if let Err(e) = self.evaluate_schedules(tick_wall) {
            log::error!("Schedule evaluation failed: {e}");
        }
        // One-shot timers after schedules, on the same tick.
        if let Err(e) = self.evaluate_one_shots(tick_wall, gap_exceeded) {
            log::error!("One-shot evaluation failed: {e}");
        }
        // Step 12: frozen wall-clock expiry, checked on every tick
        // before any sample is taken. A frozen session ends when
        // its wall-clock timer runs out (or its productive
        // remaining time hits zero), even if no productive work
        // was recorded — that is the failsafe that guarantees the
        // lock can never outlive its timer. The session completes
        // here, so the rest of the tick (which returns early with
        // no live session) publishes no policy and the overlay
        // sync destroys the window on the same tick.
        if let Some(index) = self.data.sessions.iter().position(|s| {
            s.focus.frozen.enabled
                && matches!(s.status, Status::Running | Status::Preparing)
                && (s.remaining_ms == 0
                    || crate::frozen::wall_expired(s.started_at, s.duration_minutes, tick_wall))
        }) {
            let mut next = self.data.clone();
            let s = &mut next.sessions[index];
            s.status = Status::Completed;
            s.ended_at = Some(tick_wall);
            s.deadline_ms = None;
            // Step 14: the frozen wall timer wins over grace — a
            // grace session whose timer runs out completes here,
            // even though enforcement never kicked in.
            s.grace_until_ms = None;
            s.browser_fail_closed = false;
            crate::rewards::clear(s);
            crate::locks::clear_state(&mut s.lock_state);
            s.attention.state = State::Idle;
            s.attention.reason = "Frozen lock expired".into();
            s.report.active_app.clear();
            s.report.camera_status = "off".into();
            s.report.enforcement = "Frozen lock expired; monitoring stopped".into();
            s.report.event(
                tick_wall,
                "frozen_expired",
                "Frozen lock lifted on expiry; the timer ran out and the full-computer lock ended automatically.",
            );
            if self.db.save(&next).is_err() {
                log::error!("Could not save frozen expiry; session continues.");
            } else {
                self.data = next;
                self.publish();
            }
            return Ok(());
        }
        let now = tick_wall;
        let tick = clock::continuous_ms();
        let delta = tick.saturating_sub(self.sample_anchor);
        self.sample_anchor = tick;
        let Some(index) = self
            .data
            .sessions
            .iter()
            .position(|s| matches!(s.status, Status::Preparing | Status::Running))
        else {
            self.stop_camera();
            self.attempt_tracker.clear_blocked();
            return Ok(());
        };
        let mut next = self.data.clone();
        let s = &mut next.sessions[index];
        // Step 14 grace: while grace is active, enforcement has not
        // kicked in yet, so this tick does nothing at all — no
        // sampling, no hiding, no productive/distracted accrual, no
        // Pomodoro/reward/allowance progress, no block-attempt rows,
        // and (via publish()) no website policy. The session's
        // remaining time is untouched, so grace never extends its
        // productive target. When grace has expired, clear it with
        // one report event and fall through to normal enforcement
        // on this same tick. A schedule window ending during grace
        // already ended the session above, and the frozen wall
        // timer above still wins during grace.
        if let Some(until) = s.grace_until_ms {
            if now >= until {
                s.grace_until_ms = None;
                s.report.event(
                    now,
                    "grace_ended",
                    "Grace period ended; blocking is now in force.",
                );
            }
        }
        if crate::notifications::grace_active(s.grace_until_ms, now) {
            // Grace-start notification (step 14 parity): the
            // 1-minute pre-start is the same for camera and
            // non-camera sessions, but until now it was silent —
            // only the in-app banner and tray countdown showed it.
            // Notify once, with the true remaining seconds; the
            // marker persists in the session JSON, so a later
            // engine never re-fires and an undeliverable tick
            // retries while grace is still active. publish()
            // skips grace sessions, so the extension keeps no
            // rules during grace.
            if self.notify_grace_start(s, now) {
                // This branch returns early without reaching the
                // tick's normal save: persist the marker now, or
                // the next tick (or the next engine) would fire
                // again. A failed save retries on the next tick.
                if let Err(e) = self.db.save(&next) {
                    log::error!("Could not save grace notification marker: {e}");
                } else {
                    self.data = next;
                }
            }
            return Ok(());
        }
        // Breaks (step 7 Pomodoro, step 9 reward): during a break
        // the block relaxes (apps unblocked below, no network policy
        // from publish) and the tick runs the break timer instead of
        // productive-time accounting. A pending delayed break is NOT
        // a break: focus and blocking continue while it counts down.
        // Pausing needs no special case: a Paused session gets no
        // ticks, so its break timer pauses with it.
        let in_break = crate::rewards::on_any_break(s);
        // Track the connection of this session's own browser — the status
        // of any other paired browser does not count here.
        let browser = self.browser_status_for(if s.focus.browser_app.is_empty() {
            ""
        } else {
            &s.focus.browser_app
        });
        // Step 11 anti-tamper, fail-closed vs transient distinction:
        // `applied_now` is this session's own extension, connected
        // and holding this session's policy with no bridge error.
        // Once it is ever observed, `policy_applied` sticks (and is
        // persisted) for this run, so a later disappearance is a
        // transient disconnect — never a never-applied session.
        // A session whose policy was NEVER applied and is Running
        // with website blocking required fails closed: its browser
        // app itself is blocked (see the sample block below) until
        // the extension connects and applies the policy, at which
        // point the block lifts automatically. Breaks are exempt:
        // break time relaxes restrictions by design (steps 7/9).
        let applied_now = matches!(s.status, Status::Running | Status::Preparing)
            && browser.connected
            && browser.applied_id == s.id
            && browser.error.is_empty();
        if applied_now {
            s.policy_applied = true;
        }
        let fail_closed = !s.focus.browser_app.is_empty()
            && matches!(
                crate::tamper::classify_browser(
                    s.focus.network_required(),
                    s.status == Status::Running,
                    in_break,
                    applied_now,
                    s.policy_applied,
                ),
                crate::tamper::BrowserEnforcement::FailClosed
            );
        if fail_closed != s.browser_fail_closed {
            s.browser_fail_closed = fail_closed;
            s.report.event(
                now,
                if fail_closed {
                    "extension_removed"
                } else {
                    "extension_connected"
                },
                if fail_closed {
                    "Browser blocked until the extension is connected: this session requires website blocking but its policy was never applied by any extension, so the browser app itself is blocked while the session runs. Connect the extension and the block lifts automatically."
                } else {
                    "Extension connected and applied this session's policy; the fail-closed browser block lifted."
                },
            );
        }
        // Step 8: frontmost-sample facts for allowance accrual,
        // captured inside the sample block below (empty when the
        // session has no activity sampling).
        if s.status == Status::Preparing {
            s.attention.state = State::Preparing;
        }
        let mut signals = Signals::default();
        let mut screen_allowance_changed = false;
        if s.status == Status::Preparing {
            // The session starts once every selected browser has the rules
            // installed: the policy is published to all paired browsers
            // and each extension acknowledges it separately.
            let rules_live_everywhere = s.focus.session_browsers().iter().all(|id| {
                let b = self.browser_status_for(id);
                b.connected
                    && b.applied_id == s.id
                    && b.error.is_empty()
                    && !b.client_browser.is_empty()
                    && browser::same_browser(id, &b.client_browser)
            });
            if rules_live_everywhere {
                s.status = Status::Running;
                s.started_at.get_or_insert(now);
                s.deadline_ms = if s.focus.attention.enabled {
                    None
                } else {
                    Some(now + s.remaining_ms as i64)
                };
                s.report.enforcement = "Browser filter acknowledged".into();
                s.report.event(
                    now,
                    "browser_activated",
                    "Paired browser acknowledged installed request rules.",
                );
            }
        } else {
            // When the browser disconnects during a Running session, the session
            // KEEPS GOING — apps stay blocked, timer keeps running. Only website
            // blocking is paused until the browser reconnects. The UI shows a
            // "Connect browser" button that navigates to Settings → Connections.
            // We do NOT drop to Preparing; that would pause the session.
            if !in_break
                && !fail_closed
                && s.focus.network_required()
                && (!browser.connected || browser.applied_id != s.id || !browser.error.is_empty())
            {
                // Log once when the browser disconnects (avoid spam on every tick).
                let already_logged = s
                    .report
                    .events
                    .iter()
                    .rev()
                    .take(5)
                    .any(|e| e.kind == "browser_disconnected");
                if !already_logged {
                    s.report.event(now, "browser_disconnected", "Browser disconnected. Session keeps running; apps stay blocked. Reconnect the browser from Settings → Connections to resume website blocking.");
                }
            }
            // Screen Time sampling is always on: the foreground app is
            // sampled on every tick while the app runs, so usage is
            // always recorded. Session handling below (blocked-app
            // enforcement, distracted accounting, productive signals)
            // still requires the session's own activity consent,
            // exactly as before — usage sampling never changes it.
            let session_consented = s.status == Status::Running && s.focus.activity_consent;
            let should_sample_app = self.native;
            if should_sample_app {
                match platform::sample() {
                    Ok(sample) => {
                        let usage = crate::allowances::TickUsage {
                            sample_id: &sample.id,
                            sample_idle_ms: sample.idle_ms,
                            sample_available: true,
                            browser_domain: &browser.active_domain,
                            browser_focused: browser.browser_focused,
                            browser_activity_fresh: browser.activity_fresh,
                            browser_client_id: &browser.client_browser,
                            session_browser_app: &s.focus.browser_app,
                        };
                        if s.status == Status::Running {
                            screen_allowance_changed |= crate::allowances::accrue_for_tick(
                                &mut next.allowances,
                                now,
                                delta,
                                in_break,
                                &usage,
                            );
                        }
                        // M5: this path never records screen time.
                        // The always-on sampler in `sample_screen_time`
                        // already recorded this tick's interval (it
                        // runs first, on the same tick, with the
                        // foreground browser's own attribution), so
                        // recording here too would write the same
                        // ~second twice. One sampler, one write.
                        let day_ms = crate::screen_time::local_day_start_ms(now);
                        if session_consented {
                            // Step 4: folder/title rules feed the EXACT
                            // existing blocked handling below (hide,
                            // distracted time, interruption count) — only
                            // this flag's computation changed.
                            // Step 8: an allowance-exhausted app blocks
                            // through the exact same path (hide +
                            // distracted time) as a session-blocked one,
                            // even when the session itself allows it.
                            // The paired browser and protected apps are
                            // exempt inside `app_exhausted`.
                            let allowance_blocked = !in_break
                                && crate::allowances::app_exhausted(
                                    &self.data.allowances,
                                    now,
                                    &sample.id,
                                    &s.focus.browser_app,
                                );
                            // App locks: enforced independently of
                            // sessions — a user-set timed/limit block
                            // holds even on a break (unlike
                            // allowances, so no !in_break gate), and
                            // the session browser is NOT exempt: the
                            // user targeted this app explicitly, and
                            // hiding keeps its process and the paired
                            // extension alive.
                            let lock_blocked = crate::applocks::app_locked(
                                &self.data.app_locks,
                                &self.data.screen_time_usage,
                                day_ms,
                                now,
                                &sample.id,
                            );
                            let native_site_for_session = crate::websites::native_site(&sample.id);
                            let domain_for_lock = if browser.activity_fresh
                                && browser.browser_focused
                                && !browser.active_domain.is_empty()
                            {
                                Some(browser.active_domain.as_str())
                            } else {
                                native_site_for_session.as_ref().map(|s| s.domain.as_str())
                            };
                            let site_lock_blocked = domain_for_lock.is_some_and(|d| {
                                crate::applocks::site_locked(
                                    &self.data.app_locks,
                                    &self.data.screen_time_usage,
                                    day_ms,
                                    now,
                                    d,
                                )
                            });
                            let session_rules_blocked = !in_break
                                && !sample.id.is_empty()
                                && s.focus
                                    .blocked_sample(&sample.id, &sample.path, &sample.title);
                            // Step 11 fail-closed: the session browser
                            // app itself is blocked (hidden, counted
                            // as distracted, exactly like any other
                            // blocked app) while the extension has
                            // never applied this session's policy.
                            // Transient disconnects never reach
                            // here: `fail_closed` is false for them.
                            let fail_closed_hit = fail_closed
                                && !sample.id.is_empty()
                                && crate::focus::ids_match(&sample.id, &s.focus.browser_app);
                            let blocked = session_rules_blocked
                                || allowance_blocked
                                || lock_blocked
                                || site_lock_blocked
                                || fail_closed_hit;
                            // "Why am I blocked": the true cause,
                            // threaded from the check that fired — never
                            // guessed. Priority: fail-closed > app lock >
                            // allowance > frozen > session > schedule
                            // (see blockreasons).
                            let block_cause: Option<crate::blockreasons::BlockCause> = if blocked {
                                let lock = if lock_blocked {
                                    crate::applocks::enforced_lock_for_app(
                                        &self.data.app_locks,
                                        &self.data.screen_time_usage,
                                        day_ms,
                                        now,
                                        &sample.id,
                                    )
                                } else if site_lock_blocked {
                                    domain_for_lock.and_then(|d| {
                                        crate::applocks::enforced_lock_for_site(
                                            &self.data.app_locks,
                                            &self.data.screen_time_usage,
                                            day_ms,
                                            now,
                                            d,
                                        )
                                    })
                                } else {
                                    None
                                };
                                let allowance_title = if allowance_blocked {
                                    crate::allowances::exhausted_allowance_for_app(
                                        &self.data.allowances,
                                        now,
                                        &sample.id,
                                        &s.focus.browser_app,
                                    )
                                    .map(|a| a.title.clone())
                                } else {
                                    None
                                };
                                crate::blockreasons::app_cause(
                                    fail_closed_hit,
                                    lock,
                                    allowance_title.as_deref(),
                                    if session_rules_blocked {
                                        Some(&*s)
                                    } else {
                                        None
                                    },
                                    &self.data.schedules,
                                )
                            } else {
                                None
                            };
                            signals.activity_available = !sample.id.is_empty();
                            let work_apps = if s.focus.mode == Mode::Standard {
                                &s.focus.attention.work_apps
                            } else {
                                &s.focus.apps
                            };
                            signals.approved_app = !blocked
                                && (work_apps
                                    .iter()
                                    .any(|id| crate::focus::ids_match(id, &sample.id))
                                    || crate::focus::ids_match(&sample.id, &s.focus.browser_app));
                            signals.idle_ms = sample.idle_ms;
                            if crate::focus::ids_match(&sample.id, &s.focus.browser_app)
                                && !sample.id.is_empty()
                            {
                                // In Standard mode the relevance list is the
                                // attention work domains, unless the user
                                // explicitly picked a website blocking mode —
                                // that always uses `domains`.
                                let domains = if s.focus.mode == Mode::Standard
                                    && matches!(
                                        s.focus.website_blocking,
                                        WebsiteBlocking::Auto | WebsiteBlocking::None
                                    ) {
                                    &s.focus.attention.work_domains
                                } else {
                                    &s.focus.domains
                                };
                                signals.browser_required = true;
                                // A channel-rule block page has no active domain
                                // (the tab shows the extension block page), yet it
                                // still proves distraction — so channel_blocked
                                // counts even with an empty active domain, unlike
                                // search_distracted, which always has one.
                                if browser.activity_fresh
                                    && browser.browser_focused
                                    && (browser.channel_blocked
                                        || !browser.active_domain.is_empty())
                                {
                                    // A search page whose query matches a keyword rule is
                                    // distracted, whatever the domain rules say: the engine
                                    // cannot see the query, so the extension's report is
                                    // authoritative here. The same holds for a tab the
                                    // extension reports as channel-blocked (a blocked
                                    // channel/surface page, or a watch page the content
                                    // script blocked); an excepted page never reports
                                    // channel_blocked, so exceptions still win.
                                    signals.browser_relevant = Some(
                                        if browser.search_distracted || browser.channel_blocked {
                                            false
                                        } else {
                                            browser_domain_relevant_with_exceptions(
                                                &s.focus.mode,
                                                &s.focus.website_blocking,
                                                domains,
                                                &s.focus.exceptions,
                                                &browser.active_domain,
                                                s.focus.block_adult_content,
                                            )
                                        },
                                    );
                                }
                            } else if crate::focus::known_browser(&sample.id) {
                                signals.browser_required = true;
                            }
                            // Problem B: a lecture playing in the foreground
                            // exempts the session from the five-minute input
                            // requirement. The extension measures real
                            // playback on the active tab of its focused
                            // window; the site must be allowed by this
                            // session, and the report must be fresh, so a
                            // stale heartbeat or background media never
                            // grants the exemption.
                            signals.lecture_playing = crate::attention::verified_lecture(
                                browser.media_playing,
                                browser.activity_fresh,
                                browser.browser_focused,
                                signals.browser_relevant,
                            );
                            if sample.id != s.report.active_app {
                                s.report.active_app = sample.id.clone();
                                s.report.event(
                                    now,
                                    if blocked {
                                        "distracting_app"
                                    } else {
                                        "active_app"
                                    },
                                    &sample.name,
                                );
                                if blocked {
                                    s.report.interruptions += 1;
                                }
                            }
                            // Step 13: block-attempt log. One row per
                            // transition into the blocked state, never
                            // one per tick, and at most one row per app
                            // per 60 s (tracker throttle) so a long
                            // blocked stretch or a flickering app cannot
                            // flood the log. A write failure is logged
                            // only: attempt logging must never end a
                            // session or fail a tick.
                            if blocked && !sample.id.is_empty() {
                                if !self.attempt_tracker.blocked_now.contains(&sample.id)
                                    && self.attempt_tracker.should_log(&sample.id, now)
                                {
                                    let reason = if fail_closed_hit {
                                        "Browser blocked until extension connected"
                                    } else if lock_blocked {
                                        "App lock"
                                    } else if allowance_blocked {
                                        "Allowance exhausted"
                                    } else {
                                        "Blocked app"
                                    };
                                    let attempt = crate::stats::BlockAttempt::app(
                                        now, &sample.id, &s.id, reason,
                                    );
                                    if let Err(e) = self.db.record_attempt(&attempt) {
                                        log::warn!("Could not record block attempt: {e}");
                                    }
                                    // "Why am I blocked": show the true
                                    // cause, under the same throttle as the
                                    // attempt log and the notifications
                                    // preference gate.
                                    if self.data.preferences.notifications_enabled {
                                        if let Some(cause) = &block_cause {
                                            let name = if sample.name.is_empty() {
                                                sample.id.as_str()
                                            } else {
                                                sample.name.as_str()
                                            };
                                            self.notify_general(
                                                "Preelock",
                                                &crate::blockreasons::cause_text(cause, name, now),
                                            );
                                        }
                                    }
                                    // "Why am I blocked" app page: show the
                                    // true cause full-screen. This whole
                                    // block only runs on a throttled
                                    // transition, and the page itself is
                                    // throttled per app too.
                                    if let Some(cause) = &block_cause {
                                        record_blocked_app_page(
                                            &mut self.data.blocked_app_notice,
                                            &mut self.overlay_last_shown,
                                            &sample,
                                            cause,
                                            now,
                                        );
                                    }
                                }
                                self.attempt_tracker.blocked_now.insert(sample.id.clone());
                            } else if !sample.id.is_empty() {
                                self.attempt_tracker.blocked_now.remove(&sample.id);
                            }
                            // One-second samples, not inferred activity during sleep or a process outage.
                            // Break time is counted nowhere: it is neither
                            // productive nor distracted time.
                            if !in_break {
                                if sample.idle_ms >= 60_000 {
                                    s.report.idle_ms += if delta <= 3000 { delta } else { 0 };
                                } else if blocked {
                                    s.report.distracting_ms +=
                                        if delta <= 3000 { delta } else { 0 };
                                } else {
                                    s.report.approved_ms += if delta <= 3000 { delta } else { 0 };
                                }
                            }
                            // Defense in depth: never hide Preelock itself or
                            // other protected system apps, even if a rule ever
                            // matches them (blocked_sample already guards this,
                            // but hiding the app's own window strands the user
                            // with no way to stop the session).
                            if blocked && !crate::focus::protected(&sample.id) {
                                platform::redirect_browser_blocked(&sample.name);
                                if crate::focus::safari(&sample.id) {
                                    platform::redirect_safari_blocked();
                                }
                                // Best effort: the OS can refuse a hide request (for
                                // example while the app is still launching). Never
                                // end the session over it; the next sample retries,
                                // and the time still counts as distracted.
                                if platform::hide(&sample) {
                                    s.report.enforcement =
                                        "Native app redirection active (reversible)".into();
                                } else {
                                    s.report.enforcement =
                                        "A restricted app could not be hidden; session continues"
                                            .into();
                                }
                            } else if in_break {
                                s.report.enforcement = "On break: restrictions relaxed".into();
                            } else {
                                s.report.enforcement = if s.focus.network_required() {
                                    "Browser acknowledged; native monitor active"
                                } else {
                                    "Native monitor active"
                                }
                                .into();
                            }
                        }
                    }
                    Err(e) => {
                        // A consenting session fails on sample error, as
                        // before; otherwise this is only a missed usage
                        // sample and must never interrupt the session.
                        if session_consented {
                            s.status = Status::Interrupted;
                            s.ended_at = Some(now);
                            s.deadline_ms = None;
                            s.report.event(now, "permission_or_api_failure", &e);
                        }
                    }
                }
            }
            // Step 11 fail-closed: surface the reason even when
            // there is no activity sample (no consent / non-native),
            // so the Dashboard banner and the report always explain
            // why the browser is blocked.
            if fail_closed && s.status == Status::Running {
                s.report.enforcement = "Browser blocked until the extension is connected".into();
            }
            // Step 8 allowance accrual: Focus phases only (break
            // time never consumes a budget — checked inside
            // `accrue_for_tick`), from live tick deltas only, across
            // sessions within the period. The browser_app clone
            // keeps this borrow disjoint from the session borrow.
            // Allowance usage is accrued by the independent Screen Time
            // sampler above, so it continues outside focus sessions and is
            // counted exactly once per tick.
            s.report.browser_blocks = if s.focus.network_required() {
                browser.blocks
            } else {
                0
            };
            if s.status == Status::Running && s.focus.camera && self.native {
                let status = if self.camera_enabled {
                    self.camera.poll(tick)
                } else {
                    self.attention_camera_status.clone()
                };
                signals.camera_status = status.clone();
                if status != s.report.camera_status {
                    s.report.event(now, "camera_status", &status);
                    s.report.camera_status = status.clone();
                }
            }
        }
        if s.status == Status::Running {
            if crate::rewards::on_break(s) {
                // Step 9 reward break: same rule as a Pomodoro break
                // — break time never reaches productive-time
                // accounting, and the break ends on its own when its
                // credit runs out or its max length hits (a wake tick
                // with a huge delta ends it rather than extending
                // it). The Pomodoro phase stays frozen throughout.
                crate::rewards::advance_break(s, delta, now);
            } else if in_break {
                // Break timer only: break time never reaches the
                // productive-time accounting, and the break ends on
                // its own when its time expires (a wake tick with a
                // huge delta ends it rather than extending it).
                crate::pomodoro::advance_break(s, delta, now);
            } else {
                // Productive time credited this tick moves the focus
                // phase; the session completes inside observe() as
                // soon as its target is reached, before any break.
                let focused_before = s.focused_ms;
                attention::observe(s, &signals, delta, now);
                if s.status == Status::Running {
                    let productive = s.focused_ms.saturating_sub(focused_before);
                    // Step 9: this tick's productive time earns
                    // break credit, and a pending delayed break
                    // counts down concurrently with focus. If the
                    // delay elapses this tick the reward break
                    // starts now and the Pomodoro phase is frozen
                    // for the tick (no stacking); otherwise the
                    // Pomodoro advance below may complete a phase,
                    // which cancels a pending break via its hook.
                    crate::rewards::accrue(s, productive);
                    crate::rewards::advance_delay(s, delta, now);
                    if !crate::rewards::on_break(s) {
                        crate::pomodoro::advance_focus(s, productive, now);
                    }
                }
            }
        }
        if s.status == Status::Running && !crate::notifications::grace_active(s.grace_until_ms, now)
        {
            s.durability.last_enforcement_at_ms = Some(now);
        }
        let terminal = !matches!(s.status, Status::Running | Status::Preparing);
        if terminal {
            // Step 14: a finished session keeps no grace.
            s.grace_until_ms = None;
            // Step 9: a finished or interrupted session ends any
            // break and discards unspent credit.
            crate::rewards::clear(s);
            crate::locks::clear_state(&mut s.lock_state);
            // Step 11: the fail-closed flag is live-only; a finished
            // session never carries it into a later run (the
            // `policy_applied` history is reset by RestartSession).
            s.browser_fail_closed = false;
            if s.status == Status::Interrupted {
                s.attention.state = State::Recovering;
                s.attention.reason =
                    "Enforcement failed; session interrupted and restrictions released".into();
            }
            s.report.enforcement =
                "Released; browser cleanup may be pending acknowledgement".into();
            s.report.camera_status = "off".into();
            s.report.active_app.clear();
        }
        let need_camera = s.status == Status::Running && s.focus.camera;
        if !need_camera {
            self.stop_camera();
        }
        // Journal transitions before publishing a new network policy. Activity is checkpointed every five seconds.
        let changed = next.sessions[index].status != self.data.sessions[index].status
            || next.sessions[index].attention.state != self.data.sessions[index].attention.state
            // Step 11: persist a policy-applied or fail-closed
            // transition on this same tick, so a kill right after
            // the transition still resumes with the right
            // classification instead of re-deriving it.
            || next.sessions[index].policy_applied != self.data.sessions[index].policy_applied
            || next.sessions[index].browser_fail_closed
                != self.data.sessions[index].browser_fail_closed
            // Step 14: persist a grace expiry on this same tick, so
            // a kill right after grace ends resumes as Running
            // enforcement, never as a stale grace session.
            || next.sessions[index].grace_until_ms != self.data.sessions[index].grace_until_ms;
        // Step 8: allowance usage is flushed at most every 5 s (the
        // same checkpoint cadence `commit` uses), so a crash loses
        // at most ~5 s of budget usage — and because usage is only
        // ever added from live tick deltas, never replayed, it can
        // never double-count after a restart.
        let allowance_due =
            screen_allowance_changed && tick.saturating_sub(self.allowance_checkpoint) >= 5000;
        if changed || allowance_due {
            for session in &mut next.sessions {
                if matches!(
                    session.status,
                    Status::Running | Status::Preparing | Status::Paused
                ) {
                    session.durability.last_checkpoint_at_ms = Some(now);
                }
            }
            if let Err(e) = self.db.save(&next) {
                self.fail_open(&e);
                return Err(e);
            }
            let _ = crate::lifecycle::write_session_checkpoint(
                &self.lifecycle_path,
                crate::lifecycle::current_boot_id(),
                crate::lifecycle::ending(),
                Self::active_session_checkpoint(&next, now),
            );
            if allowance_due {
                self.allowance_checkpoint = tick;
            }
        }
        self.data = next;
        self.publish();
        Ok(())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop_camera();
        if let Some(b) = &self.bridge {
            b.publish(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::FocusConfig;
    /// Client UUID + browser report used by the network tests' fake extension.
    const TEST_CLIENT: &str = "11111111-1111-1111-1111-111111111111";
    const TEST_BROWSER: &str = "com.google.Chrome";
    // M1: cross-process engine lock — the holder ticks and saves, the
    // contender observes and refuses every write.
    #[test]
    fn observer_engine_never_ticks_or_saves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        assert!(holder.owns_engine_lock());
        holder
            .execute(Action::CreateSession {
                title: "Held".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();

        // A second engine on the same database is an observer.
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        // Its tick is a no-op reload: it sees the owner's session.
        observer.tick().unwrap();
        assert_eq!(observer.data.sessions.len(), 1);
        assert_eq!(observer.data.sessions[0].title, "Held");
        // A read-only snapshot works and does not save.
        assert_eq!(observer.snapshot().unwrap().sessions.len(), 1);
        // Writes are refused, and the refusal leaves the database alone.
        assert!(observer
            .execute(Action::CreateSession {
                title: "Contender".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .is_err());
        assert!(observer.replace_data(observer.data.clone()).is_err());
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.sessions.len(), 1);
        assert_eq!(stored.sessions[0].title, "Held");
        // Shutdown is refused too: the observer must not release the
        // holder's restrictions. M3: it exits the UI only, with no
        // shutdown and no "finish your session" notice.
        assert_eq!(
            observer.quit_decision(),
            crate::quit::QuitDecision::ExitUiOnly
        );
        drop(holder);
        // Once the holder is gone, the observer's next tick takes over.
        observer.tick().unwrap();
        assert!(observer.owns_engine_lock());
        observer
            .execute(Action::CreateSession {
                title: "Takeover".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.sessions.len(), 2);
    }
    // M1: an observer's open must not persist recovery — the recovery
    // save belongs to the lock holder alone — and taking over later
    // purges one-shots that expired during the observer period instead
    // of firing them late.
    #[test]
    fn observer_open_skips_recovery_save_and_takeover_purges_expired_one_shots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock-recovery.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder
            .execute(Action::CreateSession {
                title: "Live".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        // Plant a live session plus an already-expired one-shot, as a
        // process killed mid-session would leave them.
        holder.data.sessions[0].status = Status::Running;
        holder
            .data
            .one_shots
            .push(one_shot_timer(clock::now() - 60_000));
        holder.db.save(&holder.data).unwrap();

        let observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        // Recovery transforms ran in memory only: the live session
        // reads Running (a reopened UI is not a force quit), and the
        // expired one-shot is purged from this engine's copy — but
        // nothing was persisted, so the database still holds the
        // pre-recovery rows for the owner.
        assert_eq!(observer.data.sessions[0].status, Status::Running);
        assert!(!observer.data.sessions[0].force_quit);
        assert!(observer.data.one_shots.is_empty());
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.sessions[0].status, Status::Running);
        assert_eq!(stored.one_shots.len(), 1);
        assert!(!stored.sessions[0].force_quit);
        drop(holder);
        // Taking over purges the expired timer instead of firing it.
        let mut observer = observer;
        observer.tick().unwrap();
        assert!(observer.owns_engine_lock());
        assert!(observer.data.one_shots.is_empty());
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert!(stored.one_shots.is_empty());
    }

    // M4: tests that bind the real bridge port (47832) must not run
    // concurrently — the port is one per machine, like production.
    // Tests that only use `Bridge::test_bridge` (ephemeral port) do
    // not need this guard. A panic while holding it must not cascade
    // into the other tests, so a poisoned guard is recovered.
    static FIXED_PORT_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn fixed_port_guard() -> std::sync::MutexGuard<'static, ()> {
        let guard = FIXED_PORT_GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for _ in 0..50 {
            if !Bridge::port_listening(browser::PORT) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        guard
    }

    /// M4: wait for a stopped bridge server thread to go stale
    /// (its liveness stamp freezes on `stop()` and ages out of the
    /// 5 s window). Polls instead of sleeping a fixed 5 s.
    fn wait_until_server_dead(bridge: &Bridge) {
        for _ in 0..100 {
            if !bridge.server_alive() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("server thread did not go stale after stop()");
    }

    // M4 req 4: a relaunched GUI that opens as an observer must not
    // start a second bridge — the owner's bridge (same port, same
    // token) keeps serving the extension, and this engine's tick
    // leaves it alone.
    #[test]
    fn observer_never_starts_a_second_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("second-bridge.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder.set_native_for_test(true);
        // The owner's bridge, holding bridge.lock on this data dir.
        let owner_bridge = Bridge::test_bridge(dir.path());
        assert!(owner_bridge.server_alive());
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        observer.set_native_for_test(true);
        observer.tick().unwrap();
        assert!(observer.bridge.is_none());
        assert!(Bridge::lock_held(dir.path()));
        assert!(owner_bridge.server_alive());
    }

    // M4 req 3/4: an observer reports the bridge as owned elsewhere
    // instead of "unavailable" — the extension is still served, just
    // not by this process.
    #[test]
    fn observer_reports_bridge_owned_elsewhere() {
        let _guard = fixed_port_guard();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned-elsewhere.sqlite");
        let holder = Engine::open(&path).unwrap();
        assert!(holder.owns_engine_lock());
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        observer.set_native_for_test(true);
        assert!(observer
            .browser_status()
            .error
            .contains("Another Preelock process currently owns the browser bridge."));
        assert!(!observer.browser_status().available);
    }

    // M4 req 1/7: a lock holder that opened bridge-less starts its
    // bridge on the tick retry instead of enforcing nothing until a
    // restart.
    #[test]
    fn holder_starts_bridge_on_retry() {
        let _guard = fixed_port_guard();
        let dir = tempfile::tempdir().unwrap();
        if Bridge::probe(dir.path(), browser::PORT).port_listening {
            eprintln!(
                "External process is using port {}; skipping test",
                browser::PORT
            );
            return;
        }
        let path = dir.path().join("retry.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        assert!(engine.owns_engine_lock());
        assert!(engine.bridge.is_none());
        engine.set_native_for_test(true);
        for _ in 0..50 {
            engine.bridge_retry_at = 0;
            engine.ensure_bridge();
            if engine.bridge.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            engine.bridge.is_some(),
            "Bridge failed to start: {}",
            engine.bridge_error
        );
        assert!(engine.bridge_error.is_empty());
        assert!(engine.bridge.as_ref().unwrap().server_alive());
        assert!(Bridge::port_listening(browser::PORT));
    }

    // M4 req 7: while another bridge holds the lock the holder's
    // retry fails fast and throttles (no tight loop, no stalled
    // tick); once the holder dies the next retry recovers the bridge.
    #[test]
    fn holder_retries_bridge_after_lock_contention() {
        let _guard = fixed_port_guard();
        let dir = tempfile::tempdir().unwrap();
        if Bridge::probe(dir.path(), browser::PORT).port_listening {
            eprintln!(
                "External process is using port {}; skipping test",
                browser::PORT
            );
            return;
        }
        let path = dir.path().join("contention.sqlite");
        let holder_bridge = Bridge::test_bridge(dir.path());
        let mut engine = Engine::open(&path).unwrap();
        assert!(engine.owns_engine_lock());
        engine.set_native_for_test(true);
        engine.ensure_bridge();
        assert!(engine.bridge.is_none());
        assert!(!engine.bridge_error.is_empty());
        assert!(engine.bridge_error.contains("ownership"));
        // Throttled: the next retry is armed ~5 s out, not at once.
        assert!(engine.bridge_retry_at > clock::continuous_ms());
        // The failed attempt did not disturb the live bridge.
        assert!(holder_bridge.server_alive());
        drop(holder_bridge);
        // Simulate the retry window passing (the throttle itself is
        // asserted above; waiting 5 s here would only test the clock).
        for _ in 0..50 {
            engine.bridge_retry_at = 0;
            engine.ensure_bridge();
            if engine.bridge.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            engine.bridge.is_some(),
            "Bridge failed to start: {}",
            engine.bridge_error
        );
        assert!(engine.bridge_error.is_empty());
    }

    // M4 req 2/5: a bridge whose server thread died is restarted by
    // the holder, and the restarted bridge republishes the live
    // session's policy at once — the extension's next sync gets the
    // same rules, inside the 65 s lease, with no stale policy.
    #[test]
    fn dead_server_bridge_is_restarted_and_republishes_policy() {
        use std::io::{Read, Write};
        let _guard = fixed_port_guard();
        let dir = tempfile::tempdir().unwrap();
        if Bridge::probe(dir.path(), browser::PORT).port_listening {
            eprintln!(
                "External process is using port {}; skipping test",
                browser::PORT
            );
            return;
        }
        let (mut e, id) = network_engine(dir.path());
        // Use a browser id whose resolution is independent of which
        // chromium browsers happen to be installed on the host.
        e.data.sessions[0].focus.browser_app = "com.brave.browser".into();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, "com.brave.browser", "");
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.set_native_for_test(true);
        // Kill the bridge's server thread for real (a fake stale
        // stamp would be overwritten by the live thread) and wait
        // for the liveness stamp to age out, exactly as a holder
        // would observe a dead server on its tick.
        e.bridge.as_ref().unwrap().stop();
        wait_until_server_dead(e.bridge.as_ref().unwrap());
        for _ in 0..50 {
            e.bridge_retry_at = 0;
            e.ensure_bridge();
            if e.bridge.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let bridge = e.bridge.as_ref().expect(&e.bridge_error);
        assert!(bridge.server_alive());
        assert!(Bridge::port_listening(browser::PORT));
        assert!(e.bridge_error.is_empty());
        // The restarted bridge serves the live session's policy to a
        // syncing extension (same token, same port — no reload).
        let token = e.pairing_token();
        assert_eq!(token.len(), 64);
        // The bridge's strict field validation only accepts
        // lowercase browser ids over HTTP (the in-process test
        // helpers bypass it); the sync below is a real HTTP sync,
        // so it uses the lowercase id.
        let body = serde_json::json!({
            "client": TEST_CLIENT,
            "browser": "com.brave.browser",
            "applied_id": "",
        })
        .to_string();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", browser::PORT)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        write!(stream, "POST /sync HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", browser::PORT, body.len())
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(
            response.contains(&id),
            "restarted bridge must republish the live policy: {response}"
        );
        assert!(
            response.contains("reddit.com"),
            "restarted bridge must republish the session rules: {response}"
        );
    }
    #[test]
    fn normal_shutdown_refuses_every_active_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("exit.sqlite")).unwrap();
        engine
            .execute(Action::CreateSession {
                title: "Work".into(),
                minutes: 1,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        for status in [Status::Running, Status::Paused, Status::Preparing] {
            engine.data.sessions[0].status = status;
            assert_eq!(
                engine.quit_decision(),
                crate::quit::QuitDecision::CloseUiOnly
            );
        }
        engine.data.sessions[0].status = Status::Completed;
        assert_eq!(
            engine.quit_decision(),
            crate::quit::QuitDecision::ShutdownAndExit
        );
    }
    // M3: the state-dependent quit decision. Owner + live session
    // closes only the UI: no shutdown, no save, the session and the
    // database are untouched.
    #[test]
    fn quit_decision_owner_active_closes_ui_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("quit-owner-active.sqlite")).unwrap();
        engine
            .execute(Action::CreateSession {
                title: "Work".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        engine.data.sessions[0].status = Status::Running;
        assert_eq!(
            engine.quit_decision(),
            crate::quit::QuitDecision::CloseUiOnly
        );
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert!(engine.failed.is_none());
    }
    // M3: owner + idle shuts the engine down (the ShutdownAndExit
    // outcome performs the shutdown side effect).
    #[test]
    fn quit_decision_owner_idle_shuts_down_and_exits() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("quit-owner-idle.sqlite")).unwrap();
        engine
            .execute(Action::CreateSession {
                title: "Work".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        engine.data.sessions[0].status = Status::Completed;
        assert_eq!(
            engine.quit_decision(),
            crate::quit::QuitDecision::ShutdownAndExit
        );
        assert_eq!(
            engine.failed.as_deref(),
            Some("Application is shutting down")
        );
    }
    // M3: the M1 transient, fixed — an idle observer exits the UI
    // only. It must not shut the engine down (the holder keeps its
    // restrictions) and must not take the lock.
    #[test]
    fn quit_decision_observer_idle_exits_ui_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quit-observer-idle.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder
            .execute(Action::CreateSession {
                title: "Ready".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        assert_eq!(
            observer.quit_decision(),
            crate::quit::QuitDecision::ExitUiOnly
        );
        assert!(!observer.owns_engine_lock());
        assert!(holder.failed.is_none());
        assert_eq!(holder.data.sessions[0].status, Status::Ready);
    }
    // M3: observer + live session closes only the UI — and leaves
    // the holder's session alone.
    #[test]
    fn quit_decision_observer_active_closes_ui_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quit-observer-active.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder
            .execute(Action::CreateSession {
                title: "Live".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        holder.data.sessions[0].status = Status::Running;
        holder.db.save(&holder.data).unwrap();
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        assert_eq!(
            observer.quit_decision(),
            crate::quit::QuitDecision::CloseUiOnly
        );
        assert!(!observer.owns_engine_lock());
        assert!(holder.failed.is_none());
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.sessions[0].status, Status::Running);
    }
    // M3: the observer's decision trusts a fresh database read over
    // its own snapshot. Here the snapshot is stale-idle (the holder
    // started the session after the observer opened) and the
    // database says live: close the UI only.
    #[test]
    fn quit_decision_observer_trusts_fresh_database_over_stale_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        // The production database name: the observer's fresh read
        // looks for exactly this file.
        let path = dir.path().join("preelock.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder
            .execute(Action::CreateSession {
                title: "Later".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.has_active());
        // The holder starts the session after the observer opened.
        holder.data.sessions[0].status = Status::Running;
        holder.db.save(&holder.data).unwrap();
        assert!(!observer.has_active());
        assert_eq!(
            observer.quit_decision(),
            crate::quit::QuitDecision::CloseUiOnly
        );
    }
    // M3: the mirror case — the snapshot is stale-active (the holder
    // ended the session after the observer opened) and the database
    // says idle: exit the UI only, with no "finish your session"
    // notice. This is the exact M1 transient.
    #[test]
    fn quit_decision_observer_stale_active_snapshot_yields_to_idle_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preelock.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder
            .execute(Action::CreateSession {
                title: "Ending".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        holder.data.sessions[0].status = Status::Running;
        holder.db.save(&holder.data).unwrap();
        let mut observer = Engine::open(&path).unwrap();
        assert!(observer.has_active());
        // The holder ends the session after the observer opened.
        holder.data.sessions[0].status = Status::Completed;
        holder.db.save(&holder.data).unwrap();
        assert!(observer.has_active());
        assert_eq!(
            observer.quit_decision(),
            crate::quit::QuitDecision::ExitUiOnly
        );
        assert!(!observer.owns_engine_lock());
        assert!(holder.failed.is_none());
    }
    #[test]
    fn browser_domain_relevance_follows_website_blocking_mode() {
        let domains = vec!["youtube.com".to_string()];
        // Blocklist: the listed sites are blocked, everything else counts.
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &domains,
            "youtube.com",
            false,
        ));
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &domains,
            "github.com",
            false,
        ));
        // Subdomains of a blocked site are blocked too.
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &domains,
            "www.youtube.com",
            false,
        ));
        // Allowlist: only the listed sites count.
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Allowlist,
            &domains,
            "youtube.com",
            false,
        ));
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Allowlist,
            &domains,
            "github.com",
            false,
        ));
        // No website blocking: any site counts.
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::None,
            &[],
            "github.com",
            false,
        ));
        // Auto follows the extension policy: blocklist in Standard,
        // allowlist otherwise.
        assert!(browser_domain_relevant(
            &Mode::Standard,
            &WebsiteBlocking::Auto,
            &domains,
            "github.com",
            false,
        ));
        assert!(!browser_domain_relevant(
            &Mode::Deep,
            &WebsiteBlocking::Auto,
            &domains,
            "github.com",
            false,
        ));
        // Shared matcher: wildcard and path rules, evaluated with the
        // host-only view the engine has.
        let rules = vec![
            "*.example.com".to_string(),
            "youtube.com/shorts".to_string(),
        ];
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &rules,
            "docs.example.com",
            false,
        ));
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &rules,
            "example.com",
            false,
        ));
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &rules,
            "youtube.com",
            false,
        ));
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Allowlist,
            &rules,
            "youtube.com",
            false,
        ));
        // Adult overlay: never relevant while blocking is on, even when
        // explicitly allowlisted; inert when blocking is off.
        assert!(!browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::Allowlist,
            &["pornhub.com".to_string()],
            "pornhub.com",
            true,
        ));
        assert!(browser_domain_relevant(
            &Mode::Strict,
            &WebsiteBlocking::None,
            &[],
            "pornhub.com",
            true,
        ));
    }
    #[test]
    fn exception_precedence_in_engine_relevance() {
        let blocked = vec!["youtube.com".to_string()];
        let exceptions = vec!["youtube.com".to_string()];
        let relevant = |mode: &Mode,
                        blocking: &WebsiteBlocking,
                        domains: &[String],
                        exceptions: &[String],
                        domain: &str,
                        adult: bool| {
            browser_domain_relevant_with_exceptions(
                mode, blocking, domains, exceptions, domain, adult,
            )
        };
        // Exception overrides a blocklist rule (and its subdomains).
        assert!(relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &blocked,
            &exceptions,
            "youtube.com",
            false,
        ));
        assert!(relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &blocked,
            &exceptions,
            "www.youtube.com",
            false,
        ));
        // Without the exception the same domain stays not relevant.
        assert!(!relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &blocked,
            &[],
            "youtube.com",
            false,
        ));
        // Exception does NOT override the adult overlay.
        assert!(!relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &blocked,
            &["pornhub.com".to_string()],
            "pornhub.com",
            true,
        ));
        // Exception does NOT override allowlist mode: a domain outside
        // the allowed list stays not relevant even when excepted.
        assert!(!relevant(
            &Mode::Strict,
            &WebsiteBlocking::Allowlist,
            &["docs.rs".to_string()],
            &["github.com".to_string()],
            "github.com",
            false,
        ));
        // Wildcard and path exceptions use the shared matcher.
        assert!(relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &["example.com".to_string()],
            &["*.example.com".to_string()],
            "docs.example.com",
            false,
        ));
        assert!(!relevant(
            &Mode::Strict,
            &WebsiteBlocking::Blocklist,
            &["example.com".to_string()],
            &["*.example.com".to_string()],
            "example.com",
            false,
        ));
    }
    #[test]
    fn restart_resumes_sessions_killed_mid_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("killed.sqlite");
        let mut engine = Engine::open_inner(&path, false, false).unwrap();
        for title in ["Killed", "Clean"] {
            engine
                .execute(Action::CreateSession {
                    title: title.into(),
                    minutes: 25,
                    allow_pause: true,
                    focus: FocusConfig {
                        attention: crate::attention::Policy {
                            enabled: true,
                            ..Default::default()
                        },
                        activity_consent: true,
                        ..Default::default()
                    },
                    lock: Default::default(),
                    lock_password: None,
                })
                .unwrap();
        }
        let clean_id = engine.data.sessions[1].id.clone();
        engine
            .execute(Action::EndSession {
                id: clean_id,
                password: None,
                challenge_answer: None,
            })
            .unwrap();
        // Simulate the process dying mid-session: shutdown() never ran,
        // so the on-disk status is still live.
        engine.data.sessions[0].status = Status::Running;
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        // Step 11: a same-boot kill no longer ends the block. The
        // killed session resumes with its remaining time intact and is
        // NOT recorded as a force quit (the old warning is obsolete:
        // closing or losing the UI never stops the block); the cleanly
        // ended session is untouched.
        let engine = Engine::open_inner(&path, false, false).unwrap();
        assert!(!engine.data.sessions[0].force_quit);
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert_eq!(engine.data.sessions[0].remaining_ms, 25 * 60_000);
        assert!(engine.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "session_resumed"));
        assert!(!engine.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "force_quit"));
        assert!(!engine.data.sessions[1].force_quit);
    }
    // Closing the Preelock window, quitting the app normally, or losing
    // the UI process is not a force quit: the engine owns the session, so
    // the same session continues with its remaining time and none of the
    // obsolete warning is recorded.
    #[test]
    fn normal_app_closure_resumes_strict_session_without_force_quit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closure.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let state = engine
            .execute(Action::CreateSession {
                title: "Strict".into(),
                minutes: 25,
                allow_pause: false,
                focus: FocusConfig {
                    mode: Mode::Strict,
                    apps: vec!["com.microsoft.VSCode".into()],
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        let remaining = engine.data.sessions[0].remaining_ms;
        engine.db.save(&engine.data).unwrap();
        // The desktop app closes: its engine process goes away while the
        // session is live.
        drop(engine);
        let mut background = Engine::open(&path).unwrap();
        let s = &background.data.sessions[0];
        assert_eq!(s.id, id, "the same session continues");
        assert_eq!(s.status, Status::Running);
        assert_eq!(s.remaining_ms, remaining, "no time is added or lost");
        assert!(matches!(s.focus.mode, Mode::Strict));
        assert!(!s.force_quit, "normal closure is not a force quit");
        assert!(s.report.events.iter().any(|e| e.kind == "session_resumed"));
        assert!(!s.report.events.iter().any(|e| e.kind == "force_quit"));
        // The background engine keeps the session live on its next tick.
        background.tick().unwrap();
        assert_eq!(background.data.sessions[0].status, Status::Running);
        assert!(background.has_active());
    }
    // The background engine still owns enforcement and timing after the
    // UI goes away: the session stays Running, its policy is republished,
    // and the extension's acknowledgement keeps it from failing closed.
    #[test]
    fn background_engine_keeps_enforcing_after_app_closure() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        e.db.save(&e.data).unwrap();
        let remaining = e.data.sessions[0].remaining_ms;
        drop(e);
        // The desktop app closed and the background engine takes over the
        // same database.
        let mut background = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert_eq!(background.data.sessions[0].id, id);
        assert_eq!(background.data.sessions[0].status, Status::Running);
        assert_eq!(background.data.sessions[0].remaining_ms, remaining);
        assert!(!background.data.sessions[0].force_quit);
        background.bridge = Some(Bridge::test_bridge(dir.path()));
        background.publish();
        background.tick().unwrap();
        background
            .bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        background.tick().unwrap();
        assert_eq!(background.browser_status().applied_id, id);
        assert!(!background.data.sessions[0].browser_fail_closed);
        assert_eq!(background.data.sessions[0].status, Status::Running);
        assert!(background.has_active());
    }
    // Reopening the UI during a live session shows that session (same id,
    // remaining time, no interruption) and never writes it back as
    // interrupted; the reopened UI can then take over the tick.
    #[test]
    fn reopened_ui_restores_the_live_session_without_interrupting_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reopen.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        let state = holder
            .execute(Action::CreateSession {
                title: "Strict".into(),
                minutes: 25,
                allow_pause: false,
                focus: FocusConfig {
                    mode: Mode::Strict,
                    apps: vec!["com.microsoft.VSCode".into()],
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        holder
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        holder.data.sessions[0].remaining_ms = 300_000;
        holder.db.save(&holder.data).unwrap();
        // The UI reopens while the background engine owns the session.
        let mut ui = Engine::open(&path).unwrap();
        assert!(!ui.owns_engine_lock());
        let shown = ui.snapshot().unwrap();
        assert_eq!(shown.sessions[0].id, id);
        assert_eq!(shown.sessions[0].status, Status::Running);
        assert_eq!(shown.sessions[0].remaining_ms, 300_000);
        assert!(!shown.sessions[0].force_quit);
        drop(holder);
        // The reopened UI takes over and keeps the same session running.
        ui.tick().unwrap();
        assert!(ui.owns_engine_lock());
        assert_eq!(ui.data.sessions[0].id, id);
        assert_eq!(ui.data.sessions[0].status, Status::Running);
        assert_eq!(ui.data.sessions[0].remaining_ms, 300_000);
    }
    // Historical records keep their legacy flag: removing the old
    // classification must not rewrite or hide existing data.
    #[test]
    fn legacy_force_quit_records_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let state = engine
            .execute(Action::CreateSession {
                title: "Old".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        engine
            .execute(Action::EndSession {
                id,
                password: None,
                challenge_answer: None,
            })
            .unwrap();
        // A record written by an older build that flagged the closure.
        engine.data.sessions[0].force_quit = true;
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let restored = Engine::open(&path).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Ended);
        assert!(
            restored.data.sessions[0].force_quit,
            "historical force-quit records stay truthful"
        );
    }
    // A quit request during a strict session only closes/hides the UI: the
    // session is neither ended nor interrupted and its row is untouched.
    #[test]
    fn quit_request_during_strict_session_does_not_interrupt_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quit-live.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let state = engine
            .execute(Action::CreateSession {
                title: "Strict".into(),
                minutes: 25,
                allow_pause: false,
                focus: FocusConfig {
                    mode: Mode::Strict,
                    apps: vec!["com.microsoft.VSCode".into()],
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        let remaining = engine.data.sessions[0].remaining_ms;
        engine.db.save(&engine.data).unwrap();
        assert_eq!(
            engine.quit_decision(),
            crate::quit::QuitDecision::CloseUiOnly
        );
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        let stored = Database::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.sessions[0].id, id);
        assert_eq!(stored.sessions[0].status, Status::Running);
        assert_eq!(stored.sessions[0].remaining_ms, remaining);
    }
    #[test]
    fn restart_lock_resumes_locked_session_instead_of_flagging_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock-restart.sqlite");
        let mut engine = Engine::open_inner(&path, false, false).unwrap();
        engine
            .execute(Action::CreateSession {
                title: "Locked".into(),
                minutes: 25,
                allow_pause: true,
                focus: Default::default(),
                lock: crate::locks::LockConfig {
                    password_hash: None,
                    challenge_length: 0,
                    delay_minutes: 0,
                    restart_lock: true,
                },
                lock_password: None,
            })
            .unwrap();
        // Simulate the process dying mid-session: shutdown() never ran,
        // so the on-disk status is still live.
        engine.data.sessions[0].status = Status::Running;
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let engine = Engine::open_inner(&path, false, false).unwrap();
        assert!(!engine.data.sessions[0].force_quit);
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert!(engine.data.sessions[0].lock.restart_lock);
        assert!(engine.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "lock_restored"));
    }
    #[test]
    fn os_restart_preserves_time_consent_and_user_pause() {
        for (status, camera) in [
            (Status::Running, true),
            (Status::Running, false),
            (Status::Paused, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("resume.sqlite");
            let mut e = Engine::open(&path).unwrap();
            e.execute(Action::CreateSession {
                title: "Resume".into(),
                minutes: 1,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
            let s = &mut e.data.sessions[0];
            s.status = status.clone();
            s.focus.camera = camera;
            s.focus.attention.verification = if camera {
                "camera_and_activity"
            } else {
                "activity_only"
            }
            .into();
            s.focused_ms = 12000;
            s.remaining_ms = 48000;
            s.attention.checkpoint_at = Some(clock::now() - 60000);
            e.db.save(&e.data).unwrap();
            drop(e);
            let mut e = Engine::open_recovery(&path, false, true, false).unwrap();
            let s = &e.data.sessions[0];
            assert_eq!(s.status, status);
            assert_eq!(s.focus.camera, camera);
            assert_eq!(s.focused_ms, 12000);
            assert_eq!(s.remaining_ms, 48000);
            assert!(!s.force_quit);
            e.tick().unwrap();
            assert_eq!(e.data.sessions[0].focused_ms, 12000);
            assert_ne!(e.data.sessions[0].status, Status::Interrupted);
        }
    }
    #[test]
    fn os_restart_republishes_browser_rules_before_counting() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        e.db.save(&e.data).unwrap();
        drop(e);
        let mut e =
            Engine::open_recovery(&dir.path().join("test.sqlite"), false, true, false).unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        assert!(!e.data.sessions[0].force_quit);
        e.bridge = Some(Bridge::test_bridge(dir.path()));
        e.publish();
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, 0);
        assert_eq!(e.data.sessions[0].status, Status::Running);
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
    }
    #[test]
    fn productive_engine_checkpoints_recovery_and_normal_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("productive.sqlite");
        let mut e = Engine::open(&path).unwrap();
        let state = e
            .execute(Action::CreateSession {
                title: "Read".into(),
                minutes: 1,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.anchor = clock::continuous_ms().saturating_sub(30000);
        assert_eq!(e.snapshot().unwrap().sessions[0].focused_ms, 0); // UI polling cannot earn time.
        for _ in 0..3 {
            e.sample_anchor = clock::continuous_ms().saturating_sub(1100);
            e.tick().unwrap();
        }
        let accepted = e.data.sessions[0].focused_ms;
        assert_eq!(accepted, 0); // No verification signal, no credit.
        e.sample_anchor = clock::continuous_ms().saturating_sub(60000);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, accepted);
        assert_eq!(e.data.sessions[0].attention.state, State::Recovering);
        // One more quiet tick: the attention state settles back and records
        // another interval, so the normal cleanup below has intervals
        // spanning the whole session to preserve.
        e.sample_anchor = clock::continuous_ms().saturating_sub(1100);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, accepted);
        // A flexible session ends normally; reopening preserves its history.
        e.execute(Action::EndSession {
            id,
            password: None,
            challenge_answer: None,
        })
        .unwrap();
        drop(e);
        let mut e = Engine::open(&path).unwrap();
        let state = e.snapshot().unwrap();
        assert_eq!(state.sessions[0].attention.state, State::Idle);
        assert_eq!(state.sessions[0].status, Status::Ended);
        assert_eq!(state.sessions[0].focused_ms, accepted);
        let conn = rusqlite::Connection::open(path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM attention_intervals", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 3);
    }
    #[test]
    fn sqlite_roundtrip_persists_tasks_preferences_and_paused_session() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine
            .execute(Action::CreateTask {
                title: "Read".into(),
            })
            .unwrap();
        engine
            .execute(Action::SetPreferences {
                theme: "light".into(),
                default_minutes: 45,
            })
            .unwrap();
        let state = engine
            .execute(Action::CreateSession {
                title: "Focus".into(),
                minutes: 1,
                allow_pause: true,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        engine
            .execute(Action::PauseSession {
                id,
                password: None,
                challenge_answer: None,
            })
            .unwrap();
        drop(engine);
        let mut restored = Engine::open(&path).unwrap();
        let state = restored.snapshot().unwrap();
        assert_eq!(state.tasks[0].title, "Read");
        assert_eq!(state.preferences.theme, "light");
        // A paused session on disk means the app stopped mid-session:
        // it resumes Paused (its pause and remaining time preserved)
        // without being labelled a force quit.
        assert_eq!(state.sessions[0].status, crate::models::Status::Paused);
        assert!(!state.sessions[0].force_quit);
    }
    #[test]
    fn rejected_action_does_not_persist_partial_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        assert!(engine
            .execute(Action::CreateSession {
                title: "Invalid".into(),
                minutes: 0,
                allow_pause: false,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .is_err());
        assert!(engine.snapshot().unwrap().sessions.is_empty());
    }
    #[test]
    fn starting_camera_status_is_accepted_as_transient() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        engine.set_attention_camera_status(crate::attention_agent::Signal {
            status: "starting".into(),
            sampled_at_ms: clock::continuous_ms(),
        });
        assert_eq!(engine.attention_camera_status, "starting");
        // Freshness is still enforced: a "starting" sampled 60s in the
        // future is not fresh and stays unavailable. (An old timestamp
        // cannot be built portably here: the Linux test clock counts
        // from process start, so `now - 60_000` saturates to 0 and is
        // fresh again when the process is young.)
        engine.set_attention_camera_status(crate::attention_agent::Signal {
            status: "starting".into(),
            sampled_at_ms: clock::continuous_ms() + 60_000,
        });
        assert_eq!(engine.attention_camera_status, "camera_unavailable");
    }
    fn network_engine(dir: &Path) -> (Engine, String) {
        let mut e = Engine::open(&dir.join("test.sqlite")).unwrap();
        e.bridge = Some(Bridge::test_bridge(dir));
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, "");
        let state = e
            .execute(Action::CreateSession {
                title: "Network".into(),
                minutes: 1,
                allow_pause: true,
                focus: crate::focus::FocusConfig {
                    domains: vec!["reddit.com".into()],
                    browser_app: "com.google.Chrome".into(),
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        (e, id)
    }
    #[test]
    fn network_session_waits_for_ack_then_cleans_up_on_expiration() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        let state = e.execute(Action::StartSession { id: id.clone() }).unwrap();
        assert_eq!(state.sessions[0].status, Status::Preparing);
        assert!(state.sessions[0].started_at.is_none());
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Preparing);
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        e.data.sessions[0].remaining_ms = 0;
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Completed);
        assert!(e.browser_status().cleanup_pending);
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, "");
        assert!(!e.browser_status().cleanup_pending);
    }
    #[test]
    fn missing_ack_and_browser_loss_keep_session_active() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id }).unwrap();
        // Missing browser acknowledgement must never end the commitment.
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Preparing);
        assert_eq!(e.quit_decision(), crate::quit::QuitDecision::CloseUiOnly);
        let dir2 = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir2.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, "");
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        assert_eq!(e.quit_decision(), crate::quit::QuitDecision::CloseUiOnly);
    }
    #[test]
    fn unsupported_os_deep_and_missing_camera_permission_fail_before_start() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        let state = e
            .execute(Action::CreateSession {
                title: "Test".into(),
                minutes: 1,
                allow_pause: false,
                focus: Default::default(),
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        e.data.sessions[0].focus.camera = true;
        assert!(e.execute(Action::StartSession { id: id.clone() }).is_err());
        e.data.sessions[0].focus.camera = false;
        e.data.sessions[0].focus.activity_consent = true;
        assert!(e.execute(Action::StartSession { id: id.clone() }).is_err());
        e.data.sessions[0].focus.mode = Mode::Deep;
        assert!(e.execute(Action::StartSession { id }).is_err());
        assert_eq!(e.data.sessions[0].status, Status::Ready);
    }
    #[test]
    fn restart_resumes_killed_network_session() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        drop(e);
        // The app stopped mid-session: the session resumes Running with
        // its remaining time preserved and is not labelled a force quit
        // (closing or losing the UI never stops the block).
        let restored = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Running);
        assert!(!restored.data.sessions[0].force_quit);
        assert!(restored.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "session_resumed"));
    }
    #[test]
    fn kill_resume_preserves_remaining_and_allowance_usage() {
        // Step 11: a killed session resumes with its remaining
        // time intact, and allowance usage survives the restart
        // with no double-count and no reset (usage is only ever
        // added from live tick deltas, never reconstructed).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tamper-resume.sqlite");
        let mut e = Engine::open(&path).unwrap();
        e.execute(Action::CreateSession {
            title: "Work".into(),
            minutes: 25,
            allow_pause: true,
            focus: Default::default(),
            lock: Default::default(),
            lock_password: None,
        })
        .unwrap();
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].remaining_ms = 100_000;
        e.data.sessions[0].focused_ms = 50_000;
        e.data.allowances.push(crate::allowances::Allowance {
            id: "a1".into(),
            title: "Reddit".into(),
            kind: crate::allowances::TargetKind::Site,
            target: "reddit.com".into(),
            budget_minutes: 30,
            period: crate::allowances::Period::Day,
            mode: crate::allowances::ResetMode::Rolling,
            enabled: true,
            usage: vec![crate::allowances::UsageBucket {
                minute_ms: clock::now() - 60_000,
                ms: 7_000,
            }],
            created_at: 0,
        });
        e.db.save(&e.data).unwrap();
        drop(e);
        let restored = Engine::open(&path).unwrap();
        let s = &restored.data.sessions[0];
        assert_eq!(s.status, Status::Running);
        assert_eq!(s.remaining_ms, 100_000);
        assert_eq!(s.focused_ms, 50_000);
        assert!(!s.force_quit);
        assert_eq!(restored.data.allowances.len(), 1);
        assert_eq!(restored.data.allowances[0].usage.len(), 1);
        assert_eq!(restored.data.allowances[0].usage[0].ms, 7_000);
    }

    fn frozen_focus() -> FocusConfig {
        FocusConfig {
            activity_consent: true,
            attention: crate::attention::Policy {
                work_apps: vec!["com.microsoft.VSCode".into()],
                ..Default::default()
            },
            frozen: crate::frozen::FrozenConfig {
                enabled: true,
                allowed_apps: vec!["com.microsoft.VSCode".into()],
                allowed_websites: vec![],
            },
            ..Default::default()
        }
    }

    #[test]
    fn frozen_recovery_clears_expired_and_resumes_active() {
        // Step 12, recovery: an expired frozen session completes
        // on reopen (never resumes, overlay never re-appears for
        // it); a frozen session with time left resumes Running
        // with its remaining time preserved, like any live session.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frozen-recovery.sqlite");
        let mut e = Engine::open(&path).unwrap();
        e.execute(Action::CreateSession {
            title: "Frozen".into(),
            minutes: 25,
            allow_pause: true,
            focus: frozen_focus(),
            lock: Default::default(),
            lock_password: None,
        })
        .unwrap();
        // Expired: started 30 minutes ago for a 25-minute lock.
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].started_at = Some(clock::now() - 30 * 60_000);
        e.data.sessions[0].remaining_ms = 100_000;
        e.db.save(&e.data).unwrap();
        drop(e);
        let restored = Engine::open(&path).unwrap();
        let s = &restored.data.sessions[0];
        assert_eq!(s.status, Status::Completed);
        assert!(s.report.events.iter().any(|ev| ev.kind == "frozen_expired"));

        // Active: started 1 minute ago, time left.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frozen-resume.sqlite");
        let mut e = Engine::open(&path).unwrap();
        e.execute(Action::CreateSession {
            title: "Frozen".into(),
            minutes: 25,
            allow_pause: true,
            focus: frozen_focus(),
            lock: Default::default(),
            lock_password: None,
        })
        .unwrap();
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].started_at = Some(clock::now() - 60_000);
        e.data.sessions[0].remaining_ms = 100_000;
        e.db.save(&e.data).unwrap();
        drop(e);
        let restored = Engine::open(&path).unwrap();
        let s = &restored.data.sessions[0];
        assert_eq!(s.status, Status::Running);
        assert_eq!(s.remaining_ms, 100_000);
        assert!(s.focus.frozen.enabled);
    }

    #[test]
    fn frozen_tick_expires_on_wall_clock_even_without_productive_time() {
        // Step 12, tick expiry: a frozen session whose wall-clock
        // timer ran out completes on the next tick, even though
        // its productive remaining time is untouched (no sample
        // runs in the test engine, so no productive time accrues).
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("frozen-tick.sqlite")).unwrap();
        e.execute(Action::CreateSession {
            title: "Frozen".into(),
            minutes: 25,
            allow_pause: true,
            focus: frozen_focus(),
            lock: Default::default(),
            lock_password: None,
        })
        .unwrap();
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].started_at = Some(clock::now() - 30 * 60_000);
        e.data.sessions[0].remaining_ms = 100_000;
        e.tick().unwrap();
        let s = &e.data.sessions[0];
        assert_eq!(s.status, Status::Completed);
        assert!(s.report.events.iter().any(|ev| ev.kind == "frozen_expired"));
    }

    #[test]
    fn fail_closed_vs_transient_disconnect_on_tick() {
        // Step 11: a session whose policy was applied before, then
        // lost, is a transient disconnect (session keeps running,
        // no fail-closed flag, browser_disconnected event). A
        // session whose policy was never applied fails closed
        // (browser_fail_closed, extension_removed event) until
        // the extension applies it, when the block lifts.
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        assert!(e.data.sessions[0].policy_applied);
        // Transient: extension drops after having applied.
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, "");
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        assert!(!e.data.sessions[0].browser_fail_closed);
        assert!(e.data.sessions[0]
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "browser_disconnected"));
        assert!(!e.data.sessions[0]
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "extension_removed"));

        // Never applied: a Running session with no extension history.
        let dir2 = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir2.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.data.sessions[0].status = Status::Running;
        assert!(!e.data.sessions[0].policy_applied);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        assert!(e.data.sessions[0].browser_fail_closed);
        assert!(e.data.sessions[0]
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "extension_removed"));
        // Extension connects and applies: block lifts automatically.
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        assert!(e.data.sessions[0].policy_applied);
        assert!(!e.data.sessions[0].browser_fail_closed);
        assert!(e.data.sessions[0]
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "extension_connected"));
    }

    #[test]
    fn connection_lock_follows_live_strict_sessions_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert!(!e.has_active_strict());
        // A live standard session does not lock.
        e.data.sessions.push(running_session("standard"));
        assert!(!e.has_active_strict());
        // A live strict session locks (Running, Paused, Preparing).
        let mut strict = running_session("strict");
        strict.focus.mode = Mode::Strict;
        e.data.sessions.push(strict);
        assert!(e.has_active_strict());
        e.data.sessions[1].status = Status::Paused;
        assert!(e.has_active_strict());
        e.data.sessions[1].status = Status::Preparing;
        assert!(e.has_active_strict());
        // Ending the strict session unlocks again; a completed strict
        // session on its own never locks.
        e.data.sessions[1].status = Status::Completed;
        assert!(!e.has_active_strict());
    }

    #[test]
    fn pairing_reset_is_refused_during_a_live_strict_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        e.bridge = Some(Bridge::test_bridge(dir.path()));
        // No session: reset works (rotates the pairing token).
        assert!(e.reset_pairing(dir.path()).is_ok());
        // Live strict session: reset is refused and the token is
        // untouched, so paired browsers stay paired.
        let token_before = e.pairing_token();
        let mut strict = running_session("strict");
        strict.focus.mode = Mode::Strict;
        e.data.sessions.push(strict);
        let err = e.reset_pairing(dir.path()).unwrap_err();
        assert!(err.contains("End the session"), "got: {err}");
        assert_eq!(e.pairing_token(), token_before);
        // Session over: reset works again.
        e.data.sessions[0].status = Status::Completed;
        assert!(e.reset_pairing(dir.path()).is_ok());
        assert_ne!(e.pairing_token(), token_before);
    }

    #[test]
    fn observer_pairing_token_matches_holder_token_without_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer-token.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        holder.bridge = Some(Bridge::test_bridge(dir.path()));
        let observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        assert!(observer.bridge.is_none());
        let expected = holder.pairing_token();
        assert_eq!(expected.len(), 64);
        assert_eq!(observer.pairing_token(), expected);
    }

    #[test]
    fn pairing_token_auto_heals_corrupted_or_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("heal-test.sqlite");
        let e = Engine::open(&path).unwrap();
        // Write corrupt garbage to browser-token
        std::fs::write(dir.path().join("browser-token"), "corrupted-token-payload").unwrap();
        // pairing_token auto-heals and generates a valid 64-hex token
        let token = e.pairing_token();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        // Second read returns the same healed token
        assert_eq!(e.pairing_token(), token);
    }

    #[test]
    fn publish_pushes_the_connection_lock_and_lifts_it_on_session_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        e.bridge = Some(Bridge::test_bridge(dir.path()));
        let bridge = e.bridge.as_ref().unwrap();
        e.publish();
        assert!(!bridge.connection_locked());
        // A live strict session locks on the next publish; ending it
        // unlocks on the following publish — no restart involved.
        let mut strict = running_session("strict");
        strict.focus.mode = Mode::Strict;
        e.data.sessions.push(strict);
        e.publish();
        assert!(bridge.connection_locked());
        e.data.sessions[0].status = Status::Completed;
        e.publish();
        assert!(!bridge.connection_locked());
    }

    #[test]
    fn forward_clock_jump_shifts_delay_and_preserves_remaining() {
        // Step 11: a forward wall-clock jump records a clock_jump
        // event, does not shorten remaining time, and does not
        // shortcut a pending step-10 delay lock.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clock-jump.sqlite");
        let mut e = Engine::open(&path).unwrap();
        e.execute(Action::CreateSession {
            title: "Locked".into(),
            minutes: 25,
            allow_pause: true,
            focus: Default::default(),
            lock: crate::locks::LockConfig {
                password_hash: None,
                challenge_length: 0,
                delay_minutes: 5,
                restart_lock: false,
            },
            lock_password: None,
        })
        .unwrap();
        let id = e.data.sessions[0].id.clone();
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].remaining_ms = 60_000;
        e.execute(Action::RequestLockDelay {
            id: id.clone(),
            for_action: "end".into(),
        })
        .unwrap();
        let delay_before = e.data.sessions[0]
            .lock_state
            .delay_until_ms
            .expect("delay request recorded");
        // Simulate a 10-minute forward wall jump with only ~1 s of
        // monotonic time passing: wall far behind, anchor recent.
        let now = clock::now();
        e.wall = now - 601_000;
        e.anchor = clock::continuous_ms().saturating_sub(1_000);
        e.snapshot().unwrap();
        let s = &e.data.sessions[0];
        assert_eq!(s.remaining_ms, 60_000, "jump must not shorten remaining");
        assert!(s.report.events.iter().any(|ev| ev.kind == "clock_jump"));
        let delay_after = s.lock_state.delay_until_ms.expect("delay still pending");
        let shifted = delay_after - delay_before;
        assert!(
            (595_000..=615_000).contains(&shifted),
            "delay must shift by ~the jump, got {shifted}"
        );
        // The delay still gates: ending early is refused.
        assert!(e
            .execute(Action::EndSession {
                id,
                password: None,
                challenge_answer: None,
            })
            .is_err());
    }

    fn one_shot_timer(fire_at_ms: i64) -> crate::schedules::OneShot {
        crate::schedules::OneShot {
            id: "one".into(),
            title: "Later block".into(),
            minutes: 25,
            allow_pause: true,
            focus: FocusConfig::default(),
            fire_at_ms,
            warning_fired: vec![],
            created_at: 0,
        }
    }

    fn running_session(id: &str) -> crate::models::Session {
        crate::models::Session {
            id: id.into(),
            title: "Running".into(),
            duration_minutes: 25,
            remaining_ms: 25 * 60_000,
            focused_ms: 0,
            allow_pause: true,
            status: Status::Running,
            started_at: Some(1000),
            ended_at: None,
            deadline_ms: None,
            focus: FocusConfig::default(),
            report: Default::default(),
            attention: Default::default(),
            force_quit: false,
            schedule_id: None,
            kind: crate::models::SessionKind::Focus,
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
            grace_notified: false,
            durability: Default::default(),
            start_notified: false,
            complete_notified: false,
            inactivity_notified: false,
        }
    }

    #[test]
    fn tick_gap_detection_uses_a_generous_threshold() {
        assert!(!tick_gap_exceeded(None, 1_000_000));
        // Exactly the threshold is ordinary lateness, not a gap.
        assert!(!tick_gap_exceeded(
            Some(1_000_000),
            1_000_000 + TICK_GAP_THRESHOLD_MS
        ));
        assert!(tick_gap_exceeded(
            Some(1_000_000),
            1_000_000 + TICK_GAP_THRESHOLD_MS + 1
        ));
        // A clock moving backwards is never a gap.
        assert!(!tick_gap_exceeded(Some(2_000_000), 1_000_000));
    }

    #[test]
    fn expired_one_shots_are_purged_at_launch_and_restore() {
        let mut data = Snapshot::default();
        data.one_shots.push(one_shot_timer(1000));
        data.one_shots.push(one_shot_timer(5000));
        assert!(purge_expired_one_shots(&mut data, 2000));
        assert_eq!(data.one_shots.len(), 1);
        assert_eq!(data.one_shots[0].fire_at_ms, 5000);
        assert!(!purge_expired_one_shots(&mut data, 2000));
    }

    #[test]
    fn one_shot_fires_once_when_due_and_idle() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("one.sqlite")).unwrap();
        engine.data.one_shots.push(one_shot_timer(5000));
        assert!(engine.evaluate_one_shots(5000, false).unwrap());
        assert!(engine.data.one_shots.is_empty());
        assert_eq!(engine.data.sessions.len(), 1);
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.title, "Later block");
        assert_eq!(session.duration_minutes, 25);
        assert_eq!(session.remaining_ms, 25 * 60_000);
        assert!(session.allow_pause);
        assert_eq!(session.schedule_id, None);
        // The one-shot record is already purged at this point, yet
        // the session still knows it was a one-shot: the kind was
        // recorded on the session when it fired.
        assert_eq!(session.kind, crate::models::SessionKind::OneShot);
        // A second evaluation does not fire again: the timer is gone.
        assert!(!engine.evaluate_one_shots(5000, false).unwrap());
        assert_eq!(engine.data.sessions.len(), 1);
    }

    #[test]
    fn one_shot_waits_for_its_time_and_expires_after_a_gap_or_while_busy() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("one.sqlite")).unwrap();
        // Future time: untouched, no session.
        engine.data.one_shots.push(one_shot_timer(9000));
        assert!(!engine.evaluate_one_shots(5000, false).unwrap());
        assert_eq!(engine.data.one_shots.len(), 1);
        // Time passed during a gap (sleep): expired, never fired.
        assert!(engine.evaluate_one_shots(9500, true).unwrap());
        assert!(engine.data.one_shots.is_empty());
        assert!(engine.data.sessions.is_empty());
        // Time arrived while another session runs: skipped, not queued.
        engine.data.one_shots.push(one_shot_timer(10_000));
        engine.data.sessions.push(running_session("active"));
        assert!(engine.evaluate_one_shots(10_000, false).unwrap());
        assert!(engine.data.one_shots.is_empty());
        assert_eq!(engine.data.sessions.len(), 1);
        assert_eq!(engine.data.sessions[0].id, "active");
    }

    // ---- Step 14: advance warnings + grace period ----

    /// A schedule whose next start is `lead_minutes` from now,
    /// built from the live local clock so the test is stable at any
    /// hour: start = current local minutes + lead, end an hour
    /// later (overnight-wrapped near midnight, still valid).
    fn upcoming_schedule(lead_minutes: u32) -> crate::schedules::Schedule {
        let now = clock::now();
        let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
        let start = (minutes + lead_minutes) % 1440;
        let day = if minutes + lead_minutes < 1440 {
            weekday
        } else {
            (weekday + 1) % 7
        };
        let end = (start + 60) % 1440;
        crate::schedules::Schedule {
            id: "sched".into(),
            title: "Morning block".into(),
            days: vec![day],
            start_time: format!("{:02}:{:02}", start / 60, start % 60),
            end_time: format!("{:02}:{:02}", end / 60, end % 60),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: now,
        }
    }

    #[test]
    fn schedule_warning_fires_once_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warn.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        engine.data.schedules.push(upcoming_schedule(10));
        // Truncated now + 5 minutes: exactly the 5-minute warning
        // time for the upcoming start (same construction the
        // warning evaluation uses: next start minus 5 minutes).
        let now = clock::now();
        let at_warning = now - now.rem_euclid(60_000) + 5 * 60_000;
        assert!(engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        // Same tick again, and after a restart reload from disk:
        // the point never fires twice.
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        drop(engine);
        let mut restored = Engine::open(&path).unwrap();
        assert_eq!(restored.data.schedules[0].warning_fired.len(), 1);
        assert!(!restored
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(restored.data.schedules[0].warning_fired.len(), 1);
    }

    #[test]
    fn undeliverable_warnings_are_deferred_not_consumed() {
        // Windows background engine: no Tauri app handle, so the OS
        // notification cannot be shown. A due warning must stay due
        // (nothing recorded) until a tick where delivery works —
        // otherwise the user would never see it and the dedupe
        // would stop it ever firing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warn-deferred.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        engine.data.schedules.push(upcoming_schedule(10));
        let now = clock::now();
        let at_warning = now - now.rem_euclid(60_000) + 5 * 60_000;
        let fire_at = at_warning + 5 * 60_000;
        engine.data.one_shots.push(one_shot_timer(fire_at));
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, false)
            .unwrap());
        assert!(engine.data.schedules[0].warning_fired.is_empty());
        assert!(engine.data.one_shots[0].warning_fired.is_empty());
        // Same tick again with delivery available: each fires
        // exactly once, then never again.
        assert!(engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        assert_eq!(engine.data.one_shots[0].warning_fired, vec![5]);
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        assert_eq!(engine.data.one_shots[0].warning_fired, vec![5]);
    }

    #[test]
    fn schedule_warning_with_missing_browser_still_fires_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warn-readiness.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        // A schedule whose website rules need a browser that is not
        // connected (no bridge on this test build): the readiness
        // check must not break the warning — it fires once, with
        // the same dedupe, and the notification body (verified by
        // the pure warning_body_with_readiness tests; notify is a
        // no-op here) carries the missing-browser line.
        let mut schedule = upcoming_schedule(10);
        schedule.focus.domains = vec!["example.com".into()];
        schedule.focus.browser_app = "com.google.chrome".into();
        engine.data.schedules.push(schedule);
        let now = clock::now();
        let at_warning = now - now.rem_euclid(60_000) + 5 * 60_000;
        assert!(engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
    }

    #[test]
    fn one_shot_warning_with_missing_prerequisites_still_fires_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("warn-oneshot.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        let now = clock::now();
        let fire_at = now + 10 * 60_000;
        let mut shot = one_shot_timer(fire_at);
        shot.focus.domains = vec!["example.com".into()];
        shot.focus.browser_app = "com.google.chrome".into();
        shot.focus.camera = true;
        engine.data.one_shots.push(shot);
        // Probe exactly at the 5-minute point: the one-shot fire time
        // keeps second precision (unlike minute-aligned schedule
        // starts), so flooring `now` to the minute would probe up to
        // 59s early and the warning would not be due yet.
        let at_warning = fire_at - 5 * 60_000;
        assert!(engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.one_shots[0].warning_fired, vec![5]);
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert_eq!(engine.data.one_shots[0].warning_fired, vec![5]);
    }

    #[test]
    fn warnings_respect_the_master_toggle_and_disabled_schedules() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("warn-off.sqlite")).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        engine.data.preferences.notifications_enabled = false;
        engine.data.schedules.push(upcoming_schedule(10));
        let now = clock::now();
        let at_warning = now - now.rem_euclid(60_000) + 5 * 60_000;
        // Master toggle OFF: nothing fires and nothing is marked,
        // so re-enabling before the block starts still warns.
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert!(engine.data.schedules[0].warning_fired.is_empty());
        engine.data.preferences.notifications_enabled = true;
        engine.data.schedules[0].enabled = false;
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, true)
            .unwrap());
        assert!(engine.data.schedules[0].warning_fired.is_empty());
    }

    #[test]
    fn undeliverable_warnings_defer_without_consuming_point() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("warn-defer.sqlite")).unwrap();
        engine.data.preferences.warning_minutes = "5".into();
        engine.data.schedules.push(upcoming_schedule(10));
        let now = clock::now();
        let at_warning = now - now.rem_euclid(60_000) + 5 * 60_000;
        assert!(!engine
            .evaluate_warnings_deliverable(at_warning, false)
            .unwrap());
        assert!(engine.data.schedules[0].warning_fired.is_empty());
    }

    #[test]
    fn schedule_start_gets_grace_and_window_end_ends_it_in_grace() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("grace.sqlite")).unwrap();
        engine.data.preferences.grace_seconds = 60;
        engine.data.schedules.push(upcoming_schedule(60));
        // Start the window: evaluate at a time inside it (now + 61
        // minutes is past the start and before the hour-long end,
        // whatever the hour). The session starts in grace.
        let now = clock::now();
        let inside = now - now.rem_euclid(60_000) + 61 * 60_000;
        // The schedule above starts 60 minutes out; inside the
        // window only if the evaluation time's local minutes fall
        // between start and end — with a 60-minute window starting
        // 60 minutes from the schedule's construction, `inside`
        // (61 minutes from now) is 1 minute into the window.
        assert!(engine.evaluate_schedules(inside).unwrap());
        assert_eq!(engine.data.sessions.len(), 1);
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.kind, crate::models::SessionKind::Scheduled);
        assert_eq!(
            session.grace_until_ms,
            Some(inside + 60_000),
            "grace runs from the auto-start, not from window entry"
        );
        assert!(session
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "grace_started"));
        assert_eq!(session.remaining_ms, 60 * 60_000);
        // Window end during grace ends the session normally.
        let after_window = inside + 61 * 60_000;
        assert!(engine.evaluate_schedules(after_window).unwrap());
        assert_eq!(engine.data.sessions[0].status, Status::Completed);
        assert_eq!(engine.data.sessions[0].grace_until_ms, None);
        // The finished session keeps its kind for the history card.
        assert_eq!(
            engine.data.sessions[0].kind,
            crate::models::SessionKind::Scheduled
        );
    }

    #[test]
    fn one_shot_start_gets_grace_and_tick_clears_it_without_accrual() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("grace-one.sqlite")).unwrap();
        engine.data.preferences.grace_seconds = 60;
        let now = clock::now();
        engine.data.one_shots.push(one_shot_timer(now));
        assert!(engine.evaluate_one_shots(now, false).unwrap());
        assert_eq!(engine.data.sessions.len(), 1);
        let session = &engine.data.sessions[0];
        assert_eq!(session.grace_until_ms, Some(now + 60_000));
        // A tick during grace accrues nothing and changes nothing.
        let focused_before = session.focused_ms;
        let remaining_before = session.remaining_ms;
        engine.tick().unwrap();
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.focused_ms, focused_before);
        assert_eq!(session.remaining_ms, remaining_before);
        // After grace, the next tick clears grace with one event
        // and the session keeps running as normal enforcement.
        engine.data.sessions[0].grace_until_ms = Some(clock::now() - 1000);
        engine.tick().unwrap();
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.grace_until_ms, None);
        assert!(session
            .report
            .events
            .iter()
            .any(|ev| ev.kind == "grace_ended"));
    }

    /// A schedule whose window started 1 minute ago (local time) and
    /// runs 60 minutes, so it is active at real-now tick time even
    /// if the minute rolls during the test. `tick()` re-evaluates
    /// schedules at real now and ends a live session whose window
    /// is not active, so a future window (like `upcoming_schedule`)
    /// cannot be used with `tick()`.
    fn active_schedule_now(camera: bool) -> crate::schedules::Schedule {
        let now = clock::now();
        let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
        let start = (minutes + 1440 - 1) % 1440;
        let day = if minutes == 0 {
            (weekday + 6) % 7
        } else {
            weekday
        };
        let end = (start + 60) % 1440;
        let mut sched = crate::schedules::Schedule {
            id: "sched".into(),
            title: "Morning block".into(),
            days: vec![day],
            start_time: format!("{:02}:{:02}", start / 60, start % 60),
            end_time: format!("{:02}:{:02}", end / 60, end % 60),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: now,
        };
        if camera {
            // What the wizard submits for "Apps + camera":
            // camera verification needs sampling consent and a
            // productive app, or focus::validate rejects the
            // snapshot and the schedule never starts.
            sched.focus.camera = true;
            sched.focus.activity_consent = true;
            sched.focus.attention.verification = "camera_and_activity".into();
            sched.focus.attention.work_apps = vec!["com.example.work".into()];
        }
        sched
    }

    #[test]
    fn schedule_grace_notification_fires_identically_with_and_without_camera() {
        // Step 14 parity: the 1-minute pre-start grace is the same for
        // camera and non-camera auto-starts, and the grace-start
        // notification must fire for both. Until now the grace was
        // silent (only the in-app banner and tray countdown showed
        // it), so a non-camera auto-start looked instant while a
        // camera one drew the user to the app.
        for camera in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut engine = Engine::open(&dir.path().join("grace-notify.sqlite")).unwrap();
            engine.data.preferences.grace_seconds = 60;
            engine.data.schedules.push(active_schedule_now(camera));
            let now = clock::now();
            assert!(engine.evaluate_schedules(now).unwrap());
            assert_eq!(engine.data.sessions.len(), 1);
            let session = &engine.data.sessions[0];
            assert_eq!(session.focus.camera, camera);
            assert_eq!(session.grace_until_ms, Some(now + 60_000));
            assert!(
                !session.grace_notified,
                "camera={camera}: no notification before the first tick"
            );
            assert!(session
                .report
                .events
                .iter()
                .any(|ev| ev.kind == "grace_started"));
            // The first tick during grace fires the notification once,
            // and grace still accrues nothing.
            let remaining_before = session.remaining_ms;
            engine.tick().unwrap();
            let session = &engine.data.sessions[0];
            assert_eq!(session.status, Status::Running);
            assert!(
                session.grace_notified,
                "camera={camera}: grace notification must fire"
            );
            assert_eq!(
                session.remaining_ms, remaining_before,
                "grace still accrues nothing"
            );
            // A second tick does not re-fire: the marker is exactly-once.
            engine.tick().unwrap();
            assert!(engine.data.sessions[0].grace_notified);
            // The grace notification must not consume the advance
            // warning: the window already started, so nothing fired.
            assert!(engine.data.schedules[0].warning_fired.is_empty());
        }
    }

    #[test]
    fn one_shot_grace_notification_fires_identically_with_and_without_camera() {
        for camera in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut engine = Engine::open(&dir.path().join("grace-one-notify.sqlite")).unwrap();
            engine.data.preferences.grace_seconds = 60;
            let now = clock::now();
            let mut one = one_shot_timer(now);
            one.focus.camera = camera;
            if camera {
                // What the wizard submits for "Apps + camera":
                // camera verification needs sampling consent and a
                // productive app, or focus::validate rejects the
                // snapshot and the timer never starts.
                one.focus.activity_consent = true;
                one.focus.attention.verification = "camera_and_activity".into();
                one.focus.attention.work_apps = vec!["com.example.work".into()];
            }
            engine.data.one_shots.push(one);
            assert!(engine.evaluate_one_shots(now, false).unwrap());
            assert_eq!(engine.data.sessions.len(), 1);
            let session = &engine.data.sessions[0];
            assert_eq!(session.focus.camera, camera);
            assert_eq!(session.grace_until_ms, Some(now + 60_000));
            assert!(!session.grace_notified);
            engine.tick().unwrap();
            let session = &engine.data.sessions[0];
            assert!(
                session.grace_notified,
                "camera={camera}: grace notification must fire"
            );
            assert_eq!(session.status, Status::Running);
            engine.tick().unwrap();
            assert!(engine.data.sessions[0].grace_notified);
        }
    }

    #[test]
    fn ending_during_grace_is_free_with_and_without_camera() {
        // Ending during grace needs no lock proof (sessions::apply):
        // the grace notification must not change that, for either
        // camera setting.
        for camera in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut engine = Engine::open(&dir.path().join("grace-end.sqlite")).unwrap();
            engine.data.preferences.grace_seconds = 60;
            let mut sched = upcoming_schedule(60);
            sched.focus.camera = camera;
            engine.data.schedules.push(sched);
            let now = clock::now();
            let inside = now - now.rem_euclid(60_000) + 61 * 60_000;
            assert!(engine.evaluate_schedules(inside).unwrap());
            let id = engine.data.sessions[0].id.clone();
            assert!(engine.data.sessions[0].grace_until_ms.is_some());
            engine
                .execute(Action::EndSession {
                    id,
                    password: None,
                    challenge_answer: None,
                })
                .unwrap();
            assert_eq!(engine.data.sessions[0].status, Status::Ended);
        }
    }

    #[test]
    fn manual_session_gets_no_grace_and_no_grace_notification() {
        // Instant (manually started) sessions never get grace, so no
        // grace notification may fire for them either.
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("grace-manual.sqlite")).unwrap();
        engine.data.preferences.grace_seconds = 60;
        engine.data.sessions.push(running_session("manual"));
        engine.tick().unwrap();
        let session = &engine.data.sessions[0];
        assert_eq!(session.grace_until_ms, None);
        assert!(!session.grace_notified);
    }

    #[test]
    fn restart_during_grace_resumes_with_remaining_or_as_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grace-restart.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.preferences.grace_seconds = 60;
        let now = clock::now();
        // Attention-enabled, like a real template session's
        // snapshot: a plain-timer one-shot would be interrupted
        // on recovery by the legacy-timer upgrade path, which is
        // pre-step-14 behavior, not grace behavior.
        let mut shot = one_shot_timer(now);
        shot.focus.attention.enabled = true;
        engine.data.one_shots.push(shot);
        assert!(engine.evaluate_one_shots(now, false).unwrap());
        // Grace remaining: a restart keeps the session in grace
        // with the same deadline (wall-clock time, persisted).
        let deadline = engine.data.sessions[0].grace_until_ms.unwrap();
        drop(engine);
        let restored = Engine::open(&path).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Running);
        assert_eq!(restored.data.sessions[0].grace_until_ms, Some(deadline));
        // Grace already past while down: the first tick clears it
        // and the session runs as normal enforcement, never stuck.
        drop(restored);
        let mut engine = Engine::open(&path).unwrap();
        engine.data.sessions[0].grace_until_ms = Some(clock::now() - 1000);
        engine.tick().unwrap();
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert_eq!(engine.data.sessions[0].grace_until_ms, None);
    }

    #[test]
    fn frozen_wall_timer_wins_over_grace() {
        // Step 14 + step 12: a frozen session whose wall-clock
        // timer ran out completes on tick even while in grace —
        // grace delays enforcement, never the timer.
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(&dir.path().join("grace-frozen.sqlite")).unwrap();
        e.execute(Action::CreateSession {
            title: "Frozen".into(),
            minutes: 25,
            allow_pause: true,
            focus: frozen_focus(),
            lock: Default::default(),
            lock_password: None,
        })
        .unwrap();
        e.data.sessions[0].status = Status::Running;
        e.data.sessions[0].started_at = Some(clock::now() - 30 * 60_000);
        e.data.sessions[0].remaining_ms = 100_000;
        e.data.sessions[0].grace_until_ms = Some(clock::now() + 60_000);
        e.tick().unwrap();
        let s = &e.data.sessions[0];
        assert_eq!(s.status, Status::Completed);
        assert_eq!(s.grace_until_ms, None);
    }

    #[test]
    fn gap_tick_re_evaluates_schedules_and_closes_ended_windows() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("gap.sqlite")).unwrap();
        // A schedule session whose window can never be active (no
        // days listed): any re-evaluation must end it, including a
        // tick after a long gap — the block is never extended.
        let schedule = crate::schedules::Schedule {
            id: "sched".into(),
            title: "Block".into(),
            days: vec![],
            start_time: "09:00".into(),
            end_time: "10:00".into(),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            created_at: 0,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
        };
        engine.data.schedules.push(schedule);
        let mut session = running_session("sched-session");
        session.schedule_id = Some("sched".into());
        engine.data.sessions.push(session);
        // Simulate wake: the previous tick was long ago, then a tick
        // runs the normal re-evaluation path.
        engine.last_tick_wall_ms = Some(clock::now() - 10 * TICK_GAP_THRESHOLD_MS);
        assert!(engine.evaluate_schedules(clock::now()).unwrap());
        assert_eq!(engine.data.sessions[0].status, Status::Completed);
        // And a gap tick on an idle engine runs cleanly and refreshes
        // the last-tick timestamp, so the next tick is not a gap.
        // M1: the idle engine gets its own data dir — two engines in
        // one dir are holder + observer by design (one lock per data
        // dir, matching production, where a data dir holds exactly one
        // database), and an observer's tick never refreshes timestamps.
        let idle_dir = tempfile::tempdir().unwrap();
        let mut idle = Engine::open(&idle_dir.path().join("idle.sqlite")).unwrap();
        idle.last_tick_wall_ms = Some(clock::now() - 10 * TICK_GAP_THRESHOLD_MS);
        idle.tick().unwrap();
        let refreshed = idle.last_tick_wall_ms.unwrap();
        assert!(clock::now() - refreshed < TICK_GAP_THRESHOLD_MS);
    }

    #[test]
    fn sqlite_failure_releases_policy_and_preserves_prior_disk_state() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.sqlite")).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'simulated disk failure'); END;").unwrap();
        assert!(e.execute(Action::DisableCamera { id }).is_err());
        assert!(e.failed.is_some());
        assert_eq!(e.data.sessions[0].status, Status::Interrupted);
        assert!(e.browser_status().cleanup_pending);
    }

    /// M5: a fake extension report + foreground sample for the
    /// recording tests. `record_screen_time_buckets` is the only
    /// recording path, and it takes both as parameters, so the
    /// tests can drive it without native sampling (which errors on
    /// the Linux VM).
    fn m5_browser_status(domain: &str) -> crate::browser::BrowserStatus {
        crate::browser::BrowserStatus {
            available: true,
            connected: true,
            applied_id: String::new(),
            client_browser: "com.google.chrome".into(),
            error: String::new(),
            blocks: 0,
            cleanup_pending: false,
            active_domain: domain.into(),
            browser_focused: true,
            search_distracted: false,
            channel_blocked: false,
            media_playing: false,
            activity_fresh: true,
            extension_stale: false,
            ext_v_debug: 0,
            raw_browser_debug: String::new(),
        }
    }

    fn m5_sample(id: &str, name: &str) -> crate::platform::Sample {
        crate::platform::Sample {
            id: id.into(),
            name: name.into(),
            pid: 1,
            idle_ms: 0,
            path: String::new(),
            title: String::new(),
            display_asleep: false,
            screen_locked: false,
        }
    }

    fn m5_recorded_total(engine: &Engine) -> u64 {
        engine
            .db
            .load_screen_time_usage()
            .unwrap()
            .iter()
            .map(|b| b.ms)
            .sum()
    }

    #[test]
    fn background_holder_records_screen_time_without_desktop_or_tray() {
        // M5 req 1 + 3: the GUI is gone (no desktop heartbeat), the
        // background tracker owns the engine, no tray process runs —
        // and app + website intervals are still recorded. This is
        // the exact state the old heartbeat gates dropped: with no
        // live session the session path never ran, and the always-on
        // path refused to record, so background idle time vanished.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let mut engine = Engine::open(&data.join("m5.sqlite")).unwrap();
        assert!(engine.owns_engine_lock());
        std::fs::write(data.join("background-heartbeat"), b"x").unwrap();
        assert!(!crate::background::desktop_alive(data));
        assert!(crate::background::tracker_alive(data));
        assert!(!crate::tray::lock_held(data));
        let now = clock::now();
        // Website interval: Chrome foreground, fresh focused domain.
        assert!(engine
            .record_screen_time_buckets(
                now,
                1000,
                &m5_sample("com.google.Chrome", "Google Chrome"),
                &m5_browser_status("instagram.com"),
                None,
            )
            .unwrap());
        let usage = engine.db.load_screen_time_usage().unwrap();
        assert_eq!(usage.len(), 1, "one bucket per interval, never two");
        assert_eq!(usage[0].kind, "site");
        assert_eq!(usage[0].target, "instagram.com");
        assert_eq!(usage[0].ms, 1000);
        assert_eq!(engine.data.screen_time_usage.len(), 1);
        // App interval: a non-browser app, no browser report.
        assert!(engine
            .record_screen_time_buckets(
                now,
                1000,
                &m5_sample("com.microsoft.VSCode", "Visual Studio Code"),
                &m5_browser_status(""),
                None,
            )
            .unwrap());
        assert_eq!(m5_recorded_total(&engine), 2000);
    }

    #[test]
    fn native_site_credits_browser_time_without_the_extension() {
        // Requirement 2: with no extension connection at all, the browser
        // is still tracked and the OS adapter's hostname is credited to
        // the site instead of the browser app — one bucket, no double
        // count, and nothing invented when the adapter has no page.
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("native.sqlite")).unwrap();
        let now = clock::now();
        let chrome = m5_sample("com.google.Chrome", "Google Chrome");
        // No extension client (empty browser id, nothing fresh).
        let disconnected = crate::browser::unavailable("");
        let site = crate::websites::NativeSite {
            app: "com.google.Chrome".into(),
            domain: "youtube.com".into(),
        };
        assert!(engine
            .record_screen_time_buckets(now, 1000, &chrome, &disconnected, Some(&site))
            .unwrap());
        let usage = engine.db.load_screen_time_usage().unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].kind, "site");
        assert_eq!(usage[0].target, "youtube.com");
        assert_eq!(usage[0].ms, 1000);
        // The adapter has no page (private window / browser UI): the
        // browser app takes the interval instead, so no time is lost.
        assert!(engine
            .record_screen_time_buckets(now, 1000, &chrome, &disconnected, None)
            .unwrap());
        let usage = engine.db.load_screen_time_usage().unwrap();
        let total: u64 = usage.iter().map(|b| b.ms).sum();
        assert_eq!(total, 2000);
        assert_eq!(usage.len(), 2);
        assert!(usage
            .iter()
            .any(|b| b.kind == "app" && b.target == "com.google.Chrome" && b.ms == 1000));
    }

    #[test]
    fn screen_time_recording_is_heartbeat_independent_and_single_write() {
        // M5 req 5: recording follows the engine lock, never the
        // heartbeat files — the same 1 s interval records exactly
        // 1000 ms in every liveness state, and a later snapshot save
        // (which overwrites the table from the in-memory usage) can
        // never double it. Replaces the old heartbeat-gate test,
        // whose gates were the bug: they dropped intervals in the
        // desktop-gone + tracker-alive state.
        for (desktop, tracker) in [(false, false), (true, true), (false, true)] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path();
            if desktop {
                crate::background::touch_desktop(data);
            }
            if tracker {
                std::fs::write(data.join("background-heartbeat"), b"x").unwrap();
            }
            assert_eq!(crate::background::desktop_alive(data), desktop);
            assert_eq!(crate::background::tracker_alive(data), tracker);
            let mut engine = Engine::open(&data.join("m5.sqlite")).unwrap();
            assert!(engine
                .record_screen_time_buckets(
                    clock::now(),
                    1000,
                    &m5_sample("com.google.Chrome", "Google Chrome"),
                    &m5_browser_status("instagram.com"),
                    None,
                )
                .unwrap());
            assert_eq!(m5_recorded_total(&engine), 1000);
            // Snapshot save after a record: overwrite semantics, so
            // the total stays 1000 instead of doubling to 2000.
            let snapshot = engine.data.clone();
            engine.db.save(&snapshot).unwrap();
            assert_eq!(m5_recorded_total(&engine), 1000);
            assert_eq!(
                engine
                    .data
                    .screen_time_usage
                    .iter()
                    .map(|b| b.ms)
                    .sum::<u64>(),
                1000
            );
        }
    }

    #[test]
    fn observer_tick_writes_no_screen_time() {
        // M5 req 5, observer side: a GUI that relaunched while the
        // background owns the engine must not write anything — its
        // tick is a reload only, so the owner's recorded total is
        // untouched by the overlap.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m5.sqlite");
        let mut holder = Engine::open(&path).unwrap();
        assert!(holder
            .record_screen_time_buckets(
                clock::now(),
                1000,
                &m5_sample("com.google.Chrome", "Google Chrome"),
                &m5_browser_status("instagram.com"),
                None,
            )
            .unwrap());
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        observer.tick().unwrap();
        let usage = crate::database::Database::open(&path)
            .unwrap()
            .load_screen_time_usage()
            .unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].ms, 1000);
    }

    #[test]
    fn scheduled_session_starts_without_gui_or_tray() {
        // M5 req 4: a due schedule starts from a bare holder engine
        // — no GUI process, no desktop heartbeat, no tray process —
        // exactly the background engine's position after the GUI
        // quits. The start is persisted, so the tray process (which
        // reads the database) can show it.
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path();
        let mut engine = Engine::open(&data.join("m5.sqlite")).unwrap();
        assert!(engine.owns_engine_lock());
        assert!(!crate::background::desktop_alive(data));
        assert!(!crate::background::tracker_alive(data));
        assert!(!crate::tray::lock_held(data));
        engine.data.schedules.push(upcoming_schedule(0));
        engine.tick().unwrap();
        assert_eq!(engine.data.sessions.len(), 1);
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.schedule_id.as_deref(), Some("sched"));
        assert!(session.focus.attention.enabled);
        assert!(engine.has_active());
        let reloaded = crate::database::Database::open(&data.join("m5.sqlite"))
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(reloaded.sessions.len(), 1);
        assert_eq!(reloaded.sessions[0].status, Status::Running);
    }

    #[test]
    fn wake_tick_credits_no_sleep_time_to_plain_timer_session() {
        // M5 req 6: the continuous clock runs through sleep on both
        // OSes, so without the wake-gap clamp the first tick after
        // waking would credit the whole sleep as focused time and
        // drain the session. A plain (non-attention) Running session
        // is the exposed case — attention sessions already cap
        // their tick delta at 3000 ms in `attention::observe`.
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(&dir.path().join("m5.sqlite")).unwrap();
        let mut session = running_session("wake");
        assert!(
            !session.focus.attention.enabled,
            "test needs a plain-timer session"
        );
        engine.data.sessions.push(session);
        // Fake a sleep: the monotonic anchor is pushed to the
        // process start (elapsed then spans the sleep below), the
        // wall anchor is pushed back by the same span so no false
        // clock-jump fires, and the last tick was long ago.
        engine.anchor = 0;
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let now = clock::now();
        engine.wall = now - clock::continuous_ms() as i64;
        engine.last_tick_wall_ms = Some(now - 10 * TICK_GAP_THRESHOLD_MS);
        engine.tick().unwrap();
        session = engine.data.sessions[0].clone();
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.focused_ms, 0, "sleep is not focus time");
        assert_eq!(
            session.remaining_ms,
            25 * 60_000,
            "remaining time survives the sleep"
        );
        assert!(
            !session.report.events.iter().any(|e| e.kind == "clock_jump"),
            "wall and continuous agree across sleep: no jump event"
        );
    }

    // M6: shutdown / restart recovery.
    #[test]
    fn shutdown_checkpoint_saves_session_without_ending_it() {
        // Req 1: an orderly-shutdown save keeps the session live
        // with its remaining time — unlike `shutdown()`, which
        // releases restrictions on an idle quit.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-checkpoint.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine
            .execute(Action::CreateSession {
                title: "Work".into(),
                minutes: 25,
                allow_pause: true,
                focus: FocusConfig {
                    attention: crate::attention::Policy {
                        enabled: true,
                        ..Default::default()
                    },
                    activity_consent: true,
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        // `StartSession` needs the native platform, so — like the
        // existing restart tests — mark the session Running and
        // persist it directly: the on-disk status is what a
        // shutdown checkpoint has to preserve.
        engine.data.sessions[0].status = Status::Running;
        engine.db.save(&engine.data).unwrap();
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        // Holder-only: a second engine in the same dir is an
        // observer while the first holds the lock, and must refuse
        // to checkpoint (its snapshot may be stale).
        let mut observer = Engine::open(&path).unwrap();
        assert!(!observer.owns_engine_lock());
        assert!(observer.save_checkpoint().is_err());
        drop(observer);
        engine.save_checkpoint().unwrap();
        drop(engine);
        // The process died without `shutdown()` ever running: the
        // session is still live on disk with its time intact.
        let engine = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(engine.recovered_after_os_shutdown());
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert_eq!(engine.data.sessions[0].remaining_ms, 25 * 60_000);
        assert_eq!(engine.data.sessions[0].focused_ms, 0);
    }

    #[test]
    fn plain_timer_session_survives_restart_with_remaining_intact() {
        // M6 req 2/3: a live plain-timer session (attention off —
        // reachable in production via one-shot sessions, backup
        // restores, and legacy rows) resumes after a restart with
        // its remaining time intact, on both the OS-restart path
        // and the same-boot kill path. It must NOT be ended as
        // Interrupted: ending it would silently lift its block.
        for resume in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("m6-plain.sqlite");
            let mut engine = Engine::open(&path).unwrap();
            let mut session = running_session("plain");
            assert!(!session.focus.attention.enabled);
            session.focused_ms = 5 * 60_000;
            session.remaining_ms = 20 * 60_000;
            engine.data.sessions.push(session);
            engine.db.save(&engine.data).unwrap();
            drop(engine);
            let engine = Engine::open_recovery(&path, false, resume, false).unwrap();
            assert_eq!(engine.recovered_after_os_shutdown(), resume);
            let session = &engine.data.sessions[0];
            assert_eq!(session.status, Status::Running);
            assert_eq!(session.remaining_ms, 20 * 60_000);
            assert_eq!(session.focused_ms, 5 * 60_000);
            assert!(
                !session
                    .report
                    .events
                    .iter()
                    .any(|e| e.kind == "timer_upgrade"),
                "a live plain timer is resumed, never upgraded away"
            );
        }
    }

    #[test]
    fn first_tick_after_restart_credits_no_shutdown_gap_to_plain_timer() {
        // M6 req 2/3: the M5 wake clamp applies to the shutdown gap
        // too. The powered-off span must not inflate `focused_ms`
        // or drain the session: the first tick after reopening
        // credits nothing for the gap.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-gap.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.sessions.push(running_session("gap"));
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let mut engine = Engine::open_recovery(&path, false, true, false).unwrap();
        // Fake the powered-off span the way the M5 wake test fakes
        // sleep: a stale monotonic anchor plus a long-ago last tick.
        engine.anchor = 0;
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let now = clock::now();
        engine.wall = now - clock::continuous_ms() as i64;
        engine.last_tick_wall_ms = Some(now - 10 * TICK_GAP_THRESHOLD_MS);
        engine.tick().unwrap();
        let session = &engine.data.sessions[0];
        assert_eq!(session.status, Status::Running);
        assert_eq!(session.focused_ms, 0, "powered-off time is not focus time");
        assert_eq!(
            session.remaining_ms,
            25 * 60_000,
            "remaining time survives the shutdown gap"
        );
    }

    #[test]
    fn schedule_due_after_login_starts_and_expired_window_does_not() {
        // M6 req 10: a schedule whose window is still active when
        // the machine comes back starts automatically on the first
        // tick after login; a window that ended while the machine
        // was off never starts.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-schedule.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.schedules.push(upcoming_schedule(0));
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let mut engine = Engine::open_recovery(&path, false, true, false).unwrap();
        engine.tick().unwrap();
        assert_eq!(engine.data.sessions.len(), 1);
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert_eq!(
            engine.data.sessions[0].schedule_id.as_deref(),
            Some("sched")
        );

        // A window that ended while powered off: build it from the
        // local clock so the test holds at any hour.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-schedule-past.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let now = clock::now();
        let (weekday, minutes) = crate::schedules::local_weekday_and_minutes(now);
        let (day, start, end) = if minutes >= 180 {
            (weekday, minutes - 120, minutes - 60)
        } else {
            ((weekday + 6) % 7, 22 * 60, 23 * 60)
        };
        engine.data.schedules.push(crate::schedules::Schedule {
            id: "past".into(),
            title: "Past block".into(),
            days: vec![day],
            start_time: format!("{:02}:{:02}", start / 60, start % 60),
            end_time: format!("{:02}:{:02}", end / 60, end % 60),
            focus: FocusConfig::default(),
            enabled: true,
            lock_changes: false,
            lock: Default::default(),
            lock_state: Default::default(),
            warning_fired: vec![],
            created_at: now,
        });
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let mut engine = Engine::open_recovery(&path, false, true, false).unwrap();
        engine.tick().unwrap();
        assert!(
            engine.data.sessions.is_empty(),
            "a window that ended while off never starts"
        );
    }

    #[test]
    fn one_shot_due_during_power_off_never_fires_after_login() {
        // M6 req 10 (one-shot side): a timer whose fire time passed
        // while the machine was off is purged at open, never fired
        // retroactively at login.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-oneshot.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine
            .data
            .one_shots
            .push(one_shot_timer(clock::now() - 60_000));
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let mut engine = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(engine.data.one_shots.is_empty());
        engine.tick().unwrap();
        assert!(engine.data.sessions.is_empty());
    }

    #[test]
    fn login_recovery_recovers_session_for_tray_with_correct_remaining() {
        // M6 req 5: after login recovery the tray reads the live
        // session from the database and shows the correct remaining
        // time — the tray process never computes time itself.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preelock.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let mut session = running_session("tray");
        session.remaining_ms = 90_000;
        engine.data.sessions.push(session);
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let engine = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(engine.has_active());
        let snapshot = engine.current();
        let live = crate::tray::live_session(&snapshot).expect("live session");
        assert_eq!(crate::tray::timer_text(live, clock::now()), "01:30 left");
        // The same database read the --tray process does.
        let from_db = crate::tray::read_live_session(dir.path()).expect("live session");
        assert_eq!(from_db.remaining_ms, 90_000);
    }

    #[test]
    fn camera_recovery_is_gated_on_camera_session() {
        // M6 req 8 (engine side): after login, the attention agent
        // — and therefore the camera — is only driven when a
        // recovered session actually needs camera verification.
        // Permission itself is enforced inside the agent/camera
        // layer (`camera.rs` reports `permission_required` without
        // starting capture); the engine must not ask for it when
        // nothing needs it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-camera.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let mut session = running_session("cam");
        session.focus.camera = true;
        session.focus.attention.enabled = true;
        session.focus.attention.verification = "camera_and_activity".into();
        engine.data.sessions.push(session);
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let engine = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(engine.needs_attention_agent());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m6-camera-idle.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        engine.data.sessions.push(running_session("nocam"));
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let engine = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(!engine.needs_attention_agent());
    }

    #[test]
    fn ordinary_open_is_not_an_os_shutdown_recovery() {
        // The login-window signal must stay false for a normal
        // launch, or every manual launch after a quit would hide
        // the window.
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(&dir.path().join("m6-ordinary.sqlite")).unwrap();
        assert!(!engine.recovered_after_os_shutdown());
    }

    #[test]
    fn durable_checkpoint_and_reboot_recovery_preserve_strict_session_and_remaining_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("durable.sqlite");
        let mut engine = Engine::open(&path).unwrap();
        let snap = engine
            .execute(Action::CreateSession {
                title: "Strict Focus".into(),
                minutes: 120,
                allow_pause: false,
                focus: FocusConfig {
                    mode: Mode::Strict,
                    apps: vec!["com.apple.TextEdit".into()],
                    domains: vec!["docs.rs".into()],
                    browser_app: TEST_BROWSER.into(),
                    ..Default::default()
                },
                lock: Default::default(),
                lock_password: None,
            })
            .unwrap();
        let id = snap.sessions[0].id.clone();
        let bridge = Bridge::test_bridge(dir.path());
        bridge.acknowledge(TEST_CLIENT, TEST_BROWSER, "");
        engine.bridge = Some(bridge);
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        // Simulate 40 minutes already elapsed before shutdown -> 80m remaining.
        engine.data.sessions[0].remaining_ms = 80 * 60_000;
        engine.save_checkpoint().unwrap();

        let cp = crate::lifecycle::read_checkpoint(&path.with_extension("lifecycle.json"))
            .expect("atomic lifecycle checkpoint written");
        let active = cp.active_session.expect("active session summary written");
        assert_eq!(active.session_id, id);
        assert_eq!(active.mode, "strict");
        assert_eq!(active.remaining_ms, 80 * 60_000);
        assert_eq!(active.allowed_apps, vec!["com.apple.TextEdit".to_string()]);
        assert_eq!(active.allowed_sites, vec!["docs.rs".to_string()]);
        drop(engine);

        // Reboot recovery restores the exact same session ID and remaining time.
        let recovered = Engine::open_recovery(&path, false, true, false).unwrap();
        assert!(recovered.has_active_strict());
        let restored = recovered
            .current()
            .sessions
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        assert!(matches!(
            restored.status,
            Status::Preparing | Status::Running
        ));
        assert_eq!(restored.remaining_ms, 80 * 60_000);
        assert_eq!(
            restored.durability.recovery_status.as_deref(),
            Some("recovered_after_os_shutdown")
        );
    }
}
