// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer focus (DND) state and focus policy.

use super::*;

pub(super) async fn get_focus_state(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.focus_status())
}

pub(super) async fn post_focus_state(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn get_focus_peers(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.peer_focus_states())
}

pub(super) async fn post_focus_sync(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.request_peers_focus_sync().await {
        Ok(sent) => json_ok(&serde_json::json!({
            "ok": true,
            "sync_requests_sent": sent,
            "message": "focus sync request dispatched to peers"
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_focus_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.focus_policy_report())
}

pub(super) async fn post_focus_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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
                Err(e) => return err(400, &format!("invalid default focus mode '{m_str}': {e}")),
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
            Err(e) => return err(400, &format!("invalid default focus mode '{m_str}': {e}")),
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
