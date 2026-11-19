// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback-only desktop shell. Never bind 0.0.0.0. Bodies/OTP/PIN are not
//! written to logs. Full Tauri packaging is the next desktop-shell iteration.

use crate::apiv1::hub::EventHub;
use crate::runtime::Node;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

// The loopback HTTP/SSE client used to be defined here and, separately, in
// `tetherly-cli`. It now lives in `tetherly-loopback` so both binaries share one
// implementation (and one timeout policy); re-exported so the existing
// `tetherly_node::loopback_*` call sites and the router tests are unchanged.
#[cfg(test)]
use tetherly_loopback::parse_http_response;
pub use tetherly_loopback::{
    api_token, api_token_for, loopback_delete, loopback_get, loopback_post,
    loopback_post_with_deadlines, loopback_put, loopback_request, set_api_token,
    set_port_api_token,
};

const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 2 * 1024 * 1024;
const RESPONSE_CHUNK: usize = 64 * 1024;
/// Bound each response write so a client that stops reading cannot retain a
/// file handle and request task indefinitely.
#[cfg(not(test))]
const RESPONSE_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const RESPONSE_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);
/// Whole-request read budget (headers + body). A loopback client that stalls
/// mid-request gets 408 instead of holding a task and socket open forever.
#[cfg(not(test))]
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);

// ---- feature submodules (v0.1.64 / ADR-128) ----
mod dto;
mod next;
mod util;
mod v1;

// Re-exported so every handler module can reach the DTOs and helpers with a
// plain `use super::*;`.
use dto::*;
pub use dto::{DaemonRestartBody, DaemonRestartReport};
pub use util::urlencode_path;
use util::*;

/// Bind the loopback UI and serve it in the background, returning the port
/// actually bound (a `0` request resolves to an ephemeral one). Binding
/// synchronously means a taken port is a hard error the caller can report,
/// rather than a `warn!` that leaves the UI silently absent.
pub async fn bind_and_spawn(node: Node, port: u16) -> std::io::Result<u16> {
    bind_and_spawn_with_token(node, port).await.map(|(p, _)| p)
}

/// [`bind_and_spawn`], also returning the API token minted for this listener
/// (see [`crate::apitoken`]). The token is rotated on every bind and written
/// to `<data_dir>/api.token`; when that write fails the listener still serves
/// but every privileged route answers 401 (fail closed) and the failure is a
/// WARN, so the dashboard and everyday CLI keep working.
pub async fn bind_and_spawn_with_token(
    node: Node,
    port: u16,
) -> std::io::Result<(u16, Option<Arc<str>>)> {
    let token: Option<Arc<str>> = match crate::apitoken::issue(node.data_dir()) {
        Ok(t) => Some(Arc::from(t)),
        Err(e) => {
            warn!(
                error = %e,
                path = %crate::apitoken::token_path(node.data_dir()).display(),
                "api token file not written; privileged ui routes will refuse every caller"
            );
            None
        }
    };
    // §4.4 client token: a second, lower-tier credential (Write, never
    // Admin) the shell reads from the data dir. Best-effort — when the mint
    // fails the shell falls back to the daemon token as before.
    let client_token: Option<Arc<str>> = match crate::apitoken::issue_client(node.data_dir()) {
        Ok(t) => Some(Arc::from(t)),
        Err(e) => {
            warn!(
                error = %e,
                path = %crate::apitoken::client_token_path(node.data_dir()).display(),
                "client token file not written; shell falls back to the daemon token"
            );
            None
        }
    };
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut attempts = 0;
    let listener = loop {
        match TcpListener::bind(addr).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && port != 0 && attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    };
    let bound = listener.local_addr()?.port();
    info!(
        port = bound,
        token = token.is_some(),
        client_token = client_token.is_some(),
        "ui listening (loopback only)"
    );
    let gate: Arc<str> = token.clone().unwrap_or_else(|| Arc::from(""));
    let hub = EventHub::spawn(node.clone_handle());
    tokio::spawn(async move {
        if let Err(e) = accept_loop(listener, node, bound, gate, client_token, hub).await {
            warn!(%e, "ui http");
        }
    });
    Ok((bound, token))
}

/// Extract the host name (without port) from a `Host` header value.
/// Returns `None` for values that cannot be parsed, which must fail closed.
fn host_name(host_header: &str) -> Option<String> {
    let h = host_header.trim();
    if let Some(rest) = h.strip_prefix('[') {
        // Bracketed IPv6 literal: `[::1]:45716`
        let end = rest.find(']')?;
        return Some(rest[..end].to_ascii_lowercase());
    }
    // Strip exactly one `:port` suffix. An unbracketed IPv6 host (invalid
    // per RFC 3986) splits at its last colon and fails the exact-match below.
    let name = h.rsplit_once(':').map(|(n, _)| n).unwrap_or(h);
    Some(name.to_ascii_lowercase())
}

/// The loopback origins the bundled dashboard may use. `None` (no Origin
/// header at all) is allowed so curl/CLI keep working; any browser-sent
/// Origin must match the bound UI port exactly, which defeats cross-site
/// `fetch(..., {mode: "no-cors"})` POST forgery and cross-origin EventSource
/// reads (`null`/file:// origins fail closed).
fn origin_allowed(origin: &str, ui_port: u16) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let (name, port) = match rest.rsplit_once(':') {
        Some((n, p)) => (n, p.parse::<u16>().ok()),
        None => (rest, None),
    };
    matches!(name, "127.0.0.1" | "localhost" | "[::1]") && port == Some(ui_port)
}

async fn accept_loop(
    listener: TcpListener,
    node: Node,
    ui_port: u16,
    token: Arc<str>,
    client_token: Option<Arc<str>>,
    hub: Arc<EventHub>,
) -> std::io::Result<()> {
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
                let token = token.clone();
                let client_token = client_token.clone();
                let hub = hub.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        handle(stream, node, ui_port, &token, client_token.as_deref(), &hub).await
                    {
                        // Clients hanging up mid-response (closed SSE tabs,
                        // aborted fetches) are routine, not daemon faults.
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) {
                            debug!(%e, "ui request: client went away");
                        } else {
                            warn!(%e, "ui request");
                        }
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle(
    mut stream: TcpStream,
    node: Node,
    ui_port: u16,
    token: &str,
    client_token: Option<&str>,
    hub: &Arc<EventHub>,
) -> std::io::Result<()> {
    let deadline = tokio::time::Instant::now() + REQUEST_READ_TIMEOUT;
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
        let Ok(read) = tokio::time::timeout_at(deadline, stream.read(&mut buf[n..])).await else {
            return request_timed_out(&mut stream).await;
        };
        let k = read?;
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
    let mut origin: Option<String> = None;
    let mut sec_fetch_site: Option<String> = None;
    let mut presented: Option<String> = None;
    let mut last_event_id: Option<u64> = None;
    let mut user_agent: Option<String> = None;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim();
        match key.as_str() {
            "content-length" => content_len = value.parse().unwrap_or(0),
            "host" => {
                host_ok = matches!(
                    host_name(value).as_deref(),
                    Some("127.0.0.1") | Some("localhost") | Some("::1")
                );
            }
            "origin" => origin = Some(value.to_string()),
            "sec-fetch-site" => sec_fetch_site = Some(value.to_ascii_lowercase()),
            "last-event-id" => last_event_id = value.parse().ok(),
            "user-agent" => user_agent = Some(value.to_string()),
            crate::apitoken::TOKEN_HEADER => presented = Some(value.to_string()),
            "authorization" => {
                if let Some(t) = value.strip_prefix("Bearer ") {
                    presented.get_or_insert_with(|| t.trim().to_string());
                }
            }
            _ => {}
        }
    }
    if !host_ok {
        write_http(&mut stream, 403, "text/plain", b"loopback only").await?;
        return Ok(());
    }
    hub.legacy.record(method, path, user_agent.as_deref());
    // CSRF / DNS-rebinding gate: a browser page on any other origin cannot
    // forge loopback requests. curl/CLI send no Origin and stay allowed;
    // `Sec-Fetch-Site: cross-site` rejects even Origin-less browser requests.
    if sec_fetch_site.as_deref() == Some("cross-site") {
        write_http(
            &mut stream,
            403,
            "text/plain",
            b"cross-site request rejected",
        )
        .await?;
        return Ok(());
    }
    if let Some(o) = &origin {
        if !origin_allowed(o, ui_port) {
            write_http(
                &mut stream,
                403,
                "text/plain",
                b"cross-origin request rejected",
            )
            .await?;
            return Ok(());
        }
    }
    if path == tetherly_api::PREFIX || path.starts_with("/api/v1/") || path.starts_with("/api/v1?")
    {
        if let Some(need) = crate::apitoken::v1_required_tier(method, path) {
            let granted = crate::apitoken::granted_tier(
                token,
                client_token,
                presented.as_deref().unwrap_or(""),
            );
            if granted < Some(need) {
                warn!(
                    method,
                    path = path.split_once('?').map_or(path, |(p, _)| p),
                    presented = presented.is_some(),
                    ?need,
                    "v1 route refused: insufficient api token tier"
                );
                drain_request_body(&mut stream, content_len.saturating_sub(n - head_end)).await;
                let resp = if granted.is_none() {
                    v1::error(tetherly_api::ErrorCode::Unauthorized, "api token required")
                } else {
                    v1::error(tetherly_api::ErrorCode::Forbidden, "token tier too low")
                };
                write_http_response(&mut stream, &resp).await?;
                return Ok(());
            }
        }
    }
    if method == "GET" {
        let (path_only, query) = path
            .split_once('?')
            .map_or((path, None), |(p, q)| (p, Some(q)));
        if path_only == "/api/v1/events" {
            return v1::events(&mut stream, &node, hub, query, last_event_id).await;
        }
    }
    if method == "GET"
        && (path == "/api/events"
            || path == "/api/events/stream"
            || path.starts_with("/api/events?")
            || path.starts_with("/api/events/stream?"))
    {
        // Keep the legacy stream temporarily for older local clients, but do
        // not let it bypass the v1 Read boundary while callers migrate.
        let granted =
            crate::apitoken::granted_tier(token, client_token, presented.as_deref().unwrap_or(""));
        if granted < Some(crate::apitoken::Tier::Read) {
            let resp = if granted.is_none() {
                v1::error(tetherly_api::ErrorCode::Unauthorized, "api token required")
            } else {
                v1::error(tetherly_api::ErrorCode::Forbidden, "token tier too low")
            };
            return write_http_response(&mut stream, &resp).await;
        }
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
        let Ok(read) = tokio::time::timeout_at(deadline, stream.read(&mut more)).await else {
            return request_timed_out(&mut stream).await;
        };
        let k = read?;
        if k == 0 {
            break;
        }
        body.extend_from_slice(&more[..k]);
    }
    body.truncate(content_len);
    let granted =
        crate::apitoken::granted_tier(token, client_token, presented.as_deref().unwrap_or(""));
    let resp = route(method, path, &body, &node, hub, granted).await;
    write_http_response(&mut stream, &resp).await
}

async fn request_timed_out(stream: &mut TcpStream) -> std::io::Result<()> {
    debug!("ui request read timed out; 408");
    write_http(stream, 408, "text/plain", b"request timeout").await
}

/// Best-effort read-and-discard of up to 64 KiB of request body (2 s cap)
/// before an early refusal, so the close is a clean FIN rather than a reset.
async fn drain_request_body(stream: &mut TcpStream, remaining: usize) {
    let mut left = remaining.min(64 * 1024);
    if left == 0 {
        return;
    }
    let mut scratch = [0u8; 4096];
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while left > 0 {
            let want = left.min(scratch.len());
            match stream.read(&mut scratch[..want]).await {
                Ok(0) | Err(_) => break,
                Ok(k) => left -= k,
            }
        }
    })
    .await;
}

/// Structured HTTP response for the loopback UI server.
#[derive(Debug, Clone)]
pub struct UiResponse {
    pub code: u16,
    pub ctype: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    file: Option<Arc<ResponseFile>>,
}

#[derive(Debug)]
struct ResponseFile {
    file: std::fs::File,
    len: u64,
}

impl PartialEq for UiResponse {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code
            && self.ctype == other.ctype
            && self.headers == other.headers
            && self.body == other.body
            && self.file.as_ref().map(|file| file.len) == other.file.as_ref().map(|file| file.len)
    }
}

impl Eq for UiResponse {}

impl UiResponse {
    pub fn new(code: u16, ctype: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            code,
            ctype: ctype.into(),
            headers: Vec::new(),
            body,
            file: None,
        }
    }

    pub fn from_file(code: u16, ctype: impl Into<String>, file: std::fs::File, len: u64) -> Self {
        Self {
            code,
            ctype: ctype.into(),
            headers: Vec::new(),
            body: Vec::new(),
            file: Some(Arc::new(ResponseFile { file, len })),
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
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    // RFC 9110 §8.6: no Content-Length (or body) on a 204.
    let content_len = resp
        .file
        .as_ref()
        .map_or(resp.body.len() as u64, |file| file.len);
    let mut head = if resp.code == 204 {
        format!("HTTP/1.1 204 {reason}\r\nConnection: close\r\nCache-Control: no-store\r\n")
    } else {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
            resp.code,
            reason,
            resp.ctype,
            content_len
        )
    };
    for (k, v) in &resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    write_response_bytes(stream, head.as_bytes()).await?;
    if resp.code != 204 {
        if let Some(response_file) = &resp.file {
            let file = response_file.file.try_clone()?;
            let mut file = tokio::fs::File::from_std(file).take(response_file.len);
            let mut chunk = vec![0u8; RESPONSE_CHUNK];
            loop {
                let read = file.read(&mut chunk).await?;
                if read == 0 {
                    break;
                }
                write_response_bytes(stream, &chunk[..read]).await?;
            }
        } else {
            write_response_bytes(stream, &resp.body).await?;
        }
    }
    tokio::time::timeout(RESPONSE_WRITE_TIMEOUT, stream.flush())
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "response flush timed out")
        })??;
    Ok(())
}

async fn write_response_bytes(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    tokio::time::timeout(RESPONSE_WRITE_TIMEOUT, stream.write_all(bytes))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "response write timed out")
        })??;
    Ok(())
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

/// An active Server-Sent Events stream connected to a loopback UI endpoint.
///
/// A thin typed facade over [`tetherly_loopback::SseStream`]: the transport and
/// its deadlines live in the shared crate, while decoding into this crate's
/// [`UiEvent`](crate::events::UiEvent) has to stay here because that type does.
pub struct SseStream(tetherly_loopback::SseStream);

/// Typed facade for the authenticated v1 resource stream. It keeps the
/// legacy `SseStream` unchanged for watchers that consume `UiEvent` directly.
pub struct V1SseStream(tetherly_loopback::SseStream);

impl V1SseStream {
    pub async fn next_event(&mut self) -> std::io::Result<Option<String>> {
        self.0.next_event().await
    }

    /// Unpack the shared v1 `{seq,type,data}` envelope without pretending that
    /// resource events are legacy peer/file notifications.
    pub async fn next_ui_event(&mut self) -> std::io::Result<Option<crate::events::UiEvent>> {
        while let Some(raw) = self.next_event().await? {
            if let Some(event) = decode_v1_ui_event(&raw) {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }
}

fn decode_v1_ui_event(raw: &str) -> Option<crate::events::UiEvent> {
    let envelope = serde_json::from_str::<tetherly_api::EventEnvelope>(raw).ok()?;
    Some(crate::events::UiEvent::V1 {
        seq: envelope.seq,
        event: envelope.event,
    })
}

impl SseStream {
    /// Read the next event data string from the stream.
    /// Comments (lines starting with ':') and blank separator lines are skipped.
    /// Returns `Ok(Some(data))` when a `data: ...` event arrives,
    /// `Ok(None)` when the stream is closed, or `Err` on I/O error (including
    /// the shared client's idle deadline).
    pub async fn next_event(&mut self) -> std::io::Result<Option<String>> {
        self.0.next_event().await
    }

    /// Read the next typed `UiEvent` from the legacy stream, skipping frames
    /// that do not decode.
    pub async fn next_ui_event(&mut self) -> std::io::Result<Option<crate::events::UiEvent>> {
        while let Some(raw) = self.next_event().await? {
            if let Ok(ev) = serde_json::from_str::<crate::events::UiEvent>(&raw) {
                return Ok(Some(ev));
            }
        }
        Ok(None)
    }

    /// Read the next v1 resource event envelope. Invalid frames are skipped,
    /// matching the legacy typed facade's behavior.
    pub async fn next_v1_event(&mut self) -> std::io::Result<Option<tetherly_api::EventEnvelope>> {
        while let Some(raw) = self.next_event().await? {
            if let Ok(ev) = serde_json::from_str::<tetherly_api::EventEnvelope>(&raw) {
                return Ok(Some(ev));
            }
        }
        Ok(None)
    }
}

/// Connect to the authenticated legacy event stream used by existing CLI watchers.
pub async fn loopback_events_connect(port: u16) -> std::io::Result<SseStream> {
    tetherly_loopback::loopback_events_connect(port)
        .await
        .map(SseStream)
}

/// A v1 resource-event stream with explicit envelope-to-`UiEvent` decoding.
pub async fn loopback_v1_events_connect(port: u16) -> std::io::Result<V1SseStream> {
    tetherly_loopback::loopback_v1_events_connect(port)
        .await
        .map(V1SseStream)
}

async fn handle_sse(stream: &mut TcpStream, node: &Node) -> std::io::Result<()> {
    let mut events = node.subscribe();
    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: text/event-stream; charset=utf-8\r\n\
                  Cache-Control: no-cache, no-transform\r\n\
                  Connection: keep-alive\r\n\r\n";
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

// Generated router: every arm pattern stays here so the (method, path) table
// remains greppable in one place, while the bodies live in feature modules.

async fn route(
    method: &str,
    path: &str,
    body: &[u8],
    node: &Node,
    hub: &Arc<EventHub>,
    granted: Option<crate::apitoken::Tier>,
) -> UiResponse {
    let (path_only, query_str) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    if path_only == tetherly_api::PREFIX || path_only.starts_with("/api/v1/") {
        return v1::route(method, path_only, query_str, body, node, hub, granted).await;
    }
    match (method, path_only) {
        ("GET", "/") | ("GET", "/index.html") => next::get_entry(),
        ("GET", p) if p.starts_with("/assets/") => next::get_asset(p),
        _ => UiResponse::new(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{Node, NodeConfig};
    use crate::sidecar::OverlayConfig;
    use std::sync::Arc;
    use tetherly_core::ports::ManualClock;

    #[test]
    fn v1_event_envelope_decodes_to_explicit_internal_variant() {
        let raw = r#"{"seq":7,"type":"invalidated","data":{"topic":"settings"}}"#;
        let Some(crate::events::UiEvent::V1 { seq, event }) = decode_v1_ui_event(raw) else {
            panic!("v1 envelope must decode to UiEvent::V1");
        };
        assert_eq!(seq, 7);
        assert_eq!(
            event,
            tetherly_api::Event::Invalidated {
                topic: tetherly_api::Topic::Settings,
            }
        );
        assert!(decode_v1_ui_event(r#"{"kind":"status","message":"old"}"#).is_none());
    }

    #[test]
    fn host_name_parses_and_fails_closed() {
        assert_eq!(host_name("127.0.0.1:45716").as_deref(), Some("127.0.0.1"));
        assert_eq!(host_name("localhost").as_deref(), Some("localhost"));
        assert_eq!(host_name("LOCALHOST:80").as_deref(), Some("localhost"));
        assert_eq!(host_name("[::1]:45716").as_deref(), Some("::1"));
        // Prefix lookalikes must not match the exact allowlist downstream.
        assert_eq!(
            host_name("127.0.0.1.evil.com:80").as_deref(),
            Some("127.0.0.1.evil.com")
        );
        assert_eq!(
            host_name("localhost.evil.com").as_deref(),
            Some("localhost.evil.com")
        );
    }

    #[test]
    fn origin_allowed_requires_exact_loopback_port() {
        assert!(origin_allowed("http://127.0.0.1:45716", 45716));
        assert!(origin_allowed("http://localhost:45716", 45716));
        assert!(origin_allowed("http://[::1]:45716", 45716));
        assert!(!origin_allowed("http://127.0.0.1:45717", 45716));
        assert!(!origin_allowed("http://127.0.0.1.evil.com:45716", 45716));
        assert!(!origin_allowed("https://evil.example", 45716));
        // `null` origin (file://, sandboxed iframe) fails closed.
        assert!(!origin_allowed("null", 45716));
    }

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
            screen_codec_h264: false,
            screen_bitrate_kbps: 4000,
            screen_bitrate_auto: true,
            screen_fps: 30,
            screen_fps_auto: true,
            screen_scale_pct: 100,
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(5000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        (node, dir)
    }

    /// Raw loopback request with an optional extra header line, so the
    /// token gate is exercised exactly as an arbitrary local client would.
    async fn raw_request(
        port: u16,
        method: &str,
        path: &str,
        extra_header: Option<&str>,
        body: &[u8],
    ) -> (u16, Vec<u8>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let extra = extra_header.map(|h| format!("{h}\r\n")).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        s.write_all(head.as_bytes()).await.unwrap();
        s.write_all(body).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        tetherly_loopback::parse_http_response(&out)
    }

    /// GET with only the raw response head returned (for content-type
    /// assertions on non-JSON bodies the JSON parser would mangle).
    async fn raw_head(port: u16, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let head = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        s.write_all(head.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        String::from_utf8_lossy(&out[..split]).to_string()
    }

    /// §120 P2: a client that stops mid-request (half a header, or a body
    /// shorter than its Content-Length) used to pin a task + socket forever.
    #[tokio::test]
    async fn stalled_request_is_answered_408_and_closed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (node, _dir) = temp_test_node("ui_read_timeout");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        for partial in [
            "GET /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\n".to_string(),
            "POST /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 50\r\n\r\n{}"
                .to_string(),
        ] {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            s.write_all(partial.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            tokio::time::timeout(
                REQUEST_READ_TIMEOUT + std::time::Duration::from_secs(3),
                s.read_to_end(&mut out),
            )
            .await
            .expect("server must give up on a stalled request")
            .unwrap();
            let (code, _) = tetherly_loopback::parse_http_response(&out);
            assert_eq!(code, 408, "partial: {partial:?}");
        }
        node.shutdown();
    }

    /// A refused request whose body trails the headers must still get a clean
    /// 401, not a TCP reset from closing with unread bytes in the buffer.
    #[tokio::test]
    async fn refused_request_with_late_body_gets_clean_401() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (node, _dir) = temp_test_node("api_token_late_body");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        for _ in 0..3 {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let body = br#"{}"#;
            let head = format!(
                "POST /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = s.write_all(body).await;
            let mut out = Vec::new();
            s.read_to_end(&mut out)
                .await
                .expect("401 must arrive without a connection reset");
            assert_eq!(tetherly_loopback::parse_http_response(&out).0, 401);
        }
    }

    /// Raw request returning the full response head as text plus status and
    /// body, for asserting v1 version headers.
    async fn raw_with_head(
        port: u16,
        method: &str,
        path: &str,
        extra_header: Option<&str>,
    ) -> (String, u16, serde_json::Value) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let extra = extra_header.map(|h| format!("{h}\r\n")).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
        s.write_all(head.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&out[..split]).to_string();
        let (code, body) = tetherly_loopback::parse_http_response(&out);
        let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (head, code, json)
    }

    /// §4.4: the client token minted beside the daemon token grants Write —
    /// read/write v1 routes pass, admin routes answer 403 — and unknown
    /// credentials still fail closed with 401.
    #[tokio::test]
    async fn v1_client_token_grants_write_but_not_admin() {
        let (node, dir) = temp_test_node("v1_client_token");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let client = crate::apitoken::read_client(&dir).expect("client token minted");
        let client_auth = format!("Authorization: Bearer {client}");

        // Ordinary reads still pass, but privileged file-transfer side effects
        // must not be driven by the lower-tier client credential.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/system", Some(&client_auth)).await;
        assert_eq!(code, 200, "{body}");
        let (_, code, body) =
            raw_with_head(port, "POST", "/api/v1/transfers", Some(&client_auth)).await;
        assert_eq!(code, 403, "{body}");
        let (_, code, body) = raw_with_head(
            port,
            "GET",
            "/api/v1/screen/snapshot?out=escape.bmp",
            Some(&client_auth),
        )
        .await;
        assert_eq!(code, 403, "{body}");

        // Admin tier: process lifecycle refuses with the typed envelope.
        let (_, code, body) =
            raw_with_head(port, "POST", "/api/v1/system/restart", Some(&client_auth)).await;
        assert_eq!(code, 403, "{body}");
        let e: tetherly_api::ErrorBody = serde_json::from_value(body).unwrap();
        assert_eq!(e.error.code, tetherly_api::ErrorCode::Forbidden);

        // Fail closed on a fabricated token of the right shape.
        let bogus = format!("Authorization: Bearer {}", "f".repeat(64));
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/system", Some(&bogus)).await;
        assert_eq!(code, 401);
        assert_eq!(body["error"]["code"], "unauthorized");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §160: discovery reads at Read tier and the lifecycle Admin gate —
    /// GET /discovery answers for a client token, POST /system/shutdown
    /// refuses it (the daemon token passes and schedules the exit; the test
    /// node asserts the gate, not the exit).
    #[tokio::test]
    async fn v1_discovery_reads_and_lifecycle_gates() {
        let (node, dir) = temp_test_node("v1_discovery_lifecycle");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("daemon token");
        let auth = format!("Authorization: Bearer {token}");

        // Discovery: an empty LAN is a valid, empty report (Read tier; the
        // legacy `ok` envelope folded away).
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/discovery", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");
        assert_eq!(body["peers"], serde_json::json!([]), "{body}");

        // Client token: Read passes, Admin shutdown refuses.
        let client = crate::apitoken::read_client(&dir).expect("client token");
        let client_auth = format!("Authorization: Bearer {client}");
        let (_, code, _) =
            raw_with_head(port, "GET", "/api/v1/discovery", Some(&client_auth)).await;
        assert_eq!(code, 200);
        let (_, code, body) =
            raw_with_head(port, "POST", "/api/v1/system/shutdown", Some(&client_auth)).await;
        assert_eq!(code, 403, "{body}");
        assert_eq!(body["error"]["code"], "forbidden", "{body}");

        // A missing token is 401, not 403 (fail closed before tier math).
        let (_, code, _) = raw_with_head(port, "POST", "/api/v1/system/shutdown", None).await;
        assert_eq!(code, 401);

        // Unknown v1 method on a known path is 405 with the typed envelope.
        let (_, code, _) = raw_with_head(port, "DELETE", "/api/v1/discovery", Some(&auth)).await;
        assert_eq!(code, 405);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §161: notify/message v1 resources — tier gates and error classes on
    /// real HTTP (candidates read, config read, strict-body rejects, 405).
    #[tokio::test]
    async fn v1_notify_and_message_domains_serve_and_gate() {
        let (node, dir) = temp_test_node("v1_notify_message");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("daemon token");
        let auth = format!("Authorization: Bearer {token}");

        // Empty candidates are a valid report (count folds from the array).
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/notify/candidates", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");
        assert_eq!(body["candidates"], serde_json::json!([]), "{body}");

        // Replies list is equally empty (no text — the no-content rule).
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/notify/replies", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["replies"], serde_json::json!([]), "{body}");

        // Config read: banner defaults + zeroed totals.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/notify/config", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["config"]["banners_enabled"], true, "{body}");
        assert_eq!(body["totals"]["dispatched"], 0, "{body}");

        // Message history on a fresh node: empty, zeroed.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/messages", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["total"], 0, "{body}");
        assert_eq!(body["messages"], serde_json::json!([]), "{body}");

        // Strict bodies: unknown fields rejected with the typed envelope.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/notify",
            Some(&auth),
            br#"{"title":"t","body":"b","z":1}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["error"]["code"], "bad_request", "{body}");

        // Sending with no connected peer is a Conflict, not a 500.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/notify",
            Some(&auth),
            br#"{"title":"t","body":"b"}"#,
        )
        .await;
        assert_eq!(code, 409, "{}", String::from_utf8_lossy(&resp));

        // Message send with no connected peer resolves first: the empty
        // peer falls into the legacy "no connected peer" branch (404).
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/messages",
            Some(&auth),
            br#"{"text":"  "}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // OTP for an unknown candidate maps to 404 via core_error.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/notify/otp",
            Some(&auth),
            br#"{"id":"nope"}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // Reply take for an unknown id maps to 404 (unknown reply).
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/replies/take",
            Some(&auth),
            br#"{"id":"nope"}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // Dismiss with no target at all is a BadRequest.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/notify/dismissals",
            Some(&auth),
            b"{}",
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Client token keeps Write-tier notify reads working.
        let client = crate::apitoken::read_client(&dir).expect("client token");
        let client_auth = format!("Authorization: Bearer {client}");
        let (_, code, _) =
            raw_with_head(port, "GET", "/api/v1/notify/candidates", Some(&client_auth)).await;
        assert_eq!(code, 200);

        // Unknown v1 method on a notify path is 405.
        let (_, code, _) =
            raw_with_head(port, "DELETE", "/api/v1/notify/config", Some(&auth)).await;
        assert_eq!(code, 405);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §162b: file-domain v1 resources — offers/decisions, outbox, storage,
    /// resume, auto-accept policy, inbox content — served and gated.
    #[tokio::test]
    async fn v1_file_domains_serve_and_gate() {
        let (node, dir) = temp_test_node("v1_file_domains");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("daemon token");
        let auth = format!("Authorization: Bearer {token}");

        // Offers resource is empty on a fresh node.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/offers", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");
        assert_eq!(body["offers"], serde_json::json!([]), "{body}");

        // Outbox resource is empty; status filter parses, garbage is 400.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/outbox", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["total"], 0, "{body}");
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/outbox?status=done", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/outbox?status=bogus", Some(&auth)).await;
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"]["code"], "bad_request", "{body}");

        // Outbox purge on an empty ring still reports zero.
        let (code, resp) = raw_request(port, "DELETE", "/api/v1/outbox", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["cleared_count"], 0, "{body}");

        // Storage + resume resources report the data dir (fresh node zeroes).
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/storage", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert!(!body["data_dir"].as_str().unwrap_or_default().is_empty());
        assert_eq!(body["inbox_file_count"], 0, "{body}");
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/resume", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");

        // P3: absolute filesystem layout is Admin-only. The client token gets
        // stable placeholders; the daemon token keeps the real paths.
        let client = crate::apitoken::read_client(&dir).expect("client token");
        let client_auth = format!("Authorization: Bearer {client}");
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/storage", Some(&client_auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["data_dir"], "<data-dir>", "{body}");
        assert_eq!(body["inbox_dir"], "<data-dir>/inbox", "{body}");
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/storage", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_ne!(body["data_dir"], "<data-dir>", "{body}");

        // Storage clean: default body (absent) means orphaned-only.
        let (code, resp) =
            raw_request(port, "POST", "/api/v1/storage/clean", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["deleted_files_count"], 0, "{body}");

        // Auto-accept policy: default readback, strict patch, then a real
        // patch toggles the master switch and sets a size cap.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/policy/auto-accept", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["enabled"], false, "{body}");
        assert_eq!(body["auto_accepted_count"], 0, "{body}");
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/policy/auto-accept",
            Some(&auth),
            br#"{"bogus":1}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/policy/auto-accept",
            Some(&auth),
            br#"{"enabled":true,"max_file_size":1024}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["enabled"], true, "{body}");
        assert_eq!(body["max_file_size"], 1024, "{body}");
        // JSON null lifts the cap (the key stays, value becomes null).
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/policy/auto-accept",
            Some(&auth),
            br#"{"max_file_size":null}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert!(body["max_file_size"].is_null(), "{body}");

        // Decision without id/all/peer is 400; unknown id is 404.
        let (code, resp) =
            raw_request(port, "POST", "/api/v1/offers/accept", Some(&auth), b"{}").await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/offers/accept",
            Some(&auth),
            br#"{"id":"nope"}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // Inbox content: a real file serves bytes with mime; missing is 404;
        // traversal is rejected.
        let inbox_dir = dir.join("inbox");
        std::fs::create_dir_all(&inbox_dir).unwrap();
        let large_body: Vec<u8> = (0..128 * 1024).map(|i| (i % 251) as u8).collect();
        std::fs::write(inbox_dir.join("doc.txt"), &large_body).unwrap();
        let (code, body) = raw_request(
            port,
            "GET",
            "/api/v1/inbox/doc.txt/content",
            Some(&auth),
            b"",
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(body, large_body);
        let (head, code, body) =
            raw_with_head(port, "GET", "/api/v1/inbox/doc.txt/content", Some(&auth)).await;
        assert_eq!(code, 200);
        assert!(head.contains("Content-Length: 131072"), "{head}");
        assert!(body.is_null(), "raw bytes must not be parsed as JSON");
        let (code, _) = raw_request(
            port,
            "GET",
            "/api/v1/inbox/nope.txt/content",
            Some(&auth),
            b"",
        )
        .await;
        assert_eq!(code, 404);
        let (code, _) = raw_request(
            port,
            "GET",
            "/api/v1/inbox/..%2Fetc%2Fpasswd/content",
            Some(&auth),
            b"",
        )
        .await;
        assert!(code == 400 || code == 404, "got {code}");

        // Client token keeps Read-tier GETs working; writes stay gated.
        let client = crate::apitoken::read_client(&dir).expect("client token");
        let client_auth = format!("Authorization: Bearer {client}");
        let (_, code, _) = raw_with_head(port, "GET", "/api/v1/offers", Some(&client_auth)).await;
        assert_eq!(code, 200);
        let (code, _) = raw_request(
            port,
            "POST",
            "/api/v1/offers/accept",
            Some(&client_auth),
            br#"{"all":true}"#,
        )
        .await;
        assert_eq!(code, 200);
        let (code, _) =
            raw_request(port, "DELETE", "/api/v1/outbox", Some(&client_auth), b"").await;
        assert_eq!(code, 200);

        // Unknown method on a new resource is 405 with the Allow set.
        let (code, resp) = raw_request(port, "POST", "/api/v1/storage", Some(&auth), b"").await;
        assert_eq!(code, 405, "{}", String::from_utf8_lossy(&resp));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §163: clipboard sync + audit trail v1 resources — status/policy/audit
    /// shapes, sparse patches, error classes, tier gates.
    #[tokio::test]
    async fn v1_clip_audit_serve_and_gate() {
        let (node, dir) = temp_test_node("v1_clip_audit");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("daemon token");
        let auth = format!("Authorization: Bearer {token}");

        // Status on a fresh headless test node: no text, counters zeroed.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/clip/status", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["has_text"], false, "{body}");
        assert_eq!(body["last_seq"], 0, "{body}");

        // The plaintext readback: has_text / clipboard_available present.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/clip", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["has_text"], false, "{body}");
        assert!(body["clipboard_available"].is_boolean(), "{body}");

        // Empty history is 200 with total 0.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/clip/history?limit=3", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["total"], 0, "{body}");
        assert_eq!(body["entries"], serde_json::json!([]), "{body}");

        // Policy readback (flat resource) + sparse PATCH.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/clip/policy", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["mode"], "bidirectional_auto", "{body}");
        assert!(body["policy"].is_null(), "nested legacy policy folded off");
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/clip/policy",
            Some(&auth),
            br#"{"mode":"send_only","sensitive_guard":false,"auto_clear_timeout_secs":0}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["mode"], "send_only", "{body}");
        assert_eq!(body["sensitive_guard"], false, "{body}");
        assert!(body["auto_clear_timeout_secs"].is_null(), "{body}");
        // Bad mode is 400.
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/clip/policy",
            Some(&auth),
            br#"{"mode":"bogus"}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Poll: one manual sweep tick (dispatch count + flat policy).
        let (code, resp) = raw_request(port, "POST", "/api/v1/clip/poll", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["dispatched_peers"], 0, "{body}");
        assert!(body["policy"]["mode"].is_string(), "{body}");

        // Sync with an explicit bogus peer is 400.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/clip/sync",
            Some(&auth),
            br#"{"peer":"nope"}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Clear with DELETE (history reset even headless).
        let (code, resp) = raw_request(port, "DELETE", "/api/v1/clip", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));

        // Audit trail: the policy PATCH above was itself audited (the ring
        // is shared state), so far exactly one entry; capacity is known.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/audit", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["total"], 1, "{body}");
        assert!(body["capacity"].as_u64().unwrap_or(0) > 0, "{body}");
        assert_eq!(body["entries"][0]["category"], "clipboard", "{body}");
        // Category/severity filters parse: `peer` excludes the clipboard
        // entry (returned 0; total tracks the engine ring, not the filter).
        let (_, code, body) = raw_with_head(
            port,
            "GET",
            "/api/v1/audit?category=peer&severity=alert&limit=5",
            Some(&auth),
        )
        .await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["returned"], 0, "{body}");
        // Unknown category is 400 (typed envelope).
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/audit?category=bogus", Some(&auth)).await;
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"]["code"], "bad_request", "{body}");
        // DELETE drains the ring.
        let (code, resp) = raw_request(port, "DELETE", "/api/v1/audit", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["cleared_count"], 1, "{body}");

        // Unknown method on a clip resource is 405.
        let (code, resp) = raw_request(port, "POST", "/api/v1/clip", Some(&auth), b"").await;
        assert_eq!(code, 405, "{}", String::from_utf8_lossy(&resp));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §164: remote action + link benchmark domains over real HTTP —
    /// local-action CRUD, policy GET/PATCH, bench policy + decision paths,
    /// recommendation aggregate, and the method-not-allowed gate.
    #[tokio::test]
    async fn v1_action_bench_serve_and_gate() {
        let (node, dir) = temp_test_node("v1_action_bench");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("daemon token");
        let auth = format!("Authorization: Bearer {token}");

        // Local actions start empty.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/action/local", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");
        assert_eq!(body["actions"], serde_json::json!([]), "{body}");

        // Register one action (strict body, optional fields default).
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/action/local",
            Some(&auth),
            br#"{"id":"backup","name":"Run backup","command":"/usr/local/bin/backup","args":["--full"]}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["count"], 1, "{body}");
        assert_eq!(body["actions"][0]["id"], "backup", "{body}");
        assert_eq!(body["actions"][0]["timeout_secs"], 10, "{body}");
        assert_eq!(body["actions"][0]["enabled"], true, "{body}");
        assert!(body["actions"][0]["description"].is_null(), "{body}");

        // Missing command is 400.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/action/local",
            Some(&auth),
            br#"{"id":"x","name":"x"}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Unknown-method gate: PATCH on a POST route is 405.
        let (code, resp) =
            raw_request(port, "PATCH", "/api/v1/action/local", Some(&auth), b"{}").await;
        assert_eq!(code, 405, "{}", String::from_utf8_lossy(&resp));

        // Remote actions for a bogus peer is 404 (peer not found).
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/action/remote?peer=nope", Some(&auth)).await;
        assert_eq!(code, 404, "{body}");
        assert_eq!(body["error"]["code"], "not_found", "{body}");

        // Missing peer is 400.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/action/remote", Some(&auth)).await;
        assert_eq!(code, 400, "{body}");

        // Trigger with an unknown peer is 404.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/action/trigger",
            Some(&auth),
            br#"{"peer":"nope","action_id":"backup"}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // History is empty on a fresh node.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/action/history", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["count"], 0, "{body}");

        // Policy readback (flat) + sparse PATCH.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/action/policy", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["mode"], "allow", "{body}");
        assert!(body["policy"].is_null(), "nested legacy policy folded off");
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/action/policy",
            Some(&auth),
            br#"{"mode":"prompt_only","max_timeout_secs":30}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["mode"], "prompt_only", "{body}");
        assert_eq!(body["max_timeout_secs"], 30, "{body}");
        // Bad mode is 400.
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/action/policy",
            Some(&auth),
            br#"{"mode":"bogus"}"#,
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Remove the action by id.
        let (code, resp) = raw_request(
            port,
            "DELETE",
            "/api/v1/action/local/backup",
            Some(&auth),
            b"",
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["count"], 0, "{body}");
        // Removing again is 400 (not found).
        let (code, resp) = raw_request(
            port,
            "DELETE",
            "/api/v1/action/local/backup",
            Some(&auth),
            b"",
        )
        .await;
        assert_eq!(code, 400, "{}", String::from_utf8_lossy(&resp));

        // Bench status on a fresh node: inactive, flat policy.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/bench", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["active"], false, "{body}");
        assert!(body["session"].is_null(), "{body}");
        assert!(body["last_report"].is_null(), "{body}");
        assert_eq!(body["policy"]["enabled"], true, "{body}");

        // Bench start with a bogus peer is 404.
        let (code, resp) = raw_request(
            port,
            "POST",
            "/api/v1/bench/start",
            Some(&auth),
            br#"{"peer":"nope"}"#,
        )
        .await;
        assert_eq!(code, 404, "{}", String::from_utf8_lossy(&resp));

        // Bench stop with nothing active: still a 200 decision report.
        let (code, resp) = raw_request(port, "POST", "/api/v1/bench/stop", Some(&auth), b"").await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));

        // Bench policy readback + sparse PATCH.
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/bench/policy", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert!(body["policy"].is_null(), "nested legacy policy folded off");
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/bench/policy",
            Some(&auth),
            br#"{"enabled":false,"allow_inbound_bench":false}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));
        let body: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        assert_eq!(body["enabled"], false, "{body}");
        assert_eq!(body["allow_inbound_bench"], false, "{body}");
        // Restore defaults.
        let (code, resp) = raw_request(
            port,
            "PATCH",
            "/api/v1/bench/policy",
            Some(&auth),
            br#"{"enabled":true,"allow_inbound_bench":true}"#,
        )
        .await;
        assert_eq!(code, 200, "{}", String::from_utf8_lossy(&resp));

        // Aggregate recommendation (no peers measured).
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/recommendation", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["peers"], serde_json::json!([]), "{body}");
        // Per-peer recommendation for a bogus peer is 404.
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/recommendation?peer=nope", Some(&auth)).await;
        assert_eq!(code, 404, "{body}");

        // Unknown method on a bench resource is 405.
        let (code, resp) = raw_request(port, "DELETE", "/api/v1/bench", Some(&auth), b"").await;
        assert_eq!(code, 405, "{}", String::from_utf8_lossy(&resp));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR-139 M4 switch: `/` serves the new SPA entry, `/assets/*` the
    /// embedded bundle — over real HTTP against the live listener.
    #[tokio::test]
    async fn m4_root_serves_spa_legacy_and_assets() {
        let (node, dir) = temp_test_node("v1_m4_switch");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // `/` is the SPA entry and references the rooted assets.
        let (code, body) = raw_request(port, "GET", "/", None, b"").await;
        assert_eq!(code, 200);
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("/assets/app.js"), "{html}");
        assert!(html.contains("/assets/index.css"), "{html}");

        // The referenced assets resolve with their content types.
        for (path, ctype) in [
            ("/assets/app.js", "text/javascript"),
            ("/assets/index.css", "text/css"),
        ] {
            let (code, body) = raw_request(port, "GET", path, None, b"").await;
            assert_eq!(code, 200, "GET {path}");
            assert!(!body.is_empty(), "GET {path} non-empty");
            let head = raw_head(port, path).await;
            assert!(head.contains(ctype), "GET {path}: {head}");
        }
        // Unknown assets 404 (fixed names; a miss is a bug, not an SPA route).
        let (code, _) = raw_request(port, "GET", "/assets/nope.js", None, b"").await;
        assert_eq!(code, 404);

        // /legacy/ was deleted with the M5 convergence; the old console 404s.
        for p in ["/legacy", "/legacy/", "/legacy/deep/link"] {
            let (code, _) = raw_request(port, "GET", p, None, b"").await;
            assert_eq!(code, 404, "GET {p}");
        }

        // Non-GET on the consoles falls through the GET-only arms to the
        // router's 404 fallback.
        let (code, _) = raw_request(port, "POST", "/", None, b"").await;
        assert_eq!(code, 404);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ADR-139 M1: device collection and item share the event DTO.
    #[tokio::test]
    async fn v1_devices_list_and_item() {
        let (node, dir) = temp_test_node("v1_m1_devices");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("token");
        let auth = format!("Authorization: Bearer {token}");
        let (_, code, _) = raw_with_head(port, "GET", "/api/v1/devices", None).await;
        assert_eq!(code, 401);
        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/devices", Some(&auth)).await;
        assert_eq!(code, 200, "{body}");
        let list: Vec<tetherly_api::Device> = serde_json::from_value(body).unwrap();
        assert!(list.is_empty());
        let (_, code, body) =
            raw_with_head(port, "GET", "/api/v1/devices/tdev_nope", Some(&auth)).await;
        assert_eq!(code, 404);
        assert_eq!(body["error"]["code"], "not_found");
        let (_, code, _) = raw_with_head(port, "POST", "/api/v1/devices", Some(&auth)).await;
        assert_eq!(code, 405);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Minimal SSE client for the v1 stream tests.
    struct V1Sse {
        s: tokio::net::TcpStream,
        buf: Vec<u8>,
        head: String,
    }

    impl V1Sse {
        async fn open(port: u16, path: &str, headers: &[&str]) -> Self {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let extra: String = headers.iter().map(|h| format!("{h}\r\n")).collect();
            let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{extra}\r\n");
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let split = loop {
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break i;
                }
                let k = tokio::time::timeout(std::time::Duration::from_secs(5), s.read(&mut chunk))
                    .await
                    .expect("sse head timeout")
                    .unwrap();
                assert!(k > 0, "closed before head");
                buf.extend_from_slice(&chunk[..k]);
            };
            let head = String::from_utf8_lossy(&buf[..split]).to_string();
            let rest = buf[split + 4..].to_vec();
            Self { s, buf: rest, head }
        }

        /// Next event frame `(id, event, data)`, skipping comments/retry.
        async fn next(&mut self) -> (u64, String, serde_json::Value) {
            use tokio::io::AsyncReadExt;
            let mut chunk = [0u8; 8192];
            loop {
                while let Some(i) = self.buf.windows(2).position(|w| w == b"\n\n") {
                    let block = String::from_utf8_lossy(&self.buf[..i]).to_string();
                    self.buf.drain(..i + 2);
                    let (mut id, mut ev, mut data) = (None, None, None);
                    for line in block.lines() {
                        if let Some(v) = line.strip_prefix("id: ") {
                            id = v.parse::<u64>().ok();
                        } else if let Some(v) = line.strip_prefix("event: ") {
                            ev = Some(v.to_string());
                        } else if let Some(v) = line.strip_prefix("data: ") {
                            data = serde_json::from_str(v).ok();
                        }
                    }
                    if let (Some(id), Some(ev), Some(data)) = (id, ev, data) {
                        return (id, ev, data);
                    }
                }
                let k = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.s.read(&mut chunk),
                )
                .await
                .expect("sse frame timeout")
                .unwrap();
                assert!(k > 0, "sse closed");
                self.buf.extend_from_slice(&chunk[..k]);
            }
        }
    }

    /// ADR-139 M1: snapshot first, coalesced full-state events with
    /// contiguous seq, Last-Event-ID replay, topic filter, auth.
    #[tokio::test]
    async fn v1_events_snapshot_replay_and_filters() {
        let (node, dir) = temp_test_node("v1_m1_events");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();
        let token = crate::apitoken::read(&dir).expect("token");
        let auth = format!("Authorization: Bearer {token}");

        let (_, code, body) = raw_with_head(port, "GET", "/api/v1/events", None).await;
        assert_eq!(code, 401, "{body}");
        let (_, code, body) = raw_with_head(
            port,
            "GET",
            "/api/v1/events?topics=devices,bogus",
            Some(&auth),
        )
        .await;
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"]["code"], "bad_request");
        let (_, code, _) = raw_with_head(port, "POST", "/api/v1/events", Some(&auth)).await;
        assert_eq!(code, 405);

        let mut a = V1Sse::open(port, "/api/v1/events", &[&auth]).await;
        assert!(a.head.starts_with("HTTP/1.1 200"), "{}", a.head);
        assert!(a.head.contains("text/event-stream"), "{}", a.head);
        assert!(a.head.contains("Tetherly-Api: 1"), "{}", a.head);
        let (snap_id, name, data) = a.next().await;
        assert_eq!(name, "snapshot");
        assert_eq!(data["seq"], snap_id);
        let snap: tetherly_api::EventEnvelope = serde_json::from_value(data).unwrap();
        let tetherly_api::Event::Snapshot { snapshot } = snap.event else {
            panic!("first frame must be a snapshot");
        };
        assert_eq!(
            snapshot.system.device_id,
            node.identity().device_id().to_string()
        );
        assert!(!snapshot.pairing.pin_active);

        let mut filtered = V1Sse::open(port, "/api/v1/events?topics=transfers", &[&auth]).await;
        assert_eq!(filtered.next().await.1, "snapshot");

        node.emit(crate::events::UiEvent::PinStateChanged { active: true });
        let (id1, name, data) = a.next().await;
        assert_eq!(name, "pairing.changed");
        assert_eq!(id1, snap_id + 1);
        assert_eq!(data["type"], "pairing.changed");
        assert!(data["data"]["pairing"]["pin_active"].is_boolean());
        node.emit(crate::events::UiEvent::FileDone {
            transfer_id: "t1".into(),
        });
        // M2c: transfers now carries full state instead of invalidated.
        let (id2, name, data) = a.next().await;
        assert_eq!((id2, name.as_str()), (id1 + 1, "transfers.changed"));
        assert!(data["data"]["transfers"]["active_count"].is_number());

        // The filtered stream never saw pairing.changed.
        let (fid, name, _) = filtered.next().await;
        assert_eq!((fid, name.as_str()), (id2, "transfers.changed"));

        // Resume: replay after id1, no snapshot.
        let resume = format!("Last-Event-ID: {id1}");
        let mut b = V1Sse::open(port, "/api/v1/events", &[&auth, &resume]).await;
        let (rid, name, _) = b.next().await;
        assert_eq!((rid, name.as_str()), (id2, "transfers.changed"));

        // A stale id (earlier daemon run) resyncs with a snapshot.
        let mut c = V1Sse::open(port, "/api/v1/events?last_event_id=5", &[&auth]).await;
        let (cid, name, _) = c.next().await;
        assert_eq!((cid, name.as_str()), (id2, "snapshot"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_cross_origin_requests_are_rejected() {
        let (node, dir) = temp_test_node("cross_origin");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));

        // A browser page on any other origin cannot forge loopback POSTs:
        // fetch(..., {mode:"no-cors"}) still carries the evil Origin header.
        let evil_req = format!(
            "POST /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
             Origin: https://evil.example\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        );
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        // curl/CLI style requests without Origin stay allowed (they hit the
        // real handler; a restart with empty body would kill this test node,
        // so use a read-only endpoint).
        let plain_req = format!(
            "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(plain_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 200);

        let _ = std::fs::remove_dir_all(&dir);
    }

    mod tray_keepalive_tests {
        use super::*;

        /// §147: a claimed tray must survive an idle-but-connected shell. The
        /// SSE ping is the only feed on a silent link, so the ping interval
        /// must be shorter than TRAY_TAKEOVER_RECLAIM_MS — production saw every
        /// claim reclaimed after ~10s because the 15s ping could never feed a
        /// 10s window.
        #[tokio::test]
        async fn v1_claimed_tray_survives_idle_sse_pings() {
            // Real clock: the watchdog compares wall-clock timestamps.
            let dir = std::env::temp_dir().join(format!(
                "tetherly-tray-ping-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let node = Node::start(NodeConfig {
                data_dir: dir.clone(),
                loopback_only: true,
                ui_port: 0,
                ..NodeConfig::default()
            })
            .unwrap();
            let (port, _token) = bind_and_spawn_with_token(node.clone_handle(), 0)
                .await
                .unwrap();
            let auth = format!(
                "Authorization: Bearer {}",
                crate::apitoken::read(&dir).expect("api.token minted")
            );

            let (code, _) = raw_request(
                port,
                "PATCH",
                "/api/v1/settings",
                Some(&auth),
                br#"{"tray_owner":"client"}"#,
            )
            .await;
            assert_eq!(code, 200);

            // The reclaim watchdog lives in the daemon binary's run arm; the
            // test replicates its 1s poll over the same public API so the
            // keepalive window is actually enforced here.
            let watchdog = node.clone_handle();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    if watchdog.tray_keepalive_expired()
                        && matches!(
                            watchdog.set_tray_owner(tetherly_api::TrayOwner::Daemon),
                            Ok((_, true))
                        )
                    {
                        break;
                    }
                }
            });

            // Open the shell's SSE connection and hold it open, idle.
            use tokio::io::AsyncWriteExt;
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let head = format!(
                "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}\r\nAccept: text/event-stream\r\n\r\n"
            );
            s.write_all(head.as_bytes()).await.unwrap();

            // One reclaim window plus margin — with the old 15s ping this
            // expires at ~10s; with the fixed 5s ping three pings feed it.
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
            assert_eq!(
                node.tray_owner(),
                tetherly_api::TrayOwner::Client,
                "watchdog reclaimed a claimed tray despite a live SSE subscriber"
            );
        }
    }
}
