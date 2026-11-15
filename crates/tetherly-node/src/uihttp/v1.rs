// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `/api/v1` facade (ADR-139): typed DTOs from `tetherly-api`, one error
//! envelope, version headers on every response. Auth tiers are enforced in
//! `handle()` before this runs.

use super::{write_http_response, UiResponse};
use crate::apiv1::hub::EventHub;
use crate::apiv1::journal::Frame;
use crate::apiv1::model::system;
use crate::Node;
use serde::Serialize;
use std::sync::Arc;
use tetherly_api::{ErrorBody, ErrorCode, Event, Health, Topic, API_VERSION};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;

/// Stamp the contract and daemon version on a v1 response.
fn versioned(resp: UiResponse) -> UiResponse {
    resp.with_header(tetherly_api::API_HEADER, API_VERSION.to_string())
        .with_header(tetherly_api::VERSION_HEADER, env!("CARGO_PKG_VERSION"))
}

fn json<T: Serialize>(code: u16, v: &T) -> UiResponse {
    match serde_json::to_vec(v) {
        Ok(b) => versioned(UiResponse::new(code, "application/json; charset=utf-8", b)),
        Err(e) => error(ErrorCode::Internal, format!("encode: {e}")),
    }
}

/// A v1 error response: `{"error":{"code","message"}}` with the code's status.
pub(super) fn error(code: ErrorCode, message: impl Into<String>) -> UiResponse {
    let body = serde_json::to_vec(&ErrorBody::new(code, message)).unwrap_or_default();
    versioned(UiResponse::new(
        code.http_status(),
        "application/json; charset=utf-8",
        body,
    ))
}

/// `/api/v1/devices/{id}` -> `id` (one non-empty segment).
fn device_item(path: &str) -> Option<&str> {
    path.strip_prefix("/api/v1/devices/")
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// Methods each known v1 path answers; drives 404 vs 405.
fn allowed(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/v1/health" | "/api/v1/system" | "/api/v1/events" | "/api/v1/devices" => {
            Some(&["GET"])
        }
        p if device_item(p).is_some() => Some(&["GET"]),
        _ => None,
    }
}

pub(super) async fn route(
    method: &str,
    path: &str,
    _query: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match (method, path) {
        ("GET", "/api/v1/health") => json(200, &Health::new(env!("CARGO_PKG_VERSION"))),
        ("GET", "/api/v1/system") => json(200, &system(node)),
        ("GET", "/api/v1/devices") => json(200, &crate::apiv1::model::devices(node)),
        ("GET", p) if device_item(p).is_some() => {
            let id = device_item(p).unwrap_or_default();
            match crate::apiv1::model::device(node, id) {
                Some(d) => json(200, &d),
                None => error(ErrorCode::NotFound, format!("no such device: {id}")),
            }
        }
        _ => match allowed(path) {
            Some(methods) => error(
                ErrorCode::MethodNotAllowed,
                format!("{method} not allowed; use {}", methods.join(", ")),
            )
            .with_header("Allow", methods.join(", ")),
            None => error(ErrorCode::NotFound, format!("no such resource: {path}")),
        },
    }
}

/// Keep-alive comment interval; also how fast a dead client is noticed.
const PING: std::time::Duration = std::time::Duration::from_secs(15);

/// `GET /api/v1/events`: snapshot (or `Last-Event-ID` replay), then live
/// frames. `?topics=a,b` filters; snapshots always pass. A subscriber that
/// falls behind the broadcast buffer is disconnected and resumes by id.
pub(super) async fn events(
    stream: &mut TcpStream,
    node: &Node,
    hub: &Arc<EventHub>,
    query: Option<&str>,
    header_last_id: Option<u64>,
) -> std::io::Result<()> {
    let params = query.map(super::parse_query_map).unwrap_or_default();
    let topics = match params.get("topics").map(|t| Topic::parse_list(t)) {
        None => None,
        Some(Ok(list)) => Some(list),
        Some(Err(bad)) => {
            let resp = error(ErrorCode::BadRequest, format!("unknown topic: {bad}"));
            return write_http_response(stream, &resp).await;
        }
    };
    let wanted = |f: &Frame| match (&topics, f.topic) {
        (Some(list), Some(t)) => list.contains(&t),
        _ => true,
    };
    let last_id =
        header_last_id.or_else(|| params.get("last_event_id").and_then(|v| v.parse().ok()));
    let mut sub = hub.subscribe(last_id);

    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream; charset=utf-8\r\n\
         Cache-Control: no-cache, no-transform\r\n\
         Connection: keep-alive\r\n\
         {}: {}\r\n\
         {}: {}\r\n\r\n\
         retry: 2000\n\n",
        tetherly_api::API_HEADER,
        API_VERSION,
        tetherly_api::VERSION_HEADER,
        env!("CARGO_PKG_VERSION"),
    );
    stream.write_all(head.as_bytes()).await?;

    let mut sent = sub.current;
    match sub.replay.take() {
        Some(frames) => {
            for f in frames.iter().filter(|f| wanted(f)) {
                stream.write_all(f.sse().as_bytes()).await?;
            }
            sent = frames.last().map_or(sent, |f| f.seq.max(sent));
        }
        None => {
            let snapshot = crate::apiv1::model::snapshot(node).await;
            let frame = Frame::new(sub.current, &Event::Snapshot { snapshot });
            stream.write_all(frame.sse().as_bytes()).await?;
        }
    }
    stream.flush().await?;

    let mut shutdown = node.shutdown_signal();
    let mut ping = tokio::time::interval(PING);
    ping.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            res = sub.rx.recv() => match res {
                Ok(f) => {
                    if f.seq <= sent {
                        continue;
                    }
                    sent = f.seq;
                    if wanted(&f) {
                        stream.write_all(f.sse().as_bytes()).await?;
                        stream.flush().await?;
                    }
                }
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                stream.write_all(b": ping\n\n").await?;
                stream.flush().await?;
            }
        }
    }
    Ok(())
}
