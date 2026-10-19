// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback-only desktop shell. Never bind 0.0.0.0. Bodies/OTP/PIN are not
//! written to logs. Full Tauri packaging is the next desktop-shell iteration.

use crate::runtime::Node;
use serde::Serialize;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

const INDEX: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/index.html"));
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 2 * 1024 * 1024;

/// Bind the loopback UI and serve it in the background, returning the port
/// actually bound (a `0` request resolves to an ephemeral one). Binding
/// synchronously means a taken port is a hard error the caller can report,
/// rather than a `warn!` that leaves the UI silently absent.
pub async fn bind_and_spawn(node: Node, port: u16) -> std::io::Result<u16> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?.port();
    info!(port = bound, "ui listening (loopback only)");
    tokio::spawn(async move {
        if let Err(e) = accept_loop(listener, node).await {
            warn!(%e, "ui http");
        }
    });
    Ok(bound)
}

async fn accept_loop(listener: TcpListener, node: Node) -> std::io::Result<()> {
    let mut stop = node.shutdown_signal();
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            res = listener.accept() => {
                let (stream, peer) = res?;
                if !peer.ip().is_loopback() {
                    continue;
                }
                let node = node.clone_handle();
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, node).await {
                        warn!(%e, "ui request");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle(mut stream: TcpStream, node: Node) -> std::io::Result<()> {
    let mut buf = vec![0u8; 4096];
    let mut n = 0usize;
    loop {
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if n >= MAX_HEADER {
            write_http(&mut stream, 413, "text/plain", b"headers too large").await?;
            return Ok(());
        }
        if n >= buf.len() {
            buf.resize((buf.len() * 2).min(MAX_HEADER), 0);
        }
        let k = stream.read(&mut buf[n..]).await?;
        if k == 0 {
            return Ok(());
        }
        n += k;
    }
    let head_end = buf[..n]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(n);
    let header = String::from_utf8_lossy(&buf[..head_end]);
    let mut lines = header.split("\r\n");
    let req = lines.next().unwrap_or("");
    let mut parts = req.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    let mut content_len = 0usize;
    let mut host_ok = false;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_len = v.trim().parse().unwrap_or(0);
        }
        if let Some(v) = lower.strip_prefix("host:") {
            let h = v.trim();
            host_ok = h.starts_with("127.0.0.1") || h.starts_with("localhost");
        }
    }
    if !host_ok {
        write_http(&mut stream, 403, "text/plain", b"loopback only").await?;
        return Ok(());
    }
    if method == "GET"
        && (path == "/api/events"
            || path == "/api/events/stream"
            || path.starts_with("/api/events?")
            || path.starts_with("/api/events/stream?"))
    {
        return handle_sse(&mut stream, &node).await;
    }
    if content_len > MAX_BODY {
        write_http(&mut stream, 413, "text/plain", b"too large").await?;
        return Ok(());
    }
    let mut body = buf[head_end..n].to_vec();
    while body.len() < content_len {
        let to_read = (content_len - body.len()).min(64 * 1024);
        let mut more = vec![0u8; to_read];
        let k = stream.read(&mut more).await?;
        if k == 0 {
            break;
        }
        body.extend_from_slice(&more[..k]);
    }
    body.truncate(content_len);
    let resp = route(method, path, &body, &node).await;
    write_http_response(&mut stream, &resp).await
}

/// Structured HTTP response for the loopback UI server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiResponse {
    pub code: u16,
    pub ctype: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl UiResponse {
    pub fn new(code: u16, ctype: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            code,
            ctype: ctype.into(),
            headers: Vec::new(),
            body,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

impl From<(u16, &'static str, Vec<u8>)> for UiResponse {
    fn from((code, ctype, body): (u16, &'static str, Vec<u8>)) -> Self {
        let content_type = if ctype.contains("charset") {
            ctype.to_string()
        } else if ctype.starts_with("text/") || ctype == "application/json" {
            format!("{ctype}; charset=utf-8")
        } else {
            ctype.to_string()
        };
        UiResponse::new(code, content_type, body)
    }
}

async fn write_http_response(stream: &mut TcpStream, resp: &UiResponse) -> std::io::Result<()> {
    let reason = match resp.code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
        resp.code,
        reason,
        resp.ctype,
        resp.body.len()
    );
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&resp.body).await?;
    stream.flush().await
}

async fn write_http(
    stream: &mut TcpStream,
    code: u16,
    ctype: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let content_type = if ctype.contains("charset") {
        ctype.to_string()
    } else if ctype.starts_with("text/") || ctype == "application/json" {
        format!("{ctype}; charset=utf-8")
    } else {
        ctype.to_string()
    };
    let resp = UiResponse::new(code, content_type, body.to_vec());
    write_http_response(stream, &resp).await
}

/// Binary-safe parser for raw HTTP responses.
fn parse_http_response(buf: &[u8]) -> (u16, Vec<u8>) {
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let (head_bytes, body_bytes) = match header_end {
        Some(pos) => (&buf[..pos], &buf[pos + 4..]),
        None => (buf, &[][..]),
    };
    let head_text = String::from_utf8_lossy(head_bytes);
    let code = head_text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (code, body_bytes.to_vec())
}

/// POST a small JSON body to a node's own loopback UI API and return the HTTP
/// status with the response body. The CLI uses this so `tetherly pin` arms the
/// *running* daemon: a freshly started process would print a code no listener
/// can honour. Loopback only, and the response body is the sole place a PIN is
/// ever surfaced (never logged).
pub async fn loopback_post(port: u16, path: &str, body: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(parse_http_response(&buf))
}

/// PUT a JSON payload to a node's loopback UI API and return the HTTP status
/// with the response body.
pub async fn loopback_put(port: u16, path: &str, body: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!(
        "PUT {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(parse_http_response(&buf))
}

/// GET a path from a node's loopback UI API and return the HTTP status with
/// the response body.
pub async fn loopback_get(port: u16, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(parse_http_response(&buf))
}

/// DELETE a path from a node's loopback UI API and return the HTTP status with
/// the response body.
pub async fn loopback_delete(port: u16, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!("DELETE {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(parse_http_response(&buf))
}

async fn handle_sse(stream: &mut TcpStream, node: &Node) -> std::io::Result<()> {
    let mut events = node.subscribe();
    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: text/event-stream; charset=utf-8\r\n\
                  Cache-Control: no-cache, no-transform\r\n\
                  Connection: keep-alive\r\n\
                  Access-Control-Allow-Origin: *\r\n\r\n";
    stream.write_all(header.as_bytes()).await?;
    stream.flush().await?;

    // Initial greeting comment so client knows stream is established
    stream.write_all(b": connected\n\n").await?;
    stream.flush().await?;

    let mut shutdown = node.shutdown_signal();
    let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(15));
    // Consume initial tick that fires immediately
    ping_interval.tick().await;

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            res = events.recv() => {
                match res {
                    Ok(ev) => {
                        if let Ok(json) = serde_json::to_string(&ev) {
                            let payload = format!("data: {json}\n\n");
                            if stream.write_all(payload.as_bytes()).await.is_err() {
                                break;
                            }
                            if stream.flush().await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        let payload = format!(": lagged {n}\n\n");
                        if stream.write_all(payload.as_bytes()).await.is_err() {
                            break;
                        }
                        if stream.flush().await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            _ = ping_interval.tick() => {
                if stream.write_all(b": ping\n\n").await.is_err() {
                    break;
                }
                if stream.flush().await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// An active Server-Sent Events stream connected to a loopback UI endpoint.
pub struct SseStream {
    lines: Lines<BufReader<TcpStream>>,
}

impl SseStream {
    /// Read the next event data string from the stream.
    /// Comments (lines starting with ':') and blank separator lines are skipped.
    /// Returns `Ok(Some(data))` when a `data: ...` event arrives,
    /// `Ok(None)` when the stream is closed, or `Err` on I/O error.
    pub async fn next_event(&mut self) -> std::io::Result<Option<String>> {
        while let Some(line) = self.lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(':') {
                continue;
            }
            if let Some(payload) = trimmed.strip_prefix("data:") {
                return Ok(Some(payload.trim().to_string()));
            }
        }
        Ok(None)
    }

    /// Read the next typed `UiEvent` from the stream.
    pub async fn next_ui_event(&mut self) -> std::io::Result<Option<crate::events::UiEvent>> {
        while let Some(raw) = self.next_event().await? {
            if let Ok(ev) = serde_json::from_str::<crate::events::UiEvent>(&raw) {
                return Ok(Some(ev));
            }
        }
        Ok(None)
    }
}

/// Connect to a node's loopback UI API and initiate an SSE stream on `/api/events`.
pub async fn loopback_events_connect(port: u16) -> std::io::Result<SseStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let req = "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n";
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut lines = BufReader::new(stream).lines();
    let status_line = lines.next_line().await?.unwrap_or_default();
    if !status_line.contains("200") {
        return Err(std::io::Error::other(format!(
            "SSE HTTP handshake failed: {status_line}"
        )));
    }
    while let Some(header) = lines.next_line().await? {
        if header.trim().is_empty() {
            break;
        }
    }
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.starts_with(':') {
            break;
        }
    }
    Ok(SseStream { lines })
}

#[derive(Serialize)]
struct Snapshot {
    device_id: String,
    name: String,
    platform: String,
    pin_active: bool,
    lan_only: bool,
    overlay_present: bool,
    overlay_source: String,
    overlay_peers: usize,
    candidates: Vec<crate::events::CandidateViewDto>,
    /// Inbound quick replies awaiting hand-off. No text is included.
    replies: Vec<crate::events::ReplyViewDto>,
    peers: Vec<PeerView>,
    trusted: Vec<TrustedPeerView>,
    files: Vec<FileSnap>,
    transfers: Vec<crate::runtime::ActiveTransfer>,
    transfer_stats: crate::runtime::TransferStats,
    /// "off" when no BLE transport is attached, else the ingress state.
    ancs_state: String,
    /// Allowlisted app ids, so the wizard can show what may be opened.
    open_apps: Vec<String>,
    /// Remote-screen channel state (idle/requested/allowed/streaming).
    screen_state: String,
    /// Screen counters for the local UI. Never carries pixels.
    screen: ScreenSnap,
    /// Input channel state: whether 45719 is dialled and where the seat is.
    input_channel: bool,
    input_seat: String,
    /// Every local address and why it is or is not in the LAN listen set. Lets
    /// the UI explain "nothing found" without the user reading logs.
    lan: Vec<crate::lan::IfaceReport>,
    /// Aggregate network traffic and protocol telemetry report.
    traffic: crate::telemetry::TrafficReport,
}

#[derive(Serialize)]
struct ScreenSnap {
    state: String,
    sent: u64,
    received: u64,
    dropped: u64,
    bytes: u64,
    refused: u64,
    /// p95 host-side frame interval in microseconds. Null until measurable.
    p95_frame_interval_us: Option<u64>,
}

#[derive(Serialize)]
struct PeerView {
    device_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    platform: String,
    path: String,
    caps: Vec<String>,
    features: Vec<String>,
    file_port: Option<u16>,
    input_port: Option<u16>,
    screen_port: Option<u16>,
    rtt_ms: Option<u64>,
    last_seen_ms: u64,
}

#[derive(Serialize)]
pub struct TrustedPeerView {
    pub device_id: String,
    pub alias: String,
    pub paired_at: u64,
    pub revoked: bool,
    pub online: bool,
}

#[derive(Serialize)]
struct FileSnap {
    transfer_id: String,
    source: String,
    name: String,
    size: u64,
}

#[derive(serde::Deserialize)]
struct DialBody {
    #[serde(default)]
    addr: Option<String>,
    #[serde(default)]
    peer: Option<String>,
    pin: Option<String>,
}

#[derive(serde::Deserialize)]
struct NotifyBody {
    title: String,
    body: String,
    #[serde(default = "default_notify_app")]
    app_id: String,
    peer: Option<String>,
}

fn default_notify_app() -> String {
    "com.example.sms".into()
}

/// Wall clock for a locally originated push. The node clock is the handshake
/// clock; a notify timestamp only has to be monotone enough for the peer's TTL.
fn node_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(serde::Deserialize)]
struct IdBody {
    id: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct FileDecisionBody {
    id: Option<String>,
    peer: Option<String>,
    all: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct AutoAcceptBody {
    enabled: Option<bool>,
    auto_accept_trusted: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct NotifyDismissBody {
    id: Option<String>,
    peer: Option<String>,
    app_id: Option<String>,
    all: Option<bool>,
}

#[derive(serde::Deserialize)]
struct UnpairBody {
    device_id: String,
}

#[derive(serde::Deserialize)]
struct ReplyBody {
    id: String,
    text: String,
}

#[derive(serde::Deserialize)]
struct OpenRuleBody {
    app_id: String,
    #[serde(default = "default_open_url")]
    url: String,
}

#[derive(serde::Deserialize, Default)]
struct OpenRuleRemoveBody {
    #[serde(default)]
    app_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct StorageCleanBody {
    #[serde(default)]
    orphaned_only: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct InboxDeleteBody {
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct InboxClearBody {
    #[serde(default)]
    before_secs: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
struct InboxOpenBody {
    #[serde(default)]
    name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct NotifyConfigUpdateBody {
    banners_enabled: Option<bool>,
    dnd_mode: Option<crate::runtime::DndMode>,
    sound_enabled: Option<bool>,
    mask_otp: Option<bool>,
    allow_otp_in_dnd: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct NotifyTestBody {
    title: Option<String>,
    body: Option<String>,
}

fn default_open_url() -> String {
    "tetherly://open".to_string()
}

#[derive(serde::Deserialize)]
struct SendFileBody {
    path: Option<String>,
    #[serde(default)]
    data_base64: Option<String>,
    peer: Option<String>,
    name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ClipSyncBody {
    peer: Option<String>,
    text: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerTargetBody {
    peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerConnectBody {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    addr: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerDisconnectBody {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerBlockBody {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerUnblockBody {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct TransfersQueryBody {
    id: Option<String>,
    active: Option<bool>,
    terminal: Option<bool>,
    direction: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerAliasBody {
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    alias: String,
}

#[derive(serde::Deserialize, Default)]
struct ConfigUpdateBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    advertise: Option<bool>,
    #[serde(default)]
    reconnect: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct RouteAddBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    addr: String,
}

fn percent_decode(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    let mut iter = s.as_bytes().iter().copied();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let h1 = iter.next();
            let h2 = iter.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = [c1, c2];
                if let Ok(val) = std::str::from_utf8(&hex_str)
                    .map_err(|_| ())
                    .and_then(|h| u8::from_str_radix(h, 16).map_err(|_| ()))
                {
                    bytes.push(val);
                    continue;
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
            continue;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn parse_query_map(q: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        } else {
            map.insert(percent_decode(pair), String::new());
        }
    }
    map
}

async fn route(method: &str, path: &str, body: &[u8], node: &Node) -> UiResponse {
    let (path_only, query_str) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    match (method, path_only) {
        ("GET", "/") | ("GET", "/index.html") => {
            UiResponse::new(200, "text/html; charset=utf-8", INDEX.as_bytes().to_vec())
        }
        ("GET", "/api/state") => json_ok(&snapshot(node).await),
        ("GET", "/api/config") => json_ok(&node.config_report()),
        ("POST", "/api/config") | ("PUT", "/api/config") => {
            match serde_json::from_slice::<ConfigUpdateBody>(body) {
                Ok(d) => {
                    if d.name.is_none() && d.advertise.is_none() && d.reconnect.is_none() {
                        return err(400, "no configuration fields to update");
                    }
                    match node.update_config(d.name.as_deref(), d.advertise, d.reconnect) {
                        Ok(report) => json_ok(&report),
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(400, "bad json"),
            }
        }
        ("GET", "/api/pin") => json_ok(&node.pin_report()),
        ("POST", "/api/pin") => match node.generate_pin() {
            Ok(pin) => {
                let rep = node.pin_report();
                json_ok(&serde_json::json!({
                    "ok": true,
                    "pin": pin,
                    "expires_in_secs": rep.expires_in_secs,
                    "ttl_secs": rep.ttl_secs
                }))
            }
            Err(e) => err(400, &e.to_string()),
        },
        ("POST", "/api/pin/clear") | ("DELETE", "/api/pin") => {
            let cleared = node.clear_pin();
            json_ok(&serde_json::json!({ "ok": true, "cleared": cleared }))
        }
        ("POST", "/api/dial") => match serde_json::from_slice::<DialBody>(body) {
            Ok(d) => {
                if let Some(p) = d.pin {
                    match tetherly_core::pin::parse_pin(&p) {
                        Ok(digits) => node.set_pin(digits),
                        Err(e) => return err(400, &e.to_string()),
                    }
                }
                let target = d
                    .addr
                    .as_deref()
                    .or(d.peer.as_deref())
                    .filter(|s| !s.trim().is_empty());
                let target = match target {
                    Some(t) => t,
                    None => return err(400, "missing addr or peer"),
                };
                match node.dial_peer(target).await {
                    Ok(resolved_addr) => json_ok(&serde_json::json!({
                        "ok": true,
                        "addr": resolved_addr.to_string(),
                    })),
                    Err(e) => err(400, &e.to_string()),
                }
            }
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/peers/unpair") => match serde_json::from_slice::<UnpairBody>(body) {
            Ok(d) => match node.resolve_peer(&d.device_id) {
                Ok(id) => {
                    let removed = node.unpair(&id).await;
                    json_ok(&serde_json::json!({
                        "ok": true,
                        "device_id": id.as_str(),
                        "unpaired": removed
                    }))
                }
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/peers/connect") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerConnectBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<PeerConnectBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => {
                        return err(400, "bad json, expected { peer: string, addr?: string }")
                    }
                }
            };
            let target_query = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .unwrap_or_default();
            let direct_addr = parsed_body
                .as_ref()
                .and_then(|b| b.addr.clone())
                .or_else(|| qmap.get("addr").cloned());

            if target_query.trim().is_empty() {
                return err(400, "peer identifier required (peer or device_id)");
            }
            match node
                .connect_peer(target_query.trim(), direct_addr.as_deref())
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/peers/disconnect") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerDisconnectBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<PeerDisconnectBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected { peer: string }"),
                }
            };
            let target_query = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .unwrap_or_default();

            if target_query.trim().is_empty() {
                return err(400, "peer identifier required (peer or device_id)");
            }
            match node.disconnect_peer(target_query.trim()).await {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/peers/blocked") | ("GET", "/api/blocked") => {
            json_ok(&node.blocked_peers_report())
        }
        ("POST", "/api/peers/block") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerBlockBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<PeerBlockBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => {
                        return err(400, "bad json, expected { peer: string, reason?: string }")
                    }
                }
            };
            let target_query = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .unwrap_or_default();
            let reason = parsed_body
                .as_ref()
                .and_then(|b| b.reason.clone())
                .or_else(|| qmap.get("reason").cloned());

            if target_query.trim().is_empty() {
                return err(400, "peer identifier required");
            }
            match node
                .block_peer(target_query.trim(), reason.as_deref())
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/peers/unblock")
        | ("DELETE", "/api/peers/block")
        | ("DELETE", "/api/peers/blocked") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerUnblockBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<PeerUnblockBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected { peer: string }"),
                }
            };
            let target_query = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .unwrap_or_default();

            if target_query.trim().is_empty() {
                return err(400, "peer identifier required");
            }
            match node.unblock_peer(target_query.trim()).await {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/peers/alias") | ("PUT", "/api/peers/alias") => {
            match serde_json::from_slice::<PeerAliasBody>(body) {
                Ok(d) => {
                    let target_query = d.device_id.or(d.peer).unwrap_or_default();
                    if target_query.trim().is_empty() {
                        return err(400, "missing device_id or peer query");
                    }
                    let id = match node.resolve_peer(&target_query) {
                        Ok(id) => id,
                        Err(tetherly_core::CoreError::PeerNotFound(_)) => {
                            return err(404, "peer not found in trust store");
                        }
                        Err(e) => return err(400, &e.to_string()),
                    };
                    match node.set_peer_alias(&id, &d.alias) {
                        Ok(true) => json_ok(&serde_json::json!({
                            "ok": true,
                            "device_id": id.as_str(),
                            "alias": d.alias.trim()
                        })),
                        Ok(false) => err(404, "peer not found in trust store"),
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(400, "bad json"),
            }
        }
        ("POST", "/api/peers/ping") => {
            let d: PeerTargetBody = if body.is_empty() {
                PeerTargetBody::default()
            } else {
                match serde_json::from_slice::<PeerTargetBody>(body) {
                    Ok(v) => v,
                    Err(_) => return err(400, "bad json"),
                }
            };
            let mut peers = node.live_peers();
            peers.sort_by_key(|p| p.device_id.to_string());
            let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                Some(raw) => match node.resolve_peer(raw) {
                    Ok(id) => {
                        if peers.iter().any(|p| p.device_id == id) {
                            id
                        } else {
                            return err(400, "peer is not connected");
                        }
                    }
                    Err(e) => return err(400, &format!("bad peer id: {e}")),
                },
                None => match peers.len() {
                    0 => return err(400, "no connected peer"),
                    1 => peers[0].device_id.clone(),
                    _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                },
            };
            match node.ping_peer(&target).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/peers") => {
            let live = node.live_peers();
            let trusted = node.trusted();
            let live_ids: std::collections::HashSet<tetherly_core::DeviceId> =
                live.iter().map(|p| p.device_id.clone()).collect();
            let trusted_map: std::collections::HashMap<tetherly_core::DeviceId, String> = trusted
                .iter()
                .map(|tp| (tp.device_id.clone(), tp.alias.clone()))
                .collect();

            let live_dtos: Vec<serde_json::Value> = live
                .into_iter()
                .map(|p| {
                    let path_str = match p.path {
                        tetherly_core::PathKind::Lan => "lan",
                        tetherly_core::PathKind::Overlay => "overlay",
                    };
                    let alias_opt = trusted_map.get(&p.device_id).cloned();
                    let features = crate::runtime::extract_features(&p.caps);
                    serde_json::json!({
                        "device_id": p.device_id.to_string(),
                        "name": p.name,
                        "alias": alias_opt,
                        "platform": p.platform,
                        "addr": p.addr.to_string(),
                        "path": path_str,
                        "file_port": p.file_port,
                        "input_port": p.input_port,
                        "screen_port": p.screen_port,
                        "caps": p.caps,
                        "features": features,
                        "online": true,
                        "rtt_ms": p.rtt_ms,
                        "last_seen_ms": p.last_seen_ms,
                    })
                })
                .collect();

            let trusted_dtos: Vec<serde_json::Value> = trusted
                .into_iter()
                .map(|tp| {
                    let online = live_ids.contains(&tp.device_id);
                    serde_json::json!({
                        "device_id": tp.device_id.to_string(),
                        "alias": tp.alias,
                        "paired_at": tp.paired_at,
                        "revoked": tp.revoked,
                        "online": online,
                    })
                })
                .collect();

            json_ok(&serde_json::json!({
                "ok": true,
                "self_id": node.identity().device_id().to_string(),
                "live_count": live_dtos.len(),
                "trusted_count": trusted_dtos.len(),
                "live": live_dtos,
                "trusted": trusted_dtos,
            }))
        }
        ("GET", "/api/discovery") | ("GET", "/api/peers/discovered") => {
            json_ok(&node.discovered_peers())
        }
        ("POST", "/api/discovery/rescan") | ("POST", "/api/peers/discover") => {
            json_ok(&node.rescan_discovery())
        }
        ("GET", "/api/caps") => json_ok(&node.local_caps_report()),
        ("POST", "/api/caps") | ("PUT", "/api/caps") => {
            #[derive(serde::Deserialize, Default)]
            struct CapsUpdateBody {
                #[serde(default)]
                enable: Vec<String>,
                #[serde(default)]
                disable: Vec<String>,
            }
            match serde_json::from_slice::<CapsUpdateBody>(body) {
                Ok(b) => {
                    if b.enable.is_empty() && b.disable.is_empty() {
                        return err(400, "no capability changes specified");
                    }
                    match node.update_local_caps(&b.enable, &b.disable).await {
                        Ok(rep) => json_ok(&rep),
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(
                    400,
                    "bad json, expected { enable?: string[], disable?: string[] }",
                ),
            }
        }
        ("GET", "/api/peers/caps") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let target_peer = qmap
                .get("peer")
                .or_else(|| qmap.get("device_id"))
                .map(|s| s.as_str());
            match node.peers_caps_report(target_peer) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/routes") => json_ok(&node.routes_report()),
        ("POST", "/api/routes") | ("PUT", "/api/routes") => {
            match serde_json::from_slice::<RouteAddBody>(body) {
                Ok(d) => {
                    let peer = d.peer.or(d.device_id).unwrap_or_default();
                    if peer.trim().is_empty() {
                        return err(400, "peer or device_id must be specified");
                    }
                    if d.addr.trim().is_empty() {
                        return err(400, "addr must be specified");
                    }
                    match node.add_route(&peer, &d.addr) {
                        Ok(rep) => json_ok(&rep),
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(400, "bad json, expected { peer: string, addr: string }"),
            }
        }
        ("DELETE", "/api/routes")
        | ("POST", "/api/routes/remove")
        | ("POST", "/api/routes/delete") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let peer_from_query = qmap.get("peer").or_else(|| qmap.get("device_id")).cloned();
            let peer = if let Some(p) = peer_from_query {
                p
            } else if !body.is_empty() {
                #[derive(serde::Deserialize, Default)]
                struct RouteDelBody {
                    #[serde(default)]
                    peer: Option<String>,
                    #[serde(default)]
                    device_id: Option<String>,
                }
                match serde_json::from_slice::<RouteDelBody>(body) {
                    Ok(d) => d.peer.or(d.device_id).unwrap_or_default(),
                    Err(_) => return err(400, "bad json, expected { peer: string }"),
                }
            } else {
                String::new()
            };
            if peer.trim().is_empty() {
                return err(400, "peer or device_id must be specified via query or body");
            }
            match node.remove_route(&peer) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(msg)) => err(404, &msg),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/net") => json_ok(&node.network_report_dto()),
        ("POST", "/api/net/refresh") | ("POST", "/api/net/probe") => {
            let rep = node.refresh_network().await;
            json_ok(&rep)
        }
        ("GET", "/api/metrics") | ("GET", "/api/traffic") => json_ok(&node.traffic_report()),
        ("POST", "/api/metrics/reset") | ("POST", "/api/traffic/reset") => {
            node.reset_traffic_metrics();
            json_ok(&serde_json::json!({
                "ok": true,
                "reset": true,
                "timestamp_ms": node_clock_ms()
            }))
        }
        ("GET", "/api/clip/status") => match node.clipboard_status() {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &e.to_string()),
        },
        ("GET", "/api/clip/history") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let limit = qmap.get("limit").and_then(|s| s.parse::<usize>().ok());
            json_ok(&node.clipboard_history(limit))
        }
        ("POST", "/api/clip/clear") | ("DELETE", "/api/clip") => match node.clear_clipboard() {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &e.to_string()),
        },
        ("GET", "/api/clip") => match node.get_clipboard_text() {
            Ok(Some(text)) => {
                let hash = tetherly_core::hex_sha256(text.as_bytes());
                let bytes = text.len();
                json_ok(&serde_json::json!({
                    "ok": true,
                    "has_text": true,
                    "bytes": bytes,
                    "hash": hash,
                    "text": text,
                }))
            }
            Ok(None) => json_ok(&serde_json::json!({
                "ok": true,
                "has_text": false,
                "bytes": 0,
                "hash": null,
                "text": null,
            })),
            Err(e) => err(500, &e.to_string()),
        },
        ("POST", "/api/clip/sync") => {
            let d: ClipSyncBody = if body.is_empty() {
                ClipSyncBody::default()
            } else {
                match serde_json::from_slice::<ClipSyncBody>(body) {
                    Ok(v) => v,
                    Err(_) => return err(400, "bad json"),
                }
            };
            let mut peers = node.live_peers();
            peers.sort_by_key(|p| p.device_id.to_string());
            let target_peer = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                Some(raw) => match node.resolve_peer(raw) {
                    Ok(id) => {
                        if peers.iter().any(|p| p.device_id == id) {
                            Some(id)
                        } else {
                            return err(400, "peer is not connected");
                        }
                    }
                    Err(e) => return err(400, &format!("bad peer id: {e}")),
                },
                None => None,
            };

            if let Some(text) = d.text {
                if text.len() > tetherly_core::CLIP_TEXT_MAX {
                    return err(400, "clipboard text exceeds 1 MiB limit");
                }
                let hash = tetherly_core::hex_sha256(text.as_bytes());
                let bytes = text.len();
                match node.send_clipboard_text(&text, target_peer.as_ref()).await {
                    Ok(sent_count) => json_ok(&serde_json::json!({
                        "ok": true,
                        "bytes": bytes,
                        "hash": hash,
                        "sent_peers": sent_count,
                    })),
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                let cur = match node.get_clipboard_text() {
                    Ok(Some(t)) => t,
                    Ok(None) => return err(400, "local clipboard is empty"),
                    Err(e) => return err(400, &e.to_string()),
                };
                if cur.len() > tetherly_core::CLIP_TEXT_MAX {
                    return err(400, "clipboard text exceeds 1 MiB limit");
                }
                let hash = tetherly_core::hex_sha256(cur.as_bytes());
                let bytes = cur.len();
                match node.send_clipboard_to(target_peer.as_ref()).await {
                    Ok(sent_count) => json_ok(&serde_json::json!({
                        "ok": true,
                        "bytes": bytes,
                        "hash": hash,
                        "sent_peers": sent_count,
                    })),
                    Err(e) => err(400, &e.to_string()),
                }
            }
        }
        // Deliver a notify.push on the live Noise session. Only this process
        // holds the session, so a second binary cannot do it. One live peer is
        // implied; more than one needs an explicit `peer`. The body is delivered
        // and never logged.
        ("POST", "/api/notify") => match serde_json::from_slice::<NotifyBody>(body) {
            Ok(d) => {
                let mut peers = node.live_peers();
                peers.sort_by_key(|p| p.device_id.to_string());
                let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                    Some(raw) => match node.resolve_peer(raw) {
                        Ok(id) => {
                            if peers.iter().any(|p| p.device_id == id) {
                                id
                            } else {
                                return err(400, "peer is not connected");
                            }
                        }
                        Err(e) => return err(400, &format!("bad peer id: {e}")),
                    },
                    None => match peers.len() {
                        0 => return err(400, "no connected peer"),
                        1 => peers[0].device_id.clone(),
                        _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                    },
                };
                let push = tetherly_core::NotifyPush {
                    uid: format!("local-{}", node_clock_ms()),
                    app_id: d.app_id,
                    app_name: "tetherly".into(),
                    title: d.title,
                    body: d.body,
                    ts: node_clock_ms(),
                    actions: vec!["copy".into()],
                }
                .truncate();
                match node.send_notify(&target, push).await {
                    Ok(()) => json_ok(&serde_json::json!({ "ok": true, "peer": target.as_str() })),
                    Err(e) => err(400, &e.to_string()),
                }
            }
            Err(_) => err(400, "bad json"),
        },
        ("GET", "/api/notify/candidates") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let peer = qmap.get("peer").map(String::as_str);
            let app_id = qmap.get("app_id").map(String::as_str);
            let has_otp = qmap.get("has_otp").and_then(|v| v.parse::<bool>().ok());

            match node.candidates_filtered(peer, app_id, has_otp) {
                Ok(list) => json_ok(&serde_json::json!({
                    "ok": true,
                    "candidates": list,
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/notify/replies") => {
            let replies: Vec<crate::events::ReplyViewDto> = node
                .reply_views()
                .into_iter()
                .map(crate::events::ReplyViewDto::from)
                .collect();
            json_ok(&serde_json::json!({
                "ok": true,
                "replies": replies,
            }))
        }
        ("POST", "/api/otp/get") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.get_candidate_otp(&d.id) {
                Ok(otp) => json_ok(&serde_json::json!({
                    "ok": true,
                    "id": d.id,
                    "otp": otp,
                })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/copy") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.copy_candidate(&d.id).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/insert") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.insert_candidate(&d.id) {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/dismiss") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.dismiss_candidate_sync(&d.id).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json, expected { id: string }"),
        },
        ("POST", "/api/notify/dismiss") | ("DELETE", "/api/notify/candidates") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<NotifyDismissBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<NotifyDismissBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => {
                        return err(
                            400,
                            "bad json, expected { id?: string, peer?: string, app_id?: string, all?: bool }",
                        )
                    }
                }
            };

            let id = parsed_body
                .as_ref()
                .and_then(|b| b.id.clone())
                .or_else(|| qmap.get("id").cloned());
            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned());
            let app_id = parsed_body
                .as_ref()
                .and_then(|b| b.app_id.clone())
                .or_else(|| qmap.get("app_id").cloned());
            let all = parsed_body
                .as_ref()
                .and_then(|b| b.all)
                .or_else(|| qmap.get("all").and_then(|v| v.parse::<bool>().ok()))
                .unwrap_or(false);

            if all || peer.is_some() || app_id.is_some() {
                let filter = crate::DismissFilter {
                    id,
                    peer,
                    app_id,
                    all: Some(all),
                };
                match node.dismiss_candidates(&filter).await {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                }
            } else if let Some(target_id) = id.filter(|s| !s.trim().is_empty()) {
                match node.dismiss_candidate_sync(&target_id).await {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                err(
                    400,
                    "dismiss target required: provide id, peer, app_id, or all: true",
                )
            }
        }
        ("POST", "/api/open") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => {
                let op = node.opener();
                match node.open_candidate(&d.id, op.as_ref()) {
                    Ok(url) => json_ok(&serde_json::json!({ "ok": true, "url": url })),
                    Err(e) => err(400, &e.to_string()),
                }
            }
            Err(_) => err(400, "bad json"),
        },
        // Quick reply typed on the desktop, sent back to the phone that owns
        // the notification. The text is the user's own; it is delivered, never
        // executed, and never logged.
        ("POST", "/api/reply") => match serde_json::from_slice::<ReplyBody>(body) {
            Ok(d) => match node.reply_to_candidate(&d.id, &d.text).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        // Hand a previously received reply to the local OS path exactly once.
        ("POST", "/api/reply/take") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.take_reply(&d.id) {
                Some((view, text)) => json_ok(&serde_json::json!({
                    "ok": true,
                    "id": d.id,
                    "uid": view.uid,
                    "app_id": view.app_id,
                    "text": text,
                })),
                None => err(400, "unknown reply"),
            },
            Err(_) => err(400, "bad json"),
        },
        ("GET", "/api/notify/config") | ("GET", "/api/notify/banner") => {
            json_ok(&node.notify_config_report())
        }
        ("POST", "/api/notify/config") | ("POST", "/api/notify/banner") => {
            let mut cfg = node.notify_config();
            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(b) = map.get("banners").or_else(|| map.get("banners_enabled")) {
                    cfg.banners_enabled = b == "true" || b == "1";
                }
                if let Some(d) = map.get("dnd").or_else(|| map.get("dnd_mode")) {
                    match d.as_str() {
                        "all" => cfg.dnd_mode = crate::runtime::DndMode::All,
                        "silent" => cfg.dnd_mode = crate::runtime::DndMode::Silent,
                        "off" => cfg.dnd_mode = crate::runtime::DndMode::Off,
                        _ => {}
                    }
                }
                if let Some(s) = map.get("sound").or_else(|| map.get("sound_enabled")) {
                    cfg.sound_enabled = s == "true" || s == "1";
                }
                if let Some(m) = map.get("mask_otp") {
                    cfg.mask_otp = m == "true" || m == "1";
                }
                if let Some(a) = map.get("allow_otp_in_dnd") {
                    cfg.allow_otp_in_dnd = a == "true" || a == "1";
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<NotifyConfigUpdateBody>(body) {
                    Ok(up) => {
                        if let Some(b) = up.banners_enabled {
                            cfg.banners_enabled = b;
                        }
                        if let Some(d) = up.dnd_mode {
                            cfg.dnd_mode = d;
                        }
                        if let Some(s) = up.sound_enabled {
                            cfg.sound_enabled = s;
                        }
                        if let Some(m) = up.mask_otp {
                            cfg.mask_otp = m;
                        }
                        if let Some(a) = up.allow_otp_in_dnd {
                            cfg.allow_otp_in_dnd = a;
                        }
                    }
                    Err(_) => return err(400, "bad json for notify config"),
                }
            }

            let rep = node.update_notify_config(cfg);
            json_ok(&rep)
        }
        ("POST", "/api/notify/test") => {
            let mut q_title: Option<String> = None;
            let mut q_body: Option<String> = None;
            if let Some(q) = query_str {
                let map = parse_query_map(q);
                q_title = map.get("title").cloned();
                q_body = map.get("body").or_else(|| map.get("message")).cloned();
            }

            let (title, body_text) = if !body.is_empty() {
                match serde_json::from_slice::<NotifyTestBody>(body) {
                    Ok(b) => (b.title.or(q_title), b.body.or(q_body)),
                    Err(_) => (q_title, q_body),
                }
            } else {
                (q_title, q_body)
            };

            match node.test_notification_banner(title.as_deref(), body_text.as_deref()) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/file/transfers")
        | ("GET", "/api/transfers")
        | ("POST", "/api/file/transfers")
        | ("POST", "/api/transfers") => {
            let mut q_id: Option<String> = None;
            let mut q_active: Option<bool> = None;
            let mut q_terminal: Option<bool> = None;
            let mut q_direction: Option<String> = None;

            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(id) = map.get("id").or_else(|| map.get("transfer_id")) {
                    q_id = Some(id.clone());
                }
                if let Some(a) = map.get("active") {
                    q_active = Some(a == "true" || a == "1");
                }
                if let Some(t) = map.get("terminal") {
                    q_terminal = Some(t == "true" || t == "1");
                }
                if let Some(d) = map.get("direction") {
                    q_direction = Some(d.clone());
                }
            }

            if method == "POST" && !body.is_empty() {
                match serde_json::from_slice::<TransfersQueryBody>(body) {
                    Ok(b) => {
                        if b.id.is_some() {
                            q_id = b.id;
                        }
                        if b.active.is_some() {
                            q_active = b.active;
                        }
                        if b.terminal.is_some() {
                            q_terminal = b.terminal;
                        }
                        if b.direction.is_some() {
                            q_direction = b.direction;
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            if let Some(id) = q_id.as_deref().filter(|s| !s.is_empty()) {
                match node.transfer_by_id(id) {
                    Some(t) => json_ok(&t),
                    None => err(404, "transfer not found"),
                }
            } else {
                let mut rep = node.transfers_report();
                if let Some(true) = q_active {
                    rep.transfers.retain(|t| {
                        t.phase == crate::runtime::TransferPhase::Transferring
                            || t.phase == crate::runtime::TransferPhase::Verifying
                    });
                } else if let Some(true) = q_terminal {
                    rep.transfers.retain(|t| {
                        t.phase == crate::runtime::TransferPhase::Completed
                            || t.phase == crate::runtime::TransferPhase::Failed
                    });
                }
                if let Some(dir) = q_direction.as_deref() {
                    let dir_lower = dir.to_ascii_lowercase();
                    if dir_lower == "in" || dir_lower == "inbound" {
                        rep.transfers
                            .retain(|t| t.direction == crate::runtime::TransferDirection::Inbound);
                    } else if dir_lower == "out" || dir_lower == "outbound" {
                        rep.transfers
                            .retain(|t| t.direction == crate::runtime::TransferDirection::Outbound);
                    }
                }
                rep.total_count = rep.transfers.len();
                rep.active_count = rep
                    .transfers
                    .iter()
                    .filter(|t| {
                        t.phase == crate::runtime::TransferPhase::Transferring
                            || t.phase == crate::runtime::TransferPhase::Verifying
                    })
                    .count();
                rep.terminal_count = rep.total_count.saturating_sub(rep.active_count);
                json_ok(&rep)
            }
        }
        ("GET", "/api/file/offers") | ("GET", "/api/offers") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let q_id = qmap
                .get("id")
                .or_else(|| qmap.get("transfer_id"))
                .map(String::as_str);
            let q_peer = qmap
                .get("peer")
                .or_else(|| qmap.get("device_id"))
                .map(String::as_str);
            match node.file_offers(q_id, q_peer) {
                Ok(rep) => {
                    if q_id.is_some() {
                        if let Some(first) = rep.offers.first() {
                            json_ok(first)
                        } else {
                            err(404, "transfer offer not found")
                        }
                    } else {
                        json_ok(&rep)
                    }
                }
                Err(tetherly_core::CoreError::UnknownTransfer) => {
                    err(404, "transfer offer not found")
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/file/accept") | ("POST", "/api/accept") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<FileDecisionBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<FileDecisionBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => {
                        return err(
                            400,
                            "bad json, expected { id?: string, all?: bool, peer?: string }",
                        )
                    }
                }
            };
            let is_all = parsed_body.as_ref().and_then(|b| b.all).unwrap_or(false)
                || qmap
                    .get("all")
                    .and_then(|v| v.parse::<bool>().ok())
                    .unwrap_or(false);
            let target_peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").or_else(|| qmap.get("device_id")).cloned());
            let target_id = parsed_body
                .as_ref()
                .and_then(|b| b.id.clone())
                .or_else(|| qmap.get("id").or_else(|| qmap.get("transfer_id")).cloned());

            if is_all || (target_id.is_none() && target_peer.is_some()) {
                match node.batch_accept_files(target_peer.as_deref()).await {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                }
            } else if let Some(id) = target_id {
                match node.user_accept_file_reported(&id).await {
                    Ok(rep) => json_ok(&rep),
                    Err(tetherly_core::CoreError::UnknownTransfer) => {
                        err(404, "transfer offer not found")
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                err(400, "missing required 'id', 'all', or 'peer' parameter")
            }
        }
        ("POST", "/api/file/reject") | ("POST", "/api/reject") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<FileDecisionBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<FileDecisionBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => {
                        return err(
                            400,
                            "bad json, expected { id?: string, all?: bool, peer?: string }",
                        )
                    }
                }
            };
            let is_all = parsed_body.as_ref().and_then(|b| b.all).unwrap_or(false)
                || qmap
                    .get("all")
                    .and_then(|v| v.parse::<bool>().ok())
                    .unwrap_or(false);
            let target_peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").or_else(|| qmap.get("device_id")).cloned());
            let target_id = parsed_body
                .as_ref()
                .and_then(|b| b.id.clone())
                .or_else(|| qmap.get("id").or_else(|| qmap.get("transfer_id")).cloned());

            if is_all || (target_id.is_none() && target_peer.is_some()) {
                match node.batch_reject_files(target_peer.as_deref()).await {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                }
            } else if let Some(id) = target_id {
                match node.user_reject_file_reported(&id).await {
                    Ok(rep) => json_ok(&rep),
                    Err(tetherly_core::CoreError::UnknownTransfer) => {
                        err(404, "transfer offer not found")
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                err(400, "missing required 'id', 'all', or 'peer' parameter")
            }
        }
        ("GET", "/api/file/auto-accept") | ("GET", "/api/auto-accept") => {
            json_ok(&node.auto_accept_status())
        }
        ("POST", "/api/file/auto-accept")
        | ("PUT", "/api/file/auto-accept")
        | ("POST", "/api/auto-accept") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<AutoAcceptBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<AutoAcceptBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected { enabled?: bool }"),
                }
            };
            let enabled = parsed_body
                .and_then(|b| b.enabled.or(b.auto_accept_trusted))
                .or_else(|| {
                    qmap.get("enabled")
                        .or_else(|| qmap.get("auto_accept_trusted"))
                        .and_then(|v| v.parse::<bool>().ok())
                })
                .unwrap_or(true);
            json_ok(&node.set_auto_accept(enabled))
        }
        ("POST", "/api/file/cancel") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.cancel_transfer(&d.id).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true, "transfer_id": d.id })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/file/clear") => {
            let cleared = node.clear_terminal_transfers();
            json_ok(&serde_json::json!({ "ok": true, "cleared": cleared }))
        }
        ("POST", "/api/file/send") => match serde_json::from_slice::<SendFileBody>(body) {
            Ok(d) => {
                let mut peers = node.live_peers();
                peers.sort_by_key(|p| p.device_id.to_string());
                let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                    Some(raw) => match node.resolve_peer(raw) {
                        Ok(id) => {
                            if peers.iter().any(|p| p.device_id == id) {
                                id
                            } else {
                                return err(400, "peer is not connected");
                            }
                        }
                        Err(e) => return err(400, &format!("bad peer id: {e}")),
                    },
                    None => match peers.len() {
                        0 => return err(400, "no connected peer"),
                        1 => peers[0].device_id.clone(),
                        _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                    },
                };

                if let Some(ref path_str) = d.path {
                    let p = std::path::Path::new(path_str);
                    if !p.is_file() {
                        return err(400, "file not found or not a regular file");
                    }
                    let meta = match std::fs::metadata(p) {
                        Ok(m) => m,
                        Err(e) => return err(400, &format!("cannot read file metadata: {e}")),
                    };
                    if meta.len() > tetherly_core::FILE_MAX {
                        return err(400, "file exceeds FILE_MAX (64 GiB)");
                    }
                    let size = meta.len();
                    match node
                        .offer_and_send_file(&target, p, d.name.as_deref())
                        .await
                    {
                        Ok(transfer_id) => {
                            let fname = d.name.clone().unwrap_or_else(|| {
                                p.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("file.bin")
                                    .to_string()
                            });
                            let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                                Ok(n) => n,
                                Err(e) => return err(400, &format!("invalid file name: {e}")),
                            };
                            json_ok(&serde_json::json!({
                                "ok": true,
                                "transfer_id": transfer_id,
                                "name": safe_name,
                                "size": size,
                                "peer": target.as_str(),
                            }))
                        }
                        Err(e) => err(400, &e.to_string()),
                    }
                } else if let Some(ref b64) = d.data_base64 {
                    use base64::Engine;
                    let decoded =
                        match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
                            Ok(b) => b,
                            Err(e) => return err(400, &format!("invalid base64: {e}")),
                        };
                    if decoded.len() as u64 > tetherly_core::FILE_MAX {
                        return err(400, "file exceeds FILE_MAX (64 GiB)");
                    }
                    let fname = d.name.clone().unwrap_or_else(|| "upload.bin".to_string());
                    let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                        Ok(n) => n,
                        Err(e) => return err(400, &format!("invalid file name: {e}")),
                    };
                    let digest = tetherly_core::hex_sha256(&decoded);
                    let size = decoded.len() as u64;
                    match node
                        .offer_and_send_bytes(&target, &safe_name, &decoded, false)
                        .await
                    {
                        Ok(transfer_id) => json_ok(&serde_json::json!({
                            "ok": true,
                            "transfer_id": transfer_id,
                            "name": safe_name,
                            "size": size,
                            "sha256": digest,
                            "peer": target.as_str(),
                        })),
                        Err(e) => err(400, &e.to_string()),
                    }
                } else {
                    err(400, "either path or data_base64 must be provided")
                }
            }
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/file/send-batch") | ("POST", "/api/file/batch") => {
            match serde_json::from_slice::<crate::runtime::BatchSendRequest>(body) {
                Ok(d) => {
                    let mut peers = node.live_peers();
                    peers.sort_by_key(|p| p.device_id.to_string());
                    let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                        Some(raw) => match node.resolve_peer(raw) {
                            Ok(id) => {
                                if peers.iter().any(|p| p.device_id == id) {
                                    id
                                } else {
                                    return err(400, "peer is not connected");
                                }
                            }
                            Err(e) => return err(400, &format!("bad peer id: {e}")),
                        },
                        None => match peers.len() {
                            0 => return err(400, "no connected peer"),
                            1 => peers[0].device_id.clone(),
                            _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                        },
                    };

                    let mut all_items = d.items;
                    for p in d.paths {
                        all_items.push(crate::runtime::BatchSendItem {
                            path: Some(p),
                            name: None,
                            data_base64: None,
                        });
                    }
                    if all_items.is_empty() {
                        return err(400, "no files specified in paths or items");
                    }
                    match node.send_batch_items(&target, all_items).await {
                        Ok(rep) => json_ok(&rep),
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(
                    400,
                    "bad json, expected { peer?: string, paths?: string[], items?: [] }",
                ),
            }
        }
        ("GET", "/api/file/resume") | ("GET", "/api/storage/resume") => {
            let rep = node.resume_markers_report();
            json_ok(&rep)
        }
        ("GET", "/api/ancs") => {
            let rep = node.ancs_report();
            json_ok(&rep)
        }
        // Step 1 of the phone wizard: the user connected the iPhone in the OS
        // Bluetooth settings, so we only have to subscribe to ANCS here.
        ("POST", "/api/ancs/connect") => match node.ancs_connect_reported() {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        },
        ("POST", "/api/ancs/disconnect") => {
            let rep = node.ancs_disconnect_reported();
            json_ok(&rep)
        }
        ("POST", "/api/ancs/allow") | ("POST", "/api/ancs/rules/add") => {
            match serde_json::from_slice::<OpenRuleBody>(body) {
                Ok(d) => match node.add_open_rule_reported(tetherly_core::OpenRule {
                    app_id: d.app_id,
                    url: d.url,
                }) {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                },
                Err(_) => err(400, "bad json, expected { app_id: string, url?: string }"),
            }
        }
        ("DELETE", "/api/ancs/allow")
        | ("POST", "/api/ancs/rules/remove")
        | ("DELETE", "/api/ancs/rules") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let app_id = qmap.get("app_id").cloned().or_else(|| {
                if !body.is_empty() {
                    serde_json::from_slice::<OpenRuleRemoveBody>(body)
                        .ok()
                        .and_then(|b| b.app_id)
                } else {
                    None
                }
            });
            if let Some(app_id) = app_id.filter(|s| !s.trim().is_empty()) {
                let rep = node.remove_open_rule_reported(&app_id);
                json_ok(&rep)
            } else {
                err(400, "missing required 'app_id' in query or body")
            }
        }
        ("POST", "/api/ancs/rules/clear") | ("DELETE", "/api/ancs/rules/clear") => {
            let rep = node.clear_open_rules_reported();
            json_ok(&rep)
        }
        ("GET", "/api/storage") => {
            let rep = node.storage_report();
            json_ok(&rep)
        }
        ("POST", "/api/storage/clean") => {
            let orphaned_only = if body.is_empty() {
                true
            } else {
                match serde_json::from_slice::<StorageCleanBody>(body) {
                    Ok(d) => d.orphaned_only.unwrap_or(true),
                    Err(_) => return err(400, "bad json"),
                }
            };
            let rep = node.clean_storage(orphaned_only);
            json_ok(&rep)
        }
        ("GET", "/api/file/inbox") | ("GET", "/api/inbox") => {
            let mut search: Option<String> = None;
            let mut limit: Option<usize> = None;
            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(s) = map.get("search").or_else(|| map.get("q")) {
                    search = Some(s.clone());
                }
                if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
                    limit = Some(l);
                }
            }
            let rep = node.inbox_report(search.as_deref(), limit);
            json_ok(&rep)
        }
        ("POST", "/api/file/inbox/delete") | ("DELETE", "/api/file/inbox") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let name = qmap.get("name").cloned().or_else(|| {
                if !body.is_empty() {
                    serde_json::from_slice::<InboxDeleteBody>(body)
                        .ok()
                        .and_then(|b| b.name)
                } else {
                    None
                }
            });
            if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
                match node.delete_inbox_file(&name) {
                    Ok(rep) => json_ok(&rep),
                    Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                        err(404, "file not found in inbox")
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                err(400, "missing required 'name' in query or body")
            }
        }
        ("POST", "/api/file/inbox/clear") | ("DELETE", "/api/file/inbox/all") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let before_secs = qmap
                .get("before_secs")
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    if !body.is_empty() {
                        serde_json::from_slice::<InboxClearBody>(body)
                            .ok()
                            .and_then(|b| b.before_secs)
                    } else {
                        None
                    }
                });
            match node.clear_inbox(before_secs) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/file/inbox/download") | ("GET", "/api/inbox/download") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let name = qmap
                .get("name")
                .or_else(|| qmap.get("file"))
                .map(|s| s.trim());
            match name {
                Some(n) if !n.is_empty() => match node.read_inbox_file(n) {
                    Ok((clean_name, mime, bytes)) => UiResponse::new(200, mime, bytes).with_header(
                        "Content-Disposition",
                        format!("attachment; filename=\"{clean_name}\""),
                    ),
                    Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
                    Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                        err(404, "file not found in inbox")
                    }
                    Err(tetherly_core::CoreError::FileTooLarge(sz)) => {
                        err(413, &format!("file too large ({sz} bytes)"))
                    }
                    Err(e) => err(400, &e.to_string()),
                },
                _ => err(400, "missing required 'name' query parameter"),
            }
        }
        ("GET", "/api/file/inbox/raw") | ("GET", "/api/inbox/raw") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let name = qmap
                .get("name")
                .or_else(|| qmap.get("file"))
                .map(|s| s.trim());
            match name {
                Some(n) if !n.is_empty() => match node.read_inbox_file(n) {
                    Ok((clean_name, mime, bytes)) => UiResponse::new(200, mime, bytes).with_header(
                        "Content-Disposition",
                        format!("inline; filename=\"{clean_name}\""),
                    ),
                    Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
                    Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                        err(404, "file not found in inbox")
                    }
                    Err(tetherly_core::CoreError::FileTooLarge(sz)) => {
                        err(413, &format!("file too large ({sz} bytes)"))
                    }
                    Err(e) => err(400, &e.to_string()),
                },
                _ => err(400, "missing required 'name' query parameter"),
            }
        }
        ("POST", "/api/file/inbox/open") | ("POST", "/api/inbox/open") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let name = qmap.get("name").cloned().or_else(|| {
                if !body.is_empty() {
                    serde_json::from_slice::<InboxOpenBody>(body)
                        .ok()
                        .and_then(|b| b.name)
                } else {
                    None
                }
            });
            let op = node.opener();
            match node.open_inbox_file(name.as_deref(), op.as_ref()) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
                Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                    err(404, "file not found in inbox")
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/health") => {
            let rep = node.health_report();
            json_ok(&rep)
        }
        ("POST", "/api/daemon/shutdown") | ("POST", "/api/daemon/stop") => {
            let n = node.clone_handle();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                n.shutdown();
            });
            json_ok(&serde_json::json!({
                "ok": true,
                "status": "shutting_down",
                "message": "node shutdown initiated"
            }))
        }
        // Remote screen: consent is local and explicit. `allow` is the only
        // gate into streaming; nothing a peer sends can flip it.
        ("GET", "/api/screen") => {
            let rep = node.screen_status_report().await;
            json_ok(&rep)
        }
        ("POST", "/api/screen/allow") => {
            let rep = node.screen_allow_reported();
            json_ok(&rep)
        }
        ("POST", "/api/screen/revoke") => {
            let rep = node.screen_revoke_reported();
            json_ok(&rep)
        }
        ("POST", "/api/screen/start") => {
            let target_opt = if !body.is_empty() {
                match serde_json::from_slice::<PeerTargetBody>(body) {
                    Ok(b) => b.peer,
                    Err(_) => return err(400, "bad json"),
                }
            } else {
                None
            };
            match node.screen_start_reported(target_opt.as_deref()).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/screen/stop") => match node.screen_stop_reported().await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        },
        // Local input sharing (controller → controlled). Dialling the 45719
        // channel is what lets `drive_input*` actually deliver; the established
        // capture hooks are inert until then.
        ("GET", "/api/input") => {
            let rep = node.input_status_report().await;
            json_ok(&rep)
        }
        ("POST", "/api/input/start") => {
            let target_opt = if !body.is_empty() {
                match serde_json::from_slice::<PeerTargetBody>(body) {
                    Ok(b) => b.peer,
                    Err(_) => return err(400, "bad json"),
                }
            } else {
                None
            };
            match node.input_start_reported(target_opt.as_deref()).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/input/stop") => match node.input_stop_reported().await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        },
        (_, "/api/events" | "/api/events/stream") => err(405, "method not allowed"),
        _ => UiResponse::new(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

async fn snapshot(node: &Node) -> Snapshot {
    let ov = node.overlay_status();
    let ancs_state = match node.ancs_state() {
        None => "off".to_string(),
        Some(tetherly_core::AncsState::Idle) => "idle".to_string(),
        Some(tetherly_core::AncsState::Ready) => "ready".to_string(),
        Some(tetherly_core::AncsState::Backoff { retry_at_ms }) => {
            format!("backoff until {retry_at_ms}")
        }
    };
    let live = node.live_peers();
    let live_ids: std::collections::HashSet<tetherly_core::DeviceId> =
        live.iter().map(|p| p.device_id.clone()).collect();
    let trusted_raw = node.trusted();
    let trusted_map: std::collections::HashMap<tetherly_core::DeviceId, String> = trusted_raw
        .iter()
        .map(|tp| (tp.device_id.clone(), tp.alias.clone()))
        .collect();
    let trusted: Vec<TrustedPeerView> = trusted_raw
        .into_iter()
        .map(|tp| {
            let online = live_ids.contains(&tp.device_id);
            TrustedPeerView {
                device_id: tp.device_id.to_string(),
                alias: tp.alias,
                paired_at: tp.paired_at,
                revoked: tp.revoked,
                online,
            }
        })
        .collect();
    Snapshot {
        device_id: node.identity().device_id().to_string(),
        name: node.name().to_string(),
        platform: node.platform().to_string(),
        pin_active: node.pin_if_fresh().is_some(),
        lan_only: !ov.present,
        overlay_present: ov.present,
        overlay_source: format!("{:?}", ov.source),
        overlay_peers: ov.peers.len(),
        candidates: node.candidates(),
        replies: node
            .reply_views()
            .into_iter()
            .map(crate::events::ReplyViewDto::from)
            .collect(),
        ancs_state,
        open_apps: node
            .open_allowlist()
            .rules()
            .iter()
            .map(|r| r.app_id.clone())
            .collect(),
        peers: live
            .into_iter()
            .map(|p| {
                let alias = trusted_map.get(&p.device_id).cloned();
                let features = crate::runtime::extract_features(&p.caps);
                PeerView {
                    device_id: p.device_id.to_string(),
                    name: p.name,
                    alias,
                    platform: p.platform,
                    path: match p.path {
                        tetherly_core::PathKind::Lan => "lan".into(),
                        tetherly_core::PathKind::Overlay => "overlay".into(),
                    },
                    caps: p.caps.clone(),
                    features,
                    file_port: p.file_port,
                    input_port: p.input_port,
                    screen_port: p.screen_port,
                    rtt_ms: p.rtt_ms,
                    last_seen_ms: p.last_seen_ms,
                }
            })
            .collect(),
        trusted,
        files: node
            .offered_files()
            .into_iter()
            .map(|t| FileSnap {
                transfer_id: t.transfer_id,
                source: t.source.to_string(),
                name: t.files.first().map(|f| f.name.clone()).unwrap_or_default(),
                size: t.files.first().map(|f| f.size).unwrap_or(0),
            })
            .collect(),
        transfers: node.active_transfers(),
        transfer_stats: node.transfer_stats(),
        screen_state: node.screen_state().as_str().to_string(),
        screen: {
            let s = node.screen_stats();
            // Visitor-side receiver counters (the engine is host-side only).
            let r = node.screen_rx_stats();
            ScreenSnap {
                state: node.screen_state().as_str().to_string(),
                sent: s.sent,
                received: r.received,
                dropped: r.dropped,
                bytes: s.bytes + r.bytes,
                refused: s.refused,
                p95_frame_interval_us: node.screen_latency_p95_us(),
            }
        },
        input_channel: node.input_channel_open().await,
        input_seat: node.input_seat_str().to_string(),
        lan: crate::lan::interface_report(),
        traffic: node.traffic_report(),
    }
}

fn json_ok<T: Serialize>(v: &T) -> UiResponse {
    match serde_json::to_vec(v) {
        Ok(b) => UiResponse::new(200, "application/json; charset=utf-8", b),
        Err(_) => err(400, "encode"),
    }
}

fn err(code: u16, msg: &str) -> UiResponse {
    UiResponse::new(
        code,
        "application/json; charset=utf-8",
        serde_json::to_vec(&serde_json::json!({ "error": msg })).unwrap_or_default(),
    )
}
