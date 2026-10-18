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
    manual_until: u64,
    wall: i64,
    checkpoint: u64,
    bridge: Option<Bridge>,
    bridge_error: String,
    native: bool,
    camera_on: bool,
    prepared_at: u64,
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
        let mut db = Database::open(path)?;
        let mut data = db.load()?;
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
            attention::recover(s, clock::now());
            if matches!(s.status, Status::Running | Status::Preparing) {
                s.report.event(
                    clock::now(),
                    "restart",
                    "Application restarted; checking restrictions again.",
                );
                if s.focus.network_required() {
                    s.status = Status::Preparing;
                    s.deadline_ms = None;
                    s.report.enforcement = "Waiting for browser reactivation".into();
                }
            }
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
            manual_until: 0,
            wall: clock::now(),
            checkpoint: now,
            bridge,
            bridge_error,
            native,
            camera_on: false,
            prepared_at: now,
            failed: None,
        };
        engine.publish();
        Ok(engine)
    }
    pub fn shutdown(&mut self) {
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
    fn preflight(&self, id: &str) -> Result<(), String> {
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
        if (s.focus.apps_required() || s.focus.activity_consent)
            && (!self.native || !platform::supported())
        {
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
            return Err(
                "Enable authorized camera monitoring or explicitly select activity-only fallback."
                    .into(),
            );
        }
        if s.focus.network_required() {
            let b = self.browser_status();
            if !b.connected || !b.error.is_empty() {
                return Err("Pair the browser extension and use Sync now before starting. No session has started.".into());
            }
        }
        if s.focus.apps_required() {
            platform::sample().map_err(|e| format!("Native activity API unavailable: {e}"))?;
        }
        Ok(())
    }
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.commit(None)
    }
    pub fn execute(&mut self, action: Action) -> Result<Snapshot, String> {
        if let Action::StartSession { id } | Action::ResumeSession { id } = &action {
            self.preflight(id)?;
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
        if self.camera_on {
            platform::camera_stop();
            self.camera_on = false;
        }
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
        let confirmed = matches!(&action, Some(Action::ConfirmWork { .. }));
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
            self.manual_until = 0;
            let s = next.sessions.iter_mut().find(|s| s.id == id).unwrap();
            s.attention.checkpoint_at = Some(now);
            s.attention.candidate_ms = 0;
            s.attention.state = State::Unknown;
            s.attention.reason = "Waiting for fresh observations".into();
            if !s.focus.apps_required() && !s.focus.network_required() {
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
                self.prepared_at = tick;
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
        if confirmed {
            self.manual_until = tick.saturating_add(60_000);
        }
        self.data = next;
        self.anchor = tick;
        self.wall = now;
        self.publish();
        self.data.browser = Some(self.browser_status());
        Ok(self.data.clone())
    }
    pub fn tick(&mut self) -> Result<(), String> {
        self.commit(None)?;
        let now = clock::now();
        let tick = clock::continuous_ms();
        let delta = tick.saturating_sub(self.sample_anchor);
        self.sample_anchor = tick;
        let browser = self.browser_status();
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
        if s.status == Status::Preparing {
            s.attention.state = State::Preparing;
        }
        let mut signals = Signals {
            manual_confirmed: tick < self.manual_until,
            ..Default::default()
        };
        if s.status == Status::Preparing {
            if browser.connected && browser.applied_id == s.id && browser.error.is_empty() {
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
            } else if tick.saturating_sub(self.prepared_at) > 45_000 {
                s.status = Status::Interrupted;
                s.ended_at = Some(now);
                s.report.enforcement = "Activation failed; restrictions released".into();
                s.report.event(
                    now,
                    "activation_failed",
                    "Browser did not acknowledge rules within 45 seconds. Timer did not run.",
                );
            }
        } else {
            if s.focus.network_required()
                && (!browser.connected || browser.applied_id != s.id || !browser.error.is_empty())
            {
                s.status = Status::Interrupted;
                s.ended_at = Some(now);
                s.deadline_ms = None;
                s.report.enforcement = "Released: browser disconnected".into();
                s.report.event(now,"enforcement_lost","Browser rule acknowledgement was lost; use recovery or reconnect before another session.");
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
                        if blocked && s.focus.apps_required() {
                            if !platform::hide(&sample) {
                                s.status = Status::Interrupted;
                                s.ended_at = Some(now);
                                s.deadline_ms = None;
                                s.report.event(now,"enforcement_failed","OS refused to hide/minimize a restricted app; restrictions released.");
                            } else {
                                s.report.enforcement =
                                    "Native app redirection active (reversible)".into();
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
                if !self.camera_on {
                    self.camera_on = platform::camera_start();
                }
                let status = if self.camera_on {
                    platform::camera_status()
                } else {
                    "camera_unavailable".into()
                };
                signals.camera_status = status.clone();
                if status != s.report.camera_status {
                    s.report.event(now, "camera_status", &status);
                    s.report.camera_status = status.clone();
                }
                if matches!(
                    status.as_str(),
                    "camera_unavailable"
                        | "camera_interrupted"
                        | "camera_stalled"
                        | "analysis_error"
                ) || platform::camera_permission() != "granted"
                {
                    s.focus.camera = false;
                    s.report.event(now,"camera_disabled","Camera or analysis became unavailable; capture stopped. Session continues without camera.");
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
    #[test]
    fn productive_engine_checkpoints_recovery_and_emergency_cleanup() {
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
        e.execute(Action::ConfirmWork { id: id.clone() }).unwrap();
        for _ in 0..3 {
            e.sample_anchor = clock::continuous_ms().saturating_sub(1100);
            e.tick().unwrap();
        }
        let accepted = e.data.sessions[0].focused_ms;
        assert!((2000..5000).contains(&accepted));
        e.sample_anchor = clock::continuous_ms().saturating_sub(60000);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, accepted);
        assert_eq!(e.data.sessions[0].attention.state, State::Recovering);
        e.db.save(&e.data).unwrap();
        drop(e);
        let mut e = Engine::open(&path).unwrap();
        assert_eq!(e.snapshot().unwrap().sessions[0].focused_ms, accepted);
        assert_eq!(e.manual_until, 0);
        e.sample_anchor = clock::continuous_ms().saturating_sub(1100);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].focused_ms, accepted);
        e.execute(Action::EmergencyExit {
            id,
            confirmation: "END FOCUS".into(),
        })
        .unwrap();
        drop(e);
        let mut e = Engine::open(&path).unwrap();
        let state = e.snapshot().unwrap();
        assert_eq!(state.sessions[0].attention.state, State::EmergencyExit);
        assert_eq!(state.sessions[0].status, Status::Interrupted);
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
        assert_eq!(state.sessions[0].status, crate::models::Status::Paused);
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
        e.bridge.as_ref().unwrap().acknowledge("");
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
        e.bridge.as_ref().unwrap().acknowledge(&id);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Running);
        e.data.sessions[0].remaining_ms = 0;
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Completed);
        assert!(e.browser_status().cleanup_pending);
        e.bridge.as_ref().unwrap().acknowledge("");
        assert!(!e.browser_status().cleanup_pending);
    }
    #[test]
    fn missing_ack_times_out_and_browser_loss_interrupts() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id }).unwrap();
        // Set the monotonic preparation timestamp in the past without waiting.
        e.prepared_at = clock::continuous_ms().saturating_sub(46_000);
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Interrupted);
        let dir2 = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir2.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge.as_ref().unwrap().acknowledge(&id);
        e.tick().unwrap();
        e.bridge.as_ref().unwrap().acknowledge("");
        e.tick().unwrap();
        assert_eq!(e.data.sessions[0].status, Status::Interrupted);
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
    fn restart_revalidates_network_instead_of_claiming_enforcement() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge.as_ref().unwrap().acknowledge(&id);
        e.tick().unwrap();
        drop(e);
        let restored = Engine::open(&dir.path().join("test.sqlite")).unwrap();
        assert_eq!(restored.data.sessions[0].status, Status::Preparing);
        assert!(restored.data.sessions[0]
            .report
            .events
            .iter()
            .any(|e| e.kind == "restart"));
    }
    #[test]
    fn sqlite_failure_releases_policy_and_preserves_prior_disk_state() {
        let dir = tempfile::tempdir().unwrap();
        let (mut e, id) = network_engine(dir.path());
        e.execute(Action::StartSession { id: id.clone() }).unwrap();
        e.bridge.as_ref().unwrap().acknowledge(&id);
        e.tick().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("test.sqlite")).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'simulated disk failure'); END;").unwrap();
        assert!(e
            .execute(Action::EmergencyExit {
                id,
                confirmation: "END FOCUS".into()
            })
            .is_err());
        assert!(e.failed.is_some());
        assert_eq!(e.data.sessions[0].status, Status::Interrupted);
        assert!(e.browser_status().cleanup_pending);
    }
}
