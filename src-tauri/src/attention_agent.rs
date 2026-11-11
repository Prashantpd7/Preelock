use crate::{camera::Capture, clock};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const ARG: &str = "--attention";
const LOCK: &str = "attention.lock";
const PORT: &str = "attention-agent.port";
const TOKEN: &str = "attention-agent.token";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signal {
    pub status: String,
    pub sampled_at_ms: u64,
}

impl Signal {
    pub fn fresh(&self, now_ms: u64) -> bool {
        self.sampled_at_ms <= now_ms && now_ms.saturating_sub(self.sampled_at_ms) <= 3_000
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    active: bool,
    /// Liveness probe: report the current status without touching capture.
    /// Serde-default so requests from older clients still parse.
    #[serde(default)]
    probe: bool,
}

fn token(dir: &Path) -> Result<String, String> {
    let path = dir.join(TOKEN);
    if let Ok(value) = std::fs::read_to_string(&path) {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    let value = uuid::Uuid::new_v4().simple().to_string();
    let temp = dir.join(format!(".{TOKEN}.tmp"));
    std::fs::write(&temp, &value).map_err(|e| e.to_string())?;
    if std::fs::rename(&temp, &path).is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

fn port(dir: &Path) -> Result<u16, String> {
    std::fs::read_to_string(dir.join(PORT))
        .map_err(|e| e.to_string())?
        .trim()
        .parse()
        .map_err(|e| format!("Invalid attention-agent port: {e}"))
}

fn running(dir: &Path) -> bool {
    probe(dir).is_ok()
}

pub fn ensure_running(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let _ = token(dir)?;
    if running(dir) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Command::new(exe)
        .arg(ARG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start attention agent: {e}"))
}

pub fn signal(dir: &Path, active: bool) -> Result<Signal, String> {
    request(
        dir,
        Request {
            token: token(dir)?,
            active,
            probe: false,
        },
    )
}

/// Liveness probe: answers only if the agent is responsive, and never
/// starts or stops the camera. The old probe sent `active: false`, which
/// the agent treated as a stand-down order — stopping the camera every
/// second while a session was live.
fn probe(dir: &Path) -> Result<Signal, String> {
    request(
        dir,
        Request {
            token: token(dir)?,
            active: false,
            probe: true,
        },
    )
}

fn request(dir: &Path, request: Request) -> Result<Signal, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port(dir)?))
        .map_err(|e| format!("Attention agent unavailable: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let body = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    writeln!(stream, "{body}").map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&response).map_err(|e| format!("Invalid attention signal: {e}"))
}

/// What the agent must do for a request. Classified separately from
/// `handle()` so the probe/stand-down distinction is unit-testable
/// without camera hardware.
#[derive(Debug, PartialEq, Eq)]
enum RequestAction {
    /// Authenticated `active: true` — poll the camera (may start it).
    Poll,
    /// Authenticated explicit stand-down — stop the camera.
    StandDown,
    /// Liveness probe — report the current status, never touch capture.
    Report,
    /// Invalid token or malformed/empty request — report "unknown",
    /// never touch capture.
    Reject,
}

fn classify(request: &Option<Request>, expected: &str) -> RequestAction {
    let Some(request) = request else {
        return RequestAction::Reject;
    };
    if request.token != expected {
        return RequestAction::Reject;
    }
    if request.probe {
        return RequestAction::Report;
    }
    if request.active {
        RequestAction::Poll
    } else {
        RequestAction::StandDown
    }
}

fn handle(mut stream: TcpStream, expected: &str, capture: &mut Capture) {
    let mut request = String::new();
    let request = BufReader::new(&mut stream)
        .read_line(&mut request)
        .ok()
        .and_then(|_| serde_json::from_str::<Request>(&request).ok());
    // Only an authenticated, explicit stand-down stops the camera.
    // Probes, invalid tokens, and empty requests only report status.
    let signal = match classify(&request, expected) {
        RequestAction::Poll => Signal {
            status: capture.poll(clock::continuous_ms()),
            sampled_at_ms: clock::continuous_ms(),
        },
        RequestAction::StandDown => {
            capture.stop();
            Signal {
                status: "off".into(),
                sampled_at_ms: clock::continuous_ms(),
            }
        }
        RequestAction::Report => Signal {
            status: capture.status(),
            sampled_at_ms: clock::continuous_ms(),
        },
        RequestAction::Reject => Signal {
            status: "unknown".into(),
            sampled_at_ms: clock::continuous_ms(),
        },
    };
    if let Ok(body) = serde_json::to_string(&signal) {
        let _ = writeln!(stream, "{body}");
    }
}

pub fn run() -> i32 {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let dir = match data_dir() {
        Ok(dir) => dir,
        Err(_) => return 1,
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return 1;
    }
    let lock = match File::create(dir.join(LOCK)) {
        Ok(lock) => lock,
        Err(_) => return 1,
    };
    if lock.try_lock_exclusive().is_err() {
        return 0;
    }
    let expected = match token(&dir) {
        Ok(token) => token,
        Err(_) => return 1,
    };
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => listener,
        Err(_) => return 1,
    };
    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(_) => return 1,
    };
    if std::fs::write(dir.join(PORT), port.to_string()).is_err() {
        return 1;
    }
    let mut capture = Capture::default();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle(stream, &expected, &mut capture),
            Err(error) => log::warn!("Attention agent connection failed: {error}"),
        }
    }
    capture.stop();
    0
}

#[cfg(target_os = "macos")]
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/com.preelock.desktop"))
        .ok_or("HOME is unavailable.".into())
}

#[cfg(target_os = "windows")]
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(|appdata| PathBuf::from(appdata).join("com.preelock.desktop"))
        .ok_or("APPDATA is unavailable.".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn data_dir() -> Result<PathBuf, String> {
    Err("Attention agent is supported on macOS and Windows only.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn authenticated_ipc_rejects_wrong_token() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(TOKEN), "expected").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(dir.path().join(PORT), port.to_string()).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut capture = Capture::default();
            handle(stream, "expected", &mut capture);
        });
        std::fs::write(dir.path().join(TOKEN), "wrong").unwrap();
        let response = signal(dir.path(), false).unwrap();
        worker.join().unwrap();
        assert_eq!(response.status, "unknown");
    }

    #[test]
    fn authenticated_ipc_reports_inactive_without_starting_camera() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(TOKEN), "expected").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(dir.path().join(PORT), port.to_string()).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut capture = Capture::default();
            handle(stream, "expected", &mut capture);
        });
        let response = signal(dir.path(), false).unwrap();
        worker.join().unwrap();
        assert_eq!(response.status, "off");
        // The timestamp must be fresh: on Linux the test clock counts
        // from process start, so it can legitimately be 0 in the first
        // millisecond and `> 0` is racy there.
        assert!(response.fresh(clock::continuous_ms()));
    }

    #[test]
    fn stale_signal_is_not_fresh() {
        let signal = Signal {
            status: "person_detected".into(),
            sampled_at_ms: 1_000,
        };
        assert!(!signal.fresh(4_001));
        assert!(!signal.fresh(999));
    }

    #[test]
    fn probe_is_report_only_and_never_polls_or_stops() {
        let probe = Some(Request {
            token: "expected".into(),
            active: false,
            probe: true,
        });
        assert_eq!(classify(&probe, "expected"), RequestAction::Report);
        // A probe wins over `active`: even a probe sent with active=true
        // must not start the camera.
        let probe_active = Some(Request {
            token: "expected".into(),
            active: true,
            probe: true,
        });
        assert_eq!(classify(&probe_active, "expected"), RequestAction::Report);
    }

    #[test]
    fn only_an_authenticated_explicit_stand_down_stops_the_camera() {
        let stand_down = Some(Request {
            token: "expected".into(),
            active: false,
            probe: false,
        });
        assert_eq!(classify(&stand_down, "expected"), RequestAction::StandDown);
        let poll = Some(Request {
            token: "expected".into(),
            active: true,
            probe: false,
        });
        assert_eq!(classify(&poll, "expected"), RequestAction::Poll);
    }

    #[test]
    fn invalid_token_and_empty_requests_are_rejected_without_action() {
        let wrong_token = Some(Request {
            token: "wrong".into(),
            active: false,
            probe: false,
        });
        assert_eq!(classify(&wrong_token, "expected"), RequestAction::Reject);
        let wrong_token_probe = Some(Request {
            token: "wrong".into(),
            active: false,
            probe: true,
        });
        assert_eq!(
            classify(&wrong_token_probe, "expected"),
            RequestAction::Reject
        );
        assert_eq!(classify(&None, "expected"), RequestAction::Reject);
    }

    #[test]
    fn probe_reports_status_without_touching_capture() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(TOKEN), "expected").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(dir.path().join(PORT), port.to_string()).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut capture = Capture::default();
            handle(stream, "expected", &mut capture);
        });
        let response = probe(dir.path()).unwrap();
        worker.join().unwrap();
        assert_eq!(response.status, "off");
        assert!(response.fresh(clock::continuous_ms()));
    }

    #[test]
    fn probe_with_wrong_token_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(TOKEN), "expected").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(dir.path().join(PORT), port.to_string()).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut capture = Capture::default();
            handle(stream, "expected", &mut capture);
        });
        std::fs::write(dir.path().join(TOKEN), "wrong").unwrap();
        let response = probe(dir.path()).unwrap();
        worker.join().unwrap();
        assert_eq!(response.status, "unknown");
    }
}
