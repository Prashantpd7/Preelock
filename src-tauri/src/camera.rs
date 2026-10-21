//! Retry capture without changing the user's session consent.
#[derive(Default)]
pub struct Capture {
    on: bool,
    retry_at: u64,
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
    fn stop_with(&mut self, device: &mut impl Device) {
        if self.on {
            device.stop();
        }
        self.on = false;
        self.retry_at = 0;
    }
    fn poll_with(&mut self, now: u64, device: &mut impl Device) -> String {
        if !device.allowed() {
            self.stop_with(device);
            return "permission_required".into();
        }
        if !self.on && now >= self.retry_at {
            self.on = device.start();
            self.retry_at = now.saturating_add(5000);
            if !self.on {
                device.stop();
            }
        }
        let status = if self.on {
            device.status()
        } else {
            "camera_unavailable".into()
        };
        if self.on
            && matches!(
                status.as_str(),
                "camera_unavailable"
                    | "camera_interrupted"
                    | "camera_stalled"
                    | "analysis_error"
                    | "no_camera"
                    | "camera_error"
                    | "off"
            )
        {
            device.stop();
            self.on = false;
            self.retry_at = now.saturating_add(5000);
        }
        status
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
            available: false,
            status: "person_detected",
            starts: 0,
            stops: 0,
        }
    }
    #[test]
    fn unavailable_camera_retries_and_recovers_without_new_consent() {
        let mut c = Capture::default();
        let mut d = device();
        assert_eq!(c.poll_with(0, &mut d), "camera_unavailable");
        d.available = true;
        assert_eq!(c.poll_with(4999, &mut d), "camera_unavailable");
        assert_eq!(d.starts, 1);
        assert_eq!(c.poll_with(5000, &mut d), "person_detected");
        d.status = "camera_stalled";
        assert_eq!(c.poll_with(6000, &mut d), "camera_stalled");
        assert!(!c.on);
        d.status = "person_detected";
        assert_eq!(c.poll_with(11000, &mut d), "person_detected");
    }
    #[test]
    fn revocation_stops_capture_and_permission_restoration_recovers() {
        let mut c = Capture::default();
        let mut d = device();
        d.available = true;
        c.poll_with(0, &mut d);
        d.allowed = false;
        assert_eq!(c.poll_with(1000, &mut d), "permission_required");
        assert_eq!(d.stops, 1);
        assert_eq!(d.starts, 1);
        d.allowed = true;
        assert_eq!(c.poll_with(2000, &mut d), "person_detected");
        c.stop_with(&mut d);
        assert!(!c.on);
        assert_eq!(d.stops, 2);
    }
}
