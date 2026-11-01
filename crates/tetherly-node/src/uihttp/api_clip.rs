// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard sync and clipboard policy.

use super::*;

pub(super) async fn get_clip_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.clipboard_policy_report())
}

pub(super) async fn post_clip_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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
                    Err(e) => return err(400, &format!("invalid device type '{trimmed}': {e}")),
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

pub(super) async fn post_clip_auto_sync_poll(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.poll_clipboard_auto_sync().await {
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
    }
}

pub(super) async fn get_clip_status(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.clipboard_status() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &e.to_string()),
    }
}

pub(super) async fn get_clip_history(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let limit = qmap.get("limit").and_then(|s| s.parse::<usize>().ok());
    json_ok(&node.clipboard_history(limit))
}

pub(super) async fn post_clip_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.clear_clipboard() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &e.to_string()),
    }
}

pub(super) async fn get_clip(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.get_clipboard_text() {
        Ok(Some(text)) => {
            let hash = tetherly_core::hex_sha256(text.as_bytes());
            let bytes = text.len();
            json_ok(&serde_json::json!({
                "ok": true,
                "has_text": true,
                "bytes": bytes,
                "hash": hash,
                "text": text,
                "clipboard_available": true,
            }))
        }
        Ok(None) => json_ok(&serde_json::json!({
            "ok": true,
            "has_text": false,
            "bytes": 0,
            "hash": null,
            "text": null,
            "clipboard_available": true,
        })),
        // A host with no display server has no clipboard to read: report
        // the absence (v1.87 status semantics) instead of answering 500.
        Err(e) => json_ok(&serde_json::json!({
            "ok": true,
            "has_text": false,
            "bytes": 0,
            "hash": null,
            "text": null,
            "clipboard_available": false,
            "reason": e.to_string(),
        })),
    }
}

// Deliver a notify.push on the live Noise session. Only this process
// holds the session, so a second binary cannot do it. One live peer is
// implied; more than one needs an explicit `peer`. The body is delivered
// and never logged.
pub(super) async fn post_clip_sync(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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
