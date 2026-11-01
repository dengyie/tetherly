// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Audio routing, spatial position, volumes and audio policy.

use super::*;

pub(super) async fn get_audio_routing(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.audio_routing_status())
}

pub(super) async fn post_audio_routing_attach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AudioRoutingAttachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<AudioRoutingAttachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected AudioRoutingAttachBody"),
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

    let layout = parsed_body
        .as_ref()
        .and_then(|b| b.layout.as_deref())
        .or_else(|| qmap.get("layout").map(String::as_str))
        .map(AudioChannelLayout::from_str_lenient);

    let sample_rate = parsed_body
        .as_ref()
        .and_then(|b| b.sample_rate)
        .or_else(|| qmap.get("sample_rate").and_then(|s| s.parse::<u32>().ok()));

    let channels = parsed_body
        .as_ref()
        .and_then(|b| b.channels)
        .or_else(|| qmap.get("channels").and_then(|s| s.parse::<u8>().ok()));

    let spatial_x = parsed_body
        .as_ref()
        .and_then(|b| b.spatial_x.or_else(|| b.spatial_position.and_then(|p| p.x)))
        .or_else(|| {
            qmap.get("x")
                .or_else(|| qmap.get("spatial_x"))
                .and_then(|s| s.parse::<f32>().ok())
        });

    let spatial_y = parsed_body
        .as_ref()
        .and_then(|b| b.spatial_y.or_else(|| b.spatial_position.and_then(|p| p.y)))
        .or_else(|| {
            qmap.get("y")
                .or_else(|| qmap.get("spatial_y"))
                .and_then(|s| s.parse::<f32>().ok())
        });

    let spatial_z = parsed_body
        .as_ref()
        .and_then(|b| b.spatial_z.or_else(|| b.spatial_position.and_then(|p| p.z)))
        .or_else(|| {
            qmap.get("z")
                .or_else(|| qmap.get("spatial_z"))
                .and_then(|s| s.parse::<f32>().ok())
        });

    let volume_gain = parsed_body
        .as_ref()
        .and_then(|b| b.volume_gain)
        .or_else(|| {
            qmap.get("gain")
                .or_else(|| qmap.get("volume"))
                .or_else(|| qmap.get("volume_gain"))
                .and_then(|s| s.parse::<f32>().ok())
        });

    match node
        .audio_routing_attach(
            &peer,
            layout,
            sample_rate,
            channels,
            spatial_x,
            spatial_y,
            spatial_z,
            volume_gain,
        )
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_audio_routing_detach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AudioRoutingDetachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<AudioRoutingDetachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected AudioRoutingDetachBody"),
        }
    };

    let reason = parsed_body
        .as_ref()
        .and_then(|b| b.reason.clone())
        .or_else(|| qmap.get("reason").cloned());

    match node.audio_routing_detach(reason.as_deref()).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_audio_routing_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.audio_routing_policy_report())
}

pub(super) async fn post_audio_routing_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AudioRoutingPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<AudioRoutingPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected AudioRoutingPolicyBody"),
        }
    };

    let mut policy = node.audio_routing_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled {
            policy.enabled = en;
        }
        if let Some(sink) = b.allow_inbound_sink {
            policy.allow_inbound_sink = sink;
        }
        if let Some(outbound) = b.allow_outbound_routing {
            policy.allow_outbound_routing = outbound;
        }
        if let Some(max_vol) = b.max_volume_limit {
            policy.max_volume_limit = max_vol.min(100);
        }
        if let Some(spatial) = b.allow_spatial_rendering {
            policy.allow_spatial_rendering = spatial;
        }
        if let Some(timeout) = b.idle_timeout_secs {
            policy.idle_timeout_secs = timeout;
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
    if let Some(sink) = qmap
        .get("allow_inbound_sink")
        .or_else(|| qmap.get("allow_sink"))
        .or_else(|| qmap.get("sink"))
    {
        if let Ok(b) = sink.parse::<bool>() {
            policy.allow_inbound_sink = b;
        }
    }
    if let Some(outbound) = qmap
        .get("allow_outbound_routing")
        .or_else(|| qmap.get("allow_outbound"))
        .or_else(|| qmap.get("routing"))
    {
        if let Ok(b) = outbound.parse::<bool>() {
            policy.allow_outbound_routing = b;
        }
    }
    if let Some(mv) = qmap
        .get("max_volume_limit")
        .or_else(|| qmap.get("max_volume"))
        .or_else(|| qmap.get("volume_limit"))
    {
        if let Ok(v) = mv.parse::<u8>() {
            policy.max_volume_limit = v.min(100);
        }
    }
    if let Some(sp) = qmap
        .get("allow_spatial_rendering")
        .or_else(|| qmap.get("allow_spatial"))
        .or_else(|| qmap.get("spatial"))
    {
        if let Ok(b) = sp.parse::<bool>() {
            policy.allow_spatial_rendering = b;
        }
    }
    if let Some(to) = qmap
        .get("idle_timeout_secs")
        .or_else(|| qmap.get("timeout"))
        .or_else(|| qmap.get("idle_timeout"))
    {
        if let Ok(v) = to.parse::<u32>() {
            policy.idle_timeout_secs = v;
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

    match node.set_audio_routing_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
