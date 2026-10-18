//! Contracts for future adapters. Merely reading capabilities never requests permission.
use serde::Serialize;
#[derive(Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub name: &'static str,
    pub available: bool,
    pub reason: &'static str,
}
pub trait PermissionProvider {
    fn granted(&self, permission: &str) -> bool;
    fn request(&self, permission: &str) -> Result<(), String>;
}
pub trait FocusRestrictions {
    fn apply(&self, allowed_apps: &[String]) -> Result<(), String>;
    fn emergency_release(&self) -> Result<(), String>;
}
pub trait ActivityMonitor {
    fn start(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}
pub trait CameraMonitor {
    fn start(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}
pub trait AttentionAnalyzer {
    fn analyze(&self, consented_input: &[u8]) -> Result<f32, String>;
}
pub trait TaskVerifier {
    fn verify(&self, consented_evidence: &[u8]) -> Result<bool, String>;
}
pub fn list() -> Vec<Capability> {
    [
        ("restrictions", "App and website restrictions"),
        ("network", "VPN and proxy policy"),
        ("activity", "Active-window and screen monitoring"),
        ("camera", "Camera attention monitoring"),
        ("ai", "AI attention analysis"),
        ("verification", "Task, lecture and coding verification"),
        ("reminders", "Reminders and interventions"),
    ]
    .into_iter()
    .map(|(id, name)| Capability {
        id,
        name,
        available: false,
        reason: "Planned · no monitoring or enforcement is active",
    })
    .collect()
}
