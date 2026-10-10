// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Loopback-only desktop shell. Never bind 0.0.0.0. Bodies/OTP/PIN are not
//! written to logs. Full Tauri packaging is the next desktop-shell iteration.

use crate::runtime::Node;
use serde::Serialize;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

const INDEX: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/index.html"));
const MAX_BODY: usize = 8 * 1024;

pub fn spawn(node: Node, port: u16) {
    tokio::spawn(async move {
        if let Err(e) = serve(node, port).await {
            warn!(%e, "ui http");
        }
    });
}

async fn serve(node: Node, port: u16) -> std::io::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    info!(%bound, "ui listening (loopback only)");
    loop {
        let (stream, peer) = listener.accept().await?;
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

async fn handle(mut stream: TcpStream, node: Node) -> std::io::Result<()> {
    let mut buf = vec![0u8; 4096];
    let mut n = 0usize;
    loop {
        if n >= buf.len() {
            buf.resize((buf.len() * 2).min(MAX_BODY + 4096), 0);
        }
        let k = stream.read(&mut buf[n..]).await?;
        if k == 0 {
            return Ok(());
        }
        n += k;
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if n > MAX_BODY + 2048 {
            write_http(&mut stream, 413, "text/plain", b"too large").await?;
            return Ok(());
        }
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
    if content_len > MAX_BODY {
        write_http(&mut stream, 413, "text/plain", b"too large").await?;
        return Ok(());
    }
    let mut body = buf[head_end..n].to_vec();
    while body.len() < content_len {
        let mut more = vec![0u8; content_len - body.len()];
        let k = stream.read(&mut more).await?;
        if k == 0 {
            break;
        }
        body.extend_from_slice(&more[..k]);
    }
    body.truncate(content_len);
    let (code, ctype, payload) = route(method, path, &body, &node).await;
    write_http(&mut stream, code, ctype, &payload).await
}

async fn write_http(
    stream: &mut TcpStream,
    code: u16,
    ctype: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
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
    peers: Vec<PeerView>,
    files: Vec<FileSnap>,
    /// "off" when no BLE transport is attached, else the ingress state.
    ancs_state: String,
    /// Allowlisted app ids, so the wizard can show what may be opened.
    open_apps: Vec<String>,
    /// Remote-screen channel state (idle/requested/allowed/streaming).
    screen_state: String,
    /// Screen counters for the local UI. Never carries pixels.
    screen: ScreenSnap,
}

#[derive(Serialize)]
struct ScreenSnap {
    state: String,
    sent: u64,
    received: u64,
    dropped: u64,
    bytes: u64,
    refused: u64,
}

#[derive(Serialize)]
struct PeerView {
    device_id: String,
    name: String,
    platform: String,
    path: String,
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
    addr: String,
    pin: Option<String>,
}

#[derive(serde::Deserialize)]
struct IdBody {
    id: String,
}

#[derive(serde::Deserialize)]
struct OpenRuleBody {
    app_id: String,
    url: String,
}

async fn route(method: &str, path: &str, body: &[u8], node: &Node) -> (u16, &'static str, Vec<u8>) {
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => (200, "text/html", INDEX.as_bytes().to_vec()),
        ("GET", "/api/state") => json_ok(&snapshot(node)),
        ("POST", "/api/pin") => match node.generate_pin() {
            Ok(pin) => json_ok(&serde_json::json!({ "pin": pin })),
            Err(e) => err(400, &e.to_string()),
        },
        ("POST", "/api/dial") => match serde_json::from_slice::<DialBody>(body) {
            Ok(d) => {
                if let Some(p) = d.pin {
                    match tetherly_core::pin::parse_pin(&p) {
                        Ok(digits) => node.set_pin(digits),
                        Err(e) => return err(400, &e.to_string()),
                    }
                }
                match d.addr.parse() {
                    Ok(addr) => match node.dial(addr).await {
                        Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                        Err(e) => err(400, &e.to_string()),
                    },
                    Err(_) => err(400, "bad addr"),
                }
            }
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
            Ok(d) => {
                let _ = node.dismiss_candidate(&d.id);
                json_ok(&serde_json::json!({ "ok": true }))
            }
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/open") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.open_candidate(&d.id, &crate::platform::SystemOpener) {
                Ok(url) => json_ok(&serde_json::json!({ "ok": true, "url": url })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/file/accept") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.user_accept_file(&d.id).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        ("POST", "/api/file/reject") => match serde_json::from_slice::<IdBody>(body) {
            Ok(d) => match node.user_reject_file(&d.id).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        // Step 1 of the phone wizard: the user connected the iPhone in the OS
        // Bluetooth settings, so we only have to subscribe to ANCS here.
        ("POST", "/api/ancs/connect") => match node.ancs_connect() {
            Ok(Some(tetherly_core::ConnectOutcome::Subscribed)) => {
                json_ok(&serde_json::json!({ "state": "ready" }))
            }
            Ok(Some(tetherly_core::ConnectOutcome::BackingOff { retry_at_ms })) => {
                json_ok(&serde_json::json!({ "state": "backoff", "retry_at_ms": retry_at_ms }))
            }
            Ok(None) => err(400, "no ANCS transport attached"),
            Err(e) => err(400, &e.to_string()),
        },
        ("POST", "/api/ancs/allow") => match serde_json::from_slice::<OpenRuleBody>(body) {
            Ok(d) => match node.add_open_rule(tetherly_core::OpenRule {
                app_id: d.app_id,
                url: d.url,
            }) {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
                Err(e) => err(400, &e.to_string()),
            },
            Err(_) => err(400, "bad json"),
        },
        // Remote screen: consent is local and explicit. `allow` is the only
        // gate into streaming; nothing a peer sends can flip it.
        ("POST", "/api/screen/allow") => {
            node.screen_allow();
            json_ok(&serde_json::json!({ "state": node.screen_state().as_str() }))
        }
        ("POST", "/api/screen/start") => match node.screen_start().await {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
            Err(e) => err(400, &e.to_string()),
        },
        ("POST", "/api/screen/stop") => match node.screen_stop().await {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
            Err(e) => err(400, &e.to_string()),
        },
        _ => (404, "text/plain", b"not found".to_vec()),
    }
}

fn snapshot(node: &Node) -> Snapshot {
    let ov = node.overlay_status();
    let ancs_state = match node.ancs_state() {
        None => "off".to_string(),
        Some(tetherly_core::AncsState::Idle) => "idle".to_string(),
        Some(tetherly_core::AncsState::Ready) => "ready".to_string(),
        Some(tetherly_core::AncsState::Backoff { retry_at_ms }) => {
            format!("backoff until {retry_at_ms}")
        }
    };
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
        ancs_state,
        open_apps: node
            .open_allowlist()
            .rules()
            .iter()
            .map(|r| r.app_id.clone())
            .collect(),
        peers: node
            .live_peers()
            .into_iter()
            .map(|p| PeerView {
                device_id: p.device_id.to_string(),
                name: p.name,
                platform: p.platform,
                path: match p.path {
                    tetherly_core::PathKind::Lan => "lan".into(),
                    tetherly_core::PathKind::Overlay => "overlay".into(),
                },
            })
            .collect(),
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
        screen_state: node.screen_state().as_str().to_string(),
        screen: {
            let s = node.screen_stats();
            ScreenSnap {
                state: node.screen_state().as_str().to_string(),
                sent: s.sent,
                received: s.received,
                dropped: s.dropped,
                bytes: s.bytes,
                refused: s.refused,
            }
        },
    }
}

fn json_ok<T: Serialize>(v: &T) -> (u16, &'static str, Vec<u8>) {
    match serde_json::to_vec(v) {
        Ok(b) => (200, "application/json", b),
        Err(_) => err(400, "encode"),
    }
}

fn err(code: u16, msg: &str) -> (u16, &'static str, Vec<u8>) {
    (
        code,
        "application/json",
        serde_json::to_vec(&serde_json::json!({ "error": msg })).unwrap_or_default(),
    )
}
