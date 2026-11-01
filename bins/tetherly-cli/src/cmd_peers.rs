// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer discovery, connect/disconnect/block, routes/net and link quality.

use super::util::{format_uptime, loopback_delete, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli unpair` -- Unpair arm of the top-level dispatch.
pub(crate) async fn unpair(device_id: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "device_id": device_id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/unpair", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
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

/// `tetherly-cli peers` -- Peers arm of the top-level dispatch.
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
    let (code, resp) = loopback_get(ui_port, "/api/peers").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
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

/// `tetherly-cli alias` -- Alias arm of the top-level dispatch.
pub(crate) async fn alias(peer: String, alias: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "alias": alias,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/alias", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
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

/// `tetherly-cli routes` -- Routes arm of the top-level dispatch.
pub(crate) async fn routes(json: bool, ui_port: u16) -> Result<()> {
    print_routes_list(ui_port, json).await?;
    Ok(())
}

/// `tetherly-cli route` -- Route arm of the top-level dispatch.
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
            let (code, resp) = loopback_post(ui_port, "/api/routes", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "route add failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let action = v
                .get("action")
                .and_then(|s| s.as_str())
                .unwrap_or("updated");
            let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
            let addr_res = v.get("addr").and_then(|s| s.as_str()).unwrap_or(&addr);
            println!("Route {action}: {device_id} -> {addr_res}");
        }
        RouteCmd::Remove {
            peer,
            json,
            ui_port,
        } => {
            let path = format!("/api/routes?peer={}", peer.trim());
            let (code, resp) = loopback_delete(ui_port, &path).await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                bail!(
                    "route remove failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let action = v
                .get("action")
                .and_then(|s| s.as_str())
                .unwrap_or("removed");
            let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
            println!("Route {action}: {device_id}");
        }
    }
    Ok(())
}

/// `tetherly-cli net` -- Net arm of the top-level dispatch.
pub(crate) async fn net(refresh: bool, interfaces: bool, json: bool, ui_port: u16) -> Result<()> {
    if refresh {
        let (code, resp) = loopback_post(ui_port, "/api/net/refresh", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        println!("Network & Overlay Rescanned:");
        let scanned = v["ifaces_scanned"].as_u64().unwrap_or(0);
        let listening = v["ifaces_listening"].as_u64().unwrap_or(0);
        let present = v["overlay_present"].as_bool().unwrap_or(false);
        let source = v["overlay_source"].as_str().unwrap_or("Absent");
        let peers_count = v["overlay_peers_count"].as_u64().unwrap_or(0);
        let local_ips = v["overlay_local_ips"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let local_str = if local_ips.is_empty() {
            "(none)".to_string()
        } else {
            local_ips
        };
        println!("  Interfaces Scanned: {scanned} ({listening} active listening)");
        println!("  EasyTier Sidecar:   {present} ({source})");
        println!("  Overlay Local IPs:  {local_str}");
        println!("  Overlay Mesh Peers: {peers_count}");
        return Ok(());
    }

    let (code, resp) = loopback_get(ui_port, "/api/net").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
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

    let ov = &v["overlay"];
    let ov_enabled = ov["enabled"].as_bool().unwrap_or(false);
    let ov_present = ov["present"].as_bool().unwrap_or(false);
    let ov_source = ov["source"].as_str().unwrap_or("Absent");
    let local_ips = ov["local_ips"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let peer_count = ov["peer_count"].as_u64().unwrap_or(0);
    let peers = ov["peers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    println!("EasyTier Overlay Sidecar:");
    println!("  Enabled:    {ov_enabled}");
    println!("  Present:    {ov_present} ({ov_source})");
    println!(
        "  Local IPs:  {}",
        if local_ips.is_empty() {
            "(none)"
        } else {
            &local_ips
        }
    );
    if peer_count == 0 {
        println!("  Mesh Peers: 0");
    } else {
        println!("  Mesh Peers: {peer_count} ({peers})");
    }

    let ifaces_arr = v["interfaces"].as_array();
    if interfaces {
        println!("\nInterface Classification Details:");
        match ifaces_arr {
            Some(arr) if !arr.is_empty() => {
                for iface in arr {
                    let name = iface["iface"].as_str().unwrap_or("-");
                    let ip = iface["ip"].as_str().unwrap_or("-");
                    let listen = iface["listen"].as_bool().unwrap_or(false);
                    let reason = iface["reason"].as_str();
                    if listen {
                        println!("  * [LISTEN] {name}: {ip}");
                    } else {
                        let r_str = reason.map(|r| format!(" ({r})")).unwrap_or_default();
                        println!("  * [SKIP]   {name}: {ip}{r_str}");
                    }
                }
            }
            _ => println!("  (no interfaces reported)"),
        }
    } else {
        println!("\nActive Listening Interfaces:");
        let mut listen_count = 0;
        if let Some(arr) = ifaces_arr {
            for iface in arr {
                let listen = iface["listen"].as_bool().unwrap_or(false);
                if listen {
                    let name = iface["iface"].as_str().unwrap_or("-");
                    let ip = iface["ip"].as_str().unwrap_or("-");
                    println!("  * {name}: {ip}");
                    listen_count += 1;
                }
            }
        }
        if listen_count == 0 {
            println!("  (no listening interfaces)");
        }
        println!("\n(Use --interfaces or -i to inspect all skipped interfaces and classification reasons)");
    }
    Ok(())
}

/// `tetherly-cli link` -- Link arm of the top-level dispatch.
pub(crate) async fn link(action: Option<LinkCmd>, json: bool, ui_port: u16) -> Result<()> {
    handle_link_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli ping` -- Ping arm of the top-level dispatch.
pub(crate) async fn ping(peer: Option<String>, count: u32, json: bool, ui_port: u16) -> Result<()> {
    if peer.is_some() {
        return do_peer_ping(ui_port, peer, count, json).await;
    }
    let count = count.max(1);
    let mut rtts = Vec::new();
    let mut last_v = None;
    for i in 1..=count {
        let t0 = std::time::Instant::now();
        let (code, resp) = loopback_get(ui_port, "/api/health").await.map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        let rtt = t0.elapsed();
        if code != 200 {
            bail!(
                "node ping failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let rtt_ms = rtt.as_secs_f64() * 1000.0;
        rtts.push(rtt_ms);
        last_v = Some(v.clone());
        if !json {
            let status = v["status"].as_str().unwrap_or("unknown");
            let version = v["version"].as_str().unwrap_or("?");
            let pid = v["pid"].as_u64().unwrap_or(0);
            let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
            println!(
                "[{i}/{count}] pong: node {status}, rtt: {rtt_ms:.2}ms, uptime: {} (pid: {pid}, v{version})",
                format_uptime(uptime_secs)
            );
        }
        if i < count {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    if json {
        if let Some(mut v) = last_v {
            let avg_rtt = rtts.iter().copied().sum::<f64>() / rtts.len() as f64;
            v["rtt_ms"] = serde_json::json!(avg_rtt);
            v["count"] = serde_json::json!(count);
            println!("{v}");
        }
    }
    Ok(())
}

/// `tetherly-cli connect` -- Connect arm of the top-level dispatch.
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
    let (code, resp) = loopback_post(ui_port, "/api/peers/connect", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node refused connect (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let name = v["alias"]
            .as_str()
            .or_else(|| v["name"].as_str())
            .unwrap_or("peer");
        let addr_str = v["addr"].as_str().unwrap_or("unknown");
        let already = v["already_connected"].as_bool().unwrap_or(false);
        if already {
            println!("peer {device_id} ({name}) is already connected (addr: {addr_str})");
        } else {
            println!("successfully connected to {device_id} ({name}) at {addr_str}");
        }
    }
    Ok(())
}

/// `tetherly-cli disconnect` -- Disconnect arm of the top-level dispatch.
pub(crate) async fn disconnect(peer: String, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/disconnect", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node refused disconnect (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let name = v["alias"]
            .as_str()
            .or_else(|| v["name"].as_str())
            .unwrap_or("peer");
        let already = v["already_disconnected"].as_bool().unwrap_or(false);
        if already {
            println!("peer {device_id} ({name}) is already disconnected");
        } else {
            println!("successfully disconnected session with {device_id} ({name})");
        }
    }
    Ok(())
}

/// `tetherly-cli block` -- Block arm of the top-level dispatch.
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
    let (code, resp) = loopback_post(ui_port, "/api/peers/block", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node refused block (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let name = v["alias"]
            .as_str()
            .or_else(|| v["name"].as_str())
            .unwrap_or("peer");
        let reason_str = v["reason"]
            .as_str()
            .map(|r| format!(" (reason: \"{r}\")"))
            .unwrap_or_default();
        let already = v["already_blocked"].as_bool().unwrap_or(false);
        if already {
            println!("peer {device_id} ({name}) is already blocked{reason_str}");
        } else {
            let active_str = if v["disconnected_active"].as_bool().unwrap_or(false) {
                " (active session terminated)"
            } else {
                ""
            };
            println!("successfully blocked peer {device_id} ({name}){reason_str}{active_str}");
        }
    }
    Ok(())
}

/// `tetherly-cli unblock` -- Unblock arm of the top-level dispatch.
pub(crate) async fn unblock(peer: String, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/unblock", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node refused unblock (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let was_blocked = v["was_blocked"].as_bool().unwrap_or(false);
        if was_blocked {
            println!("successfully unblocked peer {device_id}");
        } else {
            println!("peer {device_id} was not in the blocklist");
        }
    }
    Ok(())
}

/// `tetherly-cli blocked` -- Blocked arm of the top-level dispatch.
pub(crate) async fn blocked(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/peers/blocked")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node refused blocked query (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let peers = v["peers"].as_array();
        let count = peers.map(|p| p.len()).unwrap_or(0);
        if count == 0 {
            println!("no peers currently blocked");
        } else {
            println!("blocked peers ({count}):");
            for p in peers.unwrap() {
                let dev = p["device_id"].as_str().unwrap_or("?");
                let reason = p["reason"]
                    .as_str()
                    .map(|r| format!(" · reason: \"{r}\""))
                    .unwrap_or_default();
                println!("  - {dev}{reason}");
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
            let (code, resp) = loopback_get(default_ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{default_ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
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
            let (code, resp) = loopback_get(ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
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
                let (code, resp) = loopback_get(ui_port, "/api/peers/link")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
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
            let (code, resp) = loopback_post(ui_port, "/api/peers/link/probe", &payload)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
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
                let (code, resp) = loopback_post(ui_port, "/api/peers/heartbeat", &payload)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
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
                let (code, resp) = loopback_get(ui_port, "/api/peers/heartbeat")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
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
        let (code, resp) = loopback_post(ui_port, "/api/peers/ping", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
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
    let (code, resp) = loopback_get(ui_port, "/api/routes").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
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
