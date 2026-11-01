// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer discovery, connect/disconnect/block, routes/net and link quality.

use super::util::format_uptime;
use super::*;

/// `tetherly unpair` -- Unpair arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn unpair(device_id: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "device_id": device_id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/unpair", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused unpair (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let unpaired = v.get("unpaired").and_then(|u| u.as_bool()).unwrap_or(false);
    if unpaired {
        println!("unpaired and severed connection with {device_id}");
    } else {
        println!("peer {device_id} was not in trusted peers list");
    }
    Ok(())
}

/// `tetherly peers` -- Peers arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn peers(
    trusted: bool,
    ping: bool,
    peer: Option<String>,
    count: u32,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    if ping {
        return do_peer_ping(ui_port, peer, count, json).await;
    }
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node returned HTTP {code}: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let self_id = v["self_id"].as_str().unwrap_or("unknown");
    let live_arr = v["live"].as_array();
    let live_count = v["live_count"].as_u64().unwrap_or(0);
    let trusted_count = v["trusted_count"].as_u64().unwrap_or(0);

    println!("Node: {self_id} (live: {live_count}, trusted: {trusted_count})");
    println!("\nActive Live Peers:");
    match live_arr {
        Some(arr) if !arr.is_empty() => {
            for p in arr {
                let dev = p["device_id"].as_str().unwrap_or("-");
                let name = p["name"].as_str().unwrap_or("-");
                let alias = p["alias"].as_str();
                let display_name = match alias {
                    Some(a) if !a.is_empty() && a != name => {
                        format!("{name} (alias: \"{a}\")")
                    }
                    _ => name.to_string(),
                };
                let platform = p["platform"].as_str().unwrap_or("-");
                let addr = p["addr"].as_str().unwrap_or("-");
                let path = p["path"].as_str().unwrap_or("-");
                let rtt_str = match p["rtt_ms"].as_u64() {
                    Some(rtt) => format!("{rtt}ms"),
                    None => "n/a".to_string(),
                };
                let feats_str = match p["features"].as_array() {
                    Some(farr) if !farr.is_empty() => {
                        let fstrs: Vec<&str> = farr.iter().filter_map(|x| x.as_str()).collect();
                        format!(" · caps: [{}]", fstrs.join(", "))
                    }
                    _ => String::new(),
                };
                let dev_type = p["device_type"].as_str().unwrap_or("other");
                let grp_str = match p["group"].as_str() {
                    Some(g) if !g.is_empty() => format!(" · group: \"{g}\""),
                    _ => String::new(),
                };
                let tags_str = match p["tags"].as_array() {
                    Some(tarr) if !tarr.is_empty() => {
                        let tstrs: Vec<&str> = tarr.iter().filter_map(|x| x.as_str()).collect();
                        format!(" · tags: [{}]", tstrs.join(", "))
                    }
                    _ => String::new(),
                };
                println!(
                    "  * {display_name} ({platform}/{dev_type}){grp_str}{tags_str} · {addr} [{path}] · rtt: {rtt_str}{feats_str} · {dev}"
                );
            }
        }
        _ => println!("  (no live peers connected)"),
    }

    if trusted {
        println!("\nTrusted Peers Inventory:");
        if let Some(arr) = v["trusted"].as_array() {
            if arr.is_empty() {
                println!("  (no trusted peers)");
            } else {
                for tp in arr {
                    let dev = tp["device_id"].as_str().unwrap_or("-");
                    let alias = tp["alias"].as_str().unwrap_or("-");
                    let online = tp["online"].as_bool().unwrap_or(false);
                    let status = if online { "ONLINE" } else { "OFFLINE" };
                    let dev_type = tp["device_type"].as_str().unwrap_or("other");
                    let grp_str = match tp["group"].as_str() {
                        Some(g) if !g.is_empty() => format!(" · group: \"{g}\""),
                        _ => String::new(),
                    };
                    let tags_str = match tp["tags"].as_array() {
                        Some(tarr) if !tarr.is_empty() => {
                            let tstrs: Vec<&str> = tarr.iter().filter_map(|x| x.as_str()).collect();
                            format!(" · tags: [{}]", tstrs.join(", "))
                        }
                        _ => String::new(),
                    };
                    println!("  * [{status}] {alias} ({dev_type}){grp_str}{tags_str} · {dev}");
                }
            }
        }
    }
    Ok(())
}

/// `tetherly alias` -- Alias arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn alias(peer: String, alias: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "alias": alias,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/alias", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to set peer alias (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
    let new_alias = v.get("alias").and_then(|s| s.as_str()).unwrap_or(&alias);
    println!("Updated alias for {device_id} -> \"{new_alias}\"");
    Ok(())
}

/// `tetherly routes` -- Routes arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn routes(json: bool, ui_port: u16) -> Result<()> {
    print_routes_list(ui_port, json).await?;
    Ok(())
}

/// `tetherly route` -- Route arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn route(action: RouteCmd) -> Result<()> {
    match action {
        RouteCmd::List { json, ui_port } => {
            print_routes_list(ui_port, json).await?;
        }
        RouteCmd::Add {
            peer,
            addr,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "peer": peer,
                "addr": addr,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/routes", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "route add failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&resp)?;
            println!(
                "Route {}: {} -> {}",
                rep.action,
                rep.device_id,
                rep.addr.unwrap_or_default()
            );
        }
        RouteCmd::Remove {
            peer,
            json,
            ui_port,
        } => {
            let path = format!("/api/routes?peer={}", peer.trim());
            let (code, resp) = tetherly_node::loopback_delete(ui_port, &path)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "route remove failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&resp)?;
            println!("Route {}: {}", rep.action, rep.device_id);
        }
    }
    Ok(())
}

/// `tetherly net` -- Net arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn net(refresh: bool, interfaces: bool, json: bool, ui_port: u16) -> Result<()> {
    if refresh {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/net/refresh", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: tetherly_node::NetworkRefreshReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        println!("Network & Overlay Rescanned:");
        println!(
            "  Interfaces Scanned: {} ({} active listening)",
            rep.ifaces_scanned, rep.ifaces_listening
        );
        println!(
            "  EasyTier Sidecar:   {} ({})",
            rep.overlay_present, rep.overlay_source
        );
        let local_str = if rep.overlay_local_ips.is_empty() {
            "(none)".to_string()
        } else {
            rep.overlay_local_ips.join(", ")
        };
        println!("  Overlay Local IPs:  {local_str}");
        println!("  Overlay Mesh Peers: {}", rep.overlay_peers_count);
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/net")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node returned HTTP {code}: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }
    let rep: tetherly_node::NetworkReport =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

    let ov = &rep.overlay;
    let ov_enabled = ov.enabled;
    let ov_present = ov.present;
    let ov_source = &ov.source;
    let local_ips = if ov.local_ips.is_empty() {
        "(none)".to_string()
    } else {
        ov.local_ips.join(", ")
    };
    let peer_count = ov.peer_count;
    let peers = ov.peers.join(", ");

    println!("EasyTier Overlay Sidecar:");
    println!("  Enabled:    {ov_enabled}");
    println!("  Present:    {ov_present} ({ov_source})");
    println!("  Local IPs:  {local_ips}");
    if peer_count == 0 {
        println!("  Mesh Peers: 0");
    } else {
        println!("  Mesh Peers: {peer_count} ({peers})");
    }

    let ifaces_arr = &rep.interfaces;
    if interfaces {
        println!("\nInterface Classification Details:");
        if ifaces_arr.is_empty() {
            println!("  (no interfaces reported)");
        } else {
            for iface in ifaces_arr {
                let name = &iface.iface;
                let ip = &iface.ip;
                if iface.listen {
                    println!("  * [LISTEN] {name}: {ip}");
                } else {
                    let r_str = iface
                        .reason
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default();
                    println!("  * [SKIP]   {name}: {ip}{r_str}");
                }
            }
        }
    } else {
        println!("\nActive Listening Interfaces:");
        let mut listen_count = 0;
        for iface in ifaces_arr {
            if iface.listen {
                println!("  * {}: {}", iface.iface, iface.ip);
                listen_count += 1;
            }
        }
        if listen_count == 0 {
            println!("  (no listening interfaces)");
        }
        println!("\n(Use --interfaces or -i to inspect all skipped interfaces and classification reasons)");
    }
    Ok(())
}

/// `tetherly link` -- Link arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn link(action: Option<LinkCmd>, json: bool, ui_port: u16) -> Result<()> {
    handle_link_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly ping` -- Ping arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn ping(peer: Option<String>, count: u32, json: bool, ui_port: u16) -> Result<()> {
    if peer.is_some() {
        return do_peer_ping(ui_port, peer, count, json).await;
    }
    let count = count.max(1);
    let mut rtts = Vec::new();
    let mut last_rep = None;
    for i in 1..=count {
        let t0 = std::time::Instant::now();
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/health")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        let rtt = t0.elapsed();
        if code != 200 {
            anyhow::bail!(
                "node ping failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let rep: tetherly_node::HealthReport = serde_json::from_slice(&resp)?;
        let rtt_ms = rtt.as_secs_f64() * 1000.0;
        rtts.push(rtt_ms);
        last_rep = Some(rep.clone());
        if !json {
            println!(
                "[{i}/{count}] pong: node {}, rtt: {rtt_ms:.2}ms, uptime: {} (pid: {}, v{})",
                rep.status,
                format_uptime(rep.uptime_secs),
                rep.pid,
                rep.version
            );
        }
        if i < count {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    if json {
        if let Some(rep) = last_rep {
            let avg_rtt = rtts.iter().copied().sum::<f64>() / rtts.len() as f64;
            let mut val = serde_json::to_value(&rep)?;
            val["rtt_ms"] = serde_json::json!(avg_rtt);
            val["count"] = serde_json::json!(count);
            println!("{val}");
        }
    }
    Ok(())
}

/// `tetherly connect` -- Connect arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn connect(
    peer: String,
    addr: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "addr": addr,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/connect", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused connect (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::PeerConnectReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
        if rep.already_connected {
            println!(
                "peer {} ({}) is already connected (addr: {})",
                rep.device_id,
                display_name,
                rep.addr.as_deref().unwrap_or("unknown")
            );
        } else {
            println!(
                "successfully connected to {} ({}) at {}",
                rep.device_id,
                display_name,
                rep.addr.as_deref().unwrap_or("unknown")
            );
        }
    }
    Ok(())
}

/// `tetherly disconnect` -- Disconnect arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn disconnect(peer: String, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/disconnect", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused disconnect (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::PeerDisconnectReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
        if rep.already_disconnected {
            println!(
                "peer {} ({}) is already disconnected",
                rep.device_id, display_name
            );
        } else {
            println!(
                "successfully disconnected session with {} ({})",
                rep.device_id, display_name
            );
        }
    }
    Ok(())
}

/// `tetherly block` -- Block arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn block(
    peer: String,
    reason: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "reason": reason,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/block", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused block (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::PeerBlockReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
        let reason_str = rep
            .reason
            .as_deref()
            .map(|r| format!(" (reason: \"{r}\")"))
            .unwrap_or_default();
        if rep.already_blocked {
            println!(
                "peer {} ({}) is already blocked{}",
                rep.device_id, display_name, reason_str
            );
        } else {
            let active_str = if rep.disconnected_active {
                " (active session terminated)"
            } else {
                ""
            };
            println!(
                "successfully blocked peer {} ({}){}{}",
                rep.device_id, display_name, reason_str, active_str
            );
        }
    }
    Ok(())
}

/// `tetherly unblock` -- Unblock arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn unblock(peer: String, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/unblock", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused unblock (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::PeerUnblockReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if rep.was_blocked {
            println!("successfully unblocked peer {}", rep.device_id);
        } else {
            println!("peer {} was not in the blocklist", rep.device_id);
        }
    }
    Ok(())
}

/// `tetherly blocked` -- Blocked arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn blocked(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/blocked")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused blocked query (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::BlockedPeersReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if rep.peers.is_empty() {
            println!("no peers currently blocked");
        } else {
            println!("blocked peers ({}):", rep.peers.len());
            for p in rep.peers {
                let reason = p
                    .reason
                    .as_deref()
                    .map(|r| format!(" · reason: \"{r}\""))
                    .unwrap_or_default();
                println!("  - {}{}", p.device_id, reason);
            }
        }
    }
    Ok(())
}

pub(crate) async fn handle_link_cmd(
    action: Option<LinkCmd>,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = tetherly_node::loopback_get(default_ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{default_ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link quality query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if default_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let report: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_quality_report(&report);
            }
        }
        Some(LinkCmd::Status {
            watch: false,
            interval: _,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link quality query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let report: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_quality_report(&report);
            }
        }
        Some(LinkCmd::Status {
            watch: true,
            interval,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            loop {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/link")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused link quality query (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let report: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    print!("\x1b[2J\x1b[H");
                    print_link_quality_report(&report);
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
            }
        }
        Some(LinkCmd::Probe {
            peer,
            count,
            interval_ms,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            let body = serde_json::json!({
                "peer": peer,
                "count": count,
                "interval_ms": interval_ms,
            });
            let payload = serde_json::to_vec(&body)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/peers/link/probe", &payload)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link probe (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_probe_report(&rep);
            }
        }
        Some(LinkCmd::Config {
            interval,
            deadline,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            if interval.is_some() || deadline.is_some() {
                let body = serde_json::json!({
                    "interval_secs": interval,
                    "deadline_secs": deadline,
                });
                let payload = serde_json::to_vec(&body)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/peers/heartbeat", &payload)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused heartbeat config update (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let cfg: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    println!(
                        "Updated heartbeat config: interval = {}s, deadline = {}s",
                        cfg["interval_secs"].as_u64().unwrap_or(0),
                        cfg["deadline_secs"].as_u64().unwrap_or(0)
                    );
                }
            } else {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/heartbeat")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused heartbeat config query (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let cfg: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    println!(
                        "Current heartbeat config: interval = {}s, deadline = {}s",
                        cfg["interval_secs"].as_u64().unwrap_or(0),
                        cfg["deadline_secs"].as_u64().unwrap_or(0)
                    );
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn print_link_quality_report(v: &serde_json::Value) {
    let hb = &v["heartbeat"];
    let hb_interval = hb["interval_secs"].as_u64().unwrap_or(0);
    let hb_deadline = hb["deadline_secs"].as_u64().unwrap_or(0);
    let total = v["total_peers"].as_u64().unwrap_or(0);
    let online = v["online_peers"].as_u64().unwrap_or(0);

    println!(
        "Peer Link Quality Profiling (Heartbeat: {}s, Deadline: {}s) [{} online / {} total]",
        hb_interval, hb_deadline, online, total
    );
    let empty_vec = vec![];
    let links = v["links"].as_array().unwrap_or(&empty_vec);
    if links.is_empty() {
        println!("  (no peer links tracked)");
        return;
    }

    println!(
        "{:<20} {:<15} {:<8} {:<8} {:<18} {:<10} {:<8} {:<10}",
        "PEER ID", "NAME", "STATUS", "RTT", "MIN/MAX/AVG", "JITTER", "LOSS", "GRADE"
    );
    for link in links {
        let dev = link["device_id"].as_str().unwrap_or("?");
        let short_dev = if dev.len() > 18 { &dev[..18] } else { dev };
        let name = link["name"].as_str().unwrap_or("-");
        let short_name = if name.len() > 14 { &name[..14] } else { name };
        let status = if link["is_online"].as_bool().unwrap_or(false) {
            "online"
        } else {
            "offline"
        };
        let rtt = link["rtt_ms"]
            .as_u64()
            .map(|r| format!("{r}ms"))
            .unwrap_or_else(|| "-".into());
        let min_rtt = link["min_rtt_ms"].as_u64();
        let max_rtt = link["max_rtt_ms"].as_u64();
        let avg_rtt = link["avg_rtt_ms"].as_f64();
        let min_max_avg = match (min_rtt, max_rtt, avg_rtt) {
            (Some(min), Some(max), Some(avg)) => format!("{min}/{max}/{avg:.1}ms"),
            _ => "-".into(),
        };
        let jitter = link["jitter_ms"]
            .as_f64()
            .map(|j| format!("{j:.1}ms"))
            .unwrap_or_else(|| "-".into());
        let loss = format!("{:.1}%", link["packet_loss_rate"].as_f64().unwrap_or(0.0));
        let grade = link["grade"].as_str().unwrap_or("unknown");

        println!(
            "{:<20} {:<15} {:<8} {:<8} {:<18} {:<10} {:<8} {:<10}",
            short_dev, short_name, status, rtt, min_max_avg, jitter, loss, grade
        );
    }
}

pub(crate) fn print_link_probe_report(v: &serde_json::Value) {
    let dev = v["device_id"].as_str().unwrap_or("?");
    let name = v["name"].as_str().unwrap_or("-");
    let sent = v["sent_count"].as_u64().unwrap_or(0);
    let recv = v["recv_count"].as_u64().unwrap_or(0);
    let loss = v["packet_loss_rate"].as_f64().unwrap_or(0.0);
    let grade = v["grade"].as_str().unwrap_or("unknown");
    let min_rtt = v["min_rtt_ms"]
        .as_u64()
        .map(|r| format!("{r}ms"))
        .unwrap_or_else(|| "-".into());
    let max_rtt = v["max_rtt_ms"]
        .as_u64()
        .map(|r| format!("{r}ms"))
        .unwrap_or_else(|| "-".into());
    let avg_rtt = v["avg_rtt_ms"]
        .as_f64()
        .map(|a| format!("{a:.1}ms"))
        .unwrap_or_else(|| "-".into());
    let jitter = v["jitter_ms"]
        .as_f64()
        .map(|j| format!("{j:.1}ms"))
        .unwrap_or_else(|| "-".into());
    let empty_vec = vec![];
    let samples = v["samples_ms"].as_array().unwrap_or(&empty_vec);
    let samples_str: Vec<String> = samples.iter().map(|s| s.to_string()).collect();

    println!("Active Link Probe Report: {dev} ({name})");
    println!("  Probes: {sent} sent, {recv} received, loss: {loss:.1}%");
    println!("  Latency: min={min_rtt}, max={max_rtt}, avg={avg_rtt}, jitter={jitter}");
    println!("  Link Grade: {grade}");
    println!("  Samples (ms): [{}]", samples_str.join(", "));
}

pub(crate) async fn do_peer_ping(
    ui_port: u16,
    peer: Option<String>,
    count: u32,
    json: bool,
) -> anyhow::Result<()> {
    let body = serde_json::json!({ "peer": peer });
    let body_bytes = serde_json::to_vec(&body)?;
    let count = count.max(1);
    let mut rtts: Vec<u64> = Vec::new();
    let mut last_rep: Option<serde_json::Value> = None;
    for i in 1..=count {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/ping", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "peer ping failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let rtt = v["rtt_ms"].as_u64().unwrap_or(0);
        rtts.push(rtt);
        if !json {
            let dev = v["device_id"].as_str().unwrap_or("-");
            let path = v["path"].as_str().unwrap_or("-");
            let addr = v["addr"].as_str().unwrap_or("-");
            println!("[{i}/{count}] Ping {dev} ({addr} [{path}]): time={rtt}ms");
        }
        last_rep = Some(v);
        if i < count {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    if json {
        if let Some(mut rep) = last_rep {
            let min_rtt = rtts.iter().copied().min().unwrap_or(0);
            let max_rtt = rtts.iter().copied().max().unwrap_or(0);
            let avg_rtt = (rtts.iter().copied().sum::<u64>() as f64) / (rtts.len() as f64);
            rep["count"] = serde_json::json!(count);
            rep["min_rtt_ms"] = serde_json::json!(min_rtt);
            rep["max_rtt_ms"] = serde_json::json!(max_rtt);
            rep["avg_rtt_ms"] = serde_json::json!(avg_rtt);
            rep["history"] = serde_json::json!(rtts);
            println!("{rep}");
        }
    } else if count > 1 {
        let min_rtt = rtts.iter().copied().min().unwrap_or(0);
        let max_rtt = rtts.iter().copied().max().unwrap_or(0);
        let avg_rtt = (rtts.iter().copied().sum::<u64>() as f64) / (rtts.len() as f64);
        println!(
            "\n--- Ping Statistics ({count} probes) ---\nmin/avg/max = {min_rtt}/{avg_rtt:.1}/{max_rtt} ms"
        );
    }
    Ok(())
}

pub(crate) async fn print_routes_list(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/routes")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node returned HTTP {code}: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let routes_arr = v["routes"].as_array();
    let count = v["count"].as_u64().unwrap_or(0);
    println!("Cached Physical Routes ({count}):");
    match routes_arr {
        Some(arr) if !arr.is_empty() => {
            for r in arr {
                let dev = r["device_id"].as_str().unwrap_or("-");
                let addr = r["addr"].as_str().unwrap_or("-");
                let alias_str = r
                    .get("alias")
                    .and_then(|a| a.as_str())
                    .map(|a| format!(" (alias: \"{a}\")"))
                    .unwrap_or_default();
                let online_str = if r["online"].as_bool().unwrap_or(false) {
                    "[online]"
                } else {
                    "[offline]"
                };
                println!("  * {dev}{alias_str} -> {addr} {online_str}");
            }
        }
        _ => println!("  (no cached routes)"),
    }
    Ok(())
}
