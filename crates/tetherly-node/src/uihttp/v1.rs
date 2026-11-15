// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `/api/v1` facade (ADR-139): typed DTOs from `tetherly-api`, one error
//! envelope, version headers on every response. Auth tiers are enforced in
//! `handle()` before this runs.

use super::UiResponse;
use crate::Node;
use serde::Serialize;
use tetherly_api::{ErrorBody, ErrorCode, Health, PeerCounts, Ports, System, API_VERSION};

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

/// Methods each known v1 path answers; drives 404 vs 405.
fn allowed(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/v1/health" | "/api/v1/system" => Some(&["GET"]),
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

fn system(node: &Node) -> System {
    let h = node.health_report();
    let caps = node.local_caps_report();
    System {
        device_id: h.device_id,
        name: h.name,
        platform: h.platform,
        version: h.version,
        api: API_VERSION,
        pid: h.pid,
        uptime_secs: h.uptime_secs,
        ports: Ports {
            control: h.ports.control,
            file: h.ports.file,
            input: h.ports.input,
            screen: h.ports.screen,
            ui: h.ports.ui,
        },
        peers: PeerCounts {
            live: h.live_peers as u64,
            trusted: h.trusted_peers as u64,
        },
        caps: caps.caps,
        features: caps.features,
    }
}
