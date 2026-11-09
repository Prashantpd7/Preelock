use crate::{
    attention::{self, Signals, State},
    browser::{self, Bridge, Policy},
    clock,
    database::Database,
    focus::{rule_matches, Mode, WebsiteBlocking, ADULT_DOMAINS},
    models::{Action, Snapshot, Status},
    platform, sessions,
};
use std::path::Path;

pub struct Engine {
    db: Database,
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
    native: bool,
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
        Self::open_inner(path, false)
    }
    pub fn open_native(path: &Path) -> Result<Self, String> {
        Self::open_inner(path, true)
    }
    fn open_inner(path: &Path, native: bool) -> Result<Self, String> {
        let resume = if native {
            crate::lifecycle::initialize(path)?
        } else {
            false
        };
        Self::open_recovery(path, native, resume)
    }
    fn open_recovery(path: &Path, native: bool, resume: bool) -> Result<Self, String> {
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
        // Upgrade historical timers without ever crediting offline elapsed time.
        for s in &mut data.sessions {
            if !s.focus.attention.enabled
                && matches!(
                    s.status,
                    Status::Running | Status::Preparing | Status::Paused
                )
            {
                s.status = Status::Interrupted;
                s.deadline_ms = None;
                s.ended_at = Some(clock::now());
                s.report.event(clock::now(),"timer_upgrade","Legacy timer stopped on upgrade. Create a productive-time session; historical time is retained as legacy.");
            }
        }
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
            // Only a confirmed OS shutdown or a different boot restores work.
            // Same-boot process loss keeps the existing Force Quit warning.
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
                    s.report.event(
                        clock::now(),
                        "lock_restored",
                        "Locked session restored after restart; its lock still applies.",
                    );
                    continue;
                }
                // Step 11 anti-tamper: a same-boot kill no longer
                // ends the block. The session resumes with its
                // checkpointed remaining time intact (publish() at
                // the end of this function republishes its policy
                // before any tick counts time, exactly as the
                // OS-restart path does), and the force_quit flag
                // plus event are KEPT as visibility — the flag
                // now means "the app died mid-session and the
                // block resumed", not "the block ended". Only a
                // session whose state cannot be loaded at all
                // (legacy upgrade, storage failure) still ends
                // as Interrupted; every live session here is
                // recoverable from its SQLite checkpoint.
                attention::recover(s, clock::now());
                s.force_quit = true;
                s.status = crate::tamper::resume_status(&s.status);
                s.ended_at = None;
                s.deadline_ms = None;
                s.report.camera_status = "off".into();
                s.report.event(
                    clock::now(),
                    "force_quit",
                    "Preelock was closed during this session; the block resumed with its remaining time preserved instead of ending early.",
                );
                continue;
            }
            attention::recover(s, clock::now());
        }
        db.save(&data)?;
        let (bridge, bridge_error) = if native {
            match Bridge::start(path.parent().ok_or("Invalid database location.")?) {
                Ok(b) => (Some(b), String::new()),
                Err(e) => (None, e),
            }
        } else {
            (None, "Native runtime disabled in domain tests.".into())
        };
        let now = clock::continuous_ms();
        let engine = Self {
            db,
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
            native,
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
    pub fn shutdown_if_idle(&mut self) -> bool {
        if self.has_active() {
            return false;
        }
        self.shutdown();
        true
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
        self.bridge
            .as_ref()
            .map(Bridge::status)
            .unwrap_or_else(|| browser::unavailable(&self.bridge_error))
    }
    /// Connection status for one specific browser (platform browser id).
    pub fn browser_status_for(&self, browser_id: &str) -> browser::BrowserStatus {
        self.bridge
            .as_ref()
            .map(|b| b.status_for(browser_id))
            .unwrap_or_else(|| browser::unavailable(&self.bridge_error))
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
    pub fn pairing_token(&self) -> String {
        self.bridge
            .as_ref()
            .map(|b| b.token.clone())
            .unwrap_or_default()
    }
    pub fn reset_pairing(&mut self, path: &Path) -> Result<(), String> {
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
            self.bridge = Some(Bridge::start(path)?);
        }

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
    /// Step 12: record a frozen-overlay creation failure as a
    /// report event on the live frozen session, once per run of
    /// consecutive failures (the sync in `lib.rs` retries every
    /// second, so recording every attempt would spam the report).
    /// Enforcement continues regardless: app and website blocking
    /// are engine-side and never depend on the overlay window.
    pub fn note_frozen_overlay_failure(&mut self) {
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
        self.commit(None)
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
        if let Action::StartSession { id } | Action::ResumeSession { id } = &action {
            self.stop_test();
            self.preflight(id)?;
        }
        if matches!(&action, Action::EnableCamera { .. }) {
            self.stop_camera();
        }
        let result = self.commit(Some(action));
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
                        Policy::new(
                            &s.id,
                            &s.focus,
                            if s.status == Status::Preparing || s.focus.attention.enabled {
                                60_000
                            } else {
                                s.remaining_ms
                            },
                        )
                        .with_exhausted_rules(&exhausted)
                        .with_session_cause(crate::blockreasons::session_cause(
                            s,
                            &self.data.schedules,
                        ))
                        .with_rule_causes(&rule_causes)
                        .with_activity_tracking(true)
                    })
            };
            b.publish(policy);
        }
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
        crate::desktop_settings::notify(
            "Preelock",
            &crate::blockreasons::cause_text(cause, name, now),
        );
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
    fn commit(&mut self, action: Option<Action>) -> Result<Snapshot, String> {
        if let Some(error) = &self.failed {
            return Err(format!("Restrictions released after a storage error. Restart Preelock after resolving: {error}"));
        }
        let tick = clock::continuous_ms();
        let now = clock::now();
        let elapsed = tick.saturating_sub(self.anchor);
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
                sessions::advance(s, elapsed, now);
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
            if let Err(e) = self.db.save(&next) {
                self.fail_open(&e);
                return Err(e);
            }
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
    fn evaluate_warnings(&mut self, now: i64) -> Result<bool, String> {
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
                crate::desktop_settings::notify(
                    "Block starting soon",
                    &crate::notifications::warning_body(&schedule.title, start, now),
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
                crate::desktop_settings::notify(
                    "Block starting soon",
                    &crate::notifications::warning_body(&shot.title, shot.fire_at_ms, now),
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
    fn sample_screen_time(&mut self, now: i64, delta: u64) -> Result<(), String> {
        if delta == 0 || delta > 3000 || !self.native {
            return Ok(());
        }
        let sample = platform::sample()?;
        let day_ms = crate::screen_time::local_day_start_ms(now);
        let browser = self.browser_status();
        let mut buckets = Vec::new();
        if sample.idle_ms < 60_000 && !sample.id.is_empty() {
            crate::screen_time::add_usage(
                &mut buckets,
                day_ms,
                "app",
                &sample.id,
                &sample.name,
                delta,
            );
        }
        if browser.activity_fresh && browser.browser_focused && !browser.active_domain.is_empty() {
            crate::screen_time::add_usage(
                &mut buckets,
                day_ms,
                "site",
                &browser.active_domain,
                &browser.active_domain,
                delta,
            );
        }
        let has_usage = !buckets.is_empty();
        if has_usage {
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
        if sample.idle_ms < 60_000
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
        let locks_pruned = crate::applocks::prune_expired(&mut self.data.app_locks, now);
        if sample.idle_ms < 60_000
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
        if allowance_due || locks_pruned {
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
    pub fn tick(&mut self) -> Result<(), String> {
        // Step 6b: wall-clock gap detection. Ticks pause during sleep
        // on both OSes, so a gap over the threshold means wake from
        // sleep (or a long suspension). The tick below already runs
        // the existing session recovery path (commit(): monotonic
        // timer, no offline credit) and then schedule re-evaluation
        // on current local time, so a window that ended during sleep
        // ends on this same tick and is never extended. The gap flag
        // additionally expires due one-shot timers (step 6c) instead
        // of firing them late. The timestamp is captured once so
        // schedule and one-shot decisions share one moment.
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
        self.commit(None)?;
        let running = self
            .data
            .sessions
            .iter()
            .any(|s| s.status == Status::Running);
        if self.native && !running {
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
            // Nothing changed that needs saving (the session was
            // already saved with its grace deadline at creation),
            // and publish() skips grace sessions, so the extension
            // keeps no rules during grace.
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
                        let day_ms = crate::screen_time::local_day_start_ms(now);
                        let mut buckets = Vec::new();
                        if s.status == Status::Running
                            && !in_break
                            && delta <= 3000
                            && sample.idle_ms < 60_000
                            && !sample.id.is_empty()
                        {
                            crate::screen_time::add_usage(
                                &mut buckets,
                                day_ms,
                                "app",
                                &sample.id,
                                &sample.name,
                                delta,
                            );
                        }
                        if s.status == Status::Running
                            && !in_break
                            && delta <= 3000
                            && browser.activity_fresh
                            && browser.browser_focused
                            && !browser.active_domain.is_empty()
                        {
                            crate::screen_time::add_usage(
                                &mut buckets,
                                day_ms,
                                "site",
                                &browser.active_domain,
                                &browser.active_domain,
                                delta,
                            );
                        }
                        if !buckets.is_empty() {
                            self.db.record_screen_time(&buckets)?;
                            for bucket in buckets {
                                crate::screen_time::add_usage(
                                    &mut next.screen_time_usage,
                                    bucket.day_ms,
                                    &bucket.kind,
                                    &bucket.target,
                                    &bucket.name,
                                    bucket.ms,
                                );
                            }
                        }
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
                                            crate::desktop_settings::notify(
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
                let status = self.camera.poll(tick);
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
            if let Err(e) = self.db.save(&next) {
                self.fail_open(&e);
                return Err(e);
            }
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
            assert!(!engine.shutdown_if_idle());
        }
        engine.data.sessions[0].status = Status::Completed;
        assert!(engine.shutdown_if_idle());
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
        let mut engine = Engine::open_inner(&path, false).unwrap();
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
        // killed session resumes with its remaining time intact,
        // still flagged force_quit (visibility, not punishment);
        // the cleanly ended session is untouched.
        let engine = Engine::open_inner(&path, false).unwrap();
        assert!(engine.data.sessions[0].force_quit);
        assert_eq!(engine.data.sessions[0].status, Status::Running);
        assert_eq!(engine.data.sessions[0].remaining_ms, 25 * 60_000);
        assert!(engine.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "force_quit"));
        assert!(!engine.data.sessions[1].force_quit);
    }
    #[test]
    fn restart_lock_resumes_locked_session_instead_of_flagging_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock-restart.sqlite");
        let mut engine = Engine::open_inner(&path, false).unwrap();
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
        let engine = Engine::open_inner(&path, false).unwrap();
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
            let mut e = Engine::open_recovery(&path, false, true).unwrap();
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
        let mut e = Engine::open_recovery(&dir.path().join("test.sqlite"), false, true).unwrap();
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
        // Step 11: a paused session on disk means the app was killed
        // mid-session: it resumes Paused (its pause and remaining
        // time preserved), still flagged force_quit.
        assert_eq!(state.sessions[0].status, crate::models::Status::Paused);
        assert!(state.sessions[0].force_quit);
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
        assert!(!e.shutdown_if_idle());
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
        assert!(!e.shutdown_if_idle());
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
        // Step 11: the app was killed mid-session: the session
        // resumes Running with its remaining time preserved,
        // still flagged force_quit, and no enforcement is claimed
        // across the restart beyond the resumed block itself.
        let restored = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Running);
        assert!(restored.data.sessions[0].force_quit);
        assert!(restored.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "force_quit"));
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
        assert!(s.force_quit);
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
            pomodoro: Default::default(),
            reward: Default::default(),
            lock: Default::default(),
            lock_state: Default::default(),
            policy_applied: false,
            browser_fail_closed: false,
            grace_until_ms: None,
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
        assert!(engine.evaluate_warnings(at_warning).unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        // Same tick again, and after a restart reload from disk:
        // the point never fires twice.
        assert!(!engine.evaluate_warnings(at_warning).unwrap());
        assert_eq!(engine.data.schedules[0].warning_fired.len(), 1);
        drop(engine);
        let mut restored = Engine::open(&path).unwrap();
        assert_eq!(restored.data.schedules[0].warning_fired.len(), 1);
        assert!(!restored.evaluate_warnings(at_warning).unwrap());
        assert_eq!(restored.data.schedules[0].warning_fired.len(), 1);
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
        assert!(!engine.evaluate_warnings(at_warning).unwrap());
        assert!(engine.data.schedules[0].warning_fired.is_empty());
        engine.data.preferences.notifications_enabled = true;
        engine.data.schedules[0].enabled = false;
        assert!(!engine.evaluate_warnings(at_warning).unwrap());
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
        let mut idle = Engine::open(&dir.path().join("idle.sqlite")).unwrap();
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
}
