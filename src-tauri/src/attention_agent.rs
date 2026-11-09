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
    signal(dir, false).is_ok()
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
    let token = token(dir)?;
    let mut stream = TcpStream::connect(("127.0.0.1", port(dir)?))
        .map_err(|e| format!("Attention agent unavailable: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let request = serde_json::to_string(&Request { token, active }).map_err(|e| e.to_string())?;
    writeln!(stream, "{request}").map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&response).map_err(|e| format!("Invalid attention signal: {e}"))
}

fn handle(mut stream: TcpStream, expected: &str, capture: &mut Capture) {
    let mut request = String::new();
    let request = BufReader::new(&mut stream)
        .read_line(&mut request)
        .ok()
        .and_then(|_| serde_json::from_str::<Request>(&request).ok());
    let valid = request
        .as_ref()
        .is_some_and(|request| request.token == expected);
    let signal = if valid && request.as_ref().is_some_and(|request| request.active) {
        Signal {
            status: capture.poll(clock::continuous_ms()),
            sampled_at_ms: clock::continuous_ms(),
        }
    } else {
        capture.stop();
        Signal {
            status: if valid {
                "off".into()
            } else {
                "unknown".into()
            },
            sampled_at_ms: clock::continuous_ms(),
        }
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
        assert!(response.sampled_at_ms > 0);
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
}
