// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Virtual display attach/detach and display policy.

use super::*;

pub(super) async fn get_display_virtual(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.virtual_display_status())
}

pub(super) async fn post_display_virtual_attach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<VirtualDisplayAttachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<VirtualDisplayAttachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected VirtualDisplayAttachBody"),
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

    let width = parsed_body
        .as_ref()
        .and_then(|b| b.width)
        .or_else(|| qmap.get("width").and_then(|s| s.parse::<u32>().ok()));

    let height = parsed_body
        .as_ref()
        .and_then(|b| b.height)
        .or_else(|| qmap.get("height").and_then(|s| s.parse::<u32>().ok()));

    let fps = parsed_body.as_ref().and_then(|b| b.fps).or_else(|| {
        qmap.get("fps")
            .or_else(|| qmap.get("target_fps"))
            .and_then(|s| s.parse::<u32>().ok())
    });

    match node
        .virtual_display_attach(&peer, edge, width, height, fps)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_display_virtual_detach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<VirtualDisplayDetachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<VirtualDisplayDetachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected VirtualDisplayDetachBody"),
        }
    };

    let reason = parsed_body
        .as_ref()
        .and_then(|b| b.reason.clone())
        .or_else(|| qmap.get("reason").cloned());

    match node.virtual_display_detach(reason.as_deref()).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_display_virtual_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.virtual_display_policy_report())
}

pub(super) async fn post_display_virtual_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<VirtualDisplayPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<VirtualDisplayPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected VirtualDisplayPolicyBody"),
        }
    };

    let mut policy = node.virtual_display_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled {
            policy.enabled = en;
        }
        if let Some(hosting) = b.allow_hosting {
            policy.allow_hosting = hosting;
        }
        if let Some(extend) = b.allow_remote_extend {
            policy.allow_remote_extend = extend;
        }
        if let Some(w) = b.max_resolution_width {
            policy.max_resolution_width = w;
        }
        if let Some(h) = b.max_resolution_height {
            policy.max_resolution_height = h;
        }
        if let Some(fps) = b.max_fps {
            policy.max_fps = fps;
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
    if let Some(h) = qmap
        .get("allow_host")
        .or_else(|| qmap.get("allow_hosting"))
        .or_else(|| qmap.get("hosting"))
    {
        if let Ok(b) = h.parse::<bool>() {
            policy.allow_hosting = b;
        }
    }
    if let Some(ext) = qmap
        .get("allow_extend")
        .or_else(|| qmap.get("allow_remote_extend"))
        .or_else(|| qmap.get("remote_extend"))
    {
        if let Ok(b) = ext.parse::<bool>() {
            policy.allow_remote_extend = b;
        }
    }
    if let Some(mw) = qmap
        .get("max_width")
        .or_else(|| qmap.get("max_resolution_width"))
    {
        if let Ok(w) = mw.parse::<u32>() {
            policy.max_resolution_width = w;
        }
    }
    if let Some(mh) = qmap
        .get("max_height")
        .or_else(|| qmap.get("max_resolution_height"))
    {
        if let Ok(h) = mh.parse::<u32>() {
            policy.max_resolution_height = h;
        }
    }
    if let Some(fps) = qmap.get("max_fps").or_else(|| qmap.get("fps")) {
        if let Ok(f) = fps.parse::<u32>() {
            policy.max_fps = f;
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
    if qmap
        .get("clear_filters")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false)
    {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }

    match node.set_virtual_display_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
