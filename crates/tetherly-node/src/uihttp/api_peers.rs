// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer lifecycle, pairing, blocking, tags, groups, heartbeats, discovery and link probes.

use super::*;

pub(super) async fn post_peers_unpair(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<UnpairBody>(body) {
        Ok(d) => match node.resolve_peer(&d.device_id) {
            Ok(id) => {
                let removed = node.unpair(&id).await;
                json_ok(&serde_json::json!({
                    "ok": true,
                    "device_id": id.as_str(),
                    "unpaired": removed
                }))
            }
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_peers_connect(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerConnectBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<PeerConnectBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { peer: string, addr?: string }"),
        }
    };
    let target_query = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .unwrap_or_default();
    let direct_addr = parsed_body
        .as_ref()
        .and_then(|b| b.addr.clone())
        .or_else(|| qmap.get("addr").cloned());

    if target_query.trim().is_empty() {
        return err(400, "peer identifier required (peer or device_id)");
    }
    match node
        .connect_peer(target_query.trim(), direct_addr.as_deref())
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_peers_disconnect(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerDisconnectBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<PeerDisconnectBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { peer: string }"),
        }
    };
    let target_query = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .unwrap_or_default();

    if target_query.trim().is_empty() {
        return err(400, "peer identifier required (peer or device_id)");
    }
    match node.disconnect_peer(target_query.trim()).await {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_peers_blocked(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.blocked_peers_report())
}

pub(super) async fn post_peers_block(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerBlockBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<PeerBlockBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { peer: string, reason?: string }"),
        }
    };
    let target_query = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .unwrap_or_default();
    let reason = parsed_body
        .as_ref()
        .and_then(|b| b.reason.clone())
        .or_else(|| qmap.get("reason").cloned());

    if target_query.trim().is_empty() {
        return err(400, "peer identifier required");
    }
    match node
        .block_peer(target_query.trim(), reason.as_deref())
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_peers_unblock(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerUnblockBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<PeerUnblockBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { peer: string }"),
        }
    };
    let target_query = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .unwrap_or_default();

    if target_query.trim().is_empty() {
        return err(400, "peer identifier required");
    }
    match node.unblock_peer(target_query.trim()).await {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_peers_alias(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<PeerAliasBody>(body) {
        Ok(d) => {
            let target_query = d.device_id.or(d.peer).unwrap_or_default();
            if target_query.trim().is_empty() {
                return err(400, "missing device_id or peer query");
            }
            let id = match node.resolve_peer(&target_query) {
                Ok(id) => id,
                Err(tetherly_core::CoreError::PeerNotFound(_)) => {
                    return err(404, "peer not found in trust store");
                }
                Err(e) => return err(400, &e.to_string()),
            };
            match node.set_peer_alias(&id, &d.alias) {
                Ok(true) => json_ok(&serde_json::json!({
                    "ok": true,
                    "device_id": id.as_str(),
                    "alias": d.alias.trim()
                })),
                Ok(false) => err(404, "peer not found in trust store"),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_peers_ping(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let d: PeerTargetBody = if body.is_empty() {
        PeerTargetBody::default()
    } else {
        match serde_json::from_slice::<PeerTargetBody>(body) {
            Ok(v) => v,
            Err(_) => return err(400, "bad json"),
        }
    };
    let mut peers = node.live_peers();
    peers.sort_by_key(|p| p.device_id.to_string());
    let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
        Some(raw) => match node.resolve_peer(raw) {
            Ok(id) => {
                if peers.iter().any(|p| p.device_id == id) {
                    id
                } else {
                    return err(400, "peer is not connected");
                }
            }
            Err(e) => return err(400, &format!("bad peer id: {e}")),
        },
        None => match peers.len() {
            0 => return err(400, "no connected peer"),
            1 => peers[0].device_id.clone(),
            _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
        },
    };
    match node.ping_peer(&target).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_peers_link(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.link_quality_report())
}

pub(super) async fn post_peers_link_probe(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerLinkProbeBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<PeerLinkProbeBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json"),
        }
    };
    let target_query = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .unwrap_or_default();
    let count = parsed_body
        .as_ref()
        .and_then(|b| b.count)
        .or_else(|| qmap.get("count").and_then(|s| s.parse().ok()))
        .unwrap_or(5);
    let interval_ms = parsed_body
        .as_ref()
        .and_then(|b| b.interval_ms)
        .or_else(|| qmap.get("interval_ms").and_then(|s| s.parse().ok()))
        .unwrap_or(100);

    let mut peers = node.live_peers();
    peers.sort_by_key(|p| p.device_id.to_string());
    let target = match target_query.as_str().trim() {
        s if !s.is_empty() => match node.resolve_peer(s) {
            Ok(id) => {
                if peers.iter().any(|p| p.device_id == id) {
                    id
                } else {
                    return err(400, "peer is not connected");
                }
            }
            Err(e) => return err(400, &format!("bad peer id: {e}")),
        },
        _ => match peers.len() {
            0 => return err(400, "no connected peer"),
            1 => peers[0].device_id.clone(),
            _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
        },
    };
    match node
        .probe_peer_link(target.as_str(), count, interval_ms)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_peers_heartbeat(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.heartbeat_config())
}

pub(super) async fn post_peers_heartbeat(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<HeartbeatConfigBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<HeartbeatConfigBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json"),
        }
    };
    let interval_secs = parsed_body
        .as_ref()
        .and_then(|b| b.interval_secs)
        .or_else(|| qmap.get("interval_secs").and_then(|s| s.parse().ok()));
    let deadline_secs = parsed_body
        .as_ref()
        .and_then(|b| b.deadline_secs)
        .or_else(|| qmap.get("deadline_secs").and_then(|s| s.parse().ok()));
    if interval_secs.is_none() && deadline_secs.is_none() {
        return err(400, "interval_secs or deadline_secs required");
    }
    match node.update_heartbeat_config(interval_secs, deadline_secs) {
        Ok(cfg) => json_ok(&cfg),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_peers(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let live = node.live_peers();
    let trusted = node.trusted();
    let live_ids: std::collections::HashSet<tetherly_core::DeviceId> =
        live.iter().map(|p| p.device_id.clone()).collect();
    let trusted_map: std::collections::HashMap<tetherly_core::DeviceId, String> = trusted
        .iter()
        .map(|tp| (tp.device_id.clone(), tp.alias.clone()))
        .collect();

    let live_dtos: Vec<serde_json::Value> = live
        .into_iter()
        .map(|p| {
            let path_str = match p.path {
                tetherly_core::PathKind::Lan => "lan",
                tetherly_core::PathKind::Overlay => "overlay",
            };
            let alias_opt = trusted_map.get(&p.device_id).cloned();
            let features = crate::runtime::extract_features(&p.caps);
            let profile = node.peer_profile(&p.device_id);
            serde_json::json!({
                "device_id": p.device_id.to_string(),
                "name": p.name,
                "alias": alias_opt,
                "platform": p.platform,
                "device_type": profile.device_type.as_str(),
                "tags": profile.tags,
                "group": profile.group,
                "addr": p.addr.to_string(),
                "path": path_str,
                "file_port": p.file_port,
                "input_port": p.input_port,
                "screen_port": p.screen_port,
                "caps": p.caps,
                "features": features,
                "online": true,
                "rtt_ms": p.rtt_ms,
                "last_seen_ms": p.last_seen_ms,
            })
        })
        .collect();

    let trusted_dtos: Vec<serde_json::Value> = trusted
        .into_iter()
        .map(|tp| {
            let online = live_ids.contains(&tp.device_id);
            let profile = node.peer_profile(&tp.device_id);
            serde_json::json!({
                "device_id": tp.device_id.to_string(),
                "alias": tp.alias,
                "device_type": profile.device_type.as_str(),
                "tags": profile.tags,
                "group": profile.group,
                "paired_at": tp.paired_at,
                "revoked": tp.revoked,
                "online": online,
            })
        })
        .collect();

    json_ok(&serde_json::json!({
        "ok": true,
        "self_id": node.identity().device_id().to_string(),
        "live_count": live_dtos.len(),
        "trusted_count": trusted_dtos.len(),
        "live": live_dtos,
        "trusted": trusted_dtos,
    }))
}

pub(super) async fn get_peers_profiles(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let tag = qmap.get("tag").map(|s| s.as_str());
    let group = qmap.get("group").map(|s| s.as_str());
    let dev_type = qmap
        .get("type")
        .and_then(|t| t.parse::<crate::store::DeviceType>().ok());
    json_ok(&node.peer_profiles(tag, group, dev_type))
}

pub(super) async fn get_peers_profile(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let target = qmap
        .get("peer")
        .or_else(|| qmap.get("id"))
        .or_else(|| qmap.get("device_id"))
        .map(|s| s.as_str())
        .unwrap_or_default();
    if target.trim().is_empty() {
        return err(400, "peer parameter required");
    }
    match node.resolve_peer(target.trim()) {
        Ok(id) => json_ok(&node.peer_profile(&id)),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_peers_profile(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<ProfileUpdateBody>(body) {
        Ok(d) => {
            let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
            let target = match target {
                Some(t) => t,
                None => return err(400, "missing peer or device_id"),
            };
            let dev_type = match d.device_type {
                Some(ref s) => match s.parse::<crate::store::DeviceType>() {
                    Ok(dt) => Some(dt),
                    Err(e) => return err(400, &e),
                },
                None => None,
            };
            match node.update_peer_profile(&target, dev_type, d.tags, d.group, d.notes) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_peers_tags_add(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<PeerTagBody>(body) {
        Ok(d) => {
            let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
            let target = match target {
                Some(t) => t,
                None => return err(400, "missing peer or device_id"),
            };
            let tag = match d.tag.filter(|s| !s.trim().is_empty()) {
                Some(t) => t,
                None => return err(400, "tag required"),
            };
            match node.add_peer_tag(&target, &tag) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_peers_tags_remove(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PeerTagBody> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<PeerTagBody>(body).ok()
    };
    let target = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone().or_else(|| b.device_id.clone()))
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("device_id").cloned())
        .filter(|s| !s.trim().is_empty());
    let target = match target {
        Some(t) => t,
        None => return err(400, "missing peer or device_id"),
    };
    let tag = parsed_body
        .and_then(|b| b.tag)
        .or_else(|| qmap.get("tag").cloned())
        .filter(|s| !s.trim().is_empty());
    let tag = match tag {
        Some(t) => t,
        None => return err(400, "tag required"),
    };
    match node.remove_peer_tag(&target, &tag) {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_peers_group(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<PeerGroupBody>(body) {
        Ok(d) => {
            let target = d.peer.or(d.device_id).filter(|s| !s.trim().is_empty());
            let target = match target {
                Some(t) => t,
                None => return err(400, "missing peer or device_id"),
            };
            match node.set_peer_group(&target, d.group.as_deref()) {
                Ok(rep) => json_ok(&rep),
                Err(tetherly_core::CoreError::PeerNotFound(e)) => {
                    err(404, &format!("peer not found: {e}"))
                }
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn get_discovery(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.discovered_peers())
}

pub(super) async fn post_discovery_rescan(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.rescan_discovery())
}

pub(super) async fn get_peers_caps(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let target_peer = qmap
        .get("peer")
        .or_else(|| qmap.get("device_id"))
        .map(|s| s.as_str());
    match node.peers_caps_report(target_peer) {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(e)) => err(404, &format!("peer not found: {e}")),
        Err(e) => err(400, &e.to_string()),
    }
}
