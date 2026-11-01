// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Benchmark runs, recommendation engine and bench policy.

use super::*;

pub(super) async fn get_bench(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.bench_status())
}

pub(super) async fn post_bench_start(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<BenchStartBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<BenchStartBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected BenchStartBody"),
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

    let direction = parsed_body
        .as_ref()
        .and_then(|b| b.direction.clone())
        .or_else(|| qmap.get("direction").cloned())
        .or_else(|| qmap.get("dir").cloned())
        .map(|d| tetherly_core::BenchDirection::from_str_lenient(&d))
        .unwrap_or(tetherly_core::BenchDirection::Push);

    let duration_secs = parsed_body
        .as_ref()
        .and_then(|b| b.duration_secs)
        .or_else(|| {
            qmap.get("duration_secs")
                .and_then(|s| s.parse::<u32>().ok())
        });

    let chunk_size = parsed_body
        .as_ref()
        .and_then(|b| b.chunk_size)
        .or_else(|| qmap.get("chunk_size").and_then(|s| s.parse::<u32>().ok()));

    match node
        .bench_start(&peer, direction, duration_secs, chunk_size)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_bench_stop(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.bench_stop().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_recommendation(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    if let Some(peer) = qmap
        .get("peer")
        .or_else(|| qmap.get("peer_id"))
        .or_else(|| qmap.get("target"))
        .filter(|p| !p.trim().is_empty())
    {
        match node.streaming_recommendation(peer) {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        json_ok(&node.recommendation_status())
    }
}

pub(super) async fn get_bench_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.bench_policy_report())
}

pub(super) async fn post_bench_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<BenchPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<BenchPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected BenchPolicyBody"),
        }
    };

    let mut policy = node.bench_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled {
            policy.enabled = en;
        }
        if let Some(inbound) = b.allow_inbound_bench {
            policy.allow_inbound_bench = inbound;
        }
        if let Some(d) = b.max_duration_secs {
            policy.max_duration_secs = d;
        }
        if let Some(bw) = b.max_bandwidth_mbps {
            policy.max_bandwidth_mbps = bw;
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
        .get("allow_inbound_bench")
        .or_else(|| qmap.get("allow_inbound"))
        .or_else(|| qmap.get("inbound"))
    {
        if let Ok(b) = inbound.parse::<bool>() {
            policy.allow_inbound_bench = b;
        }
    }
    if let Some(d_str) = qmap
        .get("max_duration_secs")
        .or_else(|| qmap.get("max_duration"))
    {
        if let Ok(d) = d_str.parse::<u32>() {
            policy.max_duration_secs = d;
        }
    }
    if let Some(bw_str) = qmap
        .get("max_bandwidth_mbps")
        .or_else(|| qmap.get("max_bandwidth"))
    {
        if let Ok(bw) = bw_str.parse::<u32>() {
            policy.max_bandwidth_mbps = bw;
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

    match node.set_bench_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
