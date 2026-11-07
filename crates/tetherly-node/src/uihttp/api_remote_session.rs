// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ADR-138 remote desktop session REST surface: start / stop / status.

use super::*;

pub(super) async fn post_remote_session_start(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let target_opt = if !body.is_empty() {
        match serde_json::from_slice::<PeerTargetBody>(body) {
            Ok(b) => b.peer,
            Err(_) => return err(400, "bad json, expected PeerTargetBody"),
        }
    } else {
        None
    };
    let peer = match target_opt.or_else(|| qmap.get("peer").cloned()) {
        Some(p) if !p.is_empty() => p,
        _ => return err(400, "missing peer"),
    };

    let reason = qmap
        .get("reason")
        .and_then(|s| s.parse::<tetherly_core::RemoteSessionReason>().ok())
        .unwrap_or_default();

    match node.remote_session_start(&peer, reason).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_remote_session_stop(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let reason = qmap
        .get("reason")
        .cloned()
        .unwrap_or_else(|| "rest_stop".to_string());
    match node.remote_session_stop(&reason).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_remote_session(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.remote_session_report())
}
