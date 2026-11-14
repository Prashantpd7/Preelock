//! Authenticated loopback IPC between the thin desktop GUI / CLI
//! probe and the single authoritative `--background` Engine owner.
//!
//! Security model:
//! - Loopback only (`127.0.0.1:0`, ephemeral port per data directory).
//! - Requires per-user 256-bit secret bearer token stored in
//!   `data_dir/bg-ipc.json` (`0600` on Unix).
//! - Rejects any request carrying an `Origin` header or non-matching
//!   `Host` header so web pages cannot probe or invoke it via CORS/DNS
//!   rebinding.

use crate::models::{Action, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

pub const ENDPOINT_FILE: &str = "bg-ipc.json";
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum IpcRequest {
    Ping,
    UiHeartbeat { pid: u32 },
    UiExiting { pid: u32 },
    Snapshot,
    Execute { action: Action },
    BlockAttempts,
    ExportStatistics,
    DeleteStatistics,
    Backup,
    ReplaceData { data: Snapshot },
    TestCamera,
    StopTest,
    ResetBrowserPairing,
    TestBrowserConnection,
    BrowserTestResult,
    DismissBlockedAppPage,
    BrowserSetupInfo { browser_id: String },
    NoteFrozenOverlayFailure,
    CheckUninstall,
    RefreshBrowserConnection,
    TestNotification { title: String, body: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PingInfo {
    pub pid: u32,
    pub has_active: bool,
    pub has_active_strict: bool,
    pub recovered_after_os_shutdown: bool,
    pub notification_prefs: (bool, bool, u32),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserSetupInfo {
    pub browser: crate::browser::BrowserStatus,
    pub connected_browsers: Vec<String>,
    pub pairing_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UninstallCheck {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcResponse {
    pub ok: bool,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl IpcResponse {
    pub fn success<T: Serialize>(value: &T) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => Self {
                ok: true,
                payload: Some(v),
                error: None,
            },
            Err(e) => Self::err(e.to_string()),
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            payload: None,
            error: Some(msg.into()),
        }
    }
}

pub type ServerMessage = (IpcRequest, mpsc::Sender<IpcResponse>);

pub struct IpcServer {
    data_dir: PathBuf,
    endpoint: Endpoint,
    server: Arc<tiny_http::Server>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(data_dir: &Path, tx: mpsc::Sender<ServerMessage>) -> Result<Self, String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let server = Arc::new(
            tiny_http::Server::http(("127.0.0.1", 0))
                .map_err(|e| format!("Could not start background IPC server: {e}"))?,
        );
        let port = server
            .server_addr()
            .to_ip()
            .ok_or("Missing IPC server socket address.")?
            .port();
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let endpoint = Endpoint {
            port,
            token: token.clone(),
            pid: std::process::id(),
            updated_ms: crate::clock::now(),
        };
        write_endpoint_atomic(&data_dir.join(ENDPOINT_FILE), &endpoint)?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let srv = server.clone();
        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                let Ok(Some(mut req)) = srv.recv_timeout(Duration::from_millis(250)) else {
                    continue;
                };
                if !request_authorized(&req, port, &token) {
                    let _ = req.respond(tiny_http::Response::empty(403));
                    continue;
                }
                let Some(len) = req.body_length() else {
                    let _ = req.respond(tiny_http::Response::empty(400));
                    continue;
                };
                if len > MAX_BODY_BYTES {
                    let _ = req.respond(tiny_http::Response::empty(413));
                    continue;
                }
                let mut body = Vec::with_capacity(len.min(64 * 1024));
                if req
                    .as_reader()
                    .take((MAX_BODY_BYTES + 1) as u64)
                    .read_to_end(&mut body)
                    .is_err()
                {
                    let _ = req.respond(tiny_http::Response::empty(400));
                    continue;
                }
                let Ok(parsed) = serde_json::from_slice::<IpcRequest>(&body) else {
                    let _ = req.respond(tiny_http::Response::empty(400));
                    continue;
                };
                let (reply_tx, reply_rx) = mpsc::channel();
                if tx.send((parsed, reply_tx)).is_err() {
                    let resp =
                        serde_json::to_vec(&IpcResponse::err("Background engine shutting down."))
                            .unwrap_or_default();
                    let _ = req.respond(json_response(503, resp));
                    continue;
                }
                let ipc_resp = match reply_rx.recv_timeout(IO_TIMEOUT) {
                    Ok(r) => r,
                    Err(_) => IpcResponse::err("Background engine timed out handling request."),
                };
                let bytes = serde_json::to_vec(&ipc_resp).unwrap_or_default();
                let _ = req.respond(json_response(200, bytes));
            }
        });

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            endpoint,
            server,
            stop,
            handle: Some(handle),
        })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.server.unblock();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let path = self.data_dir.join(ENDPOINT_FILE);
        if let Some(current) = read_endpoint(&path) {
            if current.pid == self.endpoint.pid && current.port == self.endpoint.port {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn json_response(status: u16, body: Vec<u8>) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let len = body.len();
    let mut res = tiny_http::Response::from_data(body).with_status_code(status);
    if let Ok(h) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
        res.add_header(h);
    }
    if let Ok(h) = tiny_http::Header::from_bytes("Content-Length", len.to_string()) {
        res.add_header(h);
    }
    res
}

fn header_value<'a>(req: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

fn request_authorized(req: &tiny_http::Request, port: u16, token: &str) -> bool {
    if req.method() != &tiny_http::Method::Post || req.url() != "/rpc" {
        return false;
    }
    if header_value(req, "Origin").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    if header_value(req, "Host") != Some(format!("127.0.0.1:{port}").as_str()) {
        return false;
    }
    header_value(req, "Authorization") == Some(format!("Bearer {token}").as_str())
}

fn write_endpoint_atomic(path: &Path, endpoint: &Endpoint) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid IPC endpoint directory.")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(endpoint).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".bg-ipc.{}.tmp", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
        }
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res.map_err(|e| e.to_string())
}

pub fn read_endpoint(path: &Path) -> Option<Endpoint> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn clear_endpoint(data_dir: &Path) {
    let _ = fs::remove_file(data_dir.join(ENDPOINT_FILE));
}

#[derive(Clone, Debug)]
pub struct IpcClient {
    data_dir: PathBuf,
}

impl IpcClient {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn wait_ready(&self, timeout: Duration) -> Result<PingInfo, String> {
        let start = std::time::Instant::now();
        let mut prompted = false;
        loop {
            match self.ping() {
                Ok(info) => return Ok(info),
                Err(e) if start.elapsed() >= timeout => return Err(e),
                Err(_) => {
                    if !prompted && start.elapsed() >= Duration::from_millis(500) {
                        prompted = true;
                        let _ = crate::background::ensure_running_headless(&self.data_dir);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn call_raw(&self, req: &IpcRequest) -> Result<IpcResponse, String> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let ep = read_endpoint(&self.data_dir.join(ENDPOINT_FILE))
                .ok_or_else(|| "Background engine IPC endpoint is not available.".to_string());
            match ep.and_then(|endpoint| send_http_rpc(&endpoint, req)) {
                Ok(resp) => return Ok(resp),
                Err(_err) if attempts < 12 => {
                    // Self-heal if background died unexpectedly between calls.
                    let _ = crate::background::ensure_running_headless(&self.data_dir);
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn call_typed<T: serde::de::DeserializeOwned>(&self, req: &IpcRequest) -> Result<T, String> {
        let resp = self.call_raw(req)?;
        if !resp.ok {
            return Err(resp
                .error
                .unwrap_or_else(|| "Background request failed.".into()));
        }
        let val = resp.payload.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(val).map_err(|e| e.to_string())
    }

    fn call_unit(&self, req: &IpcRequest) -> Result<(), String> {
        let resp = self.call_raw(req)?;
        if !resp.ok {
            return Err(resp
                .error
                .unwrap_or_else(|| "Background request failed.".into()));
        }
        Ok(())
    }

    pub fn ping(&self) -> Result<PingInfo, String> {
        let ep = read_endpoint(&self.data_dir.join(ENDPOINT_FILE))
            .ok_or_else(|| "Background engine IPC endpoint is not available.".to_string())?;
        let resp = send_http_rpc(&ep, &IpcRequest::Ping)?;
        if !resp.ok {
            return Err(resp.error.unwrap_or_else(|| "Ping failed.".into()));
        }
        serde_json::from_value(resp.payload.unwrap_or(serde_json::Value::Null))
            .map_err(|e| e.to_string())
    }

    pub fn ui_heartbeat(&self) -> Result<PingInfo, String> {
        self.call_typed(&IpcRequest::UiHeartbeat {
            pid: std::process::id(),
        })
    }

    pub fn ui_exiting(&self) {
        if let Some(ep) = read_endpoint(&self.data_dir.join(ENDPOINT_FILE)) {
            let _ = send_http_rpc(
                &ep,
                &IpcRequest::UiExiting {
                    pid: std::process::id(),
                },
            );
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        self.call_typed(&IpcRequest::Snapshot)
    }

    pub fn execute(&self, action: Action) -> Result<Snapshot, String> {
        self.call_typed(&IpcRequest::Execute { action })
    }

    pub fn block_attempts(&self) -> Result<Vec<crate::stats::BlockAttempt>, String> {
        self.call_typed(&IpcRequest::BlockAttempts)
    }

    pub fn export_statistics(&self) -> Result<String, String> {
        self.call_typed(&IpcRequest::ExportStatistics)
    }

    pub fn delete_statistics(&self) -> Result<u64, String> {
        self.call_typed(&IpcRequest::DeleteStatistics)
    }

    pub fn backup(&self) -> Result<Snapshot, String> {
        self.call_typed(&IpcRequest::Backup)
    }

    pub fn replace_data(&self, data: Snapshot) -> Result<(), String> {
        self.call_unit(&IpcRequest::ReplaceData { data })
    }

    pub fn test_camera(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::TestCamera)
    }

    pub fn stop_test(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::StopTest)
    }

    pub fn reset_browser_pairing(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::ResetBrowserPairing)
    }

    pub fn test_browser_connection(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::TestBrowserConnection)
    }

    pub fn browser_test_result(&self) -> Result<String, String> {
        self.call_typed(&IpcRequest::BrowserTestResult)
    }

    pub fn dismiss_blocked_app_page(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::DismissBlockedAppPage)
    }

    pub fn browser_setup_info(&self, browser_id: String) -> Result<BrowserSetupInfo, String> {
        self.call_typed(&IpcRequest::BrowserSetupInfo { browser_id })
    }

    pub fn note_frozen_overlay_failure(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::NoteFrozenOverlayFailure)
    }

    pub fn refresh_browser_connection(&self) -> Result<(), String> {
        self.call_unit(&IpcRequest::RefreshBrowserConnection)
    }

    pub fn test_notification(&self, title: &str, body: &str) -> Result<(), String> {
        self.call_unit(&IpcRequest::TestNotification {
            title: title.to_string(),
            body: body.to_string(),
        })
    }

    pub fn check_uninstall(&self) -> Result<UninstallCheck, String> {
        let ep = read_endpoint(&self.data_dir.join(ENDPOINT_FILE))
            .ok_or_else(|| "Background engine IPC endpoint is not available.".to_string())?;
        let resp = send_http_rpc(&ep, &IpcRequest::CheckUninstall)?;
        if !resp.ok {
            return Err(resp.error.unwrap_or_else(|| "Check failed.".into()));
        }
        serde_json::from_value(resp.payload.unwrap_or(serde_json::Value::Null))
            .map_err(|e| e.to_string())
    }
}

fn send_http_rpc(endpoint: &Endpoint, req: &IpcRequest) -> Result<IpcResponse, String> {
    let addr: SocketAddr = format!("127.0.0.1:{}", endpoint.port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("Could not connect to background engine: {e}"))?;
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
    let header = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.port,
        endpoint.token,
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("IPC write failed: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("IPC read failed: {e}"))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("Malformed IPC HTTP response.")?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| "Invalid IPC header encoding.")?;
    let status_line = head.lines().next().unwrap_or("");
    let code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if code != 200 {
        return Err(format!("Background IPC rejected request (HTTP {code})."));
    }
    let is_chunked = head.lines().any(|line| {
        if let Some((name, val)) = line.split_once(':') {
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && val.to_ascii_lowercase().contains("chunked")
        } else {
            false
        }
    });
    let payload_bytes = if is_chunked {
        decode_chunked_body(&raw[split + 4..])?
    } else {
        raw[split + 4..].to_vec()
    };
    serde_json::from_slice::<IpcResponse>(&payload_bytes).map_err(|e| e.to_string())
}

fn decode_chunked_body(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < raw.len() {
        let Some(rel_crlf) = raw[pos..].windows(2).position(|w| w == b"\r\n") else {
            return Err("Truncated chunked IPC response header.".into());
        };
        let size_line = std::str::from_utf8(&raw[pos..pos + rel_crlf])
            .map_err(|_| "Invalid chunk size encoding.")?;
        let hex = size_line.split(';').next().unwrap_or("").trim();
        let size =
            usize::from_str_radix(hex, 16).map_err(|_| format!("Invalid chunk size '{hex}'."))?;
        pos += rel_crlf + 2;
        if size == 0 {
            return Ok(out);
        }
        if pos + size > raw.len() {
            return Err("Truncated chunked IPC response body.".into());
        }
        out.extend_from_slice(&raw[pos..pos + size]);
        pos += size;
        if raw.get(pos..pos + 2) == Some(b"\r\n") {
            pos += 2;
        }
    }
    Err("Incomplete chunked IPC response.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_rejects_unauthenticated_or_browser_origin_requests() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel::<ServerMessage>();
        let server = IpcServer::start(dir.path(), tx).unwrap();
        let ep = server.endpoint().clone();

        // Spawn tiny dummy responder.
        let responder = std::thread::spawn(move || {
            while let Ok((req, reply)) = rx.recv() {
                match req {
                    IpcRequest::Ping => {
                        let _ = reply.send(IpcResponse::success(&PingInfo {
                            pid: 42,
                            has_active: true,
                            has_active_strict: true,
                            recovered_after_os_shutdown: false,
                            notification_prefs: (true, true, 50),
                        }));
                    }
                    IpcRequest::CheckUninstall => {
                        let _ = reply.send(IpcResponse::success(&UninstallCheck {
                            allowed: false,
                            reason: "Strict focus session active.".into(),
                        }));
                    }
                    _ => {
                        let _ = reply.send(IpcResponse::err("unused"));
                    }
                }
            }
        });

        // 1. Valid client succeeds.
        let client = IpcClient::new(dir.path());
        let ping = client.ping().unwrap();
        assert_eq!(ping.pid, 42);
        assert!(ping.has_active_strict);
        let check = client.check_uninstall().unwrap();
        assert!(!check.allowed);

        // 2. Wrong bearer token -> HTTP 403.
        let wrong_ep = Endpoint {
            port: ep.port,
            token: "wrong-token".into(),
            pid: ep.pid,
            updated_ms: ep.updated_ms,
        };
        let err = send_http_rpc(&wrong_ep, &IpcRequest::Ping).unwrap_err();
        assert!(err.contains("403"));

        // 3. Browser Origin header -> HTTP 403.
        let mut stream = TcpStream::connect(("127.0.0.1", ep.port)).unwrap();
        let body = serde_json::to_vec(&IpcRequest::Ping).unwrap();
        let req = format!(
            "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: https://evil.example\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            ep.port, ep.token, body.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 403"));

        drop(server);
        assert!(!dir.path().join(ENDPOINT_FILE).exists());
        let _ = responder.join();
    }

    #[test]
    fn chunked_body_decoding_handles_single_and_multi_chunks() {
        let raw = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let decoded = decode_chunked_body(raw).unwrap();
        assert_eq!(decoded, b"hello world");

        let raw_empty = b"0\r\n\r\n";
        let decoded_empty = decode_chunked_body(raw_empty).unwrap();
        assert!(decoded_empty.is_empty());
    }
}
