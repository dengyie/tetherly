// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Topology links, layout and topology policy.

use super::*;

pub(super) async fn get_topology_layout(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.topology_layout())
}

pub(super) async fn post_topology_link(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TopologyLinkBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<TopologyLinkBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected TopologyLinkBody"),
        }
    };

    let edge_str = parsed_body
        .as_ref()
        .and_then(|b| b.edge.clone())
        .or_else(|| qmap.get("edge").cloned());
    let edge = match edge_str {
        Some(s) => match s.parse::<DisplayEdge>() {
            Ok(e) => e,
            Err(e) => return err(400, &format!("invalid edge '{s}': {e}")),
        },
        None => return err(400, "missing 'edge' parameter"),
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

    let peer_edge_str = parsed_body
        .as_ref()
        .and_then(|b| b.peer_edge.clone())
        .or_else(|| qmap.get("peer_edge").cloned());
    let peer_edge = match peer_edge_str {
        Some(s) => match s.parse::<DisplayEdge>() {
            Ok(e) => Some(e),
            Err(e) => return err(400, &format!("invalid peer_edge '{s}': {e}")),
        },
        None => None,
    };

    let offset = parsed_body
        .as_ref()
        .and_then(|b| b.offset)
        .or_else(|| qmap.get("offset").and_then(|s| s.parse::<i32>().ok()));

    match node
        .topology_link_neighbor(edge, &peer, peer_edge, offset)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_topology_unlink(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TopologyUnlinkBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<TopologyUnlinkBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected TopologyUnlinkBody"),
        }
    };

    let edge_str = parsed_body
        .as_ref()
        .and_then(|b| b.edge.clone())
        .or_else(|| qmap.get("edge").cloned());
    let edge = match edge_str {
        Some(s) => match s.parse::<DisplayEdge>() {
            Ok(e) => e,
            Err(e) => return err(400, &format!("invalid edge '{s}': {e}")),
        },
        None => return err(400, "missing 'edge' parameter"),
    };

    match node.topology_unlink_neighbor(edge).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_topology_config(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TopologyConfigBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<TopologyConfigBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected TopologyConfigBody"),
        }
    };

    let resistance_px = parsed_body
        .as_ref()
        .and_then(|b| b.resistance_px)
        .or_else(|| {
            qmap.get("resistance_px")
                .or_else(|| qmap.get("resistance"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let delay_ms = parsed_body.as_ref().and_then(|b| b.delay_ms).or_else(|| {
        qmap.get("delay_ms")
            .or_else(|| qmap.get("delay"))
            .and_then(|s| s.parse::<u32>().ok())
    });

    let corner_lock_size_px = parsed_body
        .as_ref()
        .and_then(|b| b.corner_lock_size_px)
        .or_else(|| {
            qmap.get("corner_lock_size_px")
                .or_else(|| qmap.get("corner_lock"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let wrap_around = parsed_body
        .as_ref()
        .and_then(|b| b.wrap_around)
        .or_else(|| {
            qmap.get("wrap_around")
                .or_else(|| qmap.get("wrap"))
                .and_then(|s| s.parse::<bool>().ok())
        });

    let redundancy_band_px = parsed_body
        .as_ref()
        .and_then(|b| b.redundancy_band_px)
        .or_else(|| {
            qmap.get("redundancy_band_px")
                .or_else(|| qmap.get("redundancy_band"))
                .or_else(|| qmap.get("band"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let slow_dwell_ms = parsed_body
        .as_ref()
        .and_then(|b| b.slow_dwell_ms)
        .or_else(|| {
            qmap.get("slow_dwell_ms")
                .or_else(|| qmap.get("slow_dwell"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let fast_push_delta_px = parsed_body
        .as_ref()
        .and_then(|b| b.fast_push_delta_px)
        .or_else(|| {
            qmap.get("fast_push_delta_px")
                .or_else(|| qmap.get("fast_push_delta"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let entry_return_grace_ms = parsed_body
        .as_ref()
        .and_then(|b| b.entry_return_grace_ms)
        .or_else(|| {
            qmap.get("entry_return_grace_ms")
                .or_else(|| qmap.get("entry_return_grace"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let remote_speed_percent = parsed_body
        .as_ref()
        .and_then(|b| b.remote_speed_percent)
        .or_else(|| {
            qmap.get("remote_speed_percent")
                .or_else(|| qmap.get("remote_speed"))
                .or_else(|| qmap.get("speed"))
                .and_then(|s| s.parse::<u32>().ok())
        });

    let width = parsed_body
        .as_ref()
        .and_then(|b| b.width)
        .or_else(|| qmap.get("width").and_then(|s| s.parse::<u32>().ok()));

    let height = parsed_body
        .as_ref()
        .and_then(|b| b.height)
        .or_else(|| qmap.get("height").and_then(|s| s.parse::<u32>().ok()));

    let scale_factor = parsed_body
        .as_ref()
        .and_then(|b| b.scale_factor)
        .or_else(|| {
            qmap.get("scale_factor")
                .or_else(|| qmap.get("scale"))
                .and_then(|s| s.parse::<f32>().ok())
        });

    match node
        .topology_update_config(
            resistance_px,
            delay_ms,
            corner_lock_size_px,
            wrap_around,
            width,
            height,
            scale_factor,
            redundancy_band_px,
            slow_dwell_ms,
            fast_push_delta_px,
            entry_return_grace_ms,
            remote_speed_percent,
        )
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_topology_peers(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.topology_peers())
}

pub(super) async fn post_topology_sync(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let sync_res = node.topology_request_sync().await;
    let bc_res = node.topology_broadcast_state().await;
    match (sync_res, bc_res) {
        (Ok(sent), Ok(bc)) => json_ok(&serde_json::json!({
            "ok": true,
            "sync_requests_sent": sent,
            "broadcasts_sent": bc,
            "broadcast_count": bc,
            "message": "topology sync requested and local state broadcast"
        })),
        (Err(e), _) | (_, Err(e)) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_topology_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.topology_policy_report())
}

pub(super) async fn post_topology_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TopologyPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<TopologyPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected TopologyPolicyBody"),
        }
    };

    let mut policy = node.topology_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled {
            policy.enabled = en;
        }
        if let Some(inbound) = b.allow_inbound_control {
            policy.allow_inbound_control = inbound;
        }
        if let Some(auto) = b.auto_link_paired_peers {
            policy.auto_link_paired_peers = auto;
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

    if let Some(en) = qmap.get("enabled").or_else(|| qmap.get("enable")) {
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
    if let Some(auto) = qmap
        .get("auto_link")
        .or_else(|| qmap.get("auto_link_paired_peers"))
    {
        if let Ok(b) = auto.parse::<bool>() {
            policy.auto_link_paired_peers = b;
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

    match node.set_topology_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
