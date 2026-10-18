use serde::Serialize;
#[derive(Serialize)]
pub struct Capability {
    pub id: &'static str,
    pub name: &'static str,
    pub available: bool,
    pub reason: &'static str,
}
pub fn list() -> Vec<Capability> {
    vec![
    Capability{id:"apps",name:"Application redirection",available:crate::platform::supported(),reason:"Opt-in native app hiding on macOS / foreground minimization on Windows; not a launch prohibition or security boundary."},
    Capability{id:"browser",name:"Browser website filtering",available:true,reason:"Requires paired Chromium extension. Blocklist, allowlist and browser-offline modes; profile scoped, not system wide."},
    Capability{id:"activity",name:"Foreground app and idle sampling",available:crate::platform::supported(),reason:"Per-session consent required. No titles, keystrokes or screen capture."},
    Capability{id:"camera",name:"Local camera presence",available:cfg!(target_os="macos"),reason:"macOS AVFoundation + Vision; explicit permission and per-session consent. Probabilistic. Windows implementation unavailable."},
    Capability{id:"deep",name:"Managed Deep Focus",available:false,reason:"Requires separately deployed managed OS policy, restricted account, entitlement/configuration and administrator recovery. Not installed by this app."},
    Capability{id:"network",name:"System-wide network / VPN enforcement",available:false,reason:"No firewall, DNS, proxy or VPN policies are installed."},
    Capability{id:"verification",name:"Phone, task and learning verification",available:false,reason:"No validated model or evidence pipeline is integrated."},
]
}
