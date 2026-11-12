// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote input tunnelling (pointer/keyboard) policy and sessions.

use super::*;

pub(super) async fn get_input(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.input_status_report().await;
    json_ok(&rep)
}

/// `GET /api/input/trace` — the input-engine decision ring (v0.2.30).
pub(super) async fn get_input_trace(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.input_trace_report().await;
    json_ok(&rep)
}

pub(super) async fn post_input_start(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn post_input_stop(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.input_stop_reported().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_input_tunnel(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.input_tunnel_status())
}

pub(super) async fn post_input_tunnel_attach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn post_input_tunnel_release(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn get_input_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.input_policy_report())
}

pub(super) async fn post_input_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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
        if let Some(ref km) = b.keymap {
            policy.keymap =
                if km.trim().is_empty() || km == "none" || km == "identity" || km == "off" {
                    None
                } else {
                    Some(km.trim().to_lowercase().replace('-', "_"))
                };
        }
        if let Some(ref edge_val) = b.edge {
            policy.edge =
                if edge_val.trim().is_empty() || edge_val == "none" || edge_val == "default" {
                    None
                } else if let Ok(e) = edge_val
                    .trim()
                    .to_lowercase()
                    .parse::<tetherly_core::ScreenEdge>()
                {
                    Some(e.to_string())
                } else {
                    return err(
                        400,
                        &format!("invalid edge '{edge_val}' (left|right|top|bottom)"),
                    );
                };
        }
        if let Some(ws) = b.wheel_speed {
            if !(ws > 0.0 && ws.is_finite() && ws <= 20.0) {
                return err(400, "wheel_speed must be within (0, 20]");
            }
            policy.wheel_speed = ws;
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
    if let Some(km) = qmap.get("keymap").or_else(|| qmap.get("key_map")) {
        policy.keymap = if km.trim().is_empty() || km == "none" || km == "identity" || km == "off" {
            None
        } else {
            Some(km.trim().to_lowercase().replace('-', "_"))
        };
    }
    if let Some(edge_str) = qmap.get("edge").or_else(|| qmap.get("screen_edge")) {
        policy.edge = if edge_str.trim().is_empty() || edge_str == "none" || edge_str == "default" {
            None
        } else if let Ok(e) = edge_str
            .trim()
            .to_lowercase()
            .parse::<tetherly_core::ScreenEdge>()
        {
            Some(e.to_string())
        } else {
            return err(
                400,
                &format!("invalid edge '{edge_str}' (left|right|top|bottom)"),
            );
        };
    }
    if let Some(ws_str) = qmap.get("wheel_speed").or_else(|| qmap.get("scroll_speed")) {
        match ws_str.parse::<f64>() {
            Ok(ws) if ws > 0.0 && ws.is_finite() && ws <= 20.0 => policy.wheel_speed = ws,
            _ => return err(400, "wheel_speed must be within (0, 20]"),
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

pub(super) async fn post_input_accessibility_open(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    _node: &Node,
) -> UiResponse {
    match crate::platform::open_accessibility_settings() {
        Ok(()) => json_ok(&serde_json::json!({
            "ok": true,
            "message": "opened system accessibility settings"
        })),
        Err(e) => err(500, &e.to_string()),
    }
}

pub(super) async fn post_input_ensure(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let trusted = crate::platform::is_accessibility_trusted();
    if !trusted {
        return json_ok(&serde_json::json!({
            "ok": false,
            "accessibility_trusted": false,
            "capture_active": false,
            "message": "macOS Accessibility permission not granted yet"
        }));
    }
    match node.ensure_input_capture() {
        Ok(active) => json_ok(&serde_json::json!({
            "ok": true,
            "accessibility_trusted": true,
            "capture_active": active,
            "message": if active { "input capture hooks active" } else { "input capture hooks not active" }
        })),
        Err(e) => err(500, &e.to_string()),
    }
}
