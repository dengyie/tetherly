// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback-only desktop shell. Never bind 0.0.0.0. Bodies/OTP/PIN are not
//! written to logs. Full Tauri packaging is the next desktop-shell iteration.

use crate::runtime::{Node, OutboxStatus};
use crate::store::{ActionAccessMode, LocalActionDefinition};
use serde::Serialize;
use std::net::SocketAddr;
use tetherly_core::{
    ActionValidator, AudioChannelLayout, CameraCodec, DisplayEdge, MediaCommand, MicCodec,
    PlaybackState, PowerSource,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

// The loopback HTTP/SSE client used to be defined here and, separately, in
// `tetherly-cli`. It now lives in `tetherly-loopback` so both binaries share one
// implementation (and one timeout policy); re-exported so the existing
// `tetherly_node::loopback_*` call sites and the router tests are unchanged.
#[cfg(test)]
use tetherly_loopback::parse_http_response;
pub use tetherly_loopback::{loopback_delete, loopback_get, loopback_post, loopback_put};

const INDEX: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/index.html"));
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 2 * 1024 * 1024;

// ---- feature submodules (v0.1.64 / ADR-128) ----
mod api_action;
mod api_ancs;
mod api_audio;
mod api_audit;
mod api_bench;
mod api_camera;
mod api_clip;
mod api_core;
mod api_display;
mod api_files;
mod api_focus;
mod api_handoff;
mod api_input;
mod api_locate;
mod api_media;
mod api_messages;
mod api_mic;
mod api_notify;
mod api_peers;
mod api_power;
mod api_screen_analysis;
mod api_screen_overlay;
mod api_screen_quality;
mod api_screen_session;
mod api_screen_tile;
mod api_topology;
mod dto;
mod util;

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
    info!(port = bound, "ui listening (loopback only)");
    tokio::spawn(async move {
        if let Err(e) = accept_loop(listener, node, bound).await {
            warn!(%e, "ui http");
        }
    });
    Ok(bound)
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

async fn accept_loop(listener: TcpListener, node: Node, ui_port: u16) -> std::io::Result<()> {
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
                    if let Err(e) = handle(stream, node, ui_port).await {
                        warn!(%e, "ui request");
                    }
                });
            }
        }
    }
    Ok(())
}

async fn handle(mut stream: TcpStream, node: Node, ui_port: u16) -> std::io::Result<()> {
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
    let mut origin: Option<String> = None;
    let mut sec_fetch_site: Option<String> = None;
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
            _ => {}
        }
    }
    if !host_ok {
        write_http(&mut stream, 403, "text/plain", b"loopback only").await?;
        return Ok(());
    }
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

/// An active Server-Sent Events stream connected to a loopback UI endpoint.
///
/// A thin typed facade over [`tetherly_loopback::SseStream`]: the transport and
/// its deadlines live in the shared crate, while decoding into this crate's
/// [`UiEvent`](crate::events::UiEvent) has to stay here because that type does.
pub struct SseStream(tetherly_loopback::SseStream);

impl SseStream {
    /// Read the next event data string from the stream.
    /// Comments (lines starting with ':') and blank separator lines are skipped.
    /// Returns `Ok(Some(data))` when a `data: ...` event arrives,
    /// `Ok(None)` when the stream is closed, or `Err` on I/O error (including
    /// the shared client's idle deadline).
    pub async fn next_event(&mut self) -> std::io::Result<Option<String>> {
        self.0.next_event().await
    }

    /// Read the next typed `UiEvent` from the stream, skipping frames that do
    /// not decode.
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
    tetherly_loopback::loopback_events_connect(port)
        .await
        .map(SseStream)
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

// Generated router: every arm pattern stays here so the (method, path) table
// remains greppable in one place, while the bodies live in feature modules.

async fn route(method: &str, path: &str, body: &[u8], node: &Node) -> UiResponse {
    let (path_only, query_str) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    match (method, path_only) {
        ("GET", "/") | ("GET", "/index.html") => {
            api_core::get_index(method, query_str, body, node).await
        }
        ("GET", "/api/state") => api_core::get_state(method, query_str, body, node).await,
        ("GET", "/api/config") => api_core::get_config(method, query_str, body, node).await,
        ("POST", "/api/config") | ("PUT", "/api/config") => {
            api_core::post_config(method, query_str, body, node).await
        }
        ("GET", "/api/pin") => api_core::get_pin(method, query_str, body, node).await,
        ("POST", "/api/pin") => api_core::post_pin(method, query_str, body, node).await,
        ("POST", "/api/pin/clear") | ("DELETE", "/api/pin") => {
            api_core::post_pin_clear(method, query_str, body, node).await
        }
        ("POST", "/api/dial") => api_core::post_dial(method, query_str, body, node).await,
        ("POST", "/api/peers/unpair") => {
            api_peers::post_peers_unpair(method, query_str, body, node).await
        }
        ("POST", "/api/peers/connect") => {
            api_peers::post_peers_connect(method, query_str, body, node).await
        }
        ("POST", "/api/peers/disconnect") => {
            api_peers::post_peers_disconnect(method, query_str, body, node).await
        }
        ("GET", "/api/peers/blocked") | ("GET", "/api/blocked") => {
            api_peers::get_peers_blocked(method, query_str, body, node).await
        }
        ("POST", "/api/peers/block") => {
            api_peers::post_peers_block(method, query_str, body, node).await
        }
        ("POST", "/api/peers/unblock")
        | ("DELETE", "/api/peers/block")
        | ("DELETE", "/api/peers/blocked") => {
            api_peers::post_peers_unblock(method, query_str, body, node).await
        }
        ("POST", "/api/peers/alias") | ("PUT", "/api/peers/alias") => {
            api_peers::post_peers_alias(method, query_str, body, node).await
        }
        ("POST", "/api/peers/ping") => {
            api_peers::post_peers_ping(method, query_str, body, node).await
        }
        ("GET", "/api/peers/link") | ("GET", "/api/link") => {
            api_peers::get_peers_link(method, query_str, body, node).await
        }
        ("POST", "/api/peers/link/probe") | ("POST", "/api/link/probe") => {
            api_peers::post_peers_link_probe(method, query_str, body, node).await
        }
        ("GET", "/api/peers/heartbeat") | ("GET", "/api/heartbeat") => {
            api_peers::get_peers_heartbeat(method, query_str, body, node).await
        }
        ("POST", "/api/peers/heartbeat")
        | ("POST", "/api/heartbeat")
        | ("PUT", "/api/peers/heartbeat")
        | ("PUT", "/api/heartbeat") => {
            api_peers::post_peers_heartbeat(method, query_str, body, node).await
        }
        ("GET", "/api/peers") => api_peers::get_peers(method, query_str, body, node).await,
        ("GET", "/api/peers/profiles") | ("GET", "/api/profiles") => {
            api_peers::get_peers_profiles(method, query_str, body, node).await
        }
        ("GET", "/api/peers/profile") | ("GET", "/api/profile") => {
            api_peers::get_peers_profile(method, query_str, body, node).await
        }
        ("POST", "/api/peers/profile")
        | ("POST", "/api/profile")
        | ("PUT", "/api/peers/profile")
        | ("PUT", "/api/profile") => {
            api_peers::post_peers_profile(method, query_str, body, node).await
        }
        ("POST", "/api/peers/tags/add") | ("POST", "/api/tags/add") => {
            api_peers::post_peers_tags_add(method, query_str, body, node).await
        }
        ("POST", "/api/peers/tags/remove")
        | ("POST", "/api/tags/remove")
        | ("DELETE", "/api/peers/tags") => {
            api_peers::post_peers_tags_remove(method, query_str, body, node).await
        }
        ("POST", "/api/peers/group")
        | ("POST", "/api/group")
        | ("PUT", "/api/peers/group")
        | ("PUT", "/api/group") => api_peers::post_peers_group(method, query_str, body, node).await,
        ("GET", "/api/discovery") | ("GET", "/api/peers/discovered") => {
            api_peers::get_discovery(method, query_str, body, node).await
        }
        ("POST", "/api/discovery/rescan") | ("POST", "/api/peers/discover") => {
            api_peers::post_discovery_rescan(method, query_str, body, node).await
        }
        ("GET", "/api/caps") => api_core::get_caps(method, query_str, body, node).await,
        ("POST", "/api/caps") | ("PUT", "/api/caps") => {
            api_core::post_caps(method, query_str, body, node).await
        }
        ("GET", "/api/peers/caps") => {
            api_peers::get_peers_caps(method, query_str, body, node).await
        }
        ("GET", "/api/routes") => api_core::get_routes(method, query_str, body, node).await,
        ("POST", "/api/routes") | ("PUT", "/api/routes") => {
            api_core::post_routes(method, query_str, body, node).await
        }
        ("DELETE", "/api/routes")
        | ("POST", "/api/routes/remove")
        | ("POST", "/api/routes/delete") => {
            api_core::delete_routes(method, query_str, body, node).await
        }
        ("GET", "/api/net") => api_core::get_net(method, query_str, body, node).await,
        ("POST", "/api/net/refresh") | ("POST", "/api/net/probe") => {
            api_core::post_net_refresh(method, query_str, body, node).await
        }
        ("GET", "/api/metrics") | ("GET", "/api/traffic") => {
            api_core::get_metrics(method, query_str, body, node).await
        }
        ("POST", "/api/metrics/reset") | ("POST", "/api/traffic/reset") => {
            api_core::post_metrics_reset(method, query_str, body, node).await
        }
        ("GET", "/api/clip/policy") => {
            api_clip::get_clip_policy(method, query_str, body, node).await
        }
        ("POST", "/api/clip/policy") | ("PUT", "/api/clip/policy") => {
            api_clip::post_clip_policy(method, query_str, body, node).await
        }
        ("POST", "/api/clip/auto-sync/poll") => {
            api_clip::post_clip_auto_sync_poll(method, query_str, body, node).await
        }
        ("GET", "/api/clip/status") => {
            api_clip::get_clip_status(method, query_str, body, node).await
        }
        ("GET", "/api/clip/history") => {
            api_clip::get_clip_history(method, query_str, body, node).await
        }
        ("POST", "/api/clip/clear") | ("DELETE", "/api/clip") => {
            api_clip::post_clip_clear(method, query_str, body, node).await
        }
        ("GET", "/api/clip") => api_clip::get_clip(method, query_str, body, node).await,
        ("POST", "/api/clip/sync") => api_clip::post_clip_sync(method, query_str, body, node).await,
        ("POST", "/api/notify") => api_notify::post_notify(method, query_str, body, node).await,
        ("GET", "/api/notify/candidates") => {
            api_notify::get_notify_candidates(method, query_str, body, node).await
        }
        ("GET", "/api/notify/replies") => {
            api_notify::get_notify_replies(method, query_str, body, node).await
        }
        ("POST", "/api/otp/get") => api_notify::post_otp_get(method, query_str, body, node).await,
        ("POST", "/api/copy") => api_notify::post_copy(method, query_str, body, node).await,
        ("POST", "/api/insert") => api_notify::post_insert(method, query_str, body, node).await,
        ("POST", "/api/dismiss") => api_notify::post_dismiss(method, query_str, body, node).await,
        ("POST", "/api/notify/dismiss") | ("DELETE", "/api/notify/candidates") => {
            api_notify::post_notify_dismiss(method, query_str, body, node).await
        }
        ("POST", "/api/open") => api_notify::post_open(method, query_str, body, node).await,
        ("POST", "/api/reply") => api_notify::post_reply(method, query_str, body, node).await,
        ("POST", "/api/reply/take") => {
            api_notify::post_reply_take(method, query_str, body, node).await
        }
        ("GET", "/api/notify/config") | ("GET", "/api/notify/banner") => {
            api_notify::get_notify_config(method, query_str, body, node).await
        }
        ("POST", "/api/notify/config") | ("POST", "/api/notify/banner") => {
            api_notify::post_notify_config(method, query_str, body, node).await
        }
        ("POST", "/api/notify/test") => {
            api_notify::post_notify_test(method, query_str, body, node).await
        }
        ("GET", "/api/file/transfers")
        | ("GET", "/api/transfers")
        | ("POST", "/api/file/transfers")
        | ("POST", "/api/transfers") => {
            api_files::get_file_transfers(method, query_str, body, node).await
        }
        ("GET", "/api/file/offers") | ("GET", "/api/offers") => {
            api_files::get_file_offers(method, query_str, body, node).await
        }
        ("POST", "/api/file/accept") | ("POST", "/api/accept") => {
            api_files::post_file_accept(method, query_str, body, node).await
        }
        ("POST", "/api/file/reject") | ("POST", "/api/reject") => {
            api_files::post_file_reject(method, query_str, body, node).await
        }
        ("GET", "/api/file/auto-accept/policy") | ("GET", "/api/auto-accept/policy") => {
            api_files::get_file_auto_accept_policy(method, query_str, body, node).await
        }
        ("POST", "/api/file/auto-accept/policy")
        | ("PUT", "/api/file/auto-accept/policy")
        | ("POST", "/api/auto-accept/policy") => {
            api_files::post_file_auto_accept_policy(method, query_str, body, node).await
        }
        ("GET", "/api/file/auto-accept") | ("GET", "/api/auto-accept") => {
            api_files::get_file_auto_accept(method, query_str, body, node).await
        }
        ("POST", "/api/file/auto-accept")
        | ("PUT", "/api/file/auto-accept")
        | ("POST", "/api/auto-accept") => {
            api_files::post_file_auto_accept(method, query_str, body, node).await
        }
        ("POST", "/api/file/cancel") => {
            api_files::post_file_cancel(method, query_str, body, node).await
        }
        ("POST", "/api/file/clear") => {
            api_files::post_file_clear(method, query_str, body, node).await
        }
        ("POST", "/api/file/send-multi") => {
            api_files::post_file_send_multi(method, query_str, body, node).await
        }
        ("POST", "/api/file/send") => {
            api_files::post_file_send(method, query_str, body, node).await
        }
        ("POST", "/api/file/send-batch") | ("POST", "/api/file/batch") => {
            api_files::post_file_send_batch(method, query_str, body, node).await
        }
        ("GET", "/api/file/resume") | ("GET", "/api/storage/resume") => {
            api_files::get_file_resume(method, query_str, body, node).await
        }
        ("GET", "/api/ancs") => api_ancs::get_ancs(method, query_str, body, node).await,
        ("POST", "/api/ancs/connect") => {
            api_ancs::post_ancs_connect(method, query_str, body, node).await
        }
        ("POST", "/api/ancs/disconnect") => {
            api_ancs::post_ancs_disconnect(method, query_str, body, node).await
        }
        ("POST", "/api/ancs/allow") | ("POST", "/api/ancs/rules/add") => {
            api_ancs::post_ancs_allow(method, query_str, body, node).await
        }
        ("DELETE", "/api/ancs/allow")
        | ("POST", "/api/ancs/rules/remove")
        | ("DELETE", "/api/ancs/rules") => {
            api_ancs::delete_ancs_allow(method, query_str, body, node).await
        }
        ("POST", "/api/ancs/rules/clear") | ("DELETE", "/api/ancs/rules/clear") => {
            api_ancs::post_ancs_rules_clear(method, query_str, body, node).await
        }
        ("GET", "/api/storage") => api_files::get_storage(method, query_str, body, node).await,
        ("POST", "/api/storage/clean") => {
            api_files::post_storage_clean(method, query_str, body, node).await
        }
        ("GET", "/api/file/inbox") | ("GET", "/api/inbox") => {
            api_files::get_file_inbox(method, query_str, body, node).await
        }
        ("POST", "/api/file/inbox/delete") | ("DELETE", "/api/file/inbox") => {
            api_files::post_file_inbox_delete(method, query_str, body, node).await
        }
        ("POST", "/api/file/inbox/clear") | ("DELETE", "/api/file/inbox/all") => {
            api_files::post_file_inbox_clear(method, query_str, body, node).await
        }
        ("GET", "/api/file/inbox/download") | ("GET", "/api/inbox/download") => {
            api_files::get_file_inbox_download(method, query_str, body, node).await
        }
        ("GET", "/api/file/inbox/raw") | ("GET", "/api/inbox/raw") => {
            api_files::get_file_inbox_raw(method, query_str, body, node).await
        }
        ("POST", "/api/file/inbox/open") | ("POST", "/api/inbox/open") => {
            api_files::post_file_inbox_open(method, query_str, body, node).await
        }
        ("GET", "/api/file/outbox")
        | ("GET", "/api/outbox")
        | ("POST", "/api/file/outbox/query")
        | ("POST", "/api/outbox/query") => {
            api_files::get_file_outbox(method, query_str, body, node).await
        }
        ("POST", "/api/file/outbox/clear")
        | ("POST", "/api/outbox/clear")
        | ("DELETE", "/api/file/outbox")
        | ("DELETE", "/api/outbox") => {
            api_files::post_file_outbox_clear(method, query_str, body, node).await
        }
        ("POST", "/api/file/outbox/resend") | ("POST", "/api/outbox/resend") => {
            api_files::post_file_outbox_resend(method, query_str, body, node).await
        }
        ("GET", "/api/health") => api_core::get_health(method, query_str, body, node).await,
        ("GET", "/api/sys") | ("GET", "/api/system") | ("GET", "/api/process") => {
            api_core::get_sys(method, query_str, body, node).await
        }
        ("POST", "/api/daemon/shutdown") | ("POST", "/api/daemon/stop") => {
            api_core::post_daemon_shutdown(method, query_str, body, node).await
        }
        ("POST", "/api/daemon/restart") | ("POST", "/api/restart") => {
            api_core::post_daemon_restart(method, query_str, body, node).await
        }
        ("GET", "/api/screen") => {
            api_screen_session::get_screen(method, query_str, body, node).await
        }
        ("POST", "/api/screen/allow") => {
            api_screen_session::post_screen_allow(method, query_str, body, node).await
        }
        ("POST", "/api/screen/revoke") => {
            api_screen_session::post_screen_revoke(method, query_str, body, node).await
        }
        ("POST", "/api/screen/start") => {
            api_screen_session::post_screen_start(method, query_str, body, node).await
        }
        ("POST", "/api/screen/stop") => {
            api_screen_session::post_screen_stop(method, query_str, body, node).await
        }
        ("POST", "/api/screen/keyframe") => {
            api_screen_session::post_screen_keyframe(method, query_str, body, node).await
        }
        ("GET", "/api/screen/bitrate") => {
            api_screen_quality::get_screen_bitrate(method, query_str, body, node).await
        }
        ("POST", "/api/screen/bitrate") => {
            api_screen_quality::post_screen_bitrate(method, query_str, body, node).await
        }
        ("GET", "/api/screen/fps") => {
            api_screen_quality::get_screen_fps(method, query_str, body, node).await
        }
        ("POST", "/api/screen/fps") => {
            api_screen_quality::post_screen_fps(method, query_str, body, node).await
        }
        ("GET", "/api/screen/scale") => {
            api_screen_quality::get_screen_scale(method, query_str, body, node).await
        }
        ("POST", "/api/screen/scale") => {
            api_screen_quality::post_screen_scale(method, query_str, body, node).await
        }
        ("GET", "/api/screen/profile") => {
            api_screen_quality::get_screen_profile(method, query_str, body, node).await
        }
        ("POST", "/api/screen/profile") => {
            api_screen_quality::post_screen_profile(method, query_str, body, node).await
        }
        ("GET", "/api/screen/matrix") => {
            api_screen_quality::get_screen_matrix(method, query_str, body, node).await
        }
        ("POST", "/api/screen/matrix") => {
            api_screen_quality::post_screen_matrix(method, query_str, body, node).await
        }
        ("GET", "/api/screen/playout") => {
            api_screen_quality::get_screen_playout(method, query_str, body, node).await
        }
        ("POST", "/api/screen/playout") => {
            api_screen_quality::post_screen_playout(method, query_str, body, node).await
        }
        ("GET", "/api/screen/cursor") => {
            api_screen_analysis::get_screen_cursor(method, query_str, body, node).await
        }
        ("POST", "/api/screen/cursor") => {
            api_screen_analysis::post_screen_cursor(method, query_str, body, node).await
        }
        ("GET", "/api/screen/squelch") => {
            api_screen_quality::get_screen_squelch(method, query_str, body, node).await
        }
        ("POST", "/api/screen/squelch") => {
            api_screen_quality::post_screen_squelch(method, query_str, body, node).await
        }
        ("GET", "/api/screen/fec") => {
            api_screen_quality::get_screen_fec(method, query_str, body, node).await
        }
        ("POST", "/api/screen/fec") => {
            api_screen_quality::post_screen_fec(method, query_str, body, node).await
        }
        ("GET", "/api/screen/pacer") => {
            api_screen_quality::get_screen_pacer(method, query_str, body, node).await
        }
        ("POST", "/api/screen/pacer") => {
            api_screen_quality::post_screen_pacer(method, query_str, body, node).await
        }
        ("GET", "/api/screen/trendline") => {
            api_screen_analysis::get_screen_trendline(method, query_str, body, node).await
        }
        ("POST", "/api/screen/trendline") => {
            api_screen_analysis::post_screen_trendline(method, query_str, body, node).await
        }
        ("GET", "/api/screen/harq") => {
            api_screen_quality::get_screen_harq(method, query_str, body, node).await
        }
        ("POST", "/api/screen/harq") => {
            api_screen_quality::post_screen_harq(method, query_str, body, node).await
        }
        ("GET", "/api/screen/twcc") => {
            api_screen_quality::get_screen_twcc(method, query_str, body, node).await
        }
        ("POST", "/api/screen/twcc") => {
            api_screen_quality::post_screen_twcc(method, query_str, body, node).await
        }
        ("GET", "/api/screen/probe") => {
            api_screen_analysis::get_screen_probe(method, query_str, body, node).await
        }
        ("POST", "/api/screen/probe") => {
            api_screen_analysis::post_screen_probe(method, query_str, body, node).await
        }
        ("POST", "/api/screen/probe/trigger") => {
            api_screen_analysis::post_screen_probe_trigger(method, query_str, body, node).await
        }
        ("GET", "/api/screen/sync") => {
            api_screen_quality::get_screen_sync(method, query_str, body, node).await
        }
        ("POST", "/api/screen/sync") => {
            api_screen_quality::post_screen_sync(method, query_str, body, node).await
        }
        ("POST", "/api/screen/sync/trigger") => {
            api_screen_quality::post_screen_sync_trigger(method, query_str, body, node).await
        }
        ("GET", "/api/screen/displays") => {
            api_screen_session::get_screen_displays(method, query_str, body, node).await
        }
        ("POST", "/api/screen/switch") => {
            api_screen_session::post_screen_switch(method, query_str, body, node).await
        }
        ("GET", "/api/screen/privacy") => {
            api_screen_analysis::get_screen_privacy(method, query_str, body, node).await
        }
        ("POST", "/api/screen/privacy") => {
            api_screen_analysis::post_screen_privacy(method, query_str, body, node).await
        }
        ("POST", "/api/screen/privacy/enable") => {
            api_screen_analysis::post_screen_privacy_enable(method, query_str, body, node).await
        }
        ("POST", "/api/screen/privacy/disable") => {
            api_screen_analysis::post_screen_privacy_disable(method, query_str, body, node).await
        }
        ("GET", "/api/screen/latency") => {
            api_screen_quality::get_screen_latency(method, query_str, body, node).await
        }
        ("POST", "/api/screen/latency") => {
            api_screen_quality::post_screen_latency(method, query_str, body, node).await
        }
        ("POST", "/api/screen/latency/reset") => {
            api_screen_quality::post_screen_latency_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tuning") => {
            api_screen_quality::get_screen_tuning(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tuning") => {
            api_screen_quality::post_screen_tuning(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tuning/reset") => {
            api_screen_quality::post_screen_tuning_reset(method, query_str, body, node).await
        }
        ("POST", "/api/screen/recording/start") => {
            api_screen_session::post_screen_recording_start(method, query_str, body, node).await
        }
        ("POST", "/api/screen/recording/stop") => {
            api_screen_session::post_screen_recording_stop(method, query_str, body, node).await
        }
        ("GET", "/api/screen/recording") => {
            api_screen_session::get_screen_recording(method, query_str, body, node).await
        }
        ("GET", "/api/screen/recording/replay") => {
            api_screen_session::get_screen_recording_replay(method, query_str, body, node).await
        }
        ("POST", "/api/screen/playback/start") => {
            api_screen_session::post_screen_playback_start(method, query_str, body, node).await
        }
        ("POST", "/api/screen/playback/pause") => {
            api_screen_session::post_screen_playback_pause(method, query_str, body, node).await
        }
        ("POST", "/api/screen/playback/resume") => {
            api_screen_session::post_screen_playback_resume(method, query_str, body, node).await
        }
        ("POST", "/api/screen/playback/stop") => {
            api_screen_session::post_screen_playback_stop(method, query_str, body, node).await
        }
        ("GET", "/api/screen/playback") => {
            api_screen_session::get_screen_playback(method, query_str, body, node).await
        }
        ("GET", "/api/screen/snapshot") => {
            api_screen_session::get_screen_snapshot(method, query_str, body, node).await
        }
        ("POST", "/api/screen/recording/snapshot") | ("GET", "/api/screen/recording/snapshot") => {
            api_screen_session::post_screen_recording_snapshot(method, query_str, body, node).await
        }
        ("GET", "/api/screen/watermark") => {
            api_screen_overlay::get_screen_watermark(method, query_str, body, node).await
        }
        ("POST", "/api/screen/watermark") => {
            api_screen_overlay::post_screen_watermark(method, query_str, body, node).await
        }
        ("POST", "/api/screen/watermark/enable") => {
            api_screen_overlay::post_screen_watermark_enable(method, query_str, body, node).await
        }
        ("POST", "/api/screen/watermark/disable") => {
            api_screen_overlay::post_screen_watermark_disable(method, query_str, body, node).await
        }
        ("GET", "/api/screen/annotation") => {
            api_screen_overlay::get_screen_annotation(method, query_str, body, node).await
        }
        ("POST", "/api/screen/annotation") => {
            api_screen_overlay::post_screen_annotation(method, query_str, body, node).await
        }
        ("POST", "/api/screen/annotation/clear") | ("DELETE", "/api/screen/annotation") => {
            api_screen_overlay::post_screen_annotation_clear(method, query_str, body, node).await
        }
        ("GET", "/api/screen/laser") => {
            api_screen_overlay::get_screen_laser(method, query_str, body, node).await
        }
        ("POST", "/api/screen/laser") => {
            api_screen_overlay::post_screen_laser(method, query_str, body, node).await
        }
        ("POST", "/api/screen/whiteboard/undo") => {
            api_screen_overlay::post_screen_whiteboard_undo(method, query_str, body, node).await
        }
        ("POST", "/api/screen/whiteboard/redo") => {
            api_screen_overlay::post_screen_whiteboard_redo(method, query_str, body, node).await
        }
        ("GET", "/api/screen/whiteboard/export") => {
            api_screen_overlay::get_screen_whiteboard_export(method, query_str, body, node).await
        }
        ("POST", "/api/screen/whiteboard/import") => {
            api_screen_overlay::post_screen_whiteboard_import(method, query_str, body, node).await
        }
        ("GET", "/api/screen/whiteboard/policy") => {
            api_screen_overlay::get_screen_whiteboard_policy(method, query_str, body, node).await
        }
        ("POST", "/api/screen/whiteboard/policy") => {
            api_screen_overlay::post_screen_whiteboard_policy(method, query_str, body, node).await
        }
        ("POST", "/api/screen/whiteboard/clear") | ("DELETE", "/api/screen/whiteboard") => {
            api_screen_overlay::post_screen_whiteboard_clear(method, query_str, body, node).await
        }
        ("GET", "/api/screen/roi") => {
            api_screen_analysis::get_screen_roi(method, query_str, body, node).await
        }
        ("POST", "/api/screen/roi") => {
            api_screen_analysis::post_screen_roi(method, query_str, body, node).await
        }
        ("POST", "/api/screen/roi/reset") | ("DELETE", "/api/screen/roi") => {
            api_screen_analysis::post_screen_roi_reset(method, query_str, body, node).await
        }
        ("POST", "/api/screen/roi/zoom") => {
            api_screen_analysis::post_screen_roi_zoom(method, query_str, body, node).await
        }
        ("GET", "/api/screen/color") => {
            api_screen_quality::get_screen_color(method, query_str, body, node).await
        }
        ("POST", "/api/screen/color") => {
            api_screen_quality::post_screen_color(method, query_str, body, node).await
        }
        ("POST", "/api/screen/color/reset") | ("DELETE", "/api/screen/color") => {
            api_screen_quality::post_screen_color_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/delta") => {
            api_screen_quality::get_screen_delta(method, query_str, body, node).await
        }
        ("POST", "/api/screen/delta") => {
            api_screen_quality::post_screen_delta(method, query_str, body, node).await
        }
        ("POST", "/api/screen/delta/reset") | ("DELETE", "/api/screen/delta") => {
            api_screen_quality::post_screen_delta_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/metric") => {
            api_screen_analysis::get_screen_metric(method, query_str, body, node).await
        }
        ("POST", "/api/screen/metric") => {
            api_screen_analysis::post_screen_metric(method, query_str, body, node).await
        }
        ("POST", "/api/screen/metric/reset") | ("DELETE", "/api/screen/metric") => {
            api_screen_analysis::post_screen_metric_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/qoe") => {
            api_screen_analysis::get_screen_qoe(method, query_str, body, node).await
        }
        ("POST", "/api/screen/qoe") => {
            api_screen_analysis::post_screen_qoe(method, query_str, body, node).await
        }
        ("POST", "/api/screen/qoe/reset") | ("DELETE", "/api/screen/qoe") => {
            api_screen_analysis::post_screen_qoe_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/pir") => {
            api_screen_analysis::get_screen_pir(method, query_str, body, node).await
        }
        ("POST", "/api/screen/pir") => {
            api_screen_analysis::post_screen_pir(method, query_str, body, node).await
        }
        ("POST", "/api/screen/pir/reset") | ("DELETE", "/api/screen/pir") => {
            api_screen_analysis::post_screen_pir_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/ltr") => {
            api_screen_session::get_screen_ltr(method, query_str, body, node).await
        }
        ("POST", "/api/screen/ltr") => {
            api_screen_session::post_screen_ltr(method, query_str, body, node).await
        }
        ("POST", "/api/screen/ltr/anchor") => {
            api_screen_session::post_screen_ltr_anchor(method, query_str, body, node).await
        }
        ("POST", "/api/screen/ltr/recover") => {
            api_screen_session::post_screen_ltr_recover(method, query_str, body, node).await
        }
        ("POST", "/api/screen/ltr/reset") | ("DELETE", "/api/screen/ltr") => {
            api_screen_session::post_screen_ltr_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/svc") => {
            api_screen_session::get_screen_svc(method, query_str, body, node).await
        }
        ("POST", "/api/screen/svc") => {
            api_screen_session::post_screen_svc(method, query_str, body, node).await
        }
        ("POST", "/api/screen/svc/drop-t1") => {
            api_screen_session::post_screen_svc_drop_t1(method, query_str, body, node).await
        }
        ("POST", "/api/screen/svc/reset") | ("DELETE", "/api/screen/svc") => {
            api_screen_session::post_screen_svc_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/motion") => {
            api_screen_analysis::get_screen_motion(method, query_str, body, node).await
        }
        ("POST", "/api/screen/motion") => {
            api_screen_analysis::post_screen_motion(method, query_str, body, node).await
        }
        ("POST", "/api/screen/motion/reset") | ("DELETE", "/api/screen/motion") => {
            api_screen_analysis::post_screen_motion_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile") => {
            api_screen_tile::get_screen_tile(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile") => {
            api_screen_tile::post_screen_tile(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/reset") | ("DELETE", "/api/screen/tile") => {
            api_screen_tile::post_screen_tile_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/cache") => {
            api_screen_tile::get_screen_tile_cache(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/cache") => {
            api_screen_tile::post_screen_tile_cache(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/cache/reset") | ("DELETE", "/api/screen/tile/cache") => {
            api_screen_tile::post_screen_tile_cache_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/focus") => {
            api_screen_analysis::get_screen_focus(method, query_str, body, node).await
        }
        ("POST", "/api/screen/focus") => {
            api_screen_analysis::post_screen_focus(method, query_str, body, node).await
        }
        ("POST", "/api/screen/focus/reset") | ("DELETE", "/api/screen/focus") => {
            api_screen_analysis::post_screen_focus_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/palette") => {
            api_screen_tile::get_screen_tile_palette(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/palette") => {
            api_screen_tile::post_screen_tile_palette(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/palette/reset") | ("DELETE", "/api/screen/tile/palette") => {
            api_screen_tile::post_screen_tile_palette_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/predict") => {
            api_screen_tile::get_screen_tile_predict(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/predict") => {
            api_screen_tile::post_screen_tile_predict(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/predict/reset") | ("DELETE", "/api/screen/tile/predict") => {
            api_screen_tile::post_screen_tile_predict_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/present") => {
            api_screen_tile::get_screen_tile_present(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/present") => {
            api_screen_tile::post_screen_tile_present(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/present/reset") | ("DELETE", "/api/screen/tile/present") => {
            api_screen_tile::post_screen_tile_present_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/nack") => {
            api_screen_tile::get_screen_tile_nack(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/nack") => {
            api_screen_tile::post_screen_tile_nack(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/nack/reset") | ("DELETE", "/api/screen/tile/nack") => {
            api_screen_tile::post_screen_tile_nack_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/saliency") => {
            api_screen_tile::get_screen_tile_saliency(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/saliency") => {
            api_screen_tile::post_screen_tile_saliency(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/saliency/reset")
        | ("DELETE", "/api/screen/tile/saliency/reset")
        | ("DELETE", "/api/screen/tile/saliency") => {
            api_screen_tile::post_screen_tile_saliency_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/lod") => {
            api_screen_tile::get_screen_tile_lod(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/lod") => {
            api_screen_tile::post_screen_tile_lod(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/lod/reset")
        | ("DELETE", "/api/screen/tile/lod/reset")
        | ("DELETE", "/api/screen/tile/lod") => {
            api_screen_tile::post_screen_tile_lod_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/batch") | ("GET", "/api/screen/batch") => {
            api_screen_tile::get_screen_tile_batch(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/batch") | ("POST", "/api/screen/batch") => {
            api_screen_tile::post_screen_tile_batch(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/batch/reset")
        | ("DELETE", "/api/screen/tile/batch/reset")
        | ("DELETE", "/api/screen/tile/batch")
        | ("POST", "/api/screen/batch/reset")
        | ("DELETE", "/api/screen/batch/reset")
        | ("DELETE", "/api/screen/batch") => {
            api_screen_tile::post_screen_tile_batch_reset(method, query_str, body, node).await
        }
        ("GET", "/api/screen/tile/motion") => {
            api_screen_tile::get_screen_tile_motion(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/motion") => {
            api_screen_tile::post_screen_tile_motion(method, query_str, body, node).await
        }
        ("POST", "/api/screen/tile/motion/reset")
        | ("DELETE", "/api/screen/tile/motion/reset")
        | ("DELETE", "/api/screen/tile/motion") => {
            api_screen_tile::post_screen_tile_motion_reset(method, query_str, body, node).await
        }
        ("GET", "/api/input") => api_input::get_input(method, query_str, body, node).await,
        ("POST", "/api/input/start") => {
            api_input::post_input_start(method, query_str, body, node).await
        }
        ("POST", "/api/input/stop") => {
            api_input::post_input_stop(method, query_str, body, node).await
        }
        ("GET", "/api/input/tunnel") | ("GET", "/api/input/tunnel/status") => {
            api_input::get_input_tunnel(method, query_str, body, node).await
        }
        ("POST", "/api/input/tunnel/attach") | ("PUT", "/api/input/tunnel/attach") => {
            api_input::post_input_tunnel_attach(method, query_str, body, node).await
        }
        ("POST", "/api/input/tunnel/release") | ("PUT", "/api/input/tunnel/release") => {
            api_input::post_input_tunnel_release(method, query_str, body, node).await
        }
        ("GET", "/api/input/policy") => {
            api_input::get_input_policy(method, query_str, body, node).await
        }
        ("POST", "/api/input/policy") | ("PUT", "/api/input/policy") => {
            api_input::post_input_policy(method, query_str, body, node).await
        }
        ("POST", "/api/input/accessibility/open") => {
            api_input::post_input_accessibility_open(method, query_str, body, node).await
        }
        ("POST", "/api/input/ensure") => {
            api_input::post_input_ensure(method, query_str, body, node).await
        }
        ("GET", "/api/display/virtual") | ("GET", "/api/display/virtual/status") => {
            api_display::get_display_virtual(method, query_str, body, node).await
        }
        ("POST", "/api/display/virtual/attach") | ("PUT", "/api/display/virtual/attach") => {
            api_display::post_display_virtual_attach(method, query_str, body, node).await
        }
        ("POST", "/api/display/virtual/detach") | ("PUT", "/api/display/virtual/detach") => {
            api_display::post_display_virtual_detach(method, query_str, body, node).await
        }
        ("GET", "/api/display/virtual/policy") => {
            api_display::get_display_virtual_policy(method, query_str, body, node).await
        }
        ("POST", "/api/display/virtual/policy") | ("PUT", "/api/display/virtual/policy") => {
            api_display::post_display_virtual_policy(method, query_str, body, node).await
        }
        ("GET", "/api/audio/routing") | ("GET", "/api/audio/routing/status") => {
            api_audio::get_audio_routing(method, query_str, body, node).await
        }
        ("POST", "/api/audio/routing/attach") | ("PUT", "/api/audio/routing/attach") => {
            api_audio::post_audio_routing_attach(method, query_str, body, node).await
        }
        ("POST", "/api/audio/routing/detach") | ("PUT", "/api/audio/routing/detach") => {
            api_audio::post_audio_routing_detach(method, query_str, body, node).await
        }
        ("GET", "/api/audio/routing/policy") => {
            api_audio::get_audio_routing_policy(method, query_str, body, node).await
        }
        ("POST", "/api/audio/routing/policy") | ("PUT", "/api/audio/routing/policy") => {
            api_audio::post_audio_routing_policy(method, query_str, body, node).await
        }
        ("GET", "/api/camera") | ("GET", "/api/camera/status") => {
            api_camera::get_camera(method, query_str, body, node).await
        }
        ("POST", "/api/camera/attach") | ("PUT", "/api/camera/attach") => {
            api_camera::post_camera_attach(method, query_str, body, node).await
        }
        ("POST", "/api/camera/detach") | ("PUT", "/api/camera/detach") => {
            api_camera::post_camera_detach(method, query_str, body, node).await
        }
        ("GET", "/api/camera/policy") => {
            api_camera::get_camera_policy(method, query_str, body, node).await
        }
        ("POST", "/api/camera/policy") | ("PUT", "/api/camera/policy") => {
            api_camera::post_camera_policy(method, query_str, body, node).await
        }
        ("GET", "/api/mic") | ("GET", "/api/mic/status") | ("GET", "/api/microphone") => {
            api_mic::get_mic(method, query_str, body, node).await
        }
        ("POST", "/api/mic/attach")
        | ("PUT", "/api/mic/attach")
        | ("POST", "/api/microphone/attach")
        | ("PUT", "/api/microphone/attach") => {
            api_mic::post_mic_attach(method, query_str, body, node).await
        }
        ("POST", "/api/mic/detach")
        | ("PUT", "/api/mic/detach")
        | ("POST", "/api/microphone/detach")
        | ("PUT", "/api/microphone/detach") => {
            api_mic::post_mic_detach(method, query_str, body, node).await
        }
        ("GET", "/api/mic/policy") | ("GET", "/api/microphone/policy") => {
            api_mic::get_mic_policy(method, query_str, body, node).await
        }
        ("POST", "/api/mic/policy")
        | ("PUT", "/api/mic/policy")
        | ("POST", "/api/microphone/policy")
        | ("PUT", "/api/microphone/policy") => {
            api_mic::post_mic_policy(method, query_str, body, node).await
        }
        ("GET", "/api/bench") | ("GET", "/api/bench/status") => {
            api_bench::get_bench(method, query_str, body, node).await
        }
        ("POST", "/api/bench/start") | ("PUT", "/api/bench/start") => {
            api_bench::post_bench_start(method, query_str, body, node).await
        }
        ("POST", "/api/bench/stop") | ("PUT", "/api/bench/stop") => {
            api_bench::post_bench_stop(method, query_str, body, node).await
        }
        ("GET", "/api/recommendation") | ("GET", "/api/recommendations") => {
            api_bench::get_recommendation(method, query_str, body, node).await
        }
        ("GET", "/api/bench/policy") => {
            api_bench::get_bench_policy(method, query_str, body, node).await
        }
        ("POST", "/api/bench/policy") | ("PUT", "/api/bench/policy") => {
            api_bench::post_bench_policy(method, query_str, body, node).await
        }
        ("GET", "/api/audit")
        | ("GET", "/api/security/audit")
        | ("GET", "/api/audit/logs")
        | ("POST", "/api/audit/query") => api_audit::get_audit(method, query_str, body, node).await,
        ("POST", "/api/audit/clear") | ("DELETE", "/api/audit") | ("DELETE", "/api/audit/logs") => {
            api_audit::post_audit_clear(method, query_str, body, node).await
        }
        ("POST", "/api/message/send-multi")
        | ("POST", "/api/msg/send-multi")
        | ("POST", "/api/messages/send-multi") => {
            api_messages::post_message_send_multi(method, query_str, body, node).await
        }
        ("POST", "/api/message/send")
        | ("POST", "/api/msg/send")
        | ("POST", "/api/messages/send") => {
            api_messages::post_message_send(method, query_str, body, node).await
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
            api_messages::get_message_history(method, query_str, body, node).await
        }
        ("POST", "/api/message/clear")
        | ("POST", "/api/messages/clear")
        | ("POST", "/api/msg/clear")
        | ("DELETE", "/api/message")
        | ("DELETE", "/api/messages")
        | ("DELETE", "/api/msg") => {
            api_messages::post_message_clear(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/send-multi") => {
            api_handoff::post_handoff_send_multi(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/send") | ("POST", "/api/handoff") => {
            api_handoff::post_handoff_send(method, query_str, body, node).await
        }
        ("GET", "/api/handoff/inbox") | ("GET", "/api/handoff") | ("GET", "/api/handoff/list") => {
            api_handoff::get_handoff_inbox(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/inbox/open") | ("POST", "/api/handoff/open") => {
            api_handoff::post_handoff_inbox_open(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/inbox/dismiss") | ("POST", "/api/handoff/dismiss") => {
            api_handoff::post_handoff_inbox_dismiss(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/inbox/clear")
        | ("DELETE", "/api/handoff/inbox")
        | ("DELETE", "/api/handoff") => {
            api_handoff::post_handoff_inbox_clear(method, query_str, body, node).await
        }
        ("GET", "/api/handoff/policy") => {
            api_handoff::get_handoff_policy(method, query_str, body, node).await
        }
        ("POST", "/api/handoff/policy") | ("PUT", "/api/handoff/policy") => {
            api_handoff::post_handoff_policy(method, query_str, body, node).await
        }
        ("POST", "/api/media/command")
        | ("POST", "/api/media/cmd")
        | ("POST", "/api/media/send") => {
            api_media::post_media_command(method, query_str, body, node).await
        }
        ("GET", "/api/media/state") | ("GET", "/api/media/status") => {
            api_media::get_media_state(method, query_str, body, node).await
        }
        ("POST", "/api/media/state") | ("PUT", "/api/media/state") => {
            api_media::post_media_state(method, query_str, body, node).await
        }
        ("GET", "/api/media/policy") => {
            api_media::get_media_policy(method, query_str, body, node).await
        }
        ("POST", "/api/media/policy") | ("PUT", "/api/media/policy") => {
            api_media::post_media_policy(method, query_str, body, node).await
        }
        ("GET", "/api/battery/state")
        | ("GET", "/api/battery/status")
        | ("GET", "/api/power/state")
        | ("GET", "/api/power/status") => {
            api_power::get_battery_state(method, query_str, body, node).await
        }
        ("POST", "/api/battery/state")
        | ("PUT", "/api/battery/state")
        | ("POST", "/api/power/state")
        | ("PUT", "/api/power/state") => {
            api_power::post_battery_state(method, query_str, body, node).await
        }
        ("GET", "/api/battery/policy") | ("GET", "/api/power/policy") => {
            api_power::get_battery_policy(method, query_str, body, node).await
        }
        ("POST", "/api/battery/policy")
        | ("PUT", "/api/battery/policy")
        | ("POST", "/api/power/policy")
        | ("PUT", "/api/power/policy") => {
            api_power::post_battery_policy(method, query_str, body, node).await
        }
        ("GET", "/api/locate/status")
        | ("GET", "/api/locate/state")
        | ("GET", "/api/find/status")
        | ("GET", "/api/find/state") => {
            api_locate::get_locate_status(method, query_str, body, node).await
        }
        ("POST", "/api/locate/ring")
        | ("PUT", "/api/locate/ring")
        | ("POST", "/api/find/ring")
        | ("PUT", "/api/find/ring") => {
            api_locate::post_locate_ring(method, query_str, body, node).await
        }
        ("POST", "/api/locate/stop")
        | ("PUT", "/api/locate/stop")
        | ("POST", "/api/find/stop")
        | ("PUT", "/api/find/stop") => {
            api_locate::post_locate_stop(method, query_str, body, node).await
        }
        ("GET", "/api/locate/policy") | ("GET", "/api/find/policy") => {
            api_locate::get_locate_policy(method, query_str, body, node).await
        }
        ("POST", "/api/locate/policy")
        | ("PUT", "/api/locate/policy")
        | ("POST", "/api/find/policy")
        | ("PUT", "/api/find/policy") => {
            api_locate::post_locate_policy(method, query_str, body, node).await
        }
        ("GET", "/api/action/local") => {
            api_action::get_action_local(method, query_str, body, node).await
        }
        ("POST", "/api/action/local") | ("PUT", "/api/action/local") => {
            api_action::post_action_local(method, query_str, body, node).await
        }
        ("DELETE", "/api/action/local") => {
            api_action::delete_action_local(method, query_str, body, node).await
        }
        ("GET", "/api/action/remote") => {
            api_action::get_action_remote(method, query_str, body, node).await
        }
        ("POST", "/api/action/remote/refresh") | ("PUT", "/api/action/remote/refresh") => {
            api_action::post_action_remote_refresh(method, query_str, body, node).await
        }
        ("POST", "/api/action/trigger")
        | ("PUT", "/api/action/trigger")
        | ("POST", "/api/action/run")
        | ("PUT", "/api/action/run") => {
            api_action::post_action_trigger(method, query_str, body, node).await
        }
        ("GET", "/api/action/history") => {
            api_action::get_action_history(method, query_str, body, node).await
        }
        ("GET", "/api/action/policy") => {
            api_action::get_action_policy(method, query_str, body, node).await
        }
        ("POST", "/api/action/policy") | ("PUT", "/api/action/policy") => {
            api_action::post_action_policy(method, query_str, body, node).await
        }
        ("GET", "/api/focus/state") => {
            api_focus::get_focus_state(method, query_str, body, node).await
        }
        ("POST", "/api/focus/state") | ("PUT", "/api/focus/state") => {
            api_focus::post_focus_state(method, query_str, body, node).await
        }
        ("GET", "/api/focus/peers") => {
            api_focus::get_focus_peers(method, query_str, body, node).await
        }
        ("POST", "/api/focus/sync") | ("PUT", "/api/focus/sync") => {
            api_focus::post_focus_sync(method, query_str, body, node).await
        }
        ("GET", "/api/focus/policy") => {
            api_focus::get_focus_policy(method, query_str, body, node).await
        }
        ("POST", "/api/focus/policy") | ("PUT", "/api/focus/policy") => {
            api_focus::post_focus_policy(method, query_str, body, node).await
        }
        ("GET", "/api/topology/layout") => {
            api_topology::get_topology_layout(method, query_str, body, node).await
        }
        ("POST", "/api/topology/link") | ("PUT", "/api/topology/link") => {
            api_topology::post_topology_link(method, query_str, body, node).await
        }
        ("POST", "/api/topology/unlink")
        | ("DELETE", "/api/topology/link")
        | ("DELETE", "/api/topology/unlink") => {
            api_topology::post_topology_unlink(method, query_str, body, node).await
        }
        ("POST", "/api/topology/config") | ("PUT", "/api/topology/config") => {
            api_topology::post_topology_config(method, query_str, body, node).await
        }
        ("GET", "/api/topology/peers") => {
            api_topology::get_topology_peers(method, query_str, body, node).await
        }
        ("POST", "/api/topology/sync") | ("PUT", "/api/topology/sync") => {
            api_topology::post_topology_sync(method, query_str, body, node).await
        }
        ("GET", "/api/topology/policy") => {
            api_topology::get_topology_policy(method, query_str, body, node).await
        }
        ("POST", "/api/topology/policy") | ("PUT", "/api/topology/policy") => {
            api_topology::post_topology_policy(method, query_str, body, node).await
        }
        (_, "/api/events" | "/api/events/stream") => {
            api_core::any_events(method, query_str, body, node).await
        }
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
                dropped: s.dropped,
                bytes: s.bytes + r.bytes,
                refused: s.refused,
                p95_frame_interval_us: node.screen_latency_p95_us(),
                acked: s.acked,
                in_flight: s.in_flight,
                rtt_latest_us: s.rtt_latest_us,
                rtt_p95_us: s.rtt_p95_us,
                is_congested: node.is_screen_congested(),
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
        audio_routing_status: Some(node.audio_routing_status()),
        camera_status: Some(node.camera_status()),
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
    use tetherly_core::{
        PixelFormat, ScreenAnnotationReport, ScreenFrame, ScreenRecordingMeta,
        ScreenRecordingWriter, ScreenRect, ScreenSnapshotReport, ScreenWatermarkReport,
    };

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

    struct EmptyScreenSource;
    impl tetherly_core::ports::ScreenSource for EmptyScreenSource {
        fn grab(&self) -> Result<Option<ScreenFrame>, tetherly_core::CoreError> {
            Ok(None)
        }
    }

    fn temp_test_node_with_source(
        tag: &str,
        source: Arc<dyn tetherly_core::ports::ScreenSource>,
    ) -> (Node, std::path::PathBuf) {
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
            screen_source: Some(source),
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

    #[tokio::test]
    async fn test_cross_origin_requests_are_rejected() {
        let (node, dir) = temp_test_node("cross_origin");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));

        // A browser page on any other origin cannot forge loopback POSTs:
        // fetch(..., {mode:"no-cors"}) still carries the evil Origin header.
        let evil_req = format!(
            "POST /api/daemon/restart HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
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
            "GET /api/input/tunnel HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
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

    #[tokio::test]
    async fn test_virtual_display_endpoints_and_host_validation() {
        let (node, dir) = temp_test_node("virtual_display");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/display/virtual (default inactive)
        let (code, body) = loopback_get(port, "/api/display/virtual").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], false);
        assert_eq!(rep["policy"]["enabled"], true);

        // 2. GET /api/display/virtual/policy (defaults)
        let (code, body) = loopback_get(port, "/api/display/virtual/policy")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_hosting"], true);
        assert_eq!(rep["policy"]["allow_remote_extend"], true);

        // 3. POST /api/display/virtual/policy
        let pol_body = serde_json::json!({
            "enabled": true,
            "allow_hosting": false,
            "allow_remote_extend": true,
            "max_resolution_width": 2560,
            "max_resolution_height": 1440,
            "max_fps": 60,
            "idle_timeout_secs": 180,
            "allowed_groups": ["displays"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/display/virtual/policy",
            &serde_json::to_vec(&pol_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_hosting"], false);
        assert_eq!(rep["policy"]["max_resolution_width"], 2560);
        assert_eq!(rep["policy"]["max_resolution_height"], 1440);
        assert_eq!(rep["policy"]["idle_timeout_secs"], 180);
        assert_eq!(
            rep["policy"]["allowed_groups"],
            serde_json::json!(["displays"])
        );

        // 4. Verify policy reflected on GET /api/display/virtual/policy
        let (code, body) = loopback_get(port, "/api/display/virtual/policy")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["policy"]["allow_hosting"], false);
        assert_eq!(rep["policy"]["max_resolution_width"], 2560);

        // 5. POST /api/display/virtual/detach (when idle/empty)
        let det_body = serde_json::json!({ "reason": "unit test cleanup" });
        let (code, body) = loopback_post(
            port,
            "/api/display/virtual/detach",
            &serde_json::to_vec(&det_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["action"], "detach");

        // 6. Host header 403 test on /api/display/virtual
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/display/virtual HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_audio_routing_endpoints_and_host_validation() {
        let (node, dir) = temp_test_node("audio_routing");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/audio/routing (default inactive)
        let (code, body) = loopback_get(port, "/api/audio/routing").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], false);
        assert_eq!(rep["policy"]["enabled"], true);

        // 2. GET /api/audio/routing/policy (defaults)
        let (code, body) = loopback_get(port, "/api/audio/routing/policy")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_inbound_sink"], true);
        assert_eq!(rep["policy"]["allow_outbound_routing"], true);

        // 3. POST /api/audio/routing/policy
        let pol_body = serde_json::json!({
            "enabled": true,
            "allow_inbound_sink": false,
            "allow_outbound_routing": true,
            "max_volume_limit": 80,
            "allow_spatial_rendering": true,
            "idle_timeout_secs": 180,
            "allowed_groups": ["sound"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/audio/routing/policy",
            &serde_json::to_vec(&pol_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_inbound_sink"], false);
        assert_eq!(rep["policy"]["max_volume_limit"], 80);

        // 4. Verify policy reflected on GET /api/audio/routing/policy
        let (code, body) = loopback_get(port, "/api/audio/routing/policy")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["policy"]["allow_inbound_sink"], false);
        assert_eq!(rep["policy"]["max_volume_limit"], 80);

        // 5. POST /api/audio/routing/detach (when idle/empty)
        let det_body = serde_json::json!({ "reason": "unit test cleanup" });
        let (code, body) = loopback_post(
            port,
            "/api/audio/routing/detach",
            &serde_json::to_vec(&det_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["action"], "detach");

        // 6. Host header 403 test on /api/audio/routing
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/audio/routing HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_camera_endpoints_and_host_validation() {
        let (node, dir) = temp_test_node("camera");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/camera (default inactive)
        let (code, body) = loopback_get(port, "/api/camera").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], false);
        assert_eq!(rep["policy"]["enabled"], true);

        // 2. GET /api/camera/policy (defaults)
        let (code, body) = loopback_get(port, "/api/camera/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["enabled"], true);
        assert_eq!(rep["policy"]["allow_inbound_stream"], true);
        assert_eq!(rep["policy"]["allow_remote_capture"], true);
        assert_eq!(rep["policy"]["enforce_privacy_shutter_indicator"], true);

        // 3. POST /api/camera/policy
        let pol_body = serde_json::json!({
            "enabled": true,
            "allow_inbound_stream": false,
            "allow_remote_capture": true,
            "max_resolution_width": 1920,
            "max_resolution_height": 1080,
            "max_fps": 30,
            "enforce_privacy_shutter_indicator": true,
            "idle_timeout_secs": 180,
            "allowed_groups": ["creators"]
        });
        let (code, body) = loopback_post(
            port,
            "/api/camera/policy",
            &serde_json::to_vec(&pol_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["policy"]["allow_inbound_stream"], false);
        assert_eq!(rep["policy"]["max_resolution_width"], 1920);
        assert_eq!(rep["policy"]["max_fps"], 30);

        // 4. Verify policy reflected on GET /api/camera/policy
        let (code, body) = loopback_get(port, "/api/camera/policy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["policy"]["allow_inbound_stream"], false);
        assert_eq!(rep["policy"]["max_resolution_width"], 1920);

        // 5. POST /api/camera/detach (when idle/empty)
        let det_body = serde_json::json!({ "reason": "unit test cleanup" });
        let (code, body) = loopback_post(
            port,
            "/api/camera/detach",
            &serde_json::to_vec(&det_body).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["action"], "detach");

        // 6. Host header 403 test on /api/camera
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/camera HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_profile_and_qp_endpoints() {
        let (node, dir) = temp_test_node("screen_profile");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/screen/profile
        let (code, body) = loopback_get(port, "/api/screen/profile").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["profile"], "auto");
        assert_eq!(rep["min_qp"], 12);
        assert_eq!(rep["max_qp"], 35);
        assert_eq!(rep["is_custom_qp"], false);
        assert_eq!(rep["auto_profile"], true);

        // 2. POST /api/screen/profile with JSON {"profile": "motion"}
        let update_prof = serde_json::json!({ "profile": "motion" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/profile",
            &serde_json::to_vec(&update_prof).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["profile"], "motion");
        assert_eq!(rep["min_qp"], 16);
        assert_eq!(rep["max_qp"], 46);
        assert_eq!(rep["is_custom_qp"], false);

        // 3. POST /api/screen/profile with custom QP range
        let update_qp = serde_json::json!({ "min_qp": 10, "max_qp": 28 });
        let (code, body) = loopback_post(
            port,
            "/api/screen/profile",
            &serde_json::to_vec(&update_qp).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["min_qp"], 10);
        assert_eq!(rep["max_qp"], 28);
        assert_eq!(rep["is_custom_qp"], true);

        // 4. POST /api/screen/profile with single QP
        let update_single_qp = serde_json::json!({ "qp": 22 });
        let (code, body) = loopback_post(
            port,
            "/api/screen/profile",
            &serde_json::to_vec(&update_single_qp).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["min_qp"], 22);
        assert_eq!(rep["max_qp"], 22);
        assert_eq!(rep["is_custom_qp"], true);

        // 5. POST /api/screen/profile with query param ?profile=text
        let (code, body) = loopback_post(port, "/api/screen/profile?profile=text", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["profile"], "text");

        // 6. Invalid profile returns 400
        let (code, _) = loopback_post(port, "/api/screen/profile?profile=invalid_xyz", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 7. Host header 403 test on /api/screen/profile
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/profile HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_playout_endpoints() {
        let (node, dir) = temp_test_node("screen_playout");
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/screen/playout
        let (code, body) = loopback_get(port, "/api/screen/playout").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "adaptive");
        assert_eq!(rep["target_delay_ms"], 15.0);
        assert_eq!(rep["buffered_frames"], 0);

        // 2. POST /api/screen/playout with JSON {"mode": "zerolatency"}
        let update_zero = serde_json::json!({ "mode": "zerolatency" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/playout",
            &serde_json::to_vec(&update_zero).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "zerolatency");
        assert_eq!(rep["target_delay_ms"], 5.0);

        // 3. POST /api/screen/playout with query param ?mode=smooth
        let (code, body) = loopback_post(port, "/api/screen/playout?mode=smooth", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "smooth");
        assert_eq!(rep["target_delay_ms"], 35.0);

        // 4. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/playout?mode=invalid_xyz", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 5. Host header 403 test on /api/screen/playout
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/playout HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_cursor_endpoints() {
        let (node, dir) = temp_test_node("screen_cursor");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/cursor
        let (code, body) = loopback_get(port, "/api/screen/cursor").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "decoupled");
        assert_eq!(rep["x"], 0);
        assert_eq!(rep["y"], 0);
        assert_eq!(rep["visible"], false);
        assert_eq!(rep["shape"], "arrow");
        assert_eq!(rep["updates_sent"], 0);
        assert_eq!(rep["updates_received"], 0);

        // 2. POST /api/screen/cursor with JSON {"mode": "embedded"}
        let update_embedded = serde_json::json!({ "mode": "embedded" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/cursor",
            &serde_json::to_vec(&update_embedded).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "embedded");

        // 3. POST /api/screen/cursor with query param ?mode=hidden
        let (code, body) = loopback_post(port, "/api/screen/cursor?mode=hidden", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "hidden");

        // 4. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/cursor?mode=invalid_xyz", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 5. Host header 403 test on /api/screen/cursor
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/cursor HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_fec_endpoints() {
        let (node, dir) = temp_test_node("screen_fec");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/fec default
        let (code, body) = loopback_get(port, "/api/screen/fec").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["redundancy_pct"], 0);
        assert_eq!(rep["blocks_encoded"], 0);
        assert_eq!(rep["recovered_shards"], 0);

        // 2. POST /api/screen/fec with JSON {"mode": "conservative"}
        let update_conservative = serde_json::json!({ "mode": "conservative" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/fec",
            &serde_json::to_vec(&update_conservative).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "conservative");
        assert_eq!(rep["redundancy_pct"], 10);

        // 3. POST /api/screen/fec with query param ?mode=aggressive
        let (code, body) = loopback_post(port, "/api/screen/fec?mode=aggressive", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "aggressive");
        assert_eq!(rep["redundancy_pct"], 25);

        // 4. POST /api/screen/fec with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/fec?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");
        assert_eq!(rep["redundancy_pct"], 0);

        // 5. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/fec?mode=invalid_fec", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 6. Host header 403 test on /api/screen/fec
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/fec HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_pacer_endpoints() {
        let (node, dir) = temp_test_node("screen_pacer");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/pacer default
        let (code, body) = loopback_get(port, "/api/screen/pacer").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["packets_paced"], 0);
        assert_eq!(rep["bursts_smoothed"], 0);

        // 2. POST /api/screen/pacer with JSON {"mode": "aggressive"}
        let update_agg = serde_json::json!({ "mode": "aggressive" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/pacer",
            &serde_json::to_vec(&update_agg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/pacer with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/pacer?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 4. POST /api/screen/pacer with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/pacer?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 5. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/pacer?mode=invalid_pacer", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 6. Host header 403 test on /api/screen/pacer
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/pacer HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_trendline_endpoints() {
        let (node, dir) = temp_test_node("screen_trendline");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/trendline default
        let (code, body) = loopback_get(port, "/api/screen/trendline").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["state"], "normal");
        assert_eq!(rep["overuse_count"], 0);

        // 2. POST /api/screen/trendline with JSON {"mode": "aggressive"}
        let update_agg = serde_json::json!({ "mode": "aggressive" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/trendline",
            &serde_json::to_vec(&update_agg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/trendline with query param ?mode=conservative
        let (code, body) = loopback_post(port, "/api/screen/trendline?mode=conservative", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "conservative");

        // 4. POST /api/screen/trendline with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/trendline?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 5. POST /api/screen/trendline with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/trendline?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 6. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/trendline?mode=invalid_trendline", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 7. Host header 403 test on /api/screen/trendline
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/trendline HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_harq_endpoints() {
        let (node, dir) = temp_test_node("screen_harq");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/harq default
        let (code, body) = loopback_get(port, "/api/screen/harq").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["nacks_sent"], 0);
        assert_eq!(rep["nacks_received"], 0);

        // 2. POST /api/screen/harq with JSON {"mode": "aggressive"}
        let update_agg = serde_json::json!({ "mode": "aggressive" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/harq",
            &serde_json::to_vec(&update_agg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/harq with query param ?mode=conservative
        let (code, body) = loopback_post(port, "/api/screen/harq?mode=conservative", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "conservative");

        // 4. POST /api/screen/harq with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/harq?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 5. POST /api/screen/harq with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/harq?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 6. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/harq?mode=invalid_harq", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 7. Host header 403 test on /api/screen/harq
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/harq HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_twcc_endpoints() {
        let (node, dir) = temp_test_node("screen_twcc");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/twcc default
        let (code, body) = loopback_get(port, "/api/screen/twcc").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["feedbacks_sent"], 0);
        assert_eq!(rep["feedbacks_received"], 0);
        assert_eq!(rep["packets_lost"], 0);

        // 2. POST /api/screen/twcc with JSON {"mode": "aggressive"}
        let update_agg = serde_json::json!({ "mode": "aggressive" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/twcc",
            &serde_json::to_vec(&update_agg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/twcc with query param ?mode=conservative
        let (code, body) = loopback_post(port, "/api/screen/twcc?mode=conservative", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "conservative");

        // 4. POST /api/screen/twcc with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/twcc?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 5. POST /api/screen/twcc with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/twcc?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 6. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/twcc?mode=invalid_twcc", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 7. Host header 403 test on /api/screen/twcc
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/twcc HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_probe_endpoints() {
        let (node, dir) = temp_test_node("screen_probe");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/probe default
        let (code, body) = loopback_get(port, "/api/screen/probe").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["state"], "idle");
        assert_eq!(rep["probes_triggered"], 0);
        assert_eq!(rep["probes_completed"], 0);
        assert_eq!(rep["probes_failed"], 0);

        // 2. POST /api/screen/probe with JSON {"mode": "aggressive"}
        let update_agg = serde_json::json!({ "mode": "aggressive" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/probe",
            &serde_json::to_vec(&update_agg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/probe with query param ?mode=conservative
        let (code, body) = loopback_post(port, "/api/screen/probe?mode=conservative", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "conservative");

        // 4. POST /api/screen/probe with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/probe?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 5. POST /api/screen/probe with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/probe?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 6. POST /api/screen/probe/trigger
        let (code, body) = loopback_post(port, "/api/screen/probe/trigger", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 7. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/probe?mode=invalid_probe", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 8. Host header 403 test on /api/screen/probe
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/probe HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_sync_endpoints() {
        let (node, dir) = temp_test_node("screen_sync");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/sync default
        let (code, body) = loopback_get(port, "/api/screen/sync").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["beacons_sent"], 0);
        assert_eq!(rep["beacons_received"], 0);
        assert_eq!(rep["current_skew_us"], 0);
        assert_eq!(rep["clock_drift_ppm"], 0.0);

        // 2. POST /api/screen/sync with JSON {"mode": "strict"}
        let update_strict = serde_json::json!({ "mode": "strict" });
        let (code, body) = loopback_post(
            port,
            "/api/screen/sync",
            &serde_json::to_vec(&update_strict).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "strict");

        // 3. POST /api/screen/sync with query param ?mode=loose
        let (code, body) = loopback_post(port, "/api/screen/sync?mode=loose", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "loose");

        // 4. POST /api/screen/sync with query param ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/sync?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 5. POST /api/screen/sync with query param ?mode=auto
        let (code, body) = loopback_post(port, "/api/screen/sync?mode=auto", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "auto");

        // 6. POST /api/screen/sync/trigger
        let (code, body) = loopback_post(port, "/api/screen/sync/trigger", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 7. Invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/sync?mode=invalid_sync", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 8. Host header 403 test on /api/screen/sync
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/sync HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_display_and_switch_endpoints() {
        let (node, dir) = temp_test_node("screen_displays");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/displays default
        let (code, body) = loopback_get(port, "/api/screen/displays").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active_display_id"], 0);
        assert_eq!(rep["switches_count"], 0);
        assert!(!rep["displays"].as_array().unwrap().is_empty());

        // 2. Set multi-display configuration
        let d0 =
            tetherly_core::ScreenDisplayInfo::new(0, "Main Display", 1920, 1080, 100, true, false);
        let d1 = tetherly_core::ScreenDisplayInfo::new(
            1,
            "Secondary Monitor",
            2560,
            1440,
            125,
            false,
            false,
        );
        let d2 = tetherly_core::ScreenDisplayInfo::new(
            2,
            "Virtual Display (ADR-073)",
            1920,
            1200,
            100,
            false,
            true,
        );
        node.screen_set_displays(vec![d0, d1, d2]);

        let (code, body) = loopback_get(port, "/api/screen/displays").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["displays"].as_array().unwrap().len(), 3);

        // 3. POST /api/screen/switch with JSON {"display_id": 1}
        let switch_req = serde_json::json!({ "display_id": 1 });
        let (code, body) = loopback_post(
            port,
            "/api/screen/switch",
            &serde_json::to_vec(&switch_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active_display_id"], 1);
        assert_eq!(rep["switches_count"], 1);

        // 4. POST /api/screen/switch with query param ?display_id=2
        let (code, body) = loopback_post(port, "/api/screen/switch?display_id=2", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active_display_id"], 2);
        assert_eq!(rep["switches_count"], 2);

        // 5. POST /api/screen/switch with missing display_id returns 400
        let (code, _) = loopback_post(port, "/api/screen/switch", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 5b. POST /api/screen/switch with unknown display_id returns 404
        let (code, _) = loopback_post(port, "/api/screen/switch?display_id=99", &[])
            .await
            .unwrap();
        assert_eq!(code, 404);

        // 6. Host header 403 test on /api/screen/displays
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/displays HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_privacy_endpoints() {
        let (node, dir) = temp_test_node("screen_privacy");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/privacy default
        let (code, body) = loopback_get(port, "/api/screen/privacy").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["enabled"], false);
        assert_eq!(rep["lock_input"], false);
        assert_eq!(rep["blanking_method"], "virtual_overlay");

        // 2. POST /api/screen/privacy with JSON body {"enabled": true, "lock_input": true}
        let priv_req = serde_json::json!({
            "enabled": true,
            "lock_input": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/privacy",
            &serde_json::to_vec(&priv_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["enabled"], true);
        assert_eq!(rep["lock_input"], true);

        // 3. POST /api/screen/privacy with query string ?enabled=false
        let (code, body) = loopback_post(port, "/api/screen/privacy?enabled=false", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["enabled"], false);
        assert_eq!(rep["lock_input"], false);

        // 4. POST /api/screen/privacy/enable?lock_input=true
        let (code, body) = loopback_post(port, "/api/screen/privacy/enable?lock_input=true", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["enabled"], true);
        assert_eq!(rep["lock_input"], true);

        // 5. POST /api/screen/privacy/disable
        let (code, body) = loopback_post(port, "/api/screen/privacy/disable", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["enabled"], false);
        assert_eq!(rep["lock_input"], false);

        // 6. POST /api/screen/privacy missing required field returns 400
        let (code, _) = loopback_post(port, "/api/screen/privacy", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 7. Host header 403 test on /api/screen/privacy
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/privacy HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_latency_endpoints() {
        let (node, dir) = temp_test_node("screen_latency");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/latency default
        let (code, body) = loopback_get(port, "/api/screen/latency").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "auto");
        assert_eq!(rep["frames_profiled"], 0);

        // 2. POST /api/screen/latency with JSON body {"mode": "detailed"}
        let lat_req = serde_json::json!({
            "mode": "detailed"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/latency",
            &serde_json::to_vec(&lat_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "detailed");

        // 3. POST /api/screen/latency with query string ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/latency?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 4. POST /api/screen/latency/reset
        let (code, body) = loopback_post(port, "/api/screen/latency/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);

        // 5. POST /api/screen/latency with invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/latency?mode=invalid_xyz", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 6. Host header 403 test on /api/screen/latency
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/latency HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tuning_endpoints() {
        let (node, dir) = temp_test_node("screen_tuning");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tuning default (standard mode, no actions)
        let (code, body) = loopback_get(port, "/api/screen/tuning").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "standard");
        assert_eq!(rep["actions_applied"], 0);
        assert_eq!(rep["recoveries_applied"], 0);
        assert_eq!(rep["last_action"], "hold");

        // 2. POST /api/screen/tuning with JSON body {"mode": "aggressive"}
        let tune_req = serde_json::json!({
            "mode": "aggressive"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tuning",
            &serde_json::to_vec(&tune_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["mode"], "aggressive");

        // 3. POST /api/screen/tuning with query string ?mode=disabled
        let (code, body) = loopback_post(port, "/api/screen/tuning?mode=disabled", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["mode"], "disabled");

        // 4. POST /api/screen/tuning/reset
        let (code, body) = loopback_post(port, "/api/screen/tuning/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["actions_applied"], 0);

        // 5. POST /api/screen/tuning with invalid mode returns 400
        let (code, _) = loopback_post(port, "/api/screen/tuning?mode=invalid_xyz", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 6. Host header 403 test on /api/screen/tuning
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/tuning HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_recording_endpoints() {
        let (node, dir) = temp_test_node("screen_recording");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/recording default (idle, no peer disclosure)
        let (code, body) = loopback_get(port, "/api/screen/recording").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], false);
        assert_eq!(rep["peer_recording"], false);
        assert_eq!(rep["frames_recorded"], 0);

        // 2. POST /api/screen/recording/start without a path (default under data_dir)
        let (code, body) = loopback_post(port, "/api/screen/recording/start", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["active"], true);
        let container_path = rep["path"].as_str().unwrap().to_string();
        assert!(container_path.ends_with(".trec"));
        assert!(container_path.contains("recordings"));

        // 3. Starting twice must be rejected
        let (code, _) = loopback_post(port, "/api/screen/recording/start", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 4. Status while active
        let (code, body) = loopback_get(port, "/api/screen/recording").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["active"], true);

        // 5. POST /api/screen/recording/stop flushes and reports idle
        let (code, body) = loopback_post(port, "/api/screen/recording/stop", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["active"], false);

        // 6. Replay of a non-existent container must fail with 400
        let (code, _) = loopback_get(
            port,
            "/api/screen/recording/replay?path=/definitely/not/here.trec",
        )
        .await
        .unwrap();
        assert_eq!(code, 400);

        // 7. Replay without ?path= must be rejected
        let (code, _) = loopback_get(port, "/api/screen/recording/replay")
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 8. Start with a path containing parent-directory traversal is refused
        let bad_req = serde_json::json!({ "path": "../escape.trec" });
        let (code, _) = loopback_post(
            port,
            "/api/screen/recording/start",
            &serde_json::to_vec(&bad_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 400);

        // 9. Host header 403 on /api/screen/recording
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/recording HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = container_path;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_playback_endpoints() {
        let (node, dir) = temp_test_node("screen_playback");
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/playback default (idle)
        let (code, body) = loopback_get(port, "/api/screen/playback").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"], "idle");
        assert_eq!(rep["current_frame"], 0);

        // 2. Synthesize a valid .trec file with 5 frames
        let trec_path = dir.join("playback_sample.trec");
        let mut buf = Vec::new();
        let frame0 = tetherly_core::ScreenFrame {
            seq: 0,
            rect: tetherly_core::ScreenRect {
                x: 0,
                y: 0,
                w: 64,
                h: 48,
            },
            format: tetherly_core::PixelFormat::Bgra8,
            codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
            stride: 64 * 4,
            bytes: vec![0u8; 64 * 48 * 4],
        };
        let meta = tetherly_core::ScreenRecordingMeta::from_frame(&frame0, 1000);
        let mut writer =
            tetherly_core::ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut buf))
                .unwrap();
        for i in 0..5 {
            let frame = tetherly_core::ScreenFrame {
                seq: i,
                rect: tetherly_core::ScreenRect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 48,
                },
                format: tetherly_core::PixelFormat::Bgra8,
                codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
                stride: 64 * 4,
                bytes: vec![0u8; 64 * 48 * 4],
            };
            writer.append(&frame, 1000 + i * 16666).unwrap();
        }
        writer.finish().unwrap();
        std::fs::write(&trec_path, &buf).unwrap();

        // 3. Start playback with non-existent file -> 400
        let (code, _) = loopback_post(
            port,
            "/api/screen/playback/start?path=/definitely/not/here.trec",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 400);

        // 4. Start playback with parent directory traversal -> 400
        let bad_req = serde_json::json!({ "path": "../escape.trec" });
        let (code, _) = loopback_post(
            port,
            "/api/screen/playback/start",
            &serde_json::to_vec(&bad_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 400);

        // 5. Start playback without path -> 400
        let (code, _) = loopback_post(port, "/api/screen/playback/start", &[])
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 6. Start playback with valid path (using loop=true to keep active for pause/resume)
        let start_req = serde_json::json!({
            "path": trec_path.to_str().unwrap(),
            "speed": 0.0,
            "loop": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/playback/start",
            &serde_json::to_vec(&start_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["total_frames"], 5);

        // 7. Pause playback
        let (code, body) = loopback_post(port, "/api/screen/playback/pause", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"], "paused");

        // 8. Resume playback
        let (code, body) = loopback_post(port, "/api/screen/playback/resume", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"], "playing");

        // 9. Stop playback
        let (code, body) = loopback_post(port, "/api/screen/playback/stop", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"], "stopped");

        // 10. GET /api/screen/playback after stop shows stopped report
        let (code, body) = loopback_get(port, "/api/screen/playback").await.unwrap();
        assert_eq!(code, 200);
        let rep: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep["ok"], true);
        assert_eq!(rep["state"], "stopped");

        // 11. Host header 403 guard on /api/screen/playback
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/playback HTTP/1.1\r\nHost: evil-attacker-site.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_snapshot_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_snapshot", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. Initial snapshot when no frame is available returns 503
        let (code, body) = loopback_get(port, "/api/screen/snapshot").await.unwrap();
        assert_eq!(code, 503);
        assert!(String::from_utf8_lossy(&body).contains("no screen frame available"));

        // 2. Seed a mock frame into node's last frame buffer
        let frame_w = 32u32;
        let frame_h = 24u32;
        let mut raw_pixels = vec![0u8; (frame_w * frame_h * 4) as usize];
        for (i, p) in raw_pixels.iter_mut().enumerate() {
            *p = ((i % 251) + 1) as u8;
        }
        let frame = ScreenFrame::raw(
            42,
            ScreenRect {
                x: 0,
                y: 0,
                w: frame_w as u16,
                h: frame_h as u16,
            },
            PixelFormat::Bgra8,
            raw_pixels.clone(),
        )
        .unwrap();
        node.set_screen_last_frame_for_test(frame);

        // 3. GET /api/screen/snapshot (default JSON format)
        let (code, body) = loopback_get(port, "/api/screen/snapshot").await.unwrap();
        assert_eq!(code, 200);
        let rep: ScreenSnapshotReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.source, "live");
        assert_eq!(rep.frame_seq, 42);
        assert!(rep.pts_us > 0);
        assert_eq!(rep.width, 32);
        assert_eq!(rep.height, 24);
        assert_eq!(rep.format, "bmp");
        assert_eq!(rep.bytes_len, 54 + (32 * 24 * 4));
        assert!(!rep.privacy_blanked);
        assert!(rep.data_base64.is_some());

        // 4. GET /api/screen/snapshot?format=bmp returns image/bmp binary stream
        let (code, body) = loopback_get(port, "/api/screen/snapshot?format=bmp")
            .await
            .unwrap();
        assert_eq!(code, 200);
        assert_eq!(&body[0..2], b"BM");
        assert_eq!(body.len(), 54 + (32 * 24 * 4));

        // 5. Save live snapshot to safe file on disk
        let out_img_path = dir.join("test_live_out.bmp");
        let q_out = format!(
            "/api/screen/snapshot?out={}",
            crate::uihttp::urlencode_path(out_img_path.to_str().unwrap())
        );
        let (code, body) = loopback_get(port, &q_out).await.unwrap();
        assert_eq!(code, 200);
        let rep: ScreenSnapshotReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.path.as_deref(), out_img_path.to_str());
        assert!(out_img_path.exists());
        let disk_bytes = std::fs::read(&out_img_path).unwrap();
        assert_eq!(&disk_bytes[0..2], b"BM");
        assert_eq!(disk_bytes.len(), 54 + (32 * 24 * 4));

        // 6. Security guard: directory traversal in `out` rejected with 400
        let (code, body) = loopback_get(port, "/api/screen/snapshot?out=../evil_escape.bmp")
            .await
            .unwrap();
        assert_eq!(code, 400);
        assert!(String::from_utf8_lossy(&body)
            .to_lowercase()
            .contains("unsafe"));

        // 7. Privacy mode test (ADR-098 & ADR-103)
        node.screen_set_privacy_mode(true, false).await.unwrap();
        let (code, body) = loopback_get(port, "/api/screen/snapshot?format=bmp")
            .await
            .unwrap();
        assert_eq!(code, 200);
        assert_eq!(&body[0..2], b"BM");
        // All pixel bytes in the BMP payload must be zeroed out
        for &b in &body[54..] {
            assert_eq!(b, 0, "privacy mode must blank all snapshot pixels");
        }
        let (code, body) = loopback_get(port, "/api/screen/snapshot?format=json")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenSnapshotReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.privacy_blanked);
        node.screen_set_privacy_mode(false, false).await.unwrap();

        // 8. Recording snapshot extraction via POST /api/screen/recording/snapshot
        let rec_file = dir.join("test_recording.trec");
        {
            let f0 = ScreenFrame::raw(
                0,
                ScreenRect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 48,
                },
                PixelFormat::Bgra8,
                vec![0x11; 64 * 48 * 4],
            )
            .unwrap();

            let f1 = ScreenFrame::raw(
                1,
                ScreenRect {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 48,
                },
                PixelFormat::Bgra8,
                vec![0x77; 64 * 48 * 4],
            )
            .unwrap();

            let file = std::fs::File::create(&rec_file).unwrap();
            let mut writer =
                ScreenRecordingWriter::new(ScreenRecordingMeta::from_frame(&f0, 2), file).unwrap();
            writer.append(&f0, 100_000).unwrap();
            writer.append(&f1, 200_000).unwrap();
            writer.finish().unwrap();
        }

        let post_req = serde_json::json!({
            "path": rec_file.to_str().unwrap(),
            "frame": 1,
            "format": "json"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/recording/snapshot",
            &serde_json::to_vec(&post_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenSnapshotReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.source, "recording");
        assert_eq!(rep.frame_seq, 1);
        assert_eq!(rep.pts_us, 200_000);
        assert_eq!(rep.width, 64);
        assert_eq!(rep.height, 48);
        assert_eq!(rep.bytes_len, 54 + (64 * 48 * 4));

        // 9. Recording snapshot returning BMP binary stream
        let post_bmp_req = serde_json::json!({
            "path": rec_file.to_str().unwrap(),
            "pts_us": 100_000,
            "format": "bmp"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/recording/snapshot",
            &serde_json::to_vec(&post_bmp_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        assert_eq!(&body[0..2], b"BM");
        assert_eq!(body.len(), 54 + (64 * 48 * 4));
        // Verify frame 0 content (0x11)
        for &b in &body[54..] {
            assert_eq!(b, 0x11);
        }

        // 10. Recording snapshot traversal security guard
        let post_evil = serde_json::json!({
            "path": "../secret_file.trec",
            "frame": 0
        });
        let (code, _) = loopback_post(
            port,
            "/api/screen/recording/snapshot",
            &serde_json::to_vec(&post_evil).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 400);

        // 11. Host header 403 guard on /api/screen/snapshot
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/snapshot HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_watermark_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_watermark", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone(), 0).await.unwrap();

        // 1. GET /api/screen/watermark returns defaults
        let (code, body) = loopback_get(port, "/api/screen/watermark").await.unwrap();
        assert_eq!(code, 200);
        let rep: ScreenWatermarkReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert!(!rep.host_enforced);
        assert_eq!(rep.pattern, "grid");
        assert_eq!(rep.scale, 1);
        assert_eq!(rep.opacity, 64);

        // 2. POST /api/screen/watermark/enable
        let (code, body) = loopback_post(port, "/api/screen/watermark/enable", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenWatermarkReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert!(!rep.host_enforced);

        // 3. POST /api/screen/watermark/disable
        let (code, body) = loopback_post(port, "/api/screen/watermark/disable", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenWatermarkReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 4. POST /api/screen/watermark with custom configuration
        let custom_cfg = serde_json::json!({
            "enabled": true,
            "text": "CONFIDENTIAL-{peer}",
            "opacity": 128,
            "color": "#FF8000",
            "scale": 2,
            "pattern": "banner"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/watermark",
            &serde_json::to_vec(&custom_cfg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenWatermarkReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.text_template, "CONFIDENTIAL-{peer}");
        assert_eq!(rep.opacity, 128);
        assert_eq!(rep.opacity_pct, 50);
        assert_eq!(rep.color_hex, "#FF8000");
        assert_eq!(rep.color_rgb, [255, 128, 0]);
        assert_eq!(rep.scale, 2);
        assert_eq!(rep.pattern, "banner");

        // 5. Host-enforced lockdown: when host enforces watermark, peer cannot disable it
        node.screen_enable_watermark(true).await.unwrap();
        node.set_screen_peer_watermark_enforced_for_test(true);

        // Attempt disable -> 403 Forbidden
        let (code, body) = loopback_post(port, "/api/screen/watermark/disable", &[])
            .await
            .unwrap();
        assert_eq!(code, 403);
        assert!(String::from_utf8_lossy(&body).contains("enforced by host"));

        // Attempt disable via config update -> 403 Forbidden
        let disable_cfg = serde_json::json!({ "enabled": false });
        let (code, body) = loopback_post(
            port,
            "/api/screen/watermark",
            &serde_json::to_vec(&disable_cfg).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 403);
        assert!(String::from_utf8_lossy(&body).contains("enforced by host"));

        // 6. Host header 403 guard on /api/screen/watermark
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/watermark HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_annotation_and_laser_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_annotation", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. Initial GET /api/screen/annotation
        let (code, body) = loopback_get(port, "/api/screen/annotation").await.unwrap();
        assert_eq!(code, 200);
        let rep: ScreenAnnotationReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.annotations_count, 0);
        assert!(!rep.laser_active);

        // 2. POST /api/screen/annotation with stroke
        let stroke_req = serde_json::json!({
            "kind": "stroke",
            "points": [[10, 10], [20, 20], [30, 25]],
            "color": "#FF0000",
            "thickness": 4,
            "ttl_ms": 5000
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/annotation",
            &serde_json::to_vec(&stroke_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["report"]["annotations_count"], 1);

        // 3. POST /api/screen/annotation with rect
        let rect_req = serde_json::json!({
            "kind": "rect",
            "x": 50,
            "y": 60,
            "w": 100,
            "h": 80,
            "color": "#00FF00"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/annotation",
            &serde_json::to_vec(&rect_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["report"]["annotations_count"], 2);

        // 4. POST /api/screen/laser
        let laser_req = serde_json::json!({
            "x": 150,
            "y": 200,
            "active": true,
            "color": "#FF0000",
            "radius": 14
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/laser",
            &serde_json::to_vec(&laser_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["laser_active"], true);
        assert_eq!(resp["laser"]["x"], 150);
        assert_eq!(resp["laser"]["y"], 200);

        // 5. GET /api/screen/laser
        let (code, body) = loopback_get(port, "/api/screen/laser").await.unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["laser_active"], true);

        // 6. POST /api/screen/annotation/clear
        let (code, body) = loopback_post(port, "/api/screen/annotation/clear", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: ScreenAnnotationReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.annotations_count, 0);

        // 7. Host header 403 guard on /api/screen/annotation
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/annotation HTTP/1.1\r\nHost: malicious-attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_whiteboard_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_wb", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/whiteboard/policy
        let (code, body) = loopback_get(port, "/api/screen/whiteboard/policy")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["policy"]["read_only"], false);
        assert_eq!(resp["policy"]["allow_peer_clear"], true);

        // 2. Add annotations for Alice and Bob
        let req_alice = serde_json::json!({
            "kind": "stroke",
            "author": "alice",
            "points": [[10, 10], [20, 20]],
            "color": "#FF0000"
        });
        let (code, _) = loopback_post(
            port,
            "/api/screen/annotation",
            &serde_json::to_vec(&req_alice).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);

        let req_bob = serde_json::json!({
            "kind": "rect",
            "author": "bob",
            "x": 30,
            "y": 30,
            "w": 40,
            "h": 40,
            "color": "#00FF00"
        });
        let (code, _) = loopback_post(
            port,
            "/api/screen/annotation",
            &serde_json::to_vec(&req_bob).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);

        // 3. POST /api/screen/whiteboard/undo?author=alice
        let (code, body) = loopback_post(port, "/api/screen/whiteboard/undo?author=alice", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["undone"]["author"], "alice");
        assert_eq!(resp["report"]["annotations_count"], 1);
        assert_eq!(resp["report"]["redo_count"], 1);

        // 4. POST /api/screen/whiteboard/redo?author=alice
        let (code, body) = loopback_post(port, "/api/screen/whiteboard/redo?author=alice", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["redone"]["author"], "alice");
        assert_eq!(resp["report"]["annotations_count"], 2);
        assert_eq!(resp["report"]["redo_count"], 0);

        // 5. GET /api/screen/whiteboard/export
        let (code, body) = loopback_get(port, "/api/screen/whiteboard/export")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let export_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(export_resp["ok"], true);
        let snapshot = export_resp["export"].clone();
        assert_eq!(snapshot["active_items"].as_array().unwrap().len(), 2);

        // 6. POST /api/screen/whiteboard/clear
        let (code, _) = loopback_post(port, "/api/screen/whiteboard/clear", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let (code, body) = loopback_get(port, "/api/screen/annotation").await.unwrap();
        assert_eq!(code, 200);
        let rep: ScreenAnnotationReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.annotations_count, 0);

        // 7. POST /api/screen/whiteboard/import
        let import_req = serde_json::json!({
            "snapshot": snapshot,
            "replace": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/whiteboard/import",
            &serde_json::to_vec(&import_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp["ok"], true);
        assert_eq!(resp["imported_count"], 2);

        // 8. POST /api/screen/whiteboard/policy
        let pol_req = serde_json::json!({
            "read_only": true,
            "allow_peer_clear": false,
            "max_items": 500
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/whiteboard/policy",
            &serde_json::to_vec(&pol_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let pol_resp: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(pol_resp["policy"]["read_only"], true);
        assert_eq!(pol_resp["policy"]["allow_peer_clear"], false);
        assert_eq!(pol_resp["policy"]["max_items"], 500);

        // 9. Path traversal rejection on export / import
        let (code, _) = loopback_get(port, "/api/screen/whiteboard/export?out=../escape.json")
            .await
            .unwrap();
        assert_eq!(code, 400);

        // 10. Host header 403 guard on /api/screen/whiteboard/policy
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/whiteboard/policy HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_roi_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_roi", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/roi default state
        let (code, body) = loopback_get(port, "/api/screen/roi").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.active);
        assert_eq!(rep.zoom_percent, 100);

        // 2. POST /api/screen/roi sets viewport parameters
        let roi_req = serde_json::json!({
            "x": 100,
            "y": 200,
            "w": 640,
            "h": 480,
            "zoom_percent": 150
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/roi",
            &serde_json::to_vec(&roi_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.active);
        assert_eq!(rep.x, 100);
        assert_eq!(rep.y, 200);
        assert_eq!(rep.w, 640);
        assert_eq!(rep.h, 480);
        assert_eq!(rep.zoom_percent, 150);

        // 3. POST /api/screen/roi/zoom adjusts zoom scale
        let zoom_req = serde_json::json!({
            "zoom_percent": 250
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/roi/zoom",
            &serde_json::to_vec(&zoom_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.zoom_percent, 250);

        // 4. Verify /api/screen status telemetry includes ROI state
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.roi_active);
        assert_eq!(status.roi_x, 100);
        assert_eq!(status.roi_y, 200);
        assert_eq!(status.roi_w, 640);
        assert_eq!(status.roi_h, 480);
        assert_eq!(status.roi_zoom_percent, 250);

        // 5. POST /api/screen/roi/reset restores full-frame streaming
        let (code, body) = loopback_post(port, "/api/screen/roi/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.active);

        // 6. DELETE /api/screen/roi also resets
        let _ = loopback_post(port, "/api/screen/roi?x=50&y=50&w=200&h=200", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/roi").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenRoiReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.active);

        // 7. Security guard: Host header 403 on /api/screen/roi
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/roi HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_color_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_color", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/color default state
        let (code, body) = loopback_get(port, "/api/screen/color").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, tetherly_core::ScreenColorMode::FullColor32);
        assert_eq!(rep.threshold, 128);
        assert!(!rep.is_active);

        // 2. POST /api/screen/color sets grayscale mode
        let gray_req = serde_json::json!({
            "mode": "grayscale"
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/color",
            &serde_json::to_vec(&gray_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, tetherly_core::ScreenColorMode::Grayscale8);
        assert!(rep.is_active);

        // 3. POST /api/screen/color sets monochrome mode with threshold
        let mono_req = serde_json::json!({
            "mode": "monochrome",
            "threshold": 160
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/color",
            &serde_json::to_vec(&mono_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, tetherly_core::ScreenColorMode::Monochrome1);
        assert_eq!(rep.threshold, 160);
        assert!(rep.is_active);

        // 4. Verify /api/screen status telemetry includes color mode telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.color_mode, "monochrome1");
        assert_eq!(status.color_threshold, 160);
        assert!(status.color_updates >= 2);

        // 5. POST /api/screen/color/reset restores full color
        let (code, body) = loopback_post(port, "/api/screen/color/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, tetherly_core::ScreenColorMode::FullColor32);
        assert!(!rep.is_active);

        // 6. DELETE /api/screen/color also resets
        let _ = loopback_post(port, "/api/screen/color?mode=reduced", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/color").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenColorReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, tetherly_core::ScreenColorMode::FullColor32);
        assert!(!rep.is_active);

        // 7. Security guard: Host header 403 on /api/screen/color
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/color HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_delta_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_delta", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/delta default state
        let (code, body) = loopback_get(port, "/api/screen/delta").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.keyframe_interval, 60);
        assert_eq!(rep.saved_bytes, 0);

        // 2. POST /api/screen/delta enables delta mode with keyframe interval
        let delta_req = serde_json::json!({
            "enabled": true,
            "keyframe_interval": 30
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/delta",
            &serde_json::to_vec(&delta_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.keyframe_interval, 30);

        // 3. Verify /api/screen status telemetry includes delta telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.delta_enabled);
        assert_eq!(status.delta_keyframe_interval, 30);

        // 4. POST /api/screen/delta/reset restores default full-frame streaming
        let (code, body) = loopback_post(port, "/api/screen/delta/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.keyframe_interval, 60);

        // 5. DELETE /api/screen/delta also resets
        let _ = loopback_post(port, "/api/screen/delta?enabled=true&interval=45", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/delta").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenDeltaReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 6. Security guard: Host header 403 on /api/screen/delta
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/delta HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_quality_metric_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_metric", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/metric default state
        let (code, body) = loopback_get(port, "/api/screen/metric").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.sample_interval, 10);
        assert_eq!(rep.evaluated_frames, 0);
        assert_eq!(rep.avg_psnr_db, 100.0);
        assert_eq!(rep.avg_ssim, 1.0);
        assert!(!rep.quality_warning);

        // 2. POST /api/screen/metric enables quality metric mode with sample interval
        let metric_req = serde_json::json!({
            "enabled": true,
            "sample_interval": 5
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/metric",
            &serde_json::to_vec(&metric_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.sample_interval, 5);

        // 3. Verify /api/screen status telemetry includes quality metric telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.metric_enabled);
        assert_eq!(status.metric_sample_interval, 5);
        assert_eq!(status.metric_psnr_db, 100.0);
        assert_eq!(status.metric_ssim, 1.0);

        // 4. POST /api/screen/metric/reset restores default disabled state
        let (code, body) = loopback_post(port, "/api/screen/metric/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.sample_interval, 10);

        // 5. DELETE /api/screen/metric also resets
        let _ = loopback_post(port, "/api/screen/metric?enabled=true&interval=12", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/metric").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQualityMetricReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 6. Security guard: Host header 403 on /api/screen/metric
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/metric HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_qoe_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_qoe", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/qoe default state
        let (code, body) = loopback_get(port, "/api/screen/qoe").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.mode, "off");
        assert_eq!(rep.target_psnr_db, 30);
        assert_eq!(rep.target_ssim_pct, 85);
        assert_eq!(rep.actions_total, 0);
        assert_eq!(rep.last_action, "hold");

        // 2. POST /api/screen/qoe enables balanced mode with custom targets
        let qoe_req = serde_json::json!({
            "mode": "balanced",
            "target_psnr": 32,
            "target_ssim": 90
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/qoe",
            &serde_json::to_vec(&qoe_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "balanced");
        assert_eq!(rep.target_psnr_db, 32);
        assert_eq!(rep.target_ssim_pct, 90);

        // 3. Verify /api/screen status telemetry includes QoE telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.qoe_enabled);
        assert_eq!(status.qoe_mode, "balanced");
        assert_eq!(status.qoe_target_psnr_db, 32);
        assert_eq!(status.qoe_target_ssim_pct, 90);
        assert_eq!(status.qoe_actions_total, 0);
        assert_eq!(status.qoe_last_action, "hold");

        // 4. POST /api/screen/qoe with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/qoe?mode=conservative&target_psnr=35&target_ssim=92",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "conservative");
        assert_eq!(rep.target_psnr_db, 35);
        assert_eq!(rep.target_ssim_pct, 92);

        // 5. POST /api/screen/qoe/reset restores default disabled state
        let (code, body) = loopback_post(port, "/api/screen/qoe/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "off");

        // 6. DELETE /api/screen/qoe also resets
        let _ = loopback_post(port, "/api/screen/qoe?mode=economy", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/qoe").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenQoeReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "off");

        // 7. Security guard: Host header 403 on /api/screen/qoe
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/qoe HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_pir_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_pir", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/pir default state
        let (code, body) = loopback_get(port, "/api/screen/pir").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert_eq!(rep.mode, "disabled");
        assert_eq!(rep.bands, tetherly_core::SCREEN_PIR_DEFAULT_BANDS);
        assert_eq!(rep.sweeps_completed, 0);
        assert_eq!(rep.slices_sent, 0);
        assert_eq!(rep.current_band, 0);
        assert!(!rep.active_sweep);

        // 2. POST /api/screen/pir enables continuous mode with bands = 12
        let pir_req = serde_json::json!({
            "mode": "continuous",
            "bands": 12
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/pir",
            &serde_json::to_vec(&pir_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "continuous");
        assert_eq!(rep.bands, 12);
        assert!(rep.active_sweep);

        // 3. Verify /api/screen status telemetry includes PIR telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.pir_enabled);
        assert_eq!(status.pir_mode, "continuous");
        assert_eq!(status.pir_bands, 12);
        assert_eq!(status.pir_sweeps_completed, 0);
        assert_eq!(status.pir_slices_sent, 0);

        // 4. POST /api/screen/pir with query parameters
        let (code, body) = loopback_post(port, "/api/screen/pir?mode=on-demand&bands=16", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "on-demand");
        assert_eq!(rep.bands, 16);
        assert!(rep.active_sweep);

        // 5. POST /api/screen/pir/reset restores default disabled state
        let (code, body) = loopback_post(port, "/api/screen/pir/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "disabled");
        assert_eq!(rep.bands, tetherly_core::SCREEN_PIR_DEFAULT_BANDS);
        assert!(!rep.active_sweep);

        // 6. DELETE /api/screen/pir also resets
        let _ = loopback_post(port, "/api/screen/pir?mode=continuous&bands=4", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/pir").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenPirReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.mode, "disabled");

        // 7. Security guard: Host header 403 on /api/screen/pir
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/pir HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_ltr_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_ltr", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/ltr default state
        let (code, body) = loopback_get(port, "/api/screen/ltr").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(
            rep.auto_interval,
            tetherly_core::SCREEN_LTR_DEFAULT_AUTO_INTERVAL
        );
        assert_eq!(rep.current_ltr_seq, 0);
        assert_eq!(rep.anchors_established, 0);
        assert_eq!(rep.recoveries_requested, 0);
        assert_eq!(rep.recoveries_completed, 0);
        assert_eq!(rep.saved_burst_bytes, 0);

        // 2. POST /api/screen/ltr enables LTR with auto_interval = 30
        let ltr_req = serde_json::json!({
            "enabled": true,
            "auto_interval": 30
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/ltr",
            &serde_json::to_vec(&ltr_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.auto_interval, 30);

        // 3. Verify /api/screen status includes LTR telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.ltr_enabled);
        assert_eq!(status.ltr_auto_interval, 30);
        assert_eq!(status.ltr_current_seq, 0);
        assert_eq!(status.ltr_anchors_established, 0);
        assert_eq!(status.ltr_recoveries_completed, 0);

        // 4. POST /api/screen/ltr/anchor triggers manual anchoring
        let (code, body) = loopback_post(port, "/api/screen/ltr/anchor", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.anchors_established, 1);
        assert_eq!(rep.current_ltr_seq, 1);

        // 5. POST /api/screen/ltr/recover triggers recovery request
        let (code, body) = loopback_post(port, "/api/screen/ltr/recover", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert_eq!(rep.recoveries_requested, 1);

        // 6. POST /api/screen/ltr with query parameters
        let (code, body) = loopback_post(port, "/api/screen/ltr?enabled=true&interval=45", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.auto_interval, 45);

        // 7. POST /api/screen/ltr/reset restores default disabled state
        let (code, body) = loopback_post(port, "/api/screen/ltr/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(
            rep.auto_interval,
            tetherly_core::SCREEN_LTR_DEFAULT_AUTO_INTERVAL
        );
        assert_eq!(rep.current_ltr_seq, 0);

        // 8. DELETE /api/screen/ltr also resets
        let _ = loopback_post(port, "/api/screen/ltr?enabled=true&interval=20", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/ltr").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenLtrReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 9. Security guard: Host header 403 on /api/screen/ltr
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/ltr HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_svc_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_svc", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/svc default state
        let (code, body) = loopback_get(port, "/api/screen/svc").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.mode, "disabled");
        assert!(!rep.drop_t1);
        assert_eq!(rep.t0_frames_sent, 0);
        assert_eq!(rep.t1_frames_sent, 0);
        assert_eq!(rep.t1_frames_dropped, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/svc enables adaptive SVC
        let svc_req = serde_json::json!({
            "mode": "adaptive",
            "drop_t1": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/svc",
            &serde_json::to_vec(&svc_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.mode, "adaptive");
        assert!(!rep.drop_t1);

        // 3. Verify /api/screen status includes SVC telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.svc_enabled);
        assert_eq!(status.svc_mode, "adaptive");
        assert!(!status.svc_drop_t1);
        assert_eq!(status.svc_t0_sent, 0);
        assert_eq!(status.svc_t1_sent, 0);
        assert_eq!(status.svc_t1_dropped, 0);

        // 4. POST /api/screen/svc/drop-t1 sets drop_t1 flag
        let drop_req = serde_json::json!({
            "drop_t1": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/svc/drop-t1",
            &serde_json::to_vec(&drop_req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.drop_t1);

        // 5. POST /api/screen/svc with query parameters
        let (code, body) = loopback_post(port, "/api/screen/svc?mode=base_only&drop_t1=false", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.mode, "base_only");

        // 6. POST /api/screen/svc/reset restores default disabled state
        let (code, body) = loopback_post(port, "/api/screen/svc/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.mode, "disabled");
        assert!(!rep.drop_t1);

        // 7. DELETE /api/screen/svc also resets
        let _ = loopback_post(port, "/api/screen/svc?mode=adaptive&drop_t1=true", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/svc").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenSvcReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 8. Security guard: Host header 403 on /api/screen/svc
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/svc HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_motion_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_motion", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/motion default state
        let (code, body) = loopback_get(port, "/api/screen/motion").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_displacement,
            tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
        );
        assert_eq!(rep.motion_frames_sent, 0);
        assert_eq!(rep.fallback_frames_sent, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/motion enables motion estimation
        let req = serde_json::json!({
            "enabled": true,
            "max_displacement": 128
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/motion",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_displacement, 128);

        // 3. Verify /api/screen status includes motion telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.motion_enabled);
        assert_eq!(status.motion_max_displacement, 128);
        assert_eq!(status.motion_frames_sent, 0);
        assert_eq!(status.motion_fallback_frames_sent, 0);
        assert_eq!(status.motion_saved_bytes, 0);

        // 4. POST /api/screen/motion with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/motion?enabled=true&max_displacement=96",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_displacement, 96);

        // 5. POST /api/screen/motion/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/motion/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_displacement,
            tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
        );

        // 6. DELETE /api/screen/motion also resets
        let _ = loopback_post(
            port,
            "/api/screen/motion?enabled=true&max_displacement=128",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/motion").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/motion
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/motion HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_tile", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile default state
        let (code, body) = loopback_get(port, "/api/screen/tile").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.tile_size, tetherly_core::SCREEN_TILE_DEFAULT_SIZE);
        assert_eq!(rep.grid_frames_sent, 0);
        assert_eq!(rep.dirty_tiles_sent, 0);
        assert_eq!(rep.skipped_tiles_count, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/tile enables spatial tile mode
        let req = serde_json::json!({
            "enabled": true,
            "tile_size": 128
        });
        let (code, body) =
            loopback_post(port, "/api/screen/tile", &serde_json::to_vec(&req).unwrap())
                .await
                .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.tile_size, 128);

        // 3. Verify /api/screen status includes tile telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_enabled);
        assert_eq!(status.tile_size, 128);
        assert_eq!(status.tile_frames_sent, 0);
        assert_eq!(status.dirty_tiles_sent, 0);
        assert_eq!(status.skipped_tiles_count, 0);
        assert_eq!(status.tile_saved_bytes, 0);

        // 4. POST /api/screen/tile with query parameters
        let (code, body) = loopback_post(port, "/api/screen/tile?enabled=true&tile_size=64", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.tile_size, 64);

        // 5. POST /api/screen/tile/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.tile_size, tetherly_core::SCREEN_TILE_DEFAULT_SIZE);

        // 6. DELETE /api/screen/tile also resets
        let _ = loopback_post(port, "/api/screen/tile?enabled=true&tile_size=128", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req =
            "GET /api/screen/tile HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_cache_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_cache", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/cache default state
        let (code, body) = loopback_get(port, "/api/screen/tile/cache").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_entries,
            tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        );
        assert_eq!(rep.current_entries, 0);
        assert_eq!(rep.cache_hits, 0);
        assert_eq!(rep.cache_misses, 0);
        assert_eq!(rep.hit_ratio_pct, 0.0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/tile/cache enables tile cache mode
        let req = serde_json::json!({
            "enabled": true,
            "max_entries": 1024
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/cache",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_entries, 1024);

        // 3. Verify /api/screen status includes tile cache telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_cache_enabled);
        assert_eq!(status.tile_cache_max_entries, 1024);
        assert_eq!(status.tile_cache_entries, 0);
        assert_eq!(status.tile_cache_hits, 0);
        assert_eq!(status.tile_cache_misses, 0);
        assert_eq!(status.tile_cache_saved_bytes, 0);

        // 4. POST /api/screen/tile/cache with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/cache?enabled=true&max_entries=256",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_entries, 256);

        // 5. POST /api/screen/tile/cache/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/cache/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_entries,
            tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        );

        // 6. DELETE /api/screen/tile/cache also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/cache?enabled=true&max_entries=128",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/cache")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileCacheReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/cache
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/cache HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_focus_endpoints() {
        let (node, dir) = temp_test_node_with_source("screen_focus", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/focus default state
        let (code, body) = loopback_get(port, "/api/screen/focus").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.mode, tetherly_core::ScreenFocusMode::Cursor);
        assert_eq!(rep.radius, tetherly_core::SCREEN_FOCUS_DEFAULT_RADIUS);
        assert_eq!(
            rep.peripheral_skip_ratio,
            tetherly_core::SCREEN_FOCUS_DEFAULT_SKIP_RATIO
        );
        assert_eq!(rep.focus_tiles_sent, 0);
        assert_eq!(rep.peripheral_tiles_sent, 0);
        assert_eq!(rep.peripheral_tiles_skipped, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/focus enables focus with cursor mode
        let req = serde_json::json!({
            "enabled": true,
            "mode": "cursor",
            "radius": 320,
            "peripheral_skip_ratio": 3
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/focus",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.mode, tetherly_core::ScreenFocusMode::Cursor);
        assert_eq!(rep.radius, 320);
        assert_eq!(rep.peripheral_skip_ratio, 3);

        // 3. Verify /api/screen status includes focus telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.focus_enabled);
        assert_eq!(status.focus_mode, "cursor");
        assert_eq!(status.focus_radius, 320);
        assert_eq!(status.focus_peripheral_skip_ratio, 3);
        assert_eq!(status.focus_tiles_sent, 0);
        assert_eq!(status.peripheral_tiles_sent, 0);
        assert_eq!(status.peripheral_tiles_skipped, 0);
        assert_eq!(status.focus_saved_bandwidth_bytes, 0);

        // 4. POST /api/screen/focus with box mode and coordinates
        let req_box = serde_json::json!({
            "enabled": true,
            "mode": "box",
            "box_x": 100,
            "box_y": 150,
            "box_w": 400,
            "box_h": 300,
            "peripheral_skip_ratio": 4
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/focus",
            &serde_json::to_vec(&req_box).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.mode, tetherly_core::ScreenFocusMode::Box);
        assert_eq!(rep.box_x, 100);
        assert_eq!(rep.box_y, 150);
        assert_eq!(rep.box_w, 400);
        assert_eq!(rep.box_h, 300);
        assert_eq!(rep.peripheral_skip_ratio, 4);

        // 5. POST /api/screen/focus with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/focus?enabled=true&mode=cursor&radius=180&peripheral_skip_ratio=2",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.mode, tetherly_core::ScreenFocusMode::Cursor);
        assert_eq!(rep.radius, 180);
        assert_eq!(rep.peripheral_skip_ratio, 2);

        // 6. POST /api/screen/focus/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/focus/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.radius, tetherly_core::SCREEN_FOCUS_DEFAULT_RADIUS);

        // 7. DELETE /api/screen/focus also resets
        let _ = loopback_post(port, "/api/screen/focus?enabled=true&radius=128", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/focus").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenFocusReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 8. Security guard: Host header 403 on /api/screen/focus
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/focus HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_palette_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_palette", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/palette default state
        let (code, body) = loopback_get(port, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_colors,
            tetherly_core::SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS
        );
        assert!(!rep.enable_rle);
        assert_eq!(rep.solid_tiles_count, 0);
        assert_eq!(rep.bilevel_tiles_count, 0);
        assert_eq!(rep.palette_tiles_count, 0);
        assert_eq!(rep.truecolor_tiles_count, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/tile/palette enables palette mode
        let req = serde_json::json!({
            "enabled": true,
            "max_colors": 8,
            "enable_rle": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/palette",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_colors, 8);
        assert!(rep.enable_rle);

        // 3. Verify /api/screen status includes tile palette telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_palette_enabled);
        assert_eq!(status.tile_palette_max_colors, 8);
        assert!(status.tile_palette_enable_rle);
        assert_eq!(status.tile_palette_solid_tiles, 0);
        assert_eq!(status.tile_palette_bilevel_tiles, 0);
        assert_eq!(status.tile_palette_palette_tiles, 0);
        assert_eq!(status.tile_palette_truecolor_tiles, 0);
        assert_eq!(status.tile_palette_saved_bytes, 0);

        // 4. POST /api/screen/tile/palette with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/palette?enabled=true&max_colors=12&enable_rle=false",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_colors, 12);
        assert!(!rep.enable_rle);

        // 5. POST /api/screen/tile/palette/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/palette/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(
            rep.max_colors,
            tetherly_core::SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS
        );

        // 6. DELETE /api/screen/tile/palette also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/palette?enabled=true&max_colors=4",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/palette")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePaletteReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/palette
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/palette HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_predict_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_predict", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/predict default state
        let (code, body) = loopback_get(port, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(
            rep.filter_mode,
            tetherly_core::ScreenTilePredictFilter::Auto
        );
        assert!(rep.enable_rct);
        assert_eq!(rep.predict_tiles_count, 0);
        assert_eq!(rep.saved_bandwidth_bytes, 0);

        // 2. POST /api/screen/tile/predict enables predict mode
        let req = serde_json::json!({
            "enabled": true,
            "filter_mode": "sub",
            "enable_rct": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/predict",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.filter_mode, tetherly_core::ScreenTilePredictFilter::Sub);
        assert!(!rep.enable_rct);

        // 3. Verify /api/screen status includes tile predict telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_predict_enabled);
        assert_eq!(status.tile_predict_filter_mode, "sub");
        assert!(!status.tile_predict_enable_rct);
        assert_eq!(status.tile_predict_tiles_sent, 0);
        assert_eq!(status.tile_predict_saved_bytes, 0);

        // 4. POST /api/screen/tile/predict with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/predict?enabled=true&filter_mode=up&enable_rct=true",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.filter_mode, tetherly_core::ScreenTilePredictFilter::Up);
        assert!(rep.enable_rct);

        // 5. POST /api/screen/tile/predict/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/predict/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(
            rep.filter_mode,
            tetherly_core::ScreenTilePredictFilter::Auto
        );
        assert!(rep.enable_rct);

        // 6. DELETE /api/screen/tile/predict also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/predict?enabled=true&filter_mode=paeth",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/predict")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePredictReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/predict
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/predict HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_present_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_present", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/present default state
        let (code, body) = loopback_get(port, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.vsync_mode, "off");
        assert!(rep.double_buffer);
        assert!(rep.tear_prevention);
        assert_eq!(rep.committed_transactions, 0);
        assert_eq!(rep.swapped_frames, 0);
        assert_eq!(rep.torn_frames_prevented, 0);
        assert_eq!(rep.avg_swap_latency_us, 0);

        // 2. POST /api/screen/tile/present enables presentation transaction mode
        let req = serde_json::json!({
            "enabled": true,
            "vsync_mode": "vsync60",
            "double_buffer": true,
            "tear_prevention": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/present",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.vsync_mode, "vsync60");
        assert!(rep.double_buffer);
        assert!(rep.tear_prevention);

        // 3. Verify /api/screen status includes tile present telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_present_enabled);
        assert_eq!(status.tile_present_vsync_mode, "vsync60");
        assert!(status.tile_present_double_buffer);
        assert!(status.tile_present_tear_prevention);
        assert_eq!(status.tile_present_committed_transactions, 0);
        assert_eq!(status.tile_present_swapped_frames, 0);
        assert_eq!(status.tile_present_torn_frames_prevented, 0);

        // 4. POST /api/screen/tile/present with query parameters
        let (code, body) = loopback_post(port, "/api/screen/tile/present?enabled=true&vsync_mode=vsync120&double_buffer=false&tear_prevention=false", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.vsync_mode, "vsync120");
        assert!(!rep.double_buffer);
        assert!(!rep.tear_prevention);

        // 5. POST /api/screen/tile/present/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/present/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.vsync_mode, "off");
        assert!(rep.double_buffer);
        assert!(rep.tear_prevention);

        // 6. DELETE /api/screen/tile/present also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/present?enabled=true&vsync_mode=adaptive",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/present")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTilePresentReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/present
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/present HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_nack_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_nack", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/nack default state
        let (code, body) = loopback_get(port, "/api/screen/tile/nack").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.timeout_ms, 20);
        assert_eq!(rep.max_retries, 2);
        assert_eq!(rep.nack_requests_sent, 0);
        assert_eq!(rep.tiles_retransmitted, 0);
        assert_eq!(rep.degraded_commits, 0);

        // 2. POST /api/screen/tile/nack enables tile nack mode
        let req = serde_json::json!({
            "enabled": true,
            "timeout_ms": 30,
            "max_retries": 3
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/nack",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.timeout_ms, 30);
        assert_eq!(rep.max_retries, 3);

        // 3. Verify /api/screen status includes tile nack telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_nack_enabled);
        assert_eq!(status.tile_nack_timeout_ms, 30);
        assert_eq!(status.tile_nack_max_retries, 3);
        assert_eq!(status.tile_nack_requests_sent, 0);
        assert_eq!(status.tile_nack_tiles_retransmitted, 0);
        assert_eq!(status.tile_nack_degraded_commits, 0);

        // 4. POST /api/screen/tile/nack with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/nack?enabled=true&timeout_ms=50&max_retries=5",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.timeout_ms, 50);
        assert_eq!(rep.max_retries, 5);

        // 5. POST /api/screen/tile/nack/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/nack/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.timeout_ms, 20);
        assert_eq!(rep.max_retries, 2);

        // 6. DELETE /api/screen/tile/nack also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/nack?enabled=true&timeout_ms=40",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/nack")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileNackReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/nack
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/nack HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_saliency_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_saliency", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/saliency reports default state
        let (code, body) = loopback_get(port, "/api/screen/tile/saliency")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.high_threshold, 120);
        assert_eq!(rep.low_threshold, 40);
        assert!(rep.drop_background_on_congestion);
        assert_eq!(rep.critical_tiles_sent, 0);
        assert_eq!(rep.normal_tiles_sent, 0);
        assert_eq!(rep.background_tiles_sent, 0);
        assert_eq!(rep.background_tiles_skipped, 0);

        // 2. POST /api/screen/tile/saliency enables tile saliency mode
        let req = serde_json::json!({
            "enabled": true,
            "high_threshold": 130,
            "low_threshold": 50,
            "drop_background_on_congestion": true
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/saliency",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.high_threshold, 130);
        assert_eq!(rep.low_threshold, 50);
        assert!(rep.drop_background_on_congestion);

        // 3. Verify /api/screen status includes tile saliency telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_saliency_enabled);
        assert_eq!(status.tile_saliency_high_threshold, 130);
        assert_eq!(status.tile_saliency_low_threshold, 50);
        assert!(status.tile_saliency_drop_background_on_congestion);
        assert_eq!(status.tile_saliency_critical_tiles, 0);
        assert_eq!(status.tile_saliency_normal_tiles, 0);
        assert_eq!(status.tile_saliency_background_tiles, 0);
        assert_eq!(status.tile_saliency_background_skipped, 0);

        // 4. POST /api/screen/tile/saliency with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/saliency?enabled=true&high_threshold=150&low_threshold=60&drop_background=false",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.high_threshold, 150);
        assert_eq!(rep.low_threshold, 60);
        assert!(!rep.drop_background_on_congestion);

        // 5. POST /api/screen/tile/saliency/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/saliency/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.high_threshold, 120);
        assert_eq!(rep.low_threshold, 40);
        assert!(rep.drop_background_on_congestion);

        // 6. DELETE /api/screen/tile/saliency also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/saliency?enabled=true&high_threshold=140",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/saliency")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileSaliencyReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/saliency
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/saliency HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_lod_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_lod", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/lod reports default state
        let (code, body) = loopback_get(port, "/api/screen/tile/lod").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.max_lod, 1);
        assert!(rep.auto_lod_on_congestion);
        assert_eq!(rep.lod0_tiles_sent, 0);
        assert_eq!(rep.lod1_tiles_sent, 0);
        assert_eq!(rep.lod2_tiles_sent, 0);
        assert_eq!(rep.bytes_saved, 0);

        // 2. POST /api/screen/tile/lod enables tile LoD mode
        let req = serde_json::json!({
            "enabled": true,
            "max_lod": 2,
            "auto_lod_on_congestion": false
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/lod",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_lod, 2);
        assert!(!rep.auto_lod_on_congestion);

        // 3. Verify /api/screen status includes tile LoD telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_lod_enabled);
        assert_eq!(status.tile_lod_max_lod, 2);
        assert!(!status.tile_lod_auto_lod_on_congestion);
        assert_eq!(status.tile_lod0_tiles, 0);
        assert_eq!(status.tile_lod1_tiles, 0);
        assert_eq!(status.tile_lod2_tiles, 0);
        assert_eq!(status.tile_lod_bytes_saved, 0);

        // 4. POST /api/screen/tile/lod with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/lod?enabled=true&max_lod=1&auto_lod=true",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_lod, 1);
        assert!(rep.auto_lod_on_congestion);

        // 5. POST /api/screen/tile/lod/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/lod/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.max_lod, 1);
        assert!(rep.auto_lod_on_congestion);

        // 6. DELETE /api/screen/tile/lod also resets
        let _ = loopback_post(port, "/api/screen/tile/lod?enabled=true&max_lod=2", &[])
            .await
            .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/lod").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileLodReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/lod
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/lod HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_batch_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_batch", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/batch reports default state
        let (code, body) = loopback_get(port, "/api/screen/tile/batch").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.target_chunk_bytes, 1200);
        assert_eq!(rep.pacing_interval_us, 200);
        assert_eq!(rep.batches_sent, 0);
        assert_eq!(rep.chunks_sent, 0);
        assert_eq!(rep.avg_chunk_bytes, 0);
        assert_eq!(rep.pacing_pauses, 0);

        // 2. POST /api/screen/tile/batch enables batch mode with JSON body
        let req = serde_json::json!({
            "enabled": true,
            "target_chunk_bytes": 1400,
            "pacing_interval_us": 100
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/batch",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.target_chunk_bytes, 1400);
        assert_eq!(rep.pacing_interval_us, 100);

        // 3. Verify /api/screen status includes tile batch telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_batch_enabled);
        assert_eq!(status.tile_batch_target_chunk_bytes, 1400);
        assert_eq!(status.tile_batch_pacing_interval_us, 100);
        assert_eq!(status.tile_batches_sent, 0);
        assert_eq!(status.tile_chunks_sent, 0);
        assert_eq!(status.tile_batch_avg_chunk_bytes, 0);
        assert_eq!(status.tile_batch_pacing_pauses, 0);

        // 4. POST /api/screen/tile/batch with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/batch?enabled=true&chunk_bytes=800&pacing=300",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.target_chunk_bytes, 800);
        assert_eq!(rep.pacing_interval_us, 300);

        // 5. POST /api/screen/tile/batch/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/batch/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.target_chunk_bytes, 1200);
        assert_eq!(rep.pacing_interval_us, 200);

        // 6. DELETE /api/screen/tile/batch also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/batch?enabled=true&chunk_bytes=900",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/batch")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Test alias /api/screen/batch
        let (code, body) = loopback_get(port, "/api/screen/batch").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileBatchReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 8. Security guard: Host header 403 on /api/screen/tile/batch
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/batch HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_uihttp_screen_tile_motion_endpoints() {
        let (node, dir) =
            temp_test_node_with_source("screen_tile_motion", Arc::new(EmptyScreenSource));
        let port = bind_and_spawn(node.clone_handle(), 0).await.unwrap();

        // 1. GET /api/screen/tile/motion reports default state
        let (code, body) = loopback_get(port, "/api/screen/tile/motion").await.unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.ok);
        assert!(!rep.enabled);
        assert_eq!(rep.max_displacement, 32);
        assert_eq!(rep.min_match_ratio, 90);
        assert_eq!(rep.motion_tiles_sent, 0);
        assert_eq!(rep.fallback_tiles_sent, 0);
        assert_eq!(rep.bytes_saved, 0);

        // 2. POST /api/screen/tile/motion enables motion estimation with JSON body
        let req = serde_json::json!({
            "enabled": true,
            "max_displacement": 64,
            "min_match_ratio": 95
        });
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/motion",
            &serde_json::to_vec(&req).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_displacement, 64);
        assert_eq!(rep.min_match_ratio, 95);

        // 3. Verify /api/screen status includes tile motion telemetry
        let (code, body) = loopback_get(port, "/api/screen").await.unwrap();
        assert_eq!(code, 200);
        let status: crate::runtime::ScreenStatusReport = serde_json::from_slice(&body).unwrap();
        assert!(status.tile_motion_enabled);
        assert_eq!(status.tile_motion_max_displacement, 64);
        assert_eq!(status.tile_motion_min_match_ratio, 95);
        assert_eq!(status.tile_motion_tiles_sent, 0);
        assert_eq!(status.tile_motion_fallback_tiles_sent, 0);
        assert_eq!(status.tile_motion_bytes_saved, 0);

        // 4. POST /api/screen/tile/motion with query parameters
        let (code, body) = loopback_post(
            port,
            "/api/screen/tile/motion?enabled=true&max_disp=48&ratio=85",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(rep.enabled);
        assert_eq!(rep.max_displacement, 48);
        assert_eq!(rep.min_match_ratio, 85);

        // 5. POST /api/screen/tile/motion/reset restores default state
        let (code, body) = loopback_post(port, "/api/screen/tile/motion/reset", &[])
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);
        assert_eq!(rep.max_displacement, 32);
        assert_eq!(rep.min_match_ratio, 90);

        // 6. DELETE /api/screen/tile/motion also resets
        let _ = loopback_post(
            port,
            "/api/screen/tile/motion?enabled=true&max_disp=50",
            &[],
        )
        .await
        .unwrap();
        let (code, body) = loopback_delete(port, "/api/screen/tile/motion")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let rep: tetherly_core::ScreenTileMotionReport = serde_json::from_slice(&body).unwrap();
        assert!(!rep.enabled);

        // 7. Security guard: Host header 403 on /api/screen/tile/motion
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let evil_req = "GET /api/screen/tile/motion HTTP/1.1\r\nHost: evil.attacker.com\r\nConnection: close\r\n\r\n";
        stream.write_all(evil_req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let (resp_code, _) = parse_http_response(&raw);
        assert_eq!(resp_code, 403);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
