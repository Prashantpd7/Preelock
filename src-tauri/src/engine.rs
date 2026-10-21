use crate::{
    attention::{self, Signals, State},
    browser::{self, Bridge, Policy},
    clock,
    database::Database,
    focus::Mode,
    models::{Action, Snapshot, Status},
    platform, sessions,
};
use std::path::Path;

pub struct Engine {
    db: Database,
    data: Snapshot,
    anchor: u64,
    sample_anchor: u64,
    wall: i64,
    checkpoint: u64,
    bridge: Option<Bridge>,
    bridge_error: String,
    native: bool,
    camera: crate::camera::Capture,
    camera_test_until: u64,
    failed: Option<String>,
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
            // Only a confirmed OS shutdown or a different boot restores work.
            // Same-boot process loss keeps the existing Force Quit warning.
            if matches!(
                s.status,
                Status::Running | Status::Preparing | Status::Paused
            ) {
                if resume {
                    attention::recover(s, clock::now());
                    if s.status != Status::Paused {
                        s.status = if s.focus.network_required() {
                            Status::Preparing
                        } else {
                            Status::Running
                        };
                    }
                    s.ended_at = None;
                    s.deadline_ms = None;
                    s.report.camera_status = "off".into();
                    s.report.event(clock::now(), "system_restart", "Session restored after system shutdown/restart; offline time does not count.");
                    continue;
                }
                s.force_quit = true;
                s.status = Status::Interrupted;
                s.ended_at = Some(clock::now());
                s.deadline_ms = None;
                s.report.enforcement = "Ended: the app was closed during the session".into();
                s.report.event(
                    clock::now(),
                    "force_quit",
                    "VYSTRIQ was closed during this session; the session was ended.",
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
            wall: clock::now(),
            checkpoint: now,
            bridge,
            bridge_error,
            native,
            camera: Default::default(),
            camera_test_until: 0,
            failed: None,
        };
        engine.publish();
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
                    "User quit VYSTRIQ; temporary restrictions released.",
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
                        "Pair the Vystriq extension in {} before starting. No session has started.",
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
        self.stop_test();
        self.publish();
        Ok(())
    }
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.commit(None)
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
                self.data
                    .sessions
                    .iter()
                    .find(|s| {
                        matches!(s.status, Status::Running | Status::Preparing)
                            && s.focus.network_required()
                    })
                    .map(|s| {
                        Policy::new(
                            &s.id,
                            &s.focus,
                            if s.status == Status::Preparing || s.focus.attention.enabled {
                                60_000
                            } else {
                                s.remaining_ms
                            },
                        )
                    })
            };
            b.publish(policy);
        }
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
            return Err(format!("Restrictions released after a storage error. Restart VYSTRIQ after resolving: {error}"));
        }
        let tick = clock::continuous_ms();
        let now = clock::now();
        let elapsed = tick.saturating_sub(self.anchor);
        let mut next = self.data.clone();
        for s in &mut next.sessions {
            if s.status == Status::Running && (now - self.wall - elapsed as i64).abs() > 5000 {
                s.report.event(
                    now,
                    "clock_change",
                    "Wall clock changed; suspend-aware monotonic timer retained.",
                );
            }
            if !s.focus.attention.enabled {
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
    pub fn tick(&mut self) -> Result<(), String> {
        if self.camera_test_until > 0 && clock::continuous_ms() >= self.camera_test_until {
            self.stop_test();
        }
        self.commit(None)?;
        let now = clock::now();
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
            return Ok(());
        };
        let mut next = self.data.clone();
        let s = &mut next.sessions[index];
        // Track the connection of this session's own browser — the status
        // of any other paired browser does not count here.
        let browser = self.browser_status_for(&s.focus.browser_app);
        if s.status == Status::Preparing {
            s.attention.state = State::Preparing;
        }
        let mut signals = Signals::default();
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
            if s.focus.network_required()
                && (!browser.connected || browser.applied_id != s.id || !browser.error.is_empty())
            {
                s.status = Status::Preparing;
                s.deadline_ms = None;
                s.attention.state = State::Preparing;
                s.report.enforcement = "Waiting for browser reconnection".into();
                s.report.event(now, "enforcement_lost", "Browser disconnected. Session stays active; focus time waits for reconnection.");
            }
            if s.status == Status::Running && s.focus.activity_consent && self.native {
                match platform::sample() {
                    Ok(sample) => {
                        let blocked = !sample.id.is_empty() && s.focus.blocked(&sample.id);
                        signals.activity_available = !sample.id.is_empty();
                        let work_apps = if s.focus.mode == Mode::Standard {
                            &s.focus.attention.work_apps
                        } else {
                            &s.focus.apps
                        };
                        signals.approved_app = !blocked
                            && (work_apps
                                .iter()
                                .any(|id| id.eq_ignore_ascii_case(&sample.id))
                                || sample.id == s.focus.browser_app);
                        signals.idle_ms = sample.idle_ms;
                        if sample.id == s.focus.browser_app && !sample.id.is_empty() {
                            let domains = if s.focus.mode == Mode::Standard {
                                &s.focus.attention.work_domains
                            } else {
                                &s.focus.domains
                            };
                            signals.browser_required = true;
                            if browser.activity_fresh
                                && browser.browser_focused
                                && !browser.active_domain.is_empty()
                            {
                                signals.browser_relevant = Some(domains.iter().any(|d| {
                                    browser.active_domain == *d
                                        || browser.active_domain.ends_with(&format!(".{d}"))
                                }));
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
                        // One-second samples, not inferred activity during sleep or a process outage.
                        if sample.idle_ms >= 60_000 {
                            s.report.idle_ms += if delta <= 3000 { delta } else { 0 };
                        } else if blocked {
                            s.report.distracting_ms += if delta <= 3000 { delta } else { 0 };
                        } else {
                            s.report.approved_ms += if delta <= 3000 { delta } else { 0 };
                        }
                        if blocked {
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
                        } else {
                            s.report.enforcement = if s.focus.network_required() {
                                "Browser acknowledged; native monitor active"
                            } else {
                                "Native monitor active"
                            }
                            .into();
                        }
                    }
                    Err(e) => {
                        s.status = Status::Interrupted;
                        s.ended_at = Some(now);
                        s.deadline_ms = None;
                        s.report.event(now, "permission_or_api_failure", &e);
                    }
                }
            }
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
            attention::observe(s, &signals, delta, now);
        }
        let terminal = !matches!(s.status, Status::Running | Status::Preparing);
        if terminal {
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
            || next.sessions[index].attention.state != self.data.sessions[index].attention.state;
        if changed {
            if let Err(e) = self.db.save(&next) {
                self.fail_open(&e);
                return Err(e);
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
    fn restart_flags_sessions_killed_mid_run() {
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
                })
                .unwrap();
        }
        let clean_id = engine.data.sessions[1].id.clone();
        engine.execute(Action::EndSession { id: clean_id }).unwrap();
        // Simulate the process dying mid-session: shutdown() never ran,
        // so the on-disk status is still live.
        engine.data.sessions[0].status = Status::Running;
        engine.db.save(&engine.data).unwrap();
        drop(engine);
        let engine = Engine::open_inner(&path, false).unwrap();
        assert!(engine.data.sessions[0].force_quit);
        assert_eq!(engine.data.sessions[0].status, Status::Interrupted);
        assert!(!engine.data.sessions[1].force_quit);
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
        assert_eq!(e.data.sessions[0].status, Status::Preparing);
        assert!(!e.data.sessions[0].force_quit);
        e.bridge = Some(Bridge::test_bridge(dir.path()));
        e.publish();
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, 0);
        assert_eq!(e.data.sessions[0].status, Status::Preparing);
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
        e.execute(Action::EndSession { id }).unwrap();
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
            })
            .unwrap();
        let id = state.sessions[0].id.clone();
        engine
            .execute(Action::StartSession { id: id.clone() })
            .unwrap();
        engine.execute(Action::PauseSession { id }).unwrap();
        drop(engine);
        let mut restored = Engine::open(&path).unwrap();
        let state = restored.snapshot().unwrap();
        assert_eq!(state.tasks[0].title, "Read");
        assert_eq!(state.preferences.theme, "light");
        // A paused session on disk means the app was killed mid-session:
        // it ends as Interrupted (flagged), it is never resumed.
        assert_eq!(state.sessions[0].status, crate::models::Status::Interrupted);
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
                focus: Default::default()
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
        assert_eq!(e.data.sessions[0].status, Status::Preparing);
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
    fn restart_flags_killed_network_session() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge
            .as_ref()
            .unwrap()
            .acknowledge(TEST_CLIENT, TEST_BROWSER, &id);
        e.tick().unwrap();
        drop(e);
        // The app was killed mid-session: the session ends flagged, it is
        // not resumed, and no enforcement is claimed across the restart.
        let restored = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Interrupted);
        assert!(restored.data.sessions[0].force_quit);
        assert!(restored.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "force_quit"));
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
