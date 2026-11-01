// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Topology show/link/unlink/config/peers/sync/policy/watch.

use super::util::{loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli topology` -- Topology arm of the top-level dispatch.
pub(crate) async fn topology(action: TopologyCmd) -> Result<()> {
    match action {
        TopologyCmd::Show { json, ui_port } => {
            do_topology_show(ui_port, json).await?;
        }
        TopologyCmd::Link {
            edge,
            peer,
            peer_edge,
            offset,
            json,
            ui_port,
        } => {
            do_topology_link(ui_port, &edge, &peer, peer_edge.as_deref(), offset, json).await?;
        }
        TopologyCmd::Unlink {
            edge,
            json,
            ui_port,
        } => {
            do_topology_unlink(ui_port, &edge, json).await?;
        }
        TopologyCmd::Config {
            resistance,
            delay,
            corner_lock,
            wrap,
            width,
            height,
            scale,
            json,
            ui_port,
        } => {
            do_topology_config(
                ui_port,
                resistance,
                delay,
                corner_lock,
                wrap,
                width,
                height,
                scale,
                json,
            )
            .await?;
        }
        TopologyCmd::Peers { json, ui_port } => {
            do_topology_peers(ui_port, json).await?;
        }
        TopologyCmd::Sync { json, ui_port } => {
            do_topology_sync(ui_port, json).await?;
        }
        TopologyCmd::Policy {
            action,
            enable,
            allow_inbound,
            auto_link,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_topology_policy(
                ui_port,
                action,
                enable,
                allow_inbound,
                auto_link,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        TopologyCmd::Watch { json, ui_port } => {
            do_topology_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_topology_show(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/topology/layout").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to get topology layout (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }

    let val: serde_json::Value = serde_json::from_slice(&body)?;
    let display = &val["display"];
    let width = display["width"].as_u64().unwrap_or(0);
    let height = display["height"].as_u64().unwrap_or(0);
    let scale = display["scale_factor"].as_f64().unwrap_or(1.0);

    let cfg = &val["config"];
    let resistance = cfg["resistance_px"].as_u64().unwrap_or(0);
    let delay = cfg["delay_ms"].as_u64().unwrap_or(0);
    let corner_lock = cfg["corner_lock_size_px"].as_u64().unwrap_or(0);
    let wrap = cfg["wrap_around"].as_bool().unwrap_or(false);

    println!("Multi-Display Spatial Topology (127.0.0.1:{ui_port})");
    println!(
        "  Display Geometry: {}x{} @ {:.2}x scale",
        width, height, scale
    );
    println!("  Crossing Guardrails:");
    println!("    Edge Resistance: {} px", resistance);
    println!("    Dwell Delay:     {} ms", delay);
    println!("    Corner Lock:     {} px", corner_lock);
    println!("    Wrap Around:     {}", if wrap { "yes" } else { "no" });

    println!("  Edge Neighbors:");
    let neighbors = val["neighbors"].as_object();
    let mut count = 0;
    for edge in &["left", "right", "top", "bottom"] {
        if let Some(n) = neighbors
            .and_then(|m| m.get(*edge))
            .filter(|v| !v.is_null())
        {
            count += 1;
            let peer_id = n["peer_id"].as_str().unwrap_or("unknown");
            let peer_edge = n["peer_edge"].as_str().unwrap_or("unknown");
            let offset = n["offset"].as_i64().unwrap_or(0);
            println!(
                "    - {:<6} -> peer {} (enters {:?}, offset {:+}px)",
                edge.to_uppercase(),
                peer_id,
                peer_edge,
                offset
            );
        }
    }
    if count == 0 {
        println!(
            "    (no edge neighbors configured; use `tetherly topology link` to attach screens)"
        );
    }
    Ok(())
}

pub(crate) async fn do_topology_link(
    ui_port: u16,
    edge: &str,
    peer: &str,
    peer_edge: Option<&str>,
    offset: Option<i32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "edge": edge,
        "peer": peer,
    });
    if let Some(pe) = peer_edge {
        req["peer_edge"] = serde_json::json!(pe);
    }
    if let Some(off) = offset {
        req["offset"] = serde_json::json!(off);
    }
    let (code, body) =
        loopback_post(ui_port, "/api/topology/link", &serde_json::to_vec(&req)?).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to link topology neighbor (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let val: serde_json::Value = serde_json::from_slice(&body)?;
        let e = val["edge"].as_str().unwrap_or(edge);
        let pid = val["peer_id"].as_str().unwrap_or(peer);
        let pe = val["peer_edge"].as_str().unwrap_or("opposite");
        let off = val["offset"].as_i64().unwrap_or(0);
        println!(
            "Successfully linked local edge '{}' to peer '{}' (enters '{}', offset {:+}px).",
            e, pid, pe, off
        );
    }
    Ok(())
}

pub(crate) async fn do_topology_unlink(ui_port: u16, edge: &str, json: bool) -> Result<()> {
    let req = serde_json::json!({ "edge": edge });
    let (code, body) =
        loopback_post(ui_port, "/api/topology/unlink", &serde_json::to_vec(&req)?).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to unlink topology neighbor (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        println!("Successfully unlinked neighbor from local edge '{}'.", edge);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_topology_config(
    ui_port: u16,
    resistance: Option<u32>,
    delay: Option<u32>,
    corner_lock: Option<u32>,
    wrap: Option<bool>,
    width: Option<u32>,
    height: Option<u32>,
    scale: Option<f32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::Map::new();
    if let Some(r) = resistance {
        req.insert("resistance_px".into(), serde_json::json!(r));
    }
    if let Some(d) = delay {
        req.insert("delay_ms".into(), serde_json::json!(d));
    }
    if let Some(c) = corner_lock {
        req.insert("corner_lock_size_px".into(), serde_json::json!(c));
    }
    if let Some(w) = wrap {
        req.insert("wrap_around".into(), serde_json::json!(w));
    }
    if let Some(w) = width {
        req.insert("width".into(), serde_json::json!(w));
    }
    if let Some(h) = height {
        req.insert("height".into(), serde_json::json!(h));
    }
    if let Some(s) = scale {
        req.insert("scale_factor".into(), serde_json::json!(s));
    }

    if req.is_empty() {
        return do_topology_show(ui_port, json).await;
    }

    let (code, body) =
        loopback_post(ui_port, "/api/topology/config", &serde_json::to_vec(&req)?).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to update topology config (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        println!("Successfully updated topology geometry and crossing parameters.");
        let val: serde_json::Value = serde_json::from_slice(&body)?;
        let display = &val["display"];
        let cfg = &val["config"];
        println!(
            "  Display: {}x{} @ {:.2}x scale",
            display["width"].as_u64().unwrap_or(0),
            display["height"].as_u64().unwrap_or(0),
            display["scale_factor"].as_f64().unwrap_or(1.0)
        );
        println!(
            "  Resistance: {}px, Delay: {}ms, Corner Lock: {}px, Wrap: {}",
            cfg["resistance_px"].as_u64().unwrap_or(0),
            cfg["delay_ms"].as_u64().unwrap_or(0),
            cfg["corner_lock_size_px"].as_u64().unwrap_or(0),
            cfg["wrap_around"].as_bool().unwrap_or(false)
        );
    }
    Ok(())
}

pub(crate) async fn do_topology_peers(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/topology/peers").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query peer topologies (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }

    let val: serde_json::Value = serde_json::from_slice(&body)?;
    let peers = val["peers"].as_array();
    println!("Known Peer Display Topologies (127.0.0.1:{ui_port})");
    let mut count = 0;
    if let Some(list) = peers {
        for p in list {
            count += 1;
            let pid = p["peer_id"].as_str().unwrap_or("unknown");
            let alias = p["peer_alias"].as_str().unwrap_or(pid);
            let display = &p["display"];
            let w = display["width"].as_u64().unwrap_or(0);
            let h = display["height"].as_u64().unwrap_or(0);
            let s = display["scale_factor"].as_f64().unwrap_or(1.0);
            let age_ms = p["age_ms"].as_u64().unwrap_or(0);
            println!(
                "  - {} ({}): {}x{} @ {:.2}x scale, updated {}s ago",
                alias,
                pid,
                w,
                h,
                s,
                age_ms / 1000
            );
        }
    }
    if count == 0 {
        println!("  (no peer topologies recorded yet; run `tetherly topology sync` to refresh)");
    }
    Ok(())
}

pub(crate) async fn do_topology_sync(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_post(ui_port, "/api/topology/sync", b"{}").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to dispatch topology sync (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let val: serde_json::Value = serde_json::from_slice(&body)?;
    let req_sent = val["sync_requests_sent"].as_u64().unwrap_or(0);
    let bc_sent = val["broadcasts_sent"].as_u64().unwrap_or(0);
    println!(
        "Dispatched topology sync to {req_sent} peer(s), broadcast local state to {bc_sent} peer(s)."
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_topology_policy(
    ui_port: u16,
    action: Option<TopologyPolicySubCmd>,
    enable: Option<bool>,
    allow_inbound: Option<bool>,
    auto_link: Option<bool>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = match &action {
        Some(TopologyPolicySubCmd::Set { .. }) => true,
        Some(TopologyPolicySubCmd::Show { .. }) => false,
        None => {
            enable.is_some()
                || allow_inbound.is_some()
                || auto_link.is_some()
                || group.is_some()
                || tag.is_some()
                || device_type.is_some()
                || clear_filters
        }
    };

    if is_mutation {
        let mut req = serde_json::Map::new();
        match action {
            Some(TopologyPolicySubCmd::Set {
                enable: e,
                allow_inbound: ai,
                auto_link: al,
                group: g,
                tag: t,
                device_type: dt,
                clear_filters: cf,
                ..
            }) => {
                if let Some(v) = e {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = ai {
                    req.insert("allow_inbound_control".into(), serde_json::json!(v));
                }
                if let Some(v) = al {
                    req.insert("auto_link_paired_peers".into(), serde_json::json!(v));
                }
                if let Some(v) = g {
                    req.insert("allowed_groups".into(), serde_json::json!(v));
                }
                if let Some(v) = t {
                    req.insert("allowed_tags".into(), serde_json::json!(v));
                }
                if let Some(v) = dt {
                    req.insert("allowed_types".into(), serde_json::json!(v));
                }
                if cf {
                    req.insert("clear_filters".into(), serde_json::json!(true));
                }
            }
            _ => {
                if let Some(v) = enable {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = allow_inbound {
                    req.insert("allow_inbound_control".into(), serde_json::json!(v));
                }
                if let Some(v) = auto_link {
                    req.insert("auto_link_paired_peers".into(), serde_json::json!(v));
                }
                if let Some(v) = group {
                    req.insert("allowed_groups".into(), serde_json::json!(v));
                }
                if let Some(v) = tag {
                    req.insert("allowed_tags".into(), serde_json::json!(v));
                }
                if let Some(v) = device_type {
                    req.insert("allowed_types".into(), serde_json::json!(v));
                }
                if clear_filters {
                    req.insert("clear_filters".into(), serde_json::json!(true));
                }
            }
        }

        let body_bytes = serde_json::to_vec(&req)?;
        let (code, body) = loopback_post(ui_port, "/api/topology/policy", &body_bytes).await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update topology policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        println!("Successfully updated topology governance policy.");
        print_topology_policy_json(&body)?;
    } else {
        let (code, body) = loopback_get(ui_port, "/api/topology/policy").await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to query topology policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        print_topology_policy_json(&body)?;
    }
    Ok(())
}

pub(crate) fn print_topology_policy_json(body: &[u8]) -> Result<()> {
    let val: serde_json::Value = serde_json::from_slice(body)?;
    let pol = if val.get("policy").is_some() {
        &val["policy"]
    } else {
        &val
    };

    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_inbound = pol["allow_inbound_control"].as_bool().unwrap_or(true);
    let auto_link = pol["auto_link_paired_peers"].as_bool().unwrap_or(true);
    let groups = pol["allowed_groups"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags = pol["allowed_tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let types = pol["allowed_types"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    println!("Spatial Topology Governance Policy:");
    println!(
        "  Enabled:                 {}",
        if enabled { "yes" } else { "no" }
    );
    println!(
        "  Allow Inbound Control:   {}",
        if allow_inbound { "yes" } else { "no" }
    );
    println!(
        "  Auto-Link Paired Peers:  {}",
        if auto_link { "yes" } else { "no" }
    );
    println!(
        "  Allowed Groups:          {}",
        if groups.is_empty() {
            "any".to_string()
        } else {
            groups.join(", ")
        }
    );
    println!(
        "  Allowed Tags:            {}",
        if tags.is_empty() {
            "any".to_string()
        } else {
            tags.join(", ")
        }
    );
    println!(
        "  Allowed Device Types:    {}",
        if types.is_empty() {
            "any".to_string()
        } else {
            types.join(", ")
        }
    );
    Ok(())
}

pub(crate) async fn do_topology_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
    })?;
    if !json {
        println!("Streaming live spatial topology events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "topology_changed"
                || kind == "peer_topology_updated"
                || kind == "cursor_edge_crossed"
                || kind == "topology_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "topology_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let count = v["neighbors_count"].as_u64().unwrap_or(0);
                            println!("[topology_changed] enabled={en} neighbors_count={count}");
                        }
                        "peer_topology_updated" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            let w = v["width"].as_u64().unwrap_or(0);
                            let h = v["height"].as_u64().unwrap_or(0);
                            let s = v["scale_factor"].as_f64().unwrap_or(1.0);
                            println!("[peer_topology_updated] peer={peer} ({pid}) geometry={w}x{h}@{s:.2}x");
                        }
                        "cursor_edge_crossed" => {
                            let from = v["from_device"].as_str().unwrap_or("local");
                            let to = v["to_device"].as_str().unwrap_or("peer");
                            let edge = v["edge"].as_str().unwrap_or("edge");
                            let x = v["x"].as_i64().unwrap_or(0);
                            let y = v["y"].as_i64().unwrap_or(0);
                            println!("[cursor_edge_crossed] {from} -> {to} across {edge} edge to entry ({x}, {y})");
                        }
                        "topology_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let inbound = v["allow_inbound_control"].as_bool().unwrap_or(true);
                            let auto = v["auto_link_paired_peers"].as_bool().unwrap_or(true);
                            println!("[topology_policy_changed] enabled={en} allow_inbound={inbound} auto_link={auto}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
