// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Profile, tag and group commands.

use super::util::{loopback_get, loopback_post};
use super::*;

/// `tetherly-cli profile` -- Profile arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn profile(
    action: Option<ProfileCmd>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => handle_profile_list(tag, group, device_type, json, ui_port).await?,
        Some(ProfileCmd::List {
            tag: t,
            group: g,
            device_type: dt,
            json: j,
            ui_port: p,
        }) => {
            let final_tag = t.or(tag);
            let final_group = g.or(group);
            let final_type = dt.or(device_type);
            let final_port = if p != 45716 { p } else { ui_port };
            handle_profile_list(final_tag, final_group, final_type, j || json, final_port).await?;
        }
        Some(ProfileCmd::Get {
            peer,
            json: j,
            ui_port: p,
        }) => {
            let final_port = if p != 45716 { p } else { ui_port };
            handle_profile_get(&peer, j || json, final_port).await?;
        }
        Some(ProfileCmd::Set {
            peer,
            device_type: dt,
            group: g,
            notes: n,
            json: j,
            ui_port: p,
        }) => {
            let final_port = if p != 45716 { p } else { ui_port };
            handle_profile_set(&peer, dt, g, n, j || json, final_port).await?;
        }
    }
    Ok(())
}

/// `tetherly-cli tag` -- Tag arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn tag(action: TagCmd) -> Result<()> {
    match action {
        TagCmd::Add {
            peer,
            tag,
            json,
            ui_port,
        } => {
            handle_tag_add(&peer, &tag, json, ui_port).await?;
        }
        TagCmd::Remove {
            peer,
            tag,
            json,
            ui_port,
        } => {
            handle_tag_remove(&peer, &tag, json, ui_port).await?;
        }
    }
    Ok(())
}

/// `tetherly-cli group` -- Group arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn group(action: GroupCmd) -> Result<()> {
    match action {
        GroupCmd::Set {
            peer,
            group,
            json,
            ui_port,
        } => {
            handle_group_set(&peer, &group, json, ui_port).await?;
        }
        GroupCmd::Clear {
            peer,
            json,
            ui_port,
        } => {
            handle_group_clear(&peer, json, ui_port).await?;
        }
    }
    Ok(())
}

pub(crate) async fn handle_profile_list(
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(t) = &tag {
        query_params.push(format!("tag={t}"));
    }
    if let Some(g) = &group {
        query_params.push(format!("group={g}"));
    }
    if let Some(dt) = &device_type {
        query_params.push(format!("type={dt}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/peers/profiles{qs}");
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "failed to list profiles (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let total = val["total"].as_u64().unwrap_or(0);
        let profiles = val["profiles"].as_array();
        println!("Peer Profiles (total: {total}):");
        match profiles {
            Some(arr) if !arr.is_empty() => {
                println!(
                    "  {:<36} {:<12} {:<16} {:<24}",
                    "DEVICE ID", "TYPE", "GROUP", "TAGS"
                );
                for p in arr {
                    let dev = p["device_id"].as_str().unwrap_or("-");
                    let dev_type = p["device_type"].as_str().unwrap_or("other");
                    let group = p["group"].as_str().unwrap_or("-");
                    let tags = p["tags"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    println!("  {:<36} {:<12} {:<16} {:<24}", dev, dev_type, group, tags);
                }
            }
            _ => println!("  (no peer profiles match criteria)"),
        }
    }
    Ok(())
}

pub(crate) async fn handle_profile_get(peer: &str, json: bool, ui_port: u16) -> Result<()> {
    let path = format!("/api/peers/profile?peer={peer}");
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "failed to get peer profile (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or("-");
        let dev_type = val["device_type"].as_str().unwrap_or("other");
        let group = val["group"].as_str().unwrap_or("<none>");
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let notes = val["notes"].as_str().unwrap_or("<none>");
        let updated = val["updated_ms"].as_u64().unwrap_or(0);
        println!("Peer Profile for: {dev}");
        println!("  Device Type : {dev_type}");
        println!("  Group       : {group}");
        println!("  Tags        : [{tags}]");
        println!("  Notes       : {notes}");
        println!("  Updated     : {updated} ms");
    }
    Ok(())
}

pub(crate) async fn handle_profile_set(
    peer: &str,
    device_type: Option<String>,
    group: Option<String>,
    notes: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "device_type": device_type,
        "group": group,
        "notes": notes,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/profile", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to update profile (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let msg = val["message"].as_str().unwrap_or("profile updated");
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Profile updated for {dev}: {msg}");
    }
    Ok(())
}

pub(crate) async fn handle_tag_add(peer: &str, tag: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "tag": tag,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/tags/add", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to add tag (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Tag '{tag}' added to {dev}. Current tags: [{tags}]");
    }
    Ok(())
}

pub(crate) async fn handle_tag_remove(
    peer: &str,
    tag: &str,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "tag": tag,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/tags/remove", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to remove tag (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Tag '{tag}' removed from {dev}. Current tags: [{tags}]");
    }
    Ok(())
}

pub(crate) async fn handle_group_set(
    peer: &str,
    group: &str,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "group": group,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to set group (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Group set to '{group}' for {dev}");
    }
    Ok(())
}

pub(crate) async fn handle_group_clear(peer: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "group": serde_json::Value::Null,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to clear group (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Group cleared for {dev}");
    }
    Ok(())
}
