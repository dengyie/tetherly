// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Locate/find-my ringing and locate policy.

use super::*;

pub(super) async fn get_locate_status(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.locate_status())
}

pub(super) async fn post_locate_ring(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn post_locate_stop(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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

pub(super) async fn get_locate_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.locator_policy_report())
}

pub(super) async fn post_locate_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
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
