//! Retry capture without changing the user's session consent.
//!
//! Presence stability: the native device reports the result of one
//! analysed frame, and a single missed frame is normal (blink, head
//! turn, brief lighting change, detector hiccup). Reporting that frame
//! directly made the session flap between "person detected" and "person
//! not detected", pausing the timer over and over. `Presence` below
//! verifies presence over consecutive frames and holds the last
//! confirmed state through a short grace window, so only a genuine
//! absence is reported. Camera failures stay separate from absence.

/// Consecutive positive frames needed before presence is reported.
const HITS_TO_PRESENT: u32 = 2;
/// Consecutive missed frames needed before presence is withdrawn.
const MISSES_TO_ABSENT: u32 = 3;
/// Time without any positive frame before presence is withdrawn.
const ABSENCE_GRACE_MS: u64 = 2_000;
/// Consecutive failed analyses tolerated before capture is restarted.
/// One failed frame is not a camera failure.
const ERROR_TOLERANCE: u32 = 3;

/// Temporal filter over the device's per-frame person detections.
#[derive(Default)]
struct Presence {
    /// Last confirmed presence. Not reported until confirmed.
    present: bool,
    hits: u32,
    misses: u32,
    last_hit_ms: u64,
    /// Status reported by the previous fold; used to hold the last
    /// verified state through a failed analysis frame.
    reported: String,
}

impl Presence {
    fn reset(&mut self) {
        *self = Self::default();
    }
    /// Fold one analysed frame into the presence state and return the
    /// status to report for it: "person_detected" only once presence is
    /// confirmed, and never "absent" on a single missed frame.
    fn fold(&mut self, raw: &str, now: u64) -> String {
        let status = match raw {
            "person_detected" => {
                self.hits = self.hits.saturating_add(1);
                self.misses = 0;
                self.last_hit_ms = now;
                if self.present || self.hits >= HITS_TO_PRESENT {
                    self.present = true;
                    "person_detected".into()
                } else {
                    // Not verified on consecutive frames yet, so do not
                    // claim a person the camera has not seen twice.
                    "person_not_detected".into()
                }
            }
            "person_not_detected" | "extended_absence" => {
                self.misses = self.misses.saturating_add(1);
                self.hits = 0;
                let confirmed = self.misses >= MISSES_TO_ABSENT
                    && now.saturating_sub(self.last_hit_ms) >= ABSENCE_GRACE_MS;
                if self.present && !confirmed {
                    // Grace: hold the verified state through a short run
                    // of missed frames.
                    "person_detected".into()
                } else {
                    self.present = false;
                    raw.into()
                }
            }
            _ => self.held(),
        };
        self.reported = status.clone();
        status
    }
    /// Last reported state, for a frame whose analysis failed. Never
    /// invents a person: before the first frame it reports none.
    fn held(&self) -> String {
        if self.reported.is_empty() {
            "person_not_detected".into()
        } else {
            self.reported.clone()
        }
    }
}

#[derive(Default)]
pub struct Capture {
    on: bool,
    retry_at: u64,
    presence: Presence,
    errors: u32,
}
trait Device {
    fn start(&mut self) -> bool;
    fn stop(&mut self);
    fn status(&mut self) -> String;
    fn allowed(&self) -> bool;
}
struct Native;
impl Device for Native {
    fn start(&mut self) -> bool {
        crate::platform::camera_start()
    }
    fn stop(&mut self) {
        crate::platform::camera_stop();
    }
    fn status(&mut self) -> String {
        crate::platform::camera_status()
    }
    fn allowed(&self) -> bool {
        crate::platform::camera_permission() == "granted"
    }
}
impl Capture {
    pub fn stop(&mut self) {
        self.stop_with(&mut Native);
    }
    pub fn poll(&mut self, now: u64) -> String {
        self.poll_with(now, &mut Native)
    }
    // Current camera status without starting or stopping capture.
    // Reports "off" while capture is down; otherwise the live device
    // status. Used by the agent's liveness probe so checking the
    // agent never restarts the camera.
    pub fn status(&mut self) -> String {
        self.status_with(&mut Native)
    }
    fn stop_with(&mut self, device: &mut impl Device) {
        if self.on {
            device.stop();
        }
        self.on = false;
        self.retry_at = 0;
        self.presence.reset();
        self.errors = 0;
    }
    fn status_with(&mut self, device: &mut impl Device) -> String {
        if self.on {
            device.status()
        } else {
            "off".into()
        }
    }
    fn poll_with(&mut self, now: u64, device: &mut impl Device) -> String {
        if !device.allowed() {
            if self.on {
                self.stop_with(device);
                return "permission_required".into();
            }
            if crate::platform::camera_permission() == "not_requested" && device.start() {
                self.on = true;
                self.retry_at = now.saturating_add(5000);
                self.presence.reset();
                self.errors = 0;
            } else {
                self.stop_with(device);
                return "permission_required".into();
            }
        }
        if !self.on && now >= self.retry_at {
            self.on = device.start();
            self.retry_at = now.saturating_add(5000);
            // A restarted camera starts unverified: presence has to be
            // confirmed again on fresh frames.
            self.presence.reset();
            self.errors = 0;
            if !self.on {
                device.stop();
            }
        }
        if !self.on {
            return "camera_unavailable".into();
        }
        let raw = device.status();
        if raw == "analysis_error" {
            // A failed analysis is a frame-level hiccup, not a missing
            // camera: hold the last verified state while it clears and
            // only restart capture if it persists.
            self.errors = self.errors.saturating_add(1);
            if self.errors < ERROR_TOLERANCE {
                return self.presence.held();
            }
            device.stop();
            self.on = false;
            self.retry_at = now.saturating_add(5000);
            self.presence.reset();
            self.errors = 0;
            return raw;
        }
        if matches!(
            raw.as_str(),
            "camera_unavailable"
                | "camera_interrupted"
                | "camera_stalled"
                | "no_camera"
                | "camera_error"
                | "off"
        ) {
            // A genuinely unavailable camera is reported as such, never
            // as an absent person.
            device.stop();
            self.on = false;
            self.retry_at = now.saturating_add(5000);
            self.presence.reset();
            self.errors = 0;
            return raw;
        }
        self.errors = 0;
        self.presence.fold(&raw, now)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        allowed: bool,
        available: bool,
        status: &'static str,
        starts: usize,
        stops: usize,
    }
    impl Device for Fake {
        fn start(&mut self) -> bool {
            self.starts += 1;
            self.available
        }
        fn stop(&mut self) {
            self.stops += 1;
        }
        fn status(&mut self) -> String {
            self.status.into()
        }
        fn allowed(&self) -> bool {
            self.allowed
        }
    }
    fn device() -> Fake {
        Fake {
            allowed: true,
            available: true,
            status: "person_detected",
            starts: 0,
            stops: 0,
        }
    }
    /// Poll at one-second intervals, exactly like the attention agent
    /// (the native layer analyses at most one frame per second).
    fn poll_from(c: &mut Capture, d: &mut Fake, start: u64, ticks: u64) -> String {
        let mut status = String::new();
        for tick in 0..ticks {
            status = c.poll_with(start + tick * 1000, d);
        }
        status
    }
    #[test]
    fn a_single_missed_frame_never_reports_absence() {
        let mut c = Capture::default();
        let mut d = device();
        assert_eq!(poll_from(&mut c, &mut d, 0, 2), "person_detected");
        // One blink/head turn: still present, no state change.
        d.status = "person_not_detected";
        assert_eq!(c.poll_with(2000, &mut d), "person_detected");
        // Back to a hit: presence was never withdrawn.
        d.status = "person_detected";
        assert_eq!(c.poll_with(3000, &mut d), "person_detected");
    }
    #[test]
    fn two_missed_frames_inside_the_grace_still_read_as_present() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "person_not_detected";
        assert_eq!(c.poll_with(2000, &mut d), "person_detected");
        assert_eq!(c.poll_with(3000, &mut d), "person_detected");
        // A hit inside the grace window never surfaced as absence.
        d.status = "person_detected";
        assert_eq!(c.poll_with(4000, &mut d), "person_detected");
    }
    #[test]
    fn a_genuine_absence_is_reported_once_confirmed() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "person_not_detected";
        for (i, expected) in [
            (2000, "person_detected"),
            (3000, "person_detected"),
            (4000, "person_not_detected"),
            (5000, "person_not_detected"),
        ] {
            assert_eq!(c.poll_with(i, &mut d), expected);
        }
        // The extended-absence signal of the native layer passes through
        // once absence is confirmed.
        d.status = "extended_absence";
        assert_eq!(c.poll_with(6000, &mut d), "extended_absence");
    }
    #[test]
    fn a_returning_person_is_confirmed_on_consecutive_frames() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "person_not_detected";
        poll_from(&mut c, &mut d, 2000, 4);
        // One hit is not enough to claim a person again.
        d.status = "person_detected";
        assert_eq!(c.poll_with(6000, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(7000, &mut d), "person_detected");
        // A single missed frame after confirmation keeps the state.
        d.status = "person_not_detected";
        assert_eq!(c.poll_with(8000, &mut d), "person_detected");
    }
    #[test]
    fn presence_is_not_reported_before_the_first_frames() {
        let mut c = Capture::default();
        let mut d = device();
        d.status = "person_detected";
        assert_eq!(c.poll_with(0, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(1000, &mut d), "person_detected");
    }
    #[test]
    fn a_single_failed_analysis_is_not_absence_and_keeps_capture() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "analysis_error";
        assert_eq!(c.poll_with(2000, &mut d), "person_detected");
        assert!(c.on);
        assert_eq!(d.stops, 0);
        // The next good frame clears the error count.
        d.status = "person_detected";
        assert_eq!(c.poll_with(3000, &mut d), "person_detected");
        assert!(c.on);
    }
    #[test]
    fn sustained_analysis_failures_restart_capture_instead_of_faking_presence() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "analysis_error";
        assert_eq!(c.poll_with(2000, &mut d), "person_detected");
        assert_eq!(c.poll_with(3000, &mut d), "person_detected");
        // Third consecutive failure: capture is restarted after the
        // usual retry delay, so the error cannot hold presence forever.
        assert_eq!(c.poll_with(4000, &mut d), "analysis_error");
        assert!(!c.on);
        assert_eq!(d.stops, 1);
        assert_eq!(c.poll_with(8000, &mut d), "camera_unavailable");
        // Recovery needs two fresh positive frames again.
        d.status = "person_detected";
        assert_eq!(c.poll_with(9000, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(10000, &mut d), "person_detected");
    }
    #[test]
    fn an_unavailable_camera_is_reported_as_unavailable_not_absent() {
        let mut c = Capture::default();
        let mut d = device();
        poll_from(&mut c, &mut d, 0, 2);
        d.status = "camera_unavailable";
        assert_eq!(c.poll_with(2000, &mut d), "camera_unavailable");
        assert!(!c.on);
        assert_eq!(
            c.poll_with(3000, &mut d),
            "camera_unavailable",
            "no retry before the delay elapses"
        );
    }
    #[test]
    fn unavailable_camera_retries_and_recovers_without_new_consent() {
        let mut c = Capture::default();
        let mut d = device();
        d.available = false;
        assert_eq!(c.poll_with(0, &mut d), "camera_unavailable");
        d.available = true;
        assert_eq!(c.poll_with(4999, &mut d), "camera_unavailable");
        assert_eq!(d.starts, 1);
        assert_eq!(c.poll_with(5000, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(6000, &mut d), "person_detected");
        d.status = "camera_stalled";
        assert_eq!(c.poll_with(7000, &mut d), "camera_stalled");
        assert!(!c.on);
        d.status = "person_detected";
        assert_eq!(c.poll_with(12000, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(13000, &mut d), "person_detected");
    }
    #[test]
    fn revocation_stops_capture_and_permission_restoration_recovers() {
        let mut c = Capture::default();
        let mut d = device();
        c.poll_with(0, &mut d);
        assert_eq!(c.poll_with(1000, &mut d), "person_detected");
        d.allowed = false;
        assert_eq!(c.poll_with(2000, &mut d), "permission_required");
        assert_eq!(d.stops, 1);
        assert_eq!(d.starts, 1);
        d.allowed = true;
        assert_eq!(c.poll_with(3000, &mut d), "person_not_detected");
        assert_eq!(c.poll_with(4000, &mut d), "person_detected");
        c.stop_with(&mut d);
        assert!(!c.on);
        assert_eq!(d.stops, 2);
    }
    #[test]
    fn status_reports_without_starting_or_stopping_capture() {
        // The agent's liveness probe reads status: it must never start
        // a stopped camera, and never stop a running one. Only an
        // explicit stand-down stops capture.
        let mut c = Capture::default();
        let mut d = device();
        assert_eq!(c.status_with(&mut d), "off");
        assert_eq!(d.starts, 0);
        assert_eq!(d.stops, 0);
        assert_eq!(c.poll_with(0, &mut d), "person_not_detected");
        assert_eq!(d.starts, 1);
        assert!(c.on);
        assert_eq!(c.status_with(&mut d), "person_detected");
        assert_eq!(d.starts, 1);
        assert_eq!(d.stops, 0);
        assert!(c.on);
        c.stop_with(&mut d);
        assert!(!c.on);
        assert_eq!(d.stops, 1);
        assert_eq!(c.status_with(&mut d), "off");
        assert_eq!(d.starts, 1);
        assert_eq!(d.stops, 1);
    }
}
