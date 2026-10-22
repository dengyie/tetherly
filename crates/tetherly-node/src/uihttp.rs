// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback-only desktop shell. Never bind 0.0.0.0. Bodies/OTP/PIN are not
//! written to logs. Full Tauri packaging is the next desktop-shell iteration.

use crate::runtime::{Node, OutboxStatus};
use crate::store::{ActionAccessMode, LocalActionDefinition};
use serde::Serialize;
use std::net::SocketAddr;
use tetherly_core::{ActionValidator, DisplayEdge, MediaCommand, PlaybackState, PowerSource};
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
    /// Total audit entries recorded in ring buffer.
    audit_count: usize,
    /// Total unread direct messages in ring buffer.
    unread_messages_count: usize,
    /// Total resource handoff entries in inbox.
    handoff_inbox_count: usize,
    /// Current handoff action policy string.
    handoff_action: String,
    /// Local media playback state if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    media_state: Option<tetherly_core::MediaStateFrame>,
    /// Media ingress control mode string.
    media_mode: String,
    /// Local battery state if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    battery_state: Option<tetherly_core::BatteryStateFrame>,
    /// Low battery threshold percentage.
    battery_low_threshold: u8,
    /// Focus status report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    focus_status: Option<crate::runtime::FocusStatusReport>,
    /// Spatial topology layout report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    topology_layout: Option<crate::runtime::TopologyLayoutReport>,
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
    device_type: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
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
    pub device_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
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

#[derive(serde::Deserialize, Default)]
struct ProfileUpdateBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    group: Option<Option<String>>,
    #[serde(default)]
    notes: Option<Option<String>>,
}

#[derive(serde::Deserialize, Default)]
struct PeerTagBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    tag: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct PeerGroupBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    group: Option<String>,
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
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    max_file_size: Option<u64>,
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
    #[serde(default, alias = "data")]
    data_base64: Option<String>,
    peer: Option<String>,
    #[serde(default, alias = "filename")]
    name: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct ClipSyncBody {
    peer: Option<String>,
    text: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ClipPolicyBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    sensitive_guard: Option<bool>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    max_payload_bytes: Option<usize>,
    #[serde(default)]
    auto_clear_timeout_secs: Option<u64>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct HandoffSendBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, alias = "auto_open_requested")]
    auto_open: Option<bool>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct HandoffInboxActionBody {
    #[serde(default)]
    id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct HandoffPolicyBody {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    max_inbox_entries: Option<usize>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct MediaCommandBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    volume: Option<u8>,
    #[serde(default, alias = "volume_percent")]
    volume_percent: Option<u8>,
    #[serde(default)]
    offset_ms: Option<i64>,
    #[serde(default)]
    target_player: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct MediaStateUpdateBody {
    #[serde(default)]
    player_id: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    album: Option<String>,
    #[serde(default, alias = "position", alias = "pos")]
    position_ms: Option<u64>,
    #[serde(default, alias = "duration", alias = "dur")]
    duration_ms: Option<u64>,
    #[serde(default, alias = "volume", alias = "vol")]
    volume_percent: Option<u8>,
    #[serde(default)]
    muted: Option<bool>,
    #[serde(default)]
    can_play: Option<bool>,
    #[serde(default)]
    can_pause: Option<bool>,
    #[serde(default)]
    can_go_next: Option<bool>,
    #[serde(default)]
    can_go_previous: Option<bool>,
    #[serde(default)]
    can_seek: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct MediaPolicyBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    max_volume_limit: Option<u8>,
    #[serde(default)]
    allow_seek: Option<bool>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct BatteryStateUpdateBody {
    #[serde(default, alias = "level", alias = "battery_percent", alias = "percent")]
    battery_level: Option<u8>,
    #[serde(default, alias = "charging")]
    is_charging: Option<bool>,
    #[serde(default, alias = "source", alias = "source_type")]
    power_source: Option<String>,
    #[serde(default, alias = "saver", alias = "low_power_mode")]
    battery_saver: Option<bool>,
    #[serde(default, alias = "health")]
    health_percent: Option<u8>,
}

#[derive(serde::Deserialize, Default)]
struct BatteryPolicyBody {
    #[serde(default, alias = "low_threshold", alias = "low")]
    low_battery_threshold: Option<u8>,
    #[serde(default, alias = "critical_threshold", alias = "critical")]
    critical_battery_threshold: Option<u8>,
    #[serde(default, alias = "alert")]
    alert_on_low_battery: Option<bool>,
    #[serde(default, alias = "throttle", alias = "auto_throttle")]
    auto_throttle_on_battery_saver: Option<bool>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct LocateRingBody {
    #[serde(default, alias = "target")]
    peer: Option<String>,
    #[serde(default, alias = "duration")]
    duration_secs: Option<u32>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    override_silent: Option<bool>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct LocateStopBody {
    #[serde(default, alias = "target")]
    peer: Option<String>,
    #[serde(default, alias = "id")]
    request_id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct LocatePolicyBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    allow_override_silent: Option<bool>,
    #[serde(default, alias = "max_duration")]
    max_duration_secs: Option<u32>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct ActionLocalBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default, alias = "timeout")]
    timeout_secs: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct ActionTriggerBody {
    #[serde(default, alias = "target")]
    peer: Option<String>,
    #[serde(default, alias = "id")]
    action_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ActionPolicyBody {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, alias = "timeout", alias = "max_timeout")]
    max_timeout_secs: Option<u32>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default, alias = "expose")]
    expose_action_list: Option<bool>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct FocusSetBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, alias = "duration", alias = "duration_minutes")]
    duration_mins: Option<u32>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    broadcast: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct FocusPolicyBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    default_mode: Option<String>,
    #[serde(default, alias = "mirror")]
    mirror_inbound_focus: Option<bool>,
    #[serde(default, alias = "breakthrough")]
    allow_emergency_breakthrough: Option<bool>,
    #[serde(default, alias = "quiet_hours")]
    quiet_hours_enabled: Option<bool>,
    #[serde(default, alias = "start_minute")]
    quiet_hours_start_minute: Option<u16>,
    #[serde(default, alias = "end_minute")]
    quiet_hours_end_minute: Option<u16>,
    #[serde(default, alias = "start")]
    quiet_hours_start: Option<String>,
    #[serde(default, alias = "end")]
    quiet_hours_end: Option<String>,
    #[serde(default)]
    allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct TopologyLinkBody {
    #[serde(default)]
    edge: Option<String>,
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    peer: Option<String>,
    #[serde(default)]
    peer_edge: Option<String>,
    #[serde(default)]
    offset: Option<i32>,
}

#[derive(serde::Deserialize, Default)]
struct TopologyUnlinkBody {
    #[serde(default)]
    edge: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct TopologyConfigBody {
    #[serde(default, alias = "resistance")]
    resistance_px: Option<u32>,
    #[serde(default, alias = "delay")]
    delay_ms: Option<u32>,
    #[serde(default, alias = "corner_lock", alias = "corner_lock_size")]
    corner_lock_size_px: Option<u32>,
    #[serde(default, alias = "wrap")]
    wrap_around: Option<bool>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default, alias = "scale")]
    scale_factor: Option<f32>,
}

#[derive(serde::Deserialize, Default)]
struct TopologyPolicyBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default, alias = "allow_inbound")]
    allow_inbound_control: Option<bool>,
    #[serde(default, alias = "auto_link")]
    auto_link_paired_peers: Option<bool>,
    #[serde(default, alias = "group", alias = "groups")]
    allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct InputTunnelAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    peer: Option<String>,
    #[serde(default)]
    edge: Option<String>,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
}

#[derive(serde::Deserialize, Default)]
struct InputTunnelReleaseBody {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct InputTunnelPolicyBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default, alias = "allow_inbound")]
    allow_inbound_control: Option<bool>,
    #[serde(default, alias = "keyboard")]
    allow_keyboard: Option<bool>,
    #[serde(default, alias = "mouse")]
    allow_mouse: Option<bool>,
    #[serde(default, alias = "hotkey", alias = "hotkey_release")]
    hotkey_release_enabled: Option<bool>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    allowed_types: Option<Vec<String>>,
    #[serde(default)]
    clear_filters: Option<bool>,
}

fn parse_hh_mm(s: &str) -> Option<u16> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u16 = parts[0].trim().parse().ok()?;
    let m: u16 = parts[1].trim().parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

#[derive(serde::Deserialize, Default)]
struct AuditQueryBody {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    since: Option<u64>,
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

#[derive(serde::Deserialize, Default)]
struct PeerLinkProbeBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    interval_ms: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
struct HeartbeatConfigBody {
    #[serde(default)]
    interval_secs: Option<u64>,
    #[serde(default)]
    deadline_secs: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
struct MessageSendBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    all: Option<bool>,
    #[serde(default)]
    text: String,
}

#[derive(serde::Deserialize, Default)]
struct MessageQueryBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    unread_only: Option<bool>,
    #[serde(default)]
    mark_read: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
struct MessageClearBody {
    #[serde(default)]
    peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct OutboxQueryBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
struct OutboxClearBody {
    #[serde(default)]
    peer: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct OutboxResendBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    transfer_id: Option<String>,
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
        ("GET", "/api/peers/link") | ("GET", "/api/link") => json_ok(&node.link_quality_report()),
        ("POST", "/api/peers/link/probe") | ("POST", "/api/link/probe") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerLinkProbeBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<PeerLinkProbeBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json"),
                }
            };
            let target_query = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .unwrap_or_default();
            let count = parsed_body
                .as_ref()
                .and_then(|b| b.count)
                .or_else(|| qmap.get("count").and_then(|s| s.parse().ok()))
                .unwrap_or(5);
            let interval_ms = parsed_body
                .as_ref()
                .and_then(|b| b.interval_ms)
                .or_else(|| qmap.get("interval_ms").and_then(|s| s.parse().ok()))
                .unwrap_or(100);

            let mut peers = node.live_peers();
            peers.sort_by_key(|p| p.device_id.to_string());
            let target = match target_query.as_str().trim() {
                s if !s.is_empty() => match node.resolve_peer(s) {
                    Ok(id) => {
                        if peers.iter().any(|p| p.device_id == id) {
                            id
                        } else {
                            return err(400, "peer is not connected");
                        }
                    }
                    Err(e) => return err(400, &format!("bad peer id: {e}")),
                },
                _ => match peers.len() {
                    0 => return err(400, "no connected peer"),
                    1 => peers[0].device_id.clone(),
                    _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                },
            };
            match node
                .probe_peer_link(target.as_str(), count, interval_ms)
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/peers/heartbeat") | ("GET", "/api/heartbeat") => {
            json_ok(&node.heartbeat_config())
        }
        ("POST", "/api/peers/heartbeat")
        | ("POST", "/api/heartbeat")
        | ("PUT", "/api/peers/heartbeat")
        | ("PUT", "/api/heartbeat") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<HeartbeatConfigBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<HeartbeatConfigBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json"),
                }
            };
            let interval_secs = parsed_body
                .as_ref()
                .and_then(|b| b.interval_secs)
                .or_else(|| qmap.get("interval_secs").and_then(|s| s.parse().ok()));
            let deadline_secs = parsed_body
                .as_ref()
                .and_then(|b| b.deadline_secs)
                .or_else(|| qmap.get("deadline_secs").and_then(|s| s.parse().ok()));
            if interval_secs.is_none() && deadline_secs.is_none() {
                return err(400, "interval_secs or deadline_secs required");
            }
            match node.update_heartbeat_config(interval_secs, deadline_secs) {
                Ok(cfg) => json_ok(&cfg),
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
                    let profile = node.peer_profile(&p.device_id);
                    serde_json::json!({
                        "device_id": p.device_id.to_string(),
                        "name": p.name,
                        "alias": alias_opt,
                        "platform": p.platform,
                        "device_type": profile.device_type.as_str(),
                        "tags": profile.tags,
                        "group": profile.group,
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
                    let profile = node.peer_profile(&tp.device_id);
                    serde_json::json!({
                        "device_id": tp.device_id.to_string(),
                        "alias": tp.alias,
                        "device_type": profile.device_type.as_str(),
                        "tags": profile.tags,
                        "group": profile.group,
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
        ("GET", "/api/peers/profiles") | ("GET", "/api/profiles") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let tag = qmap.get("tag").map(|s| s.as_str());
            let group = qmap.get("group").map(|s| s.as_str());
            let dev_type = qmap
                .get("type")
                .and_then(|t| t.parse::<crate::store::DeviceType>().ok());
            json_ok(&node.peer_profiles(tag, group, dev_type))
        }
        ("GET", "/api/peers/profile") | ("GET", "/api/profile") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let target = qmap
                .get("peer")
                .or_else(|| qmap.get("id"))
                .or_else(|| qmap.get("device_id"))
                .map(|s| s.as_str())
                .unwrap_or_default();
            if target.trim().is_empty() {
                return err(400, "peer parameter required");
            }
            match node.resolve_peer(target.trim()) {
                Ok(id) => json_ok(&node.peer_profile(&id)),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/peers/profile")
        | ("POST", "/api/profile")
        | ("PUT", "/api/peers/profile")
        | ("PUT", "/api/profile") => match serde_json::from_slice::<ProfileUpdateBody>(body) {
            Ok(d) => {
                let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
                let target = match target {
                    Some(t) => t,
                    None => return err(400, "missing peer or device_id"),
                };
                let dev_type = match d.device_type {
                    Some(ref s) => match s.parse::<crate::store::DeviceType>() {
                        Ok(dt) => Some(dt),
                        Err(e) => return err(400, &e),
                    },
                    None => None,
                };
                match node.update_peer_profile(&target, dev_type, d.tags, d.group, d.notes) {
                    Ok(rep) => json_ok(&rep),
                    Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                        err(404, &format!("peer not found: {e}"))
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            }
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/peers/tags/add") | ("POST", "/api/tags/add") => {
            match serde_json::from_slice::<PeerTagBody>(body) {
                Ok(d) => {
                    let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
                    let target = match target {
                        Some(t) => t,
                        None => return err(400, "missing peer or device_id"),
                    };
                    let tag = match d.tag.filter(|s| !s.trim().is_empty()) {
                        Some(t) => t,
                        None => return err(400, "tag required"),
                    };
                    match node.add_peer_tag(&target, &tag) {
                        Ok(rep) => json_ok(&rep),
                        Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                            err(404, &format!("peer not found: {e}"))
                        }
                        Err(e) => err(400, &e.to_string()),
                    }
                }
                Err(_) => err(400, "bad json"),
            }
        }
        ("POST", "/api/peers/tags/remove")
        | ("POST", "/api/tags/remove")
        | ("DELETE", "/api/peers/tags") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<PeerTagBody> = if body.is_empty() {
                None
            } else {
                serde_json::from_slice::<PeerTagBody>(body).ok()
            };
            let target = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("device_id").cloned())
                .filter(|s| !s.trim().is_empty());
            let target = match target {
                Some(t) => t,
                None => return err(400, "missing peer or device_id"),
            };
            let tag = parsed_body
                .and_then(|b| b.tag)
                .or_else(|| qmap.get("tag").cloned())
                .filter(|s| !s.trim().is_empty());
            let tag = match tag {
                Some(t) => t,
                None => return err(400, "tag required"),
            };
            match node.remove_peer_tag(&target, &tag) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/peers/group")
        | ("POST", "/api/group")
        | ("PUT", "/api/peers/group")
        | ("PUT", "/api/group") => match serde_json::from_slice::<PeerGroupBody>(body) {
            Ok(d) => {
                let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
                let target = match target {
                    Some(t) => t,
                    None => return err(400, "missing peer or device_id"),
                };
                match node.set_peer_group(&target, d.group.as_deref()) {
                    Ok(rep) => json_ok(&rep),
                    Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                        err(404, &format!("peer not found: {e}"))
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            }
            Err(_) => err(400, "bad json"),
        },
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
        ("GET", "/api/clip/policy") => json_ok(&node.clipboard_policy_report()),
        ("POST", "/api/clip/policy") | ("PUT", "/api/clip/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ClipPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<ClipPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected clipboard policy fields"),
                }
            };
            let mut policy = node.clipboard_policy();

            if let Some(ref b) = parsed_body {
                if let Some(ref m) = b.mode {
                    match m.parse::<tetherly_core::ClipSyncMode>() {
                        Ok(mode) => policy.mode = mode,
                        Err(e) => return err(400, &format!("invalid clip sync mode '{m}': {e}")),
                    }
                }
                if let Some(sg) = b.sensitive_guard {
                    policy.sensitive_guard = sg;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if let Some(bytes) = b.max_payload_bytes {
                    policy.max_payload_bytes = bytes;
                }
                if let Some(secs) = b.auto_clear_timeout_secs {
                    policy.auto_clear_timeout_secs = if secs == 0 { None } else { Some(secs) };
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(m_str) = qmap.get("mode") {
                match m_str.parse::<tetherly_core::ClipSyncMode>() {
                    Ok(mode) => policy.mode = mode,
                    Err(e) => return err(400, &format!("invalid clip sync mode '{m_str}': {e}")),
                }
            }
            if let Some(sg_str) = qmap.get("sensitive_guard") {
                policy.sensitive_guard = sg_str == "true" || sg_str == "1";
            }
            if let Some(grp_str) = qmap.get("group").or_else(|| qmap.get("groups")) {
                policy.allowed_groups = grp_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            if let Some(tag_str) = qmap.get("tag").or_else(|| qmap.get("tags")) {
                policy.allowed_tags = tag_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            if let Some(type_str) = qmap.get("type").or_else(|| qmap.get("types")) {
                let mut dtypes = Vec::new();
                for t in type_str.split(',') {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        match trimmed.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => {
                                return err(400, &format!("invalid device type '{trimmed}': {e}"))
                            }
                        }
                    }
                }
                policy.allowed_types = dtypes;
            }
            if let Some(sz_str) = qmap
                .get("max_payload_bytes")
                .or_else(|| qmap.get("max_bytes"))
            {
                if let Ok(sz) = sz_str.parse::<usize>() {
                    policy.max_payload_bytes = sz;
                }
            }
            if let Some(ac_str) = qmap
                .get("auto_clear_timeout_secs")
                .or_else(|| qmap.get("auto_clear"))
            {
                if let Ok(secs) = ac_str.parse::<u64>() {
                    policy.auto_clear_timeout_secs = if secs == 0 { None } else { Some(secs) };
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_clipboard_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(500, &e.to_string()),
            }
        }
        ("POST", "/api/clip/auto-sync/poll") => match node.poll_clipboard_auto_sync().await {
            Ok(dispatched_peers) => {
                let rep = node.clipboard_policy_report();
                json_ok(&serde_json::json!({
                    "ok": true,
                    "dispatched_peers": dispatched_peers,
                    "policy": rep.policy,
                    "stats": {
                        "auto_synced_out_count": rep.auto_synced_out_count,
                        "auto_synced_in_count": rep.auto_synced_in_count,
                        "sensitive_blocked_count": rep.sensitive_blocked_count,
                    }
                }))
            }
            Err(e) => err(500, &e.to_string()),
        },
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
        ("GET", "/api/file/auto-accept/policy") | ("GET", "/api/auto-accept/policy") => {
            json_ok(&node.auto_accept_policy_report())
        }
        ("POST", "/api/file/auto-accept/policy")
        | ("PUT", "/api/file/auto-accept/policy")
        | ("POST", "/api/auto-accept/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<AutoAcceptBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<AutoAcceptBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected auto-accept policy fields"),
                }
            };
            let mut policy = node.auto_accept_policy();
            if let Some(ref b) = parsed_body {
                if let Some(en) = b.enabled.or(b.auto_accept_trusted) {
                    policy.enabled = en;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if let Some(sz) = b.max_file_size {
                    policy.max_file_size = Some(sz);
                }
            }
            if let Some(en_str) = qmap
                .get("enabled")
                .or_else(|| qmap.get("auto_accept_trusted"))
            {
                if let Ok(en) = en_str.parse::<bool>() {
                    policy.enabled = en;
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(sz_str) = qmap.get("max_size").or_else(|| qmap.get("max_file_size")) {
                if let Ok(sz) = sz_str.parse::<u64>() {
                    policy.max_file_size = Some(sz);
                }
            }
            match node.set_auto_accept_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/file/auto-accept") | ("GET", "/api/auto-accept") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            if qmap
                .get("policy")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                json_ok(&node.auto_accept_policy_report())
            } else {
                json_ok(&node.auto_accept_status())
            }
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
            let has_policy_fields = parsed_body
                .as_ref()
                .map(|b| {
                    b.allowed_groups.is_some()
                        || b.allowed_tags.is_some()
                        || b.allowed_types.is_some()
                        || b.max_file_size.is_some()
                })
                .unwrap_or(false)
                || qmap.contains_key("group")
                || qmap.contains_key("tag")
                || qmap.contains_key("max_size");

            if has_policy_fields {
                let mut policy = node.auto_accept_policy();
                if let Some(ref b) = parsed_body {
                    if let Some(en) = b.enabled.or(b.auto_accept_trusted) {
                        policy.enabled = en;
                    }
                    if let Some(ref grps) = b.allowed_groups {
                        policy.allowed_groups = grps.clone();
                    }
                    if let Some(ref tags) = b.allowed_tags {
                        policy.allowed_tags = tags.clone();
                    }
                    if let Some(ref types) = b.allowed_types {
                        let mut dtypes = Vec::new();
                        for t in types {
                            match t.parse::<crate::store::DeviceType>() {
                                Ok(dt) => dtypes.push(dt),
                                Err(e) => {
                                    return err(400, &format!("invalid device type '{t}': {e}"))
                                }
                            }
                        }
                        policy.allowed_types = dtypes;
                    }
                    if let Some(sz) = b.max_file_size {
                        policy.max_file_size = Some(sz);
                    }
                }
                if let Some(en_str) = qmap
                    .get("enabled")
                    .or_else(|| qmap.get("auto_accept_trusted"))
                {
                    if let Ok(en) = en_str.parse::<bool>() {
                        policy.enabled = en;
                    }
                }
                if let Some(g) = qmap.get("group") {
                    if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                        policy.allowed_groups.push(g.clone());
                    }
                }
                if let Some(t) = qmap.get("tag") {
                    if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                        policy.allowed_tags.push(t.clone());
                    }
                }
                if let Some(sz_str) = qmap.get("max_size").or_else(|| qmap.get("max_file_size")) {
                    if let Ok(sz) = sz_str.parse::<u64>() {
                        policy.max_file_size = Some(sz);
                    }
                }
                match node.set_auto_accept_policy(policy) {
                    Ok(rep) => json_ok(&rep),
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
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
        ("POST", "/api/file/send-multi") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let d: SendFileBody = if body.is_empty() {
                SendFileBody {
                    path: qmap.get("path").cloned(),
                    data_base64: None,
                    peer: qmap.get("peer").cloned(),
                    name: qmap.get("name").cloned(),
                    tag: qmap.get("tag").cloned(),
                    group: qmap.get("group").cloned(),
                    device_type: qmap
                        .get("type")
                        .or_else(|| qmap.get("device_type"))
                        .cloned(),
                    r#type: None,
                    all: qmap.get("all").map(|s| s == "true" || s == "1"),
                }
            } else {
                match serde_json::from_slice::<SendFileBody>(body) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad json, expected SendFileBody"),
                }
            };
            let is_all = d.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = d
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = d
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = d
                .device_type
                .as_deref()
                .or(d.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });
            let peer = d
                .peer
                .as_deref()
                .or_else(|| qmap.get("peer").map(String::as_str));

            let selector = match crate::store::TargetSelector::from_query(
                peer, tag, group, dev_type, is_all,
            ) {
                Ok(s) => s,
                Err(e) => return err(400, &e),
            };

            if let Some(b64) = d.data_base64 {
                use base64::Engine;
                let bytes = match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad base64 in data_base64"),
                };
                let filename = d.name.as_deref().unwrap_or("uploaded_file");
                match node
                    .offer_and_send_bytes_multi(&selector, filename, &bytes)
                    .await
                {
                    Ok(report) => return json_ok(&report),
                    Err(e) => return err(400, &e.to_string()),
                }
            }

            let path_opt = d
                .path
                .as_deref()
                .or_else(|| qmap.get("path").map(String::as_str));
            let Some(path_str) = path_opt else {
                return err(400, "missing required 'path' or 'data_base64' parameter");
            };
            let p = std::path::Path::new(path_str);
            if !p.is_file() {
                return err(400, "file not found or not a regular file");
            }

            let name_opt = d
                .name
                .as_deref()
                .or_else(|| qmap.get("name").map(String::as_str));
            match node.offer_and_send_file_multi(&selector, p, name_opt).await {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/file/send") => match serde_json::from_slice::<SendFileBody>(body) {
            Ok(d) => {
                let qmap = query_str.map(parse_query_map).unwrap_or_default();
                let is_all = d.all.unwrap_or(false)
                    || qmap
                        .get("all")
                        .map(|s| s == "true" || s == "1")
                        .unwrap_or(false)
                    || qmap.contains_key("all");
                let tag = d
                    .tag
                    .as_deref()
                    .or_else(|| qmap.get("tag").map(String::as_str));
                let group = d
                    .group
                    .as_deref()
                    .or_else(|| qmap.get("group").map(String::as_str));
                let dev_type = d
                    .device_type
                    .as_deref()
                    .or(d.r#type.as_deref())
                    .or_else(|| {
                        qmap.get("type")
                            .or_else(|| qmap.get("device_type"))
                            .map(String::as_str)
                    });

                if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
                    let peer = d
                        .peer
                        .as_deref()
                        .or_else(|| qmap.get("peer").map(String::as_str));
                    let selector = match crate::store::TargetSelector::from_query(
                        peer, tag, group, dev_type, is_all,
                    ) {
                        Ok(s) => s,
                        Err(e) => return err(400, &e),
                    };
                    if let Some(b64) = d.data_base64 {
                        use base64::Engine;
                        let bytes = match base64::engine::general_purpose::STANDARD
                            .decode(b64.as_bytes())
                        {
                            Ok(b) => b,
                            Err(_) => return err(400, "bad base64 in data_base64"),
                        };
                        let filename = d.name.as_deref().unwrap_or("uploaded_file");
                        match node
                            .offer_and_send_bytes_multi(&selector, filename, &bytes)
                            .await
                        {
                            Ok(report) => return json_ok(&report),
                            Err(e) => return err(400, &e.to_string()),
                        }
                    }
                    let path_opt = d
                        .path
                        .as_deref()
                        .or_else(|| qmap.get("path").map(String::as_str));
                    let Some(path_str) = path_opt else {
                        return err(400, "missing required 'path' or 'data_base64' parameter for targeted dispatch");
                    };
                    let p = std::path::Path::new(path_str);
                    if !p.is_file() {
                        return err(400, "file not found or not a regular file");
                    }
                    let name_opt = d
                        .name
                        .as_deref()
                        .or_else(|| qmap.get("name").map(String::as_str));
                    match node.offer_and_send_file_multi(&selector, p, name_opt).await {
                        Ok(report) => return json_ok(&report),
                        Err(e) => return err(400, &e.to_string()),
                    }
                }

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
        ("GET", "/api/file/outbox")
        | ("GET", "/api/outbox")
        | ("POST", "/api/file/outbox/query")
        | ("POST", "/api/outbox/query") => {
            let mut q_peer: Option<String> = None;
            let mut q_status: Option<OutboxStatus> = None;
            let mut q_limit: Option<usize> = None;

            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(p) = map
                    .get("peer")
                    .or_else(|| map.get("to"))
                    .or_else(|| map.get("target"))
                {
                    q_peer = Some(p.clone());
                }
                if let Some(s) = map.get("status") {
                    match s.parse::<OutboxStatus>() {
                        Ok(st) => q_status = Some(st),
                        Err(e) => return err(400, &e),
                    }
                }
                if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
                    q_limit = Some(l);
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<OutboxQueryBody>(body) {
                    Ok(b) => {
                        if b.peer.is_some() {
                            q_peer = b.peer;
                        }
                        if let Some(ref st_str) = b.status {
                            match st_str.parse::<OutboxStatus>() {
                                Ok(st) => q_status = Some(st),
                                Err(e) => return err(400, &e),
                            }
                        }
                        if b.limit.is_some() {
                            q_limit = b.limit;
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            match node.outbox_history(q_peer.as_deref(), q_status, q_limit) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/file/outbox/clear")
        | ("POST", "/api/outbox/clear")
        | ("DELETE", "/api/file/outbox")
        | ("DELETE", "/api/outbox") => {
            let mut q_peer: Option<String> = None;
            let mut q_status: Option<OutboxStatus> = None;

            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(p) = map
                    .get("peer")
                    .or_else(|| map.get("to"))
                    .or_else(|| map.get("target"))
                {
                    q_peer = Some(p.clone());
                }
                if let Some(s) = map.get("status") {
                    match s.parse::<OutboxStatus>() {
                        Ok(st) => q_status = Some(st),
                        Err(e) => return err(400, &e),
                    }
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<OutboxClearBody>(body) {
                    Ok(b) => {
                        if b.peer.is_some() {
                            q_peer = b.peer;
                        }
                        if let Some(ref st_str) = b.status {
                            match st_str.parse::<OutboxStatus>() {
                                Ok(st) => q_status = Some(st),
                                Err(e) => return err(400, &e),
                            }
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            match node.clear_outbox(q_peer.as_deref(), q_status) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/file/outbox/resend") | ("POST", "/api/outbox/resend") => {
            let mut target_id: Option<String> = None;
            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(id) = map.get("id").or_else(|| map.get("transfer_id")) {
                    target_id = Some(id.clone());
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<OutboxResendBody>(body) {
                    Ok(b) => {
                        if b.id.is_some() {
                            target_id = b.id;
                        } else if b.transfer_id.is_some() {
                            target_id = b.transfer_id;
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            let Some(id) = target_id else {
                return err(400, "missing required transfer or outbox id");
            };

            match node.resend_outbox_file(&id).await {
                Ok(report) => json_ok(&report),
                Err(tetherly_core::CoreError::UnknownTransfer) => {
                    err(404, "outbox entry not found")
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/health") => {
            let rep = node.health_report();
            json_ok(&rep)
        }
        ("GET", "/api/sys") | ("GET", "/api/system") | ("GET", "/api/process") => {
            match node.system_report() {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(500, &e.to_string()),
            }
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
        ("GET", "/api/input/tunnel") | ("GET", "/api/input/tunnel/status") => {
            json_ok(&node.input_tunnel_status())
        }
        ("POST", "/api/input/tunnel/attach") | ("PUT", "/api/input/tunnel/attach") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<InputTunnelAttachBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<InputTunnelAttachBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected InputTunnelAttachBody"),
                }
            };

            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("peer_id").cloned())
                .or_else(|| qmap.get("target").cloned());
            let peer = match peer {
                Some(p) if !p.trim().is_empty() => p,
                _ => return err(400, "missing 'peer' parameter"),
            };

            let edge_str = parsed_body
                .as_ref()
                .and_then(|b| b.edge.clone())
                .or_else(|| qmap.get("edge").cloned());
            let edge = match edge_str {
                Some(s) => match s.parse::<DisplayEdge>() {
                    Ok(e) => Some(e),
                    Err(e) => return err(400, &format!("invalid edge '{s}': {e}")),
                },
                None => None,
            };

            let x = parsed_body
                .as_ref()
                .and_then(|b| b.x)
                .or_else(|| qmap.get("x").and_then(|s| s.parse::<i32>().ok()));

            let y = parsed_body
                .as_ref()
                .and_then(|b| b.y)
                .or_else(|| qmap.get("y").and_then(|s| s.parse::<i32>().ok()));

            match node.input_tunnel_attach(&peer, edge, x, y).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/input/tunnel/release") | ("PUT", "/api/input/tunnel/release") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<InputTunnelReleaseBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<InputTunnelReleaseBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected InputTunnelReleaseBody"),
                }
            };

            let reason = parsed_body
                .as_ref()
                .and_then(|b| b.reason.clone())
                .or_else(|| qmap.get("reason").cloned());

            match node.input_tunnel_release(reason.as_deref()).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/input/policy") => json_ok(&node.input_policy_report()),
        ("POST", "/api/input/policy") | ("PUT", "/api/input/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<InputTunnelPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<InputTunnelPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected InputTunnelPolicyBody"),
                }
            };

            let mut policy = node.input_policy();
            if let Some(ref b) = parsed_body {
                if let Some(en) = b.enabled {
                    policy.enabled = en;
                }
                if let Some(inbound) = b.allow_inbound_control {
                    policy.allow_inbound_control = inbound;
                }
                if let Some(kb) = b.allow_keyboard {
                    policy.allow_keyboard = kb;
                }
                if let Some(mouse) = b.allow_mouse {
                    policy.allow_mouse = mouse;
                }
                if let Some(hk) = b.hotkey_release_enabled {
                    policy.hotkey_release_enabled = hk;
                }
                if let Some(to) = b.idle_timeout_secs {
                    policy.idle_timeout_secs = to;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(en) = qmap.get("enable").or_else(|| qmap.get("enabled")) {
                if let Ok(b) = en.parse::<bool>() {
                    policy.enabled = b;
                }
            }
            if let Some(inbound) = qmap
                .get("allow_inbound")
                .or_else(|| qmap.get("allow_inbound_control"))
            {
                if let Ok(b) = inbound.parse::<bool>() {
                    policy.allow_inbound_control = b;
                }
            }
            if let Some(kb) = qmap.get("keyboard").or_else(|| qmap.get("allow_keyboard")) {
                if let Ok(b) = kb.parse::<bool>() {
                    policy.allow_keyboard = b;
                }
            }
            if let Some(mouse) = qmap.get("mouse").or_else(|| qmap.get("allow_mouse")) {
                if let Ok(b) = mouse.parse::<bool>() {
                    policy.allow_mouse = b;
                }
            }
            if let Some(hk) = qmap
                .get("hotkey")
                .or_else(|| qmap.get("hotkey_release_enabled"))
            {
                if let Ok(b) = hk.parse::<bool>() {
                    policy.hotkey_release_enabled = b;
                }
            }
            if let Some(to) = qmap
                .get("timeout")
                .or_else(|| qmap.get("idle_timeout_secs"))
            {
                if let Ok(secs) = to.parse::<u32>() {
                    policy.idle_timeout_secs = secs;
                }
            }
            if let Some(grp) = qmap.get("group").or_else(|| qmap.get("allowed_group")) {
                if !policy.allowed_groups.contains(grp) {
                    policy.allowed_groups.push(grp.clone());
                }
            }
            if let Some(tag) = qmap.get("tag").or_else(|| qmap.get("allowed_tag")) {
                if !policy.allowed_tags.contains(tag) {
                    policy.allowed_tags.push(tag.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_input_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/audit")
        | ("GET", "/api/security/audit")
        | ("GET", "/api/audit/logs")
        | ("POST", "/api/audit/query") => {
            let mut q_cat: Option<String> = None;
            let mut q_sev: Option<String> = None;
            let mut q_lim: Option<usize> = None;
            let mut q_since: Option<u64> = None;

            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(c) = map.get("category").or_else(|| map.get("cat")) {
                    q_cat = Some(c.clone());
                }
                if let Some(s) = map.get("severity").or_else(|| map.get("sev")) {
                    q_sev = Some(s.clone());
                }
                if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
                    q_lim = Some(l);
                }
                if let Some(since) = map
                    .get("since")
                    .or_else(|| map.get("since_ms"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    q_since = Some(since);
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<AuditQueryBody>(body) {
                    Ok(b) => {
                        if b.category.is_some() {
                            q_cat = b.category;
                        }
                        if b.severity.is_some() {
                            q_sev = b.severity;
                        }
                        if b.limit.is_some() {
                            q_lim = b.limit;
                        }
                        if b.since.is_some() {
                            q_since = b.since;
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            let cat = match q_cat.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(c) => match c.parse::<crate::runtime::AuditCategory>() {
                    Ok(cat) => Some(cat),
                    Err(e) => return err(400, &e),
                },
                None => None,
            };

            let sev = match q_sev.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(s) => match s.parse::<crate::runtime::AuditSeverity>() {
                    Ok(sev) => Some(sev),
                    Err(e) => return err(400, &e),
                },
                None => None,
            };

            let report = node.audit_report(cat, sev, q_lim, q_since);
            json_ok(&report)
        }
        ("POST", "/api/audit/clear") | ("DELETE", "/api/audit") | ("DELETE", "/api/audit/logs") => {
            let report = node.clear_audit_log();
            json_ok(&report)
        }
        ("POST", "/api/message/send-multi")
        | ("POST", "/api/msg/send-multi")
        | ("POST", "/api/messages/send-multi") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let req: MessageSendBody = if body.is_empty() {
                MessageSendBody {
                    peer: qmap.get("peer").cloned(),
                    tag: qmap.get("tag").cloned(),
                    group: qmap.get("group").cloned(),
                    device_type: qmap
                        .get("type")
                        .or_else(|| qmap.get("device_type"))
                        .cloned(),
                    r#type: None,
                    all: qmap.get("all").map(|s| s == "true" || s == "1"),
                    text: qmap.get("text").cloned().unwrap_or_default(),
                }
            } else {
                match serde_json::from_slice::<MessageSendBody>(body) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad json"),
                }
            };
            let text = if !req.text.trim().is_empty() {
                req.text.trim()
            } else if let Some(t) = qmap.get("text") {
                t.trim()
            } else {
                return err(400, "message text cannot be empty");
            };

            let is_all = req.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = req
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = req
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = req
                .device_type
                .as_deref()
                .or(req.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });
            let peer = req
                .peer
                .as_deref()
                .or_else(|| qmap.get("peer").map(String::as_str));

            let selector = match crate::store::TargetSelector::from_query(
                peer, tag, group, dev_type, is_all,
            ) {
                Ok(s) => s,
                Err(e) => return err(400, &e),
            };

            match node.send_message_multi(&selector, text).await {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/message/send")
        | ("POST", "/api/msg/send")
        | ("POST", "/api/messages/send") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let req: MessageSendBody = match serde_json::from_slice(body) {
                Ok(b) => b,
                Err(_) => return err(400, "bad json"),
            };
            let is_all = req.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = req
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = req
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = req
                .device_type
                .as_deref()
                .or(req.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });

            if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
                let peer = req
                    .peer
                    .as_deref()
                    .or_else(|| qmap.get("peer").map(String::as_str));
                let selector = match crate::store::TargetSelector::from_query(
                    peer, tag, group, dev_type, is_all,
                ) {
                    Ok(s) => s,
                    Err(e) => return err(400, &e),
                };
                match node.send_message_multi(&selector, &req.text).await {
                    Ok(report) => return json_ok(&report),
                    Err(e) => return err(400, &e.to_string()),
                }
            }

            match node
                .send_message_to_peer(req.peer.as_deref().unwrap_or(""), &req.text)
                .await
            {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/message/history")
        | ("GET", "/api/message/list")
        | ("GET", "/api/messages")
        | ("GET", "/api/messages/history")
        | ("GET", "/api/msg/history")
        | ("GET", "/api/msg/list")
        | ("GET", "/api/msg")
        | ("POST", "/api/message/query")
        | ("POST", "/api/msg/query") => {
            let mut q_peer: Option<String> = None;
            let mut q_limit: Option<usize> = None;
            let mut q_unread_only = false;
            let mut q_mark_read = false;

            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(p) = map
                    .get("peer")
                    .or_else(|| map.get("to"))
                    .or_else(|| map.get("target"))
                {
                    q_peer = Some(p.clone());
                }
                if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
                    q_limit = Some(l);
                }
                if let Some(u) = map.get("unread_only").or_else(|| map.get("unread")) {
                    q_unread_only = u == "true" || u == "1";
                }
                if let Some(m) = map.get("mark_read").or_else(|| map.get("read")) {
                    q_mark_read = m == "true" || m == "1";
                }
            }

            if !body.is_empty() {
                match serde_json::from_slice::<MessageQueryBody>(body) {
                    Ok(b) => {
                        if b.peer.is_some() {
                            q_peer = b.peer;
                        }
                        if b.limit.is_some() {
                            q_limit = b.limit;
                        }
                        if let Some(u) = b.unread_only {
                            q_unread_only = u;
                        }
                        if let Some(m) = b.mark_read {
                            q_mark_read = m;
                        }
                    }
                    Err(_) => return err(400, "bad json"),
                }
            }

            match node.message_history(q_peer.as_deref(), q_limit, q_unread_only, q_mark_read) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/message/clear")
        | ("POST", "/api/messages/clear")
        | ("POST", "/api/msg/clear")
        | ("DELETE", "/api/message")
        | ("DELETE", "/api/messages")
        | ("DELETE", "/api/msg") => {
            let mut q_peer: Option<String> = None;
            if let Some(q) = query_str {
                let map = parse_query_map(q);
                if let Some(p) = map
                    .get("peer")
                    .or_else(|| map.get("to"))
                    .or_else(|| map.get("target"))
                {
                    q_peer = Some(p.clone());
                }
            }
            if !body.is_empty() {
                if let Ok(b) = serde_json::from_slice::<MessageClearBody>(body) {
                    if b.peer.is_some() {
                        q_peer = b.peer;
                    }
                }
            }
            match node.clear_messages(q_peer.as_deref()) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        // Cross-device Resource & URL Handoff endpoints (v1.72 / ADR-065)
        ("POST", "/api/handoff/send-multi") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let req: HandoffSendBody = if body.is_empty() {
                HandoffSendBody {
                    peer: qmap.get("peer").cloned(),
                    url: qmap.get("url").cloned().unwrap_or_default(),
                    title: qmap.get("title").cloned(),
                    auto_open: qmap.get("auto_open").map(|s| s == "true" || s == "1"),
                    tag: qmap.get("tag").cloned(),
                    group: qmap.get("group").cloned(),
                    device_type: qmap
                        .get("type")
                        .or_else(|| qmap.get("device_type"))
                        .cloned(),
                    r#type: None,
                    all: qmap.get("all").map(|s| s == "true" || s == "1"),
                }
            } else {
                match serde_json::from_slice::<HandoffSendBody>(body) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad json, expected HandoffSendBody"),
                }
            };
            let url = if !req.url.trim().is_empty() {
                req.url.trim()
            } else if let Some(u) = qmap.get("url") {
                u.trim()
            } else {
                return err(400, "url cannot be empty");
            };
            let is_all = req.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = req
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = req
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = req
                .device_type
                .as_deref()
                .or(req.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });
            let peer = req
                .peer
                .as_deref()
                .or_else(|| qmap.get("peer").map(String::as_str));
            let auto_open = req.auto_open.unwrap_or(false)
                || qmap
                    .get("auto_open")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false);
            let title = req
                .title
                .as_deref()
                .or_else(|| qmap.get("title").map(String::as_str));

            let selector = match crate::store::TargetSelector::from_query(
                peer, tag, group, dev_type, is_all,
            ) {
                Ok(s) => s,
                Err(e) => return err(400, &e),
            };

            match node
                .handoff_send_multi(&selector, url, title, auto_open)
                .await
            {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/handoff/send") | ("POST", "/api/handoff") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let req: HandoffSendBody = if body.is_empty() {
                HandoffSendBody {
                    peer: qmap.get("peer").cloned(),
                    url: qmap.get("url").cloned().unwrap_or_default(),
                    title: qmap.get("title").cloned(),
                    auto_open: qmap.get("auto_open").map(|s| s == "true" || s == "1"),
                    tag: qmap.get("tag").cloned(),
                    group: qmap.get("group").cloned(),
                    device_type: qmap
                        .get("type")
                        .or_else(|| qmap.get("device_type"))
                        .cloned(),
                    r#type: None,
                    all: qmap.get("all").map(|s| s == "true" || s == "1"),
                }
            } else {
                match serde_json::from_slice::<HandoffSendBody>(body) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad json, expected HandoffSendBody"),
                }
            };
            let url = if !req.url.trim().is_empty() {
                req.url.trim()
            } else if let Some(u) = qmap.get("url") {
                u.trim()
            } else {
                return err(400, "url cannot be empty");
            };
            let is_all = req.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = req
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = req
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = req
                .device_type
                .as_deref()
                .or(req.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });
            let auto_open = req.auto_open.unwrap_or(false)
                || qmap
                    .get("auto_open")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false);
            let title = req
                .title
                .as_deref()
                .or_else(|| qmap.get("title").map(String::as_str));

            if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
                let peer = req
                    .peer
                    .as_deref()
                    .or_else(|| qmap.get("peer").map(String::as_str));
                let selector = match crate::store::TargetSelector::from_query(
                    peer, tag, group, dev_type, is_all,
                ) {
                    Ok(s) => s,
                    Err(e) => return err(400, &e),
                };
                match node
                    .handoff_send_multi(&selector, url, title, auto_open)
                    .await
                {
                    Ok(report) => return json_ok(&report),
                    Err(e) => return err(400, &e.to_string()),
                }
            }

            let target_peer = req
                .peer
                .as_deref()
                .or_else(|| qmap.get("peer").map(String::as_str))
                .unwrap_or("");
            match node.handoff_send(target_peer, url, title, auto_open).await {
                Ok(res) => json_ok(&res),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/handoff/inbox") | ("GET", "/api/handoff") | ("GET", "/api/handoff/list") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let status_filter = qmap
                .get("status")
                .and_then(|s| s.parse::<tetherly_core::HandoffStatus>().ok());
            let list = node.handoff_inbox_list(status_filter);
            json_ok(&serde_json::json!({
                "ok": true,
                "total": list.len(),
                "inbox": list,
                "entries": list,
            }))
        }
        ("POST", "/api/handoff/inbox/open") | ("POST", "/api/handoff/open") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let target_id = qmap.get("id").cloned().or_else(|| {
                if !body.is_empty() {
                    serde_json::from_slice::<HandoffInboxActionBody>(body)
                        .ok()
                        .and_then(|b| b.id)
                } else {
                    None
                }
            });
            let Some(id) = target_id else {
                return err(400, "missing required 'id' parameter");
            };
            match node.handoff_inbox_open(&id) {
                Ok(entry) => json_ok(&serde_json::json!({
                    "ok": true,
                    "id": entry.id,
                    "url": entry.url,
                    "status": entry.status.as_str(),
                    "opened_ms": entry.opened_ms,
                    "entry": entry,
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/handoff/inbox/dismiss") | ("POST", "/api/handoff/dismiss") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let target_id = qmap.get("id").cloned().or_else(|| {
                if !body.is_empty() {
                    serde_json::from_slice::<HandoffInboxActionBody>(body)
                        .ok()
                        .and_then(|b| b.id)
                } else {
                    None
                }
            });
            let Some(id) = target_id else {
                return err(400, "missing required 'id' parameter");
            };
            match node.handoff_inbox_dismiss(&id) {
                Ok(entry) => json_ok(&serde_json::json!({
                    "ok": true,
                    "id": entry.id,
                    "status": entry.status.as_str(),
                    "entry": entry,
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/handoff/inbox/clear")
        | ("DELETE", "/api/handoff/inbox")
        | ("DELETE", "/api/handoff") => match node.handoff_inbox_clear() {
            Ok(cleared) => json_ok(&serde_json::json!({
                "ok": true,
                "cleared": cleared,
            })),
            Err(e) => err(400, &e.to_string()),
        },
        ("GET", "/api/handoff/policy") => json_ok(&node.handoff_policy_report()),
        ("POST", "/api/handoff/policy") | ("PUT", "/api/handoff/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<HandoffPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<HandoffPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected handoff policy fields"),
                }
            };
            let mut policy = node.handoff_policy();

            if let Some(ref b) = parsed_body {
                if let Some(ref a) = b.action {
                    match a.parse::<tetherly_core::HandoffAction>() {
                        Ok(act) => policy.action = act,
                        Err(e) => return err(400, &format!("invalid handoff action '{a}': {e}")),
                    }
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if let Some(max_entries) = b.max_inbox_entries {
                    policy.max_inbox_entries = max_entries;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(act_str) = qmap.get("action") {
                match act_str.parse::<tetherly_core::HandoffAction>() {
                    Ok(act) => policy.action = act,
                    Err(e) => return err(400, &format!("invalid handoff action '{act_str}': {e}")),
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if let Some(max_str) = qmap
                .get("max_inbox_entries")
                .or_else(|| qmap.get("max_entries"))
            {
                if let Ok(m) = max_str.parse::<usize>() {
                    policy.max_inbox_entries = m;
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_handoff_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/media/command")
        | ("POST", "/api/media/cmd")
        | ("POST", "/api/media/send") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let req: MediaCommandBody = if body.is_empty() {
                MediaCommandBody {
                    peer: qmap.get("peer").cloned(),
                    target: qmap.get("target").cloned(),
                    command: qmap.get("command").or_else(|| qmap.get("cmd")).cloned(),
                    action: qmap.get("action").cloned(),
                    volume: qmap
                        .get("volume")
                        .or_else(|| qmap.get("volume_percent"))
                        .and_then(|v| v.parse::<u8>().ok()),
                    volume_percent: None,
                    offset_ms: qmap
                        .get("offset_ms")
                        .or_else(|| qmap.get("offset"))
                        .and_then(|v| v.parse::<i64>().ok()),
                    target_player: qmap
                        .get("target_player")
                        .or_else(|| qmap.get("player"))
                        .cloned(),
                    tag: qmap.get("tag").cloned(),
                    group: qmap.get("group").cloned(),
                    device_type: qmap
                        .get("type")
                        .or_else(|| qmap.get("device_type"))
                        .cloned(),
                    r#type: None,
                    all: qmap.get("all").map(|s| s == "true" || s == "1"),
                }
            } else {
                match serde_json::from_slice::<MediaCommandBody>(body) {
                    Ok(b) => b,
                    Err(_) => return err(400, "bad json, expected MediaCommandBody"),
                }
            };

            let cmd_str = req
                .command
                .or(req.action)
                .or_else(|| {
                    qmap.get("command")
                        .or_else(|| qmap.get("cmd"))
                        .or_else(|| qmap.get("action"))
                        .cloned()
                })
                .unwrap_or_default();
            let cmd_str = cmd_str.trim().to_ascii_lowercase();
            if cmd_str.is_empty() {
                return err(400, "missing required 'command' or 'action'");
            }

            let command = match cmd_str.as_str() {
                "play" => MediaCommand::Play,
                "pause" => MediaCommand::Pause,
                "play_pause" | "playpause" | "toggle" => MediaCommand::PlayPause,
                "stop" => MediaCommand::Stop,
                "next" => MediaCommand::Next,
                "prev" | "previous" => MediaCommand::Previous,
                "seek" => {
                    let offset = req
                        .offset_ms
                        .or_else(|| {
                            qmap.get("offset_ms")
                                .or_else(|| qmap.get("offset"))
                                .and_then(|s| s.parse::<i64>().ok())
                        })
                        .unwrap_or(0);
                    MediaCommand::Seek { offset_ms: offset }
                }
                "volume" | "set_volume" => {
                    let vol = req
                        .volume
                        .or(req.volume_percent)
                        .or_else(|| {
                            qmap.get("volume")
                                .or_else(|| qmap.get("volume_percent"))
                                .and_then(|s| s.parse::<u8>().ok())
                        })
                        .unwrap_or(0);
                    MediaCommand::SetVolume {
                        volume_percent: vol,
                    }
                }
                "toggle_mute" | "mute" => MediaCommand::ToggleMute,
                other => return err(400, &format!("unsupported media command: '{other}'")),
            };

            let target_player = req.target_player.or_else(|| {
                qmap.get("target_player")
                    .or_else(|| qmap.get("player"))
                    .cloned()
            });

            let is_all = req.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = req
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = req
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = req
                .device_type
                .as_deref()
                .or(req.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });
            let peer = req.peer.as_deref().or(req.target.as_deref()).or_else(|| {
                qmap.get("peer")
                    .or_else(|| qmap.get("target"))
                    .map(String::as_str)
            });

            if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
                let selector = match crate::store::TargetSelector::from_query(
                    peer, tag, group, dev_type, is_all,
                ) {
                    Ok(s) => s,
                    Err(e) => return err(400, &e),
                };
                match node
                    .media_send_command_multi(&selector, command, target_player)
                    .await
                {
                    Ok(report) => return json_ok(&report),
                    Err(e) => return err(400, &e.to_string()),
                }
            }

            let target_peer = peer.unwrap_or("");
            match node
                .media_send_command(target_peer, command, target_player)
                .await
            {
                Ok(res) => json_ok(&res),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/media/state") | ("GET", "/api/media/status") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let peer = qmap
                .get("peer")
                .or_else(|| qmap.get("target"))
                .map(String::as_str);
            match node.media_get_state(peer) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/media/state") | ("PUT", "/api/media/state") => {
            let parsed_body = match serde_json::from_slice::<MediaStateUpdateBody>(body) {
                Ok(b) => b,
                Err(_) => return err(400, "bad json, expected MediaStateUpdateBody"),
            };
            let mut current = match node.media_get_state(None) {
                Ok(rep) => rep.state.unwrap_or_default(),
                Err(e) => return err(400, &e.to_string()),
            };

            if let Some(pid) = parsed_body.player_id {
                current.player_id = pid;
            }
            if let Some(ref st_str) = parsed_body.state {
                match st_str.parse::<PlaybackState>() {
                    Ok(ps) => current.state = ps,
                    Err(e) => return err(400, &e),
                }
            }
            if let Some(title) = parsed_body.title {
                current.title = title;
            }
            if let Some(artist) = parsed_body.artist {
                current.artist = artist;
            }
            if let Some(album) = parsed_body.album {
                current.album = album;
            }
            if let Some(pos) = parsed_body.position_ms {
                current.position_ms = pos;
            }
            if let Some(dur) = parsed_body.duration_ms {
                current.duration_ms = dur;
            }
            if let Some(vol) = parsed_body.volume_percent {
                current.volume_percent = vol;
            }
            if let Some(muted) = parsed_body.muted {
                current.muted = muted;
            }
            if let Some(cp) = parsed_body.can_play {
                current.can_play = cp;
            }
            if let Some(cp) = parsed_body.can_pause {
                current.can_pause = cp;
            }
            if let Some(cgn) = parsed_body.can_go_next {
                current.can_go_next = cgn;
            }
            if let Some(cgp) = parsed_body.can_go_previous {
                current.can_go_previous = cgp;
            }
            if let Some(cs) = parsed_body.can_seek {
                current.can_seek = cs;
            }

            match node.media_update_local_state(current) {
                Ok(st) => json_ok(&serde_json::json!({
                    "ok": true,
                    "state": st,
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/media/policy") => json_ok(&node.media_policy_report()),
        ("POST", "/api/media/policy") | ("PUT", "/api/media/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<MediaPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<MediaPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected MediaPolicyBody"),
                }
            };
            let mut policy = node.media_policy();

            if let Some(ref b) = parsed_body {
                if let Some(ref m) = b.mode {
                    match m.parse::<tetherly_core::MediaControlMode>() {
                        Ok(mode) => policy.mode = mode,
                        Err(e) => {
                            return err(400, &format!("invalid media control mode '{m}': {e}"))
                        }
                    }
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if let Some(limit) = b.max_volume_limit {
                    policy.max_volume_limit = limit.min(100);
                }
                if let Some(seek) = b.allow_seek {
                    policy.allow_seek = seek;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(m_str) = qmap.get("mode") {
                match m_str.parse::<tetherly_core::MediaControlMode>() {
                    Ok(mode) => policy.mode = mode,
                    Err(e) => {
                        return err(400, &format!("invalid media control mode '{m_str}': {e}"))
                    }
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if let Some(lim_str) = qmap
                .get("max_volume_limit")
                .or_else(|| qmap.get("max_volume"))
            {
                if let Ok(l) = lim_str.parse::<u8>() {
                    policy.max_volume_limit = l.min(100);
                }
            }
            if let Some(seek_str) = qmap.get("allow_seek") {
                if let Ok(s) = seek_str.parse::<bool>() {
                    policy.allow_seek = s;
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_media_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/battery/state")
        | ("GET", "/api/battery/status")
        | ("GET", "/api/power/state")
        | ("GET", "/api/power/status") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let peer = qmap
                .get("peer")
                .or_else(|| qmap.get("target"))
                .map(String::as_str);
            match node.battery_get_state(peer) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/battery/state")
        | ("PUT", "/api/battery/state")
        | ("POST", "/api/power/state")
        | ("PUT", "/api/power/state") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<BatteryStateUpdateBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<BatteryStateUpdateBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected BatteryStateUpdateBody"),
                }
            };
            let mut current = match node.battery_get_state(None) {
                Ok(rep) => rep.state.unwrap_or_default(),
                Err(e) => return err(400, &e.to_string()),
            };

            if let Some(ref b) = parsed_body {
                if let Some(lvl) = b.battery_level {
                    current.battery_level = lvl.min(100);
                }
                if let Some(chg) = b.is_charging {
                    current.is_charging = chg;
                }
                if let Some(ref src_str) = b.power_source {
                    match src_str.parse::<PowerSource>() {
                        Ok(ps) => current.power_source = ps,
                        Err(e) => return err(400, &e),
                    }
                }
                if let Some(saver) = b.battery_saver {
                    current.battery_saver = saver;
                }
                if let Some(hp) = b.health_percent {
                    current.health_percent = Some(hp.min(100));
                }
            }

            if let Some(lvl_str) = qmap
                .get("level")
                .or_else(|| qmap.get("battery_level"))
                .or_else(|| qmap.get("percent"))
            {
                if let Ok(lvl) = lvl_str.parse::<u8>() {
                    current.battery_level = lvl.min(100);
                }
            }
            if let Some(chg_str) = qmap.get("charging").or_else(|| qmap.get("is_charging")) {
                if let Ok(chg) = chg_str.parse::<bool>() {
                    current.is_charging = chg;
                }
            }
            if let Some(src_str) = qmap.get("source").or_else(|| qmap.get("power_source")) {
                match src_str.parse::<PowerSource>() {
                    Ok(ps) => current.power_source = ps,
                    Err(e) => return err(400, &e),
                }
            }
            if let Some(saver_str) = qmap.get("saver").or_else(|| qmap.get("battery_saver")) {
                if let Ok(saver) = saver_str.parse::<bool>() {
                    current.battery_saver = saver;
                }
            }
            if let Some(health_str) = qmap.get("health").or_else(|| qmap.get("health_percent")) {
                if let Ok(hp) = health_str.parse::<u8>() {
                    current.health_percent = Some(hp.min(100));
                }
            }

            match node.battery_update_local_state(current).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/battery/policy") | ("GET", "/api/power/policy") => {
            json_ok(&node.battery_policy_report())
        }
        ("POST", "/api/battery/policy")
        | ("PUT", "/api/battery/policy")
        | ("POST", "/api/power/policy")
        | ("PUT", "/api/power/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<BatteryPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<BatteryPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected BatteryPolicyBody"),
                }
            };
            let mut policy = node.battery_policy();

            if let Some(ref b) = parsed_body {
                if let Some(low) = b.low_battery_threshold {
                    policy.low_battery_threshold = low.min(100);
                }
                if let Some(crit) = b.critical_battery_threshold {
                    policy.critical_battery_threshold = crit.min(100);
                }
                if let Some(alert) = b.alert_on_low_battery {
                    policy.alert_on_low_battery = alert;
                }
                if let Some(throttle) = b.auto_throttle_on_battery_saver {
                    policy.auto_throttle_on_battery_saver = throttle;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(low_str) = qmap
                .get("low_battery_threshold")
                .or_else(|| qmap.get("low_threshold"))
                .or_else(|| qmap.get("low"))
            {
                if let Ok(l) = low_str.parse::<u8>() {
                    policy.low_battery_threshold = l.min(100);
                }
            }
            if let Some(crit_str) = qmap
                .get("critical_battery_threshold")
                .or_else(|| qmap.get("critical_threshold"))
                .or_else(|| qmap.get("critical"))
            {
                if let Ok(c) = crit_str.parse::<u8>() {
                    policy.critical_battery_threshold = c.min(100);
                }
            }
            if let Some(alert_str) = qmap
                .get("alert_on_low_battery")
                .or_else(|| qmap.get("alert"))
            {
                if let Ok(a) = alert_str.parse::<bool>() {
                    policy.alert_on_low_battery = a;
                }
            }
            if let Some(throttle_str) = qmap
                .get("auto_throttle_on_battery_saver")
                .or_else(|| qmap.get("throttle"))
            {
                if let Ok(t) = throttle_str.parse::<bool>() {
                    policy.auto_throttle_on_battery_saver = t;
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_battery_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/locate/status")
        | ("GET", "/api/locate/state")
        | ("GET", "/api/find/status")
        | ("GET", "/api/find/state") => json_ok(&node.locate_status()),
        ("POST", "/api/locate/ring")
        | ("PUT", "/api/locate/ring")
        | ("POST", "/api/find/ring")
        | ("PUT", "/api/find/ring") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<LocateRingBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<LocateRingBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected LocateRingBody"),
                }
            };

            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("target").cloned());

            let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
                return err(400, "missing target peer (specify 'peer' or 'target')");
            };

            let duration_secs = parsed_body
                .as_ref()
                .and_then(|b| b.duration_secs)
                .or_else(|| {
                    qmap.get("duration")
                        .or_else(|| qmap.get("duration_secs"))
                        .and_then(|s| s.parse::<u32>().ok())
                });

            let pattern_str = parsed_body
                .as_ref()
                .and_then(|b| b.pattern.as_deref())
                .or_else(|| qmap.get("pattern").map(|s| s.as_str()));

            let pattern = match pattern_str {
                Some(p) => match p.parse::<tetherly_core::LocatePattern>() {
                    Ok(pat) => Some(pat),
                    Err(e) => return err(400, &format!("invalid locate pattern: {e}")),
                },
                None => None,
            };

            let override_silent = parsed_body
                .as_ref()
                .and_then(|b| b.override_silent)
                .or_else(|| {
                    qmap.get("override_silent")
                        .or_else(|| qmap.get("override"))
                        .and_then(|s| s.parse::<bool>().ok())
                });

            let message = parsed_body
                .as_ref()
                .and_then(|b| b.message.clone())
                .or_else(|| qmap.get("message").cloned());

            match node
                .locate_ring(
                    &peer_target,
                    duration_secs,
                    pattern,
                    override_silent,
                    message,
                )
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/locate/stop")
        | ("PUT", "/api/locate/stop")
        | ("POST", "/api/find/stop")
        | ("PUT", "/api/find/stop") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<LocateStopBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<LocateStopBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected LocateStopBody"),
                }
            };

            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("target").cloned());

            let request_id = parsed_body
                .as_ref()
                .and_then(|b| b.request_id.clone())
                .or_else(|| qmap.get("id").cloned())
                .or_else(|| qmap.get("request_id").cloned());

            let reason = parsed_body
                .as_ref()
                .and_then(|b| b.reason.clone())
                .or_else(|| qmap.get("reason").cloned());

            match node.locate_stop(peer.as_deref(), request_id, reason).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/locate/policy") | ("GET", "/api/find/policy") => {
            json_ok(&node.locator_policy_report())
        }
        ("POST", "/api/locate/policy")
        | ("PUT", "/api/locate/policy")
        | ("POST", "/api/find/policy")
        | ("PUT", "/api/find/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<LocatePolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<LocatePolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected LocatePolicyBody"),
                }
            };

            let mut policy = node.locator_policy();

            if let Some(ref b) = parsed_body {
                if let Some(enabled) = b.enabled {
                    policy.enabled = enabled;
                }
                if let Some(override_silent) = b.allow_override_silent {
                    policy.allow_override_silent = override_silent;
                }
                if let Some(max_dur) = b.max_duration_secs {
                    policy.max_duration_secs = max_dur.clamp(1, 60);
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(en_str) = qmap.get("enabled") {
                if let Ok(en) = en_str.parse::<bool>() {
                    policy.enabled = en;
                }
            }
            if let Some(ov_str) = qmap
                .get("allow_override_silent")
                .or_else(|| qmap.get("override_silent"))
                .or_else(|| qmap.get("override"))
            {
                if let Ok(ov) = ov_str.parse::<bool>() {
                    policy.allow_override_silent = ov;
                }
            }
            if let Some(max_dur_str) = qmap
                .get("max_duration_secs")
                .or_else(|| qmap.get("max_duration"))
            {
                if let Ok(md) = max_dur_str.parse::<u32>() {
                    policy.max_duration_secs = md.clamp(1, 60);
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_locator_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/action/local") => json_ok(&node.local_action_report()),
        ("POST", "/api/action/local") | ("PUT", "/api/action/local") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ActionLocalBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<ActionLocalBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected ActionLocalBody"),
                }
            };

            let id = parsed_body
                .as_ref()
                .and_then(|b| b.id.clone())
                .or_else(|| qmap.get("id").cloned());
            let Some(action_id) = id.filter(|s| !s.trim().is_empty()) else {
                return err(400, "missing 'id' for action");
            };

            let name = parsed_body
                .as_ref()
                .and_then(|b| b.name.clone())
                .or_else(|| qmap.get("name").cloned())
                .unwrap_or_else(|| action_id.clone());

            let description = parsed_body
                .as_ref()
                .and_then(|b| b.description.clone())
                .or_else(|| {
                    qmap.get("description")
                        .or_else(|| qmap.get("desc"))
                        .cloned()
                });

            let icon = parsed_body
                .as_ref()
                .and_then(|b| b.icon.clone())
                .or_else(|| qmap.get("icon").cloned());

            let command = parsed_body
                .as_ref()
                .and_then(|b| b.command.clone())
                .or_else(|| qmap.get("command").or_else(|| qmap.get("cmd")).cloned());
            let Some(cmd) = command.filter(|s| !s.trim().is_empty()) else {
                return err(400, "missing 'command' for action");
            };

            let args = parsed_body
                .as_ref()
                .and_then(|b| b.args.clone())
                .or_else(|| {
                    qmap.get("args").map(|s| {
                        s.split(',')
                            .map(|arg| arg.trim().to_string())
                            .filter(|arg| !arg.is_empty())
                            .collect()
                    })
                })
                .unwrap_or_default();

            let timeout_secs = parsed_body
                .as_ref()
                .and_then(|b| b.timeout_secs)
                .or_else(|| {
                    qmap.get("timeout_secs")
                        .or_else(|| qmap.get("timeout"))
                        .and_then(|s| s.parse::<u32>().ok())
                })
                .unwrap_or(10);

            let enabled = parsed_body
                .as_ref()
                .and_then(|b| b.enabled)
                .or_else(|| qmap.get("enabled").and_then(|s| s.parse::<bool>().ok()))
                .unwrap_or(true);

            let def = LocalActionDefinition {
                id: action_id,
                name,
                description,
                icon,
                command: cmd,
                args,
                timeout_secs,
                enabled,
            };

            match node.add_local_action(def) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("DELETE", "/api/action/local") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ActionLocalBody> = if body.is_empty() {
                None
            } else {
                serde_json::from_slice::<ActionLocalBody>(body).ok()
            };

            let id = parsed_body
                .as_ref()
                .and_then(|b| b.id.clone())
                .or_else(|| qmap.get("id").cloned());
            let Some(action_id) = id.filter(|s| !s.trim().is_empty()) else {
                return err(400, "missing 'id' for action to delete");
            };

            match node.remove_local_action(&action_id) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/action/remote") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let peer = qmap.get("peer").or_else(|| qmap.get("target"));
            let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
                return err(400, "missing target peer (specify 'peer' or 'target')");
            };
            match node.remote_actions(peer_target) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/action/remote/refresh") | ("PUT", "/api/action/remote/refresh") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ActionTriggerBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<ActionTriggerBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected ActionTriggerBody"),
                }
            };
            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("target").cloned());
            let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
                return err(400, "missing target peer (specify 'peer' or 'target')");
            };
            match node.query_remote_actions(&peer_target).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/action/trigger")
        | ("PUT", "/api/action/trigger")
        | ("POST", "/api/action/run")
        | ("PUT", "/api/action/run") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ActionTriggerBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<ActionTriggerBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected ActionTriggerBody"),
                }
            };

            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("target").cloned());
            let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
                return err(400, "missing target peer (specify 'peer' or 'target')");
            };

            let action_id = parsed_body
                .as_ref()
                .and_then(|b| b.action_id.clone())
                .or_else(|| qmap.get("action_id").cloned())
                .or_else(|| qmap.get("id").cloned());
            let Some(act_id) = action_id.filter(|s| !s.trim().is_empty()) else {
                return err(400, "missing 'action_id' (or 'id')");
            };

            match node.trigger_remote_action(&peer_target, &act_id).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/action/history") => json_ok(&node.action_history()),
        ("GET", "/api/action/policy") => json_ok(&node.action_policy_report()),
        ("POST", "/api/action/policy") | ("PUT", "/api/action/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<ActionPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<ActionPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected ActionPolicyBody"),
                }
            };

            let mut policy = node.action_policy();
            if let Some(ref b) = parsed_body {
                if let Some(ref m_str) = b.mode {
                    match m_str.parse::<ActionAccessMode>() {
                        Ok(m) => policy.mode = m,
                        Err(e) => return err(400, &format!("invalid action mode: {e}")),
                    }
                }
                if let Some(timeout) = b.max_timeout_secs {
                    policy.max_timeout_secs = ActionValidator::clamp_timeout(timeout);
                }
                if let Some(exp) = b.expose_action_list {
                    policy.expose_action_list = exp;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(m_str) = qmap.get("mode") {
                match m_str.parse::<ActionAccessMode>() {
                    Ok(m) => policy.mode = m,
                    Err(e) => return err(400, &format!("invalid action mode: {e}")),
                }
            }
            if let Some(t_str) = qmap.get("timeout").or_else(|| qmap.get("max_timeout_secs")) {
                if let Ok(t) = t_str.parse::<u32>() {
                    policy.max_timeout_secs = ActionValidator::clamp_timeout(t);
                }
            }
            if let Some(exp_str) = qmap
                .get("expose")
                .or_else(|| qmap.get("expose_action_list"))
            {
                if let Ok(exp) = exp_str.parse::<bool>() {
                    policy.expose_action_list = exp;
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_action_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/focus/state") => json_ok(&node.focus_status()),
        ("POST", "/api/focus/state") | ("PUT", "/api/focus/state") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<FocusSetBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<FocusSetBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected FocusSetBody"),
                }
            };

            let enabled = parsed_body
                .as_ref()
                .and_then(|b| b.enabled)
                .or_else(|| qmap.get("enabled").and_then(|s| s.parse::<bool>().ok()))
                .unwrap_or(true);

            let mode_str = parsed_body
                .as_ref()
                .and_then(|b| b.mode.clone())
                .or_else(|| qmap.get("mode").cloned());
            let mode = if let Some(ref m_str) = mode_str {
                match m_str.parse::<tetherly_core::FocusMode>() {
                    Ok(m) => Some(m),
                    Err(e) => return err(400, &format!("invalid focus mode '{m_str}': {e}")),
                }
            } else {
                None
            };

            let duration_mins = parsed_body
                .as_ref()
                .and_then(|b| b.duration_mins)
                .or_else(|| {
                    qmap.get("duration_mins")
                        .or_else(|| qmap.get("duration"))
                        .and_then(|s| s.parse::<u32>().ok())
                });

            let reason = parsed_body
                .as_ref()
                .and_then(|b| b.reason.clone())
                .or_else(|| qmap.get("reason").cloned());

            let broadcast = parsed_body
                .as_ref()
                .and_then(|b| b.broadcast)
                .or_else(|| qmap.get("broadcast").and_then(|s| s.parse::<bool>().ok()))
                .unwrap_or(true);

            match node
                .set_focus_state(enabled, mode, duration_mins, reason, broadcast)
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/focus/peers") => json_ok(&node.peer_focus_states()),
        ("POST", "/api/focus/sync") | ("PUT", "/api/focus/sync") => {
            match node.request_peers_focus_sync().await {
                Ok(sent) => json_ok(&serde_json::json!({
                    "ok": true,
                    "sync_requests_sent": sent,
                    "message": "focus sync request dispatched to peers"
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/focus/policy") => json_ok(&node.focus_policy_report()),
        ("POST", "/api/focus/policy") | ("PUT", "/api/focus/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<FocusPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<FocusPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected FocusPolicyBody"),
                }
            };

            let mut policy = node.focus_policy();
            if let Some(ref b) = parsed_body {
                if let Some(en) = b.enabled {
                    policy.enabled = en;
                }
                if let Some(ref m_str) = b.default_mode {
                    match m_str.parse::<tetherly_core::FocusMode>() {
                        Ok(m) => policy.default_mode = m,
                        Err(e) => {
                            return err(400, &format!("invalid default focus mode '{m_str}': {e}"))
                        }
                    }
                }
                if let Some(m) = b.mirror_inbound_focus {
                    policy.mirror_inbound_focus = m;
                }
                if let Some(brk) = b.allow_emergency_breakthrough {
                    policy.allow_emergency_breakthrough = brk;
                }
                if let Some(qh) = b.quiet_hours_enabled {
                    policy.quiet_hours_enabled = qh;
                }
                if let Some(ref s_str) = b.quiet_hours_start {
                    if let Some(start_min) = parse_hh_mm(s_str) {
                        policy.quiet_hours_start_minute = start_min;
                    } else {
                        return err(400, "invalid quiet_hours_start format (expected HH:MM)");
                    }
                } else if let Some(s_min) = b.quiet_hours_start_minute {
                    policy.quiet_hours_start_minute = s_min % 1440;
                }
                if let Some(ref e_str) = b.quiet_hours_end {
                    if let Some(end_min) = parse_hh_mm(e_str) {
                        policy.quiet_hours_end_minute = end_min;
                    } else {
                        return err(400, "invalid quiet_hours_end format (expected HH:MM)");
                    }
                } else if let Some(e_min) = b.quiet_hours_end_minute {
                    policy.quiet_hours_end_minute = e_min % 1440;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            // Also support query params for quick CLI / curl access
            if let Some(en) = qmap.get("enabled").or_else(|| qmap.get("enable")) {
                if let Ok(b) = en.parse::<bool>() {
                    policy.enabled = b;
                }
            }
            if let Some(m_str) = qmap.get("default_mode").or_else(|| qmap.get("mode")) {
                match m_str.parse::<tetherly_core::FocusMode>() {
                    Ok(m) => policy.default_mode = m,
                    Err(e) => {
                        return err(400, &format!("invalid default focus mode '{m_str}': {e}"))
                    }
                }
            }
            if let Some(m) = qmap
                .get("mirror")
                .or_else(|| qmap.get("mirror_inbound_focus"))
            {
                if let Ok(b) = m.parse::<bool>() {
                    policy.mirror_inbound_focus = b;
                }
            }
            if let Some(b) = qmap
                .get("breakthrough")
                .or_else(|| qmap.get("allow_emergency_breakthrough"))
            {
                if let Ok(v) = b.parse::<bool>() {
                    policy.allow_emergency_breakthrough = v;
                }
            }
            if let Some(qh) = qmap
                .get("quiet_hours")
                .or_else(|| qmap.get("quiet_hours_enabled"))
            {
                if let Ok(b) = qh.parse::<bool>() {
                    policy.quiet_hours_enabled = b;
                }
            }
            if let Some(s_str) = qmap.get("start").or_else(|| qmap.get("quiet_hours_start")) {
                if let Some(start_min) = parse_hh_mm(s_str) {
                    policy.quiet_hours_start_minute = start_min;
                } else if let Ok(min) = s_str.parse::<u16>() {
                    policy.quiet_hours_start_minute = min % 1440;
                }
            }
            if let Some(e_str) = qmap.get("end").or_else(|| qmap.get("quiet_hours_end")) {
                if let Some(end_min) = parse_hh_mm(e_str) {
                    policy.quiet_hours_end_minute = end_min;
                } else if let Ok(min) = e_str.parse::<u16>() {
                    policy.quiet_hours_end_minute = min % 1440;
                }
            }
            if let Some(g) = qmap.get("group") {
                if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
                    policy.allowed_groups.push(g.clone());
                }
            }
            if let Some(t) = qmap.get("tag") {
                if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
                    policy.allowed_tags.push(t.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_focus_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/topology/layout") => json_ok(&node.topology_layout()),
        ("POST", "/api/topology/link") | ("PUT", "/api/topology/link") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<TopologyLinkBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<TopologyLinkBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected TopologyLinkBody"),
                }
            };

            let edge_str = parsed_body
                .as_ref()
                .and_then(|b| b.edge.clone())
                .or_else(|| qmap.get("edge").cloned());
            let edge = match edge_str {
                Some(s) => match s.parse::<DisplayEdge>() {
                    Ok(e) => e,
                    Err(e) => return err(400, &format!("invalid edge '{s}': {e}")),
                },
                None => return err(400, "missing 'edge' parameter"),
            };

            let peer = parsed_body
                .as_ref()
                .and_then(|b| b.peer.clone())
                .or_else(|| qmap.get("peer").cloned())
                .or_else(|| qmap.get("peer_id").cloned())
                .or_else(|| qmap.get("target").cloned());
            let peer = match peer {
                Some(p) if !p.trim().is_empty() => p,
                _ => return err(400, "missing 'peer' parameter"),
            };

            let peer_edge_str = parsed_body
                .as_ref()
                .and_then(|b| b.peer_edge.clone())
                .or_else(|| qmap.get("peer_edge").cloned());
            let peer_edge = match peer_edge_str {
                Some(s) => match s.parse::<DisplayEdge>() {
                    Ok(e) => Some(e),
                    Err(e) => return err(400, &format!("invalid peer_edge '{s}': {e}")),
                },
                None => None,
            };

            let offset = parsed_body
                .as_ref()
                .and_then(|b| b.offset)
                .or_else(|| qmap.get("offset").and_then(|s| s.parse::<i32>().ok()));

            match node
                .topology_link_neighbor(edge, &peer, peer_edge, offset)
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/topology/unlink")
        | ("DELETE", "/api/topology/link")
        | ("DELETE", "/api/topology/unlink") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<TopologyUnlinkBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<TopologyUnlinkBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected TopologyUnlinkBody"),
                }
            };

            let edge_str = parsed_body
                .as_ref()
                .and_then(|b| b.edge.clone())
                .or_else(|| qmap.get("edge").cloned());
            let edge = match edge_str {
                Some(s) => match s.parse::<DisplayEdge>() {
                    Ok(e) => e,
                    Err(e) => return err(400, &format!("invalid edge '{s}': {e}")),
                },
                None => return err(400, "missing 'edge' parameter"),
            };

            match node.topology_unlink_neighbor(edge).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("POST", "/api/topology/config") | ("PUT", "/api/topology/config") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<TopologyConfigBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<TopologyConfigBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected TopologyConfigBody"),
                }
            };

            let resistance_px = parsed_body
                .as_ref()
                .and_then(|b| b.resistance_px)
                .or_else(|| {
                    qmap.get("resistance_px")
                        .or_else(|| qmap.get("resistance"))
                        .and_then(|s| s.parse::<u32>().ok())
                });

            let delay_ms = parsed_body.as_ref().and_then(|b| b.delay_ms).or_else(|| {
                qmap.get("delay_ms")
                    .or_else(|| qmap.get("delay"))
                    .and_then(|s| s.parse::<u32>().ok())
            });

            let corner_lock_size_px = parsed_body
                .as_ref()
                .and_then(|b| b.corner_lock_size_px)
                .or_else(|| {
                    qmap.get("corner_lock_size_px")
                        .or_else(|| qmap.get("corner_lock"))
                        .and_then(|s| s.parse::<u32>().ok())
                });

            let wrap_around = parsed_body
                .as_ref()
                .and_then(|b| b.wrap_around)
                .or_else(|| {
                    qmap.get("wrap_around")
                        .or_else(|| qmap.get("wrap"))
                        .and_then(|s| s.parse::<bool>().ok())
                });

            let width = parsed_body
                .as_ref()
                .and_then(|b| b.width)
                .or_else(|| qmap.get("width").and_then(|s| s.parse::<u32>().ok()));

            let height = parsed_body
                .as_ref()
                .and_then(|b| b.height)
                .or_else(|| qmap.get("height").and_then(|s| s.parse::<u32>().ok()));

            let scale_factor = parsed_body
                .as_ref()
                .and_then(|b| b.scale_factor)
                .or_else(|| {
                    qmap.get("scale_factor")
                        .or_else(|| qmap.get("scale"))
                        .and_then(|s| s.parse::<f32>().ok())
                });

            match node
                .topology_update_config(
                    resistance_px,
                    delay_ms,
                    corner_lock_size_px,
                    wrap_around,
                    width,
                    height,
                    scale_factor,
                )
                .await
            {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/topology/peers") => json_ok(&node.topology_peers()),
        ("POST", "/api/topology/sync") | ("PUT", "/api/topology/sync") => {
            let sync_res = node.topology_request_sync().await;
            let bc_res = node.topology_broadcast_state().await;
            match (sync_res, bc_res) {
                (Ok(sent), Ok(bc)) => json_ok(&serde_json::json!({
                    "ok": true,
                    "sync_requests_sent": sent,
                    "broadcasts_sent": bc,
                    "broadcast_count": bc,
                    "message": "topology sync requested and local state broadcast"
                })),
                (Err(e), _) | (_, Err(e)) => err(400, &e.to_string()),
            }
        }
        ("GET", "/api/topology/policy") => json_ok(&node.topology_policy_report()),
        ("POST", "/api/topology/policy") | ("PUT", "/api/topology/policy") => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let parsed_body: Option<TopologyPolicyBody> = if body.is_empty() {
                None
            } else {
                match serde_json::from_slice::<TopologyPolicyBody>(body) {
                    Ok(b) => Some(b),
                    Err(_) => return err(400, "bad json, expected TopologyPolicyBody"),
                }
            };

            let mut policy = node.topology_policy();
            if let Some(ref b) = parsed_body {
                if let Some(en) = b.enabled {
                    policy.enabled = en;
                }
                if let Some(inbound) = b.allow_inbound_control {
                    policy.allow_inbound_control = inbound;
                }
                if let Some(auto) = b.auto_link_paired_peers {
                    policy.auto_link_paired_peers = auto;
                }
                if let Some(ref grps) = b.allowed_groups {
                    policy.allowed_groups = grps.clone();
                }
                if let Some(ref tags) = b.allowed_tags {
                    policy.allowed_tags = tags.clone();
                }
                if let Some(ref types) = b.allowed_types {
                    let mut dtypes = Vec::new();
                    for t in types {
                        match t.parse::<crate::store::DeviceType>() {
                            Ok(dt) => dtypes.push(dt),
                            Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                        }
                    }
                    policy.allowed_types = dtypes;
                }
                if b.clear_filters == Some(true) {
                    policy.allowed_groups.clear();
                    policy.allowed_tags.clear();
                    policy.allowed_types.clear();
                }
            }

            if let Some(en) = qmap.get("enabled").or_else(|| qmap.get("enable")) {
                if let Ok(b) = en.parse::<bool>() {
                    policy.enabled = b;
                }
            }
            if let Some(inbound) = qmap
                .get("allow_inbound")
                .or_else(|| qmap.get("allow_inbound_control"))
            {
                if let Ok(b) = inbound.parse::<bool>() {
                    policy.allow_inbound_control = b;
                }
            }
            if let Some(auto) = qmap
                .get("auto_link")
                .or_else(|| qmap.get("auto_link_paired_peers"))
            {
                if let Ok(b) = auto.parse::<bool>() {
                    policy.auto_link_paired_peers = b;
                }
            }
            if let Some(grp) = qmap.get("group").or_else(|| qmap.get("allowed_group")) {
                if !policy.allowed_groups.contains(grp) {
                    policy.allowed_groups.push(grp.clone());
                }
            }
            if let Some(tag) = qmap.get("tag").or_else(|| qmap.get("allowed_tag")) {
                if !policy.allowed_tags.contains(tag) {
                    policy.allowed_tags.push(tag.clone());
                }
            }
            if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
                match dt_str.parse::<crate::store::DeviceType>() {
                    Ok(dt) => {
                        if !policy.allowed_types.contains(&dt) {
                            policy.allowed_types.push(dt);
                        }
                    }
                    Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
                }
            }
            if qmap
                .get("clear_filters")
                .map(|s| s == "true" || s == "1")
                .unwrap_or(false)
            {
                policy.allowed_groups.clear();
                policy.allowed_tags.clear();
                policy.allowed_types.clear();
            }

            match node.set_topology_policy(policy) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
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
            let profile = node.peer_profile(&tp.device_id);
            TrustedPeerView {
                device_id: tp.device_id.to_string(),
                alias: tp.alias,
                device_type: profile.device_type.as_str().to_string(),
                tags: profile.tags,
                group: profile.group,
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
                let profile = node.peer_profile(&p.device_id);
                PeerView {
                    device_id: p.device_id.to_string(),
                    name: p.name,
                    alias,
                    platform: p.platform,
                    device_type: profile.device_type.as_str().to_string(),
                    tags: profile.tags,
                    group: profile.group,
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
        audit_count: node.audit_report(None, None, Some(0), None).total,
        unread_messages_count: node
            .message_history(None, Some(0), false, false)
            .map(|r| r.unread)
            .unwrap_or(0),
        handoff_inbox_count: node.handoff_inbox_list(None).len(),
        handoff_action: node.handoff_policy().action.as_str().to_string(),
        media_state: node.media_get_state(None).ok().and_then(|r| r.state),
        media_mode: node.media_policy().mode.as_str().to_string(),
        battery_state: node.battery_get_state(None).ok().and_then(|r| r.state),
        battery_low_threshold: node.battery_policy().low_battery_threshold,
        focus_status: Some(node.focus_status()),
        topology_layout: Some(node.topology_layout()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Node, NodeConfig};
    use crate::sidecar::OverlayConfig;
    use std::sync::Arc;
    use tetherly_core::ports::ManualClock;

    fn temp_test_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-uihttp-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "uihttp-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(5000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        (node, dir)
    }

    #[tokio::test]
    async fn test_uihttp_media_policy_and_state_endpoints() {
        let (node, dir) = temp_test_node("media_endpoints");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/media/policy
        let (code, body) = loopback_get(port, "/api/media/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["mode"], "allow");
        assert_eq!(rep["policy"]["max_volume_limit"], 100);
        assert_eq!(rep["policy"]["allow_seek"], true);

        // 2. POST /api/media/policy (mutate policy)
        let update_req = serde_json::json!({
            "mode": "allow",
            "allowed_groups": ["living_room"],
            "max_volume_limit": 80,
            "allow_seek": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/media/policy",
            &serde_json::to_vec(&update_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["max_volume_limit"], 80);
        assert_eq!(rep["policy"]["allow_seek"], false);
        assert_eq!(rep["policy"]["allowed_groups"][0], "living_room");

        // 3. POST /api/media/state (update local state)
        let state_req = serde_json::json!({
            "title": "Bohemian Rhapsody",
            "artist": "Queen",
            "album": "A Night at the Opera",
            "volume_percent": 75,
            "state": "playing"
        });
        let (code, body) = loopback_post(
            port,
            "/api/media/state",
            &serde_json::to_vec(&state_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"]["title"], "Bohemian Rhapsody");
        assert_eq!(rep["state"]["artist"], "Queen");
        assert_eq!(rep["state"]["state"], "playing");
        assert_eq!(rep["state"]["volume_percent"], 75);

        // 4. GET /api/media/state
        let (code, body) = loopback_get(port, "/api/media/state").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"]["title"], "Bohemian Rhapsody");
        assert_eq!(rep["state"]["artist"], "Queen");

        // 5. Host header 403 test: attack with external host
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/media/state HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_battery_policy_and_state_endpoints() {
        let (node, dir) = temp_test_node("battery");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/battery/policy (default)
        let (code, body) = loopback_get(port, "/api/battery/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["low_battery_threshold"], 20);
        assert_eq!(rep["policy"]["critical_battery_threshold"], 10);
        assert_eq!(rep["policy"]["alert_on_low_battery"], true);
        assert_eq!(rep["policy"]["auto_throttle_on_battery_saver"], false);

        // 2. POST /api/battery/policy (update)
        let update_req = serde_json::json!({
            "low_battery_threshold": 25,
            "critical_battery_threshold": 12,
            "alert_on_low_battery": true,
            "auto_throttle_on_battery_saver": true,
            "allowed_groups": ["family"],
            "allowed_tags": ["mobile"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/battery/policy",
            &serde_json::to_vec(&update_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["low_battery_threshold"], 25);
        assert_eq!(rep["policy"]["critical_battery_threshold"], 12);
        assert_eq!(rep["policy"]["auto_throttle_on_battery_saver"], true);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["family"])
        );
        assert_eq!(rep["policy"]["allowed_tags"], serde_json::json!(["mobile"]));

        // 3. POST /api/battery/state
        let state_req = serde_json::json!({
            "battery_level": 78,
            "is_charging": true,
            "power_source": "ac",
            "battery_saver": false,
            "health_percent": 95
        });
        let (code, body) = loopback_post(
            port,
            "/api/battery/state",
            &serde_json::to_vec(&state_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"]["battery_level"], 78);
        assert_eq!(rep["state"]["is_charging"], true);
        assert_eq!(rep["state"]["power_source"], "ac");
        assert_eq!(rep["state"]["battery_saver"], false);
        assert_eq!(rep["state"]["health_percent"], 95);

        // 4. GET /api/battery/state
        let (code, body) = loopback_get(port, "/api/battery/state").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"]["battery_level"], 78);
        assert_eq!(rep["state"]["is_charging"], true);
        assert_eq!(rep["state"]["power_source"], "ac");

        // 5. Host header 403 test on /api/battery/state
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/battery/state HTTP/1.1\r\nHost: malicious.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_locate_endpoints_and_host_403() {
        let (node, dir) = temp_test_node("locate_endpoints");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/locate/policy
        let (code, body) = loopback_get(port, "/api/locate/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_override_silent"], true);
        assert_eq!(rep["policy"]["max_duration_secs"], 60);

        // 2. POST /api/locate/policy (update)
        let update_req = serde_json::json!({
            "allow_override_silent": false,
            "max_duration_secs": 45,
            "allowed_groups": ["security_desk"],
            "allowed_tags": ["watch"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/locate/policy",
            &serde_json::to_vec(&update_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_override_silent"], false);
        assert_eq!(rep["policy"]["max_duration_secs"], 45);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["security_desk"])
        );
        assert_eq!(rep["policy"]["allowed_tags"], serde_json::json!(["watch"]));

        // 3. GET /api/locate/status
        let (code, body) = loopback_get(port, "/api/locate/status").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["is_ringing"], false);
        assert!(rep["active_ring"].is_null());

        // 4. POST /api/locate/stop (local stop when idle)
        let stop_req = serde_json::json!({
            "reason": "testing"
        });
        let (code, body) = loopback_post(
            port,
            "/api/locate/stop",
            &serde_json::to_vec(&stop_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 5. Host header 403 test on /api/locate/status
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/locate/status HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_action_endpoints_and_host_403() {
        let (node, dir) = temp_test_node("action_endpoints");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/action/policy
        let (code, body) = loopback_get(port, "/api/action/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["mode"], "allow");
        assert_eq!(rep["policy"]["expose_action_list"], true);

        // 2. POST /api/action/policy (update)
        let update_req = serde_json::json!({
            "mode": "prompt_only",
            "max_timeout_secs": 25,
            "allowed_groups": ["trusted_ops"],
            "allowed_tags": ["admin"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/action/policy",
            &serde_json::to_vec(&update_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["mode"], "prompt_only");
        assert_eq!(rep["policy"]["max_timeout_secs"], 25);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["trusted_ops"])
        );

        // 3. GET /api/action/local (initially empty)
        let (code, body) = loopback_get(port, "/api/action/local").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["count"], 0);

        // 4. POST /api/action/local (add local action)
        let add_req = serde_json::json!({
            "id": "test_echo",
            "name": "Test Echo",
            "description": "Echo command for test",
            "command": "echo",
            "args": ["hi"],
            "timeout_secs": 10,
            "enabled": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/action/local",
            &serde_json::to_vec(&add_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["count"], 1);

        // 5. GET /api/action/local (now 1)
        let (code, body) = loopback_get(port, "/api/action/local").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["count"], 1);
        assert_eq!(rep["actions"][0]["id"], "test_echo");

        // 6. DELETE /api/action/local
        let (code, body) = loopback_delete(port, "/api/action/local?id=test_echo")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["count"], 0);

        // 7. Host header 403 test on /api/action/policy
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/action/policy HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_focus_endpoints_and_host_validation() {
        let (node, dir) = temp_test_node("focus");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/focus/state (default inactive)
        let (code, body) = loopback_get(port, "/api/focus/state").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["enabled"], false);
        assert_eq!(rep["active"], false);

        // 2. POST /api/focus/state (activate Work focus mode for 45 mins)
        let set_req = serde_json::json!({
            "enabled": true,
            "mode": "work",
            "duration": 45,
            "reason": "Deep focused coding",
            "broadcast": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/focus/state",
            &serde_json::to_vec(&set_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["enabled"], true);
        assert_eq!(rep["active"], true);
        assert_eq!(rep["mode"], "work");
        assert_eq!(rep["reason"], "Deep focused coding");

        // 3. GET /api/focus/state (now active)
        let (code, body) = loopback_get(port, "/api/focus/state").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["active"], true);
        assert_eq!(rep["mode"], "work");

        // 4. GET /api/focus/peers
        let (code, body) = loopback_get(port, "/api/focus/peers").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["count"], 0);

        // 5. GET /api/focus/policy
        let (code, body) = loopback_get(port, "/api/focus/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);

        // 6. POST /api/focus/policy (update quiet hours to 22:30 -> 06:30)
        let pol_req = serde_json::json!({
            "quiet_hours": true,
            "start": "22:30",
            "end": "06:30",
            "mirror": true,
            "breakthrough": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/focus/policy",
            &serde_json::to_vec(&pol_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["quiet_hours_enabled"], true);
        assert_eq!(rep["policy"]["quiet_hours_start_minute"], 1350);
        assert_eq!(rep["policy"]["quiet_hours_end_minute"], 390);
        assert_eq!(rep["policy"]["mirror_inbound_focus"], true);

        // 7. POST /api/focus/state (deactivate)
        let deact_req = serde_json::json!({
            "enabled": false,
            "broadcast": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/focus/state",
            &serde_json::to_vec(&deact_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["enabled"], false);
        assert_eq!(rep["active"], false);

        // 8. Host header 403 test on /api/focus/state
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/focus/state HTTP/1.1\r\nHost: attacker-phishing.org\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_topology_http_api_and_host_validation() {
        let (node, dir) = temp_test_node("topology-api");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/topology/layout (defaults)
        let (code, body) = loopback_get(port, "/api/topology/layout").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["display"]["width"], 1920);
        assert_eq!(rep["display"]["height"], 1080);
        assert_eq!(rep["config"]["resistance_px"], 15);

        // 2. POST /api/topology/config (update config and display)
        let config_body = serde_json::json!({
            "resistance_px": 35,
            "delay_ms": 150,
            "corner_lock_size_px": 25,
            "width": 2560,
            "height": 1440,
            "scale_factor": 1.25
        });
        let (code, body) = loopback_post(
            port,
            "/api/topology/config",
            &serde_json::to_vec(&config_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["display"]["width"], 2560);
        assert_eq!(rep["display"]["height"], 1440);
        assert_eq!(rep["config"]["resistance_px"], 35);
        assert_eq!(rep["config"]["delay_ms"], 150);

        // 3. POST /api/topology/link (link Right edge)
        let peer_id_str = "tdev_abcdefgh234567aa";
        let link_body = serde_json::json!({
            "edge": "right",
            "peer": peer_id_str,
            "peer_edge": "left",
            "offset": 50
        });
        let (code, body) = loopback_post(
            port,
            "/api/topology/link",
            &serde_json::to_vec(&link_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["edge"], "right");
        assert_eq!(rep["peer_id"], peer_id_str);
        assert_eq!(rep["peer_edge"], "left");
        assert_eq!(rep["offset"], 50);

        // Verify layout has neighbor
        let (code, body) = loopback_get(port, "/api/topology/layout").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rep["neighbors"]["right"].is_object());
        assert_eq!(rep["neighbors"]["right"]["peer_id"], peer_id_str);

        // 4. GET /api/topology/peers
        let (code, body) = loopback_get(port, "/api/topology/peers").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 5. POST /api/topology/unlink (unlink Right edge)
        let unlink_body = serde_json::json!({ "edge": "right" });
        let (code, body) = loopback_post(
            port,
            "/api/topology/unlink",
            &serde_json::to_vec(&unlink_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["edge"], "right");

        // Verify layout unlinked
        let (code, body) = loopback_get(port, "/api/topology/layout").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(rep["neighbors"]["right"].is_null());

        // 6. POST /api/topology/sync
        let (code, body) = loopback_post(port, "/api/topology/sync", b"{}")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 7. GET /api/topology/policy
        let (code, body) = loopback_get(port, "/api/topology/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_inbound_control"], true);

        // 8. POST /api/topology/policy
        let pol_body = serde_json::json!({
            "enabled": true,
            "allow_inbound_control": false,
            "allowed_groups": ["studio"],
            "allowed_tags": ["display"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/topology/policy",
            &serde_json::to_vec(&pol_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_inbound_control"], false);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["studio"])
        );

        // 9. Host header 403 test on /api/topology/layout
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/topology/layout HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_input_tunnel_endpoints_and_host_validation() {
        let (node, dir) = temp_test_node("input_tunnel");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/input/tunnel (default inactive)
        let (code, body) = loopback_get(port, "/api/input/tunnel").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], false);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_inbound_control"], true);

        // 2. GET /api/input/policy
        let (code, body) = loopback_get(port, "/api/input/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);

        // 3. POST /api/input/policy
        let pol_body = serde_json::json!({
            "enabled": true,
            "allow_inbound_control": false,
            "idle_timeout_secs": 120,
            "allowed_groups": ["office"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/input/policy",
            &serde_json::to_vec(&pol_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_inbound_control"], false);
        assert_eq!(rep["policy"]["idle_timeout_secs"], 120);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["office"])
        );

        // 4. Verify policy reflected on GET /api/input/policy
        let (code, body) = loopback_get(port, "/api/input/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["policy"]["allow_inbound_control"], false);
        assert_eq!(rep["policy"]["idle_timeout_secs"], 120);

        // 5. POST /api/input/tunnel/release (when idle/empty)
        let rel_body = serde_json::json!({ "reason": "unit test cleanup" });
        let (code, body) = loopback_post(
            port,
            "/api/input/tunnel/release",
            &serde_json::to_vec(&rel_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["action"], "release");

        // 6. Host header 403 test on /api/input/tunnel
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/input/tunnel HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
