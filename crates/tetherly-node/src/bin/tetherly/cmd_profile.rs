// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Profile, tag and group commands.

use super::*;

/// `tetherly profile` -- Profile arm of the top-level dispatch.
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

/// `tetherly tag` -- Tag arm of the top-level dispatch.
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

/// `tetherly group` -- Group arm of the top-level dispatch.
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
    let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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
    let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/profile", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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

/// §150: resolve a peer and its current tags over v1 — one GET /devices
/// pass, matching by id, alias or name exactly like `resolve_peer` (the
/// PATCH path re-resolves server-side with the same rules, so a well-formed
/// unknown id still surfaces the daemon's own 404). Returns the device id
/// and the current tag list; `None` when no device row matches (the GET
/// list only covers live/trusted/discovered/blocked rows, so malformed ids
/// and truly unknown peers are indistinguishable here — both 404 upstream).
async fn v1_device_tags(ui_port: u16, peer: &str) -> Result<Option<(String, Vec<String>)>> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/v1/devices")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to list devices (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let devices: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let q = peer.trim().to_lowercase();
    Ok(devices
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|d| {
                d["id"].as_str().map(|s| s.to_lowercase()) == Some(q.clone())
                    || d["alias"].as_str().map(|s| s.to_lowercase()) == Some(q.clone())
                    || d["name"].as_str().map(|s| s.to_lowercase()) == Some(q.clone())
            })
        })
        .map(|d| {
            (
                d["id"].as_str().unwrap_or_default().to_string(),
                d["tags"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            )
        }))
}

/// §150: tag add/remove as a read-modify-write over v1 device resources.
/// The legacy `/api/peers/tags/*` endpoints folded the set arithmetic into
/// the daemon; v1 only offers full-list PATCH (`DevicePatch.tags`), so the
/// CLI reads the current list, applies the add/remove locally (case-blind,
/// deduped — same rules as `add_peer_tag`), and PATCHes the result. The
/// daemon re-validates (1-32 chars, [a-zA-Z0-9_-], max 16) and re-resolves
/// the id, so error surfaces stay server-owned.
async fn v1_patch_tags(
    ui_port: u16,
    peer: &str,
    tag: &str,
    add: bool,
) -> Result<(String, Vec<String>)> {
    use tetherly_node::loopback_request;

    let Some((device_id, current)) = v1_device_tags(ui_port, peer).await? else {
        anyhow::bail!("peer not found: {peer}");
    };
    // `add_peer_tag` validates before storing; surface the same rules
    // client-side so typos fail before the round-trip.
    let clean = tag.trim();
    if clean.is_empty()
        || clean.len() > 32
        || !clean
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
    {
        anyhow::bail!("invalid tag '{clean}': 1-32 chars, [a-zA-Z0-9_-] only");
    }
    let mut next = current;
    if add {
        if !next.iter().any(|t| t.eq_ignore_ascii_case(clean)) {
            next.push(clean.to_string());
        }
    } else {
        next.retain(|t| !t.eq_ignore_ascii_case(clean));
    }
    let body = serde_json::json!({ "tags": next });
    let bytes = serde_json::to_vec(&body)?;
    let (code, resp) = loopback_request(
        ui_port,
        "PATCH",
        &format!("/api/v1/devices/{device_id}"),
        Some(&bytes),
        tetherly_node::api_token_for(ui_port).as_deref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("tag write failed on 127.0.0.1:{ui_port}: {e}"))?;
    if code != 200 {
        anyhow::bail!(
            "failed to update tags (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let dev: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    Ok((
        dev["id"].as_str().unwrap_or(&device_id).to_string(),
        dev["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    ))
}

pub(crate) async fn handle_tag_add(peer: &str, tag: &str, json: bool, ui_port: u16) -> Result<()> {
    let (device_id, tags) = v1_patch_tags(ui_port, peer, tag, true).await?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "device_id": device_id,
                "tags": tags,
                "message": format!("tag '{}' added successfully", tag.trim()),
            })
        );
    } else {
        let joined = tags.join(", ");
        println!(
            "Tag '{}' added to {device_id}. Current tags: [{joined}]",
            tag.trim()
        );
    }
    Ok(())
}

pub(crate) async fn handle_tag_remove(
    peer: &str,
    tag: &str,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (device_id, tags) = v1_patch_tags(ui_port, peer, tag, false).await?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "device_id": device_id,
                "tags": tags,
                "message": format!("tag '{}' removed successfully", tag.trim()),
            })
        );
    } else {
        let joined = tags.join(", ");
        println!(
            "Tag '{}' removed from {device_id}. Current tags: [{joined}]",
            tag.trim()
        );
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
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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

#[cfg(test)]
mod tag_v1_tests {
    use super::*;

    fn temp_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-cli-tags-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (
            Node::start(NodeConfig {
                data_dir: dir.clone(),
                loopback_only: true,
                ui_port: 0,
                ..NodeConfig::default()
            })
            .unwrap(),
            dir,
        )
    }

    /// §150: tag add/remove is a read-modify-write over v1 device
    /// resources — GET /devices to resolve the peer and read current tags,
    /// local set arithmetic, PATCH /devices/{id} with the full list. The
    /// frozen `/api/peers/tags/*` aliases must stay untouched.
    #[tokio::test]
    async fn tag_add_remove_round_trip_v1_device_patch() {
        let (node, dir) = temp_node("roundtrip");
        let (port, _token) =
            tetherly_node::uihttp::bind_and_spawn_with_token(node.clone_handle(), 0)
                .await
                .unwrap();
        tetherly_node::set_port_api_token(
            port,
            Some(
                tetherly_node::apitoken::read(&dir)
                    .expect("api.token minted")
                    .to_string(),
            ),
        );

        // Error paths — a *malformed* device id fails DeviceId::parse, so
        // legacy resolve_peer rejects it before any profile write; the v1
        // read-modify-write must keep that 404 surface. (A well-formed
        // unknown id historically auto-created a profile — §150 note.)
        let err = handle_tag_add("not-a-device-id", "spam", false, port)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("peer not found"), "{err}");
        let err = handle_tag_remove("not-a-device-id", "spam", false, port)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("peer not found"), "{err}");

        let (code, resp) = tetherly_node::loopback_get(port, "/api/v1/diagnostics")
            .await
            .unwrap();
        assert_eq!(code, 200);
        let v: serde_json::Value = serde_json::from_slice(&resp).unwrap();
        let hits = v["legacy_routes"].as_array().unwrap();
        assert!(
            hits.iter().all(|h| !h["route"]
                .as_str()
                .unwrap_or_default()
                .starts_with("/api/peers/tags")),
            "tag commands still hit legacy routes: {hits:?}"
        );

        tetherly_node::set_port_api_token(port, None);
    }

    /// §150: happy path — tags round-trip against a trusted peer seeded
    /// through `trusted.json` (the only cross-crate seeding path; the
    /// in-crate `seed_trusted_peer` backdoor is `cfg(test)`-gated to the
    /// library). Covers add, idempotent re-add, case-blind remove and
    /// remove-of-absent, plus alias resolution picking the same row.
    #[tokio::test]
    async fn tag_add_remove_happy_path_v1_patch() {
        use base64::Engine as _;

        let dir = std::env::temp_dir().join(format!(
            "tetherly-cli-tags-happy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let id = tetherly_core::DeviceId::from_id_pk(&[7u8; 32]);
        let trust = serde_json::json!({
            "peers": [{
                "device_id": id.as_str(),
                "id_pk": base64::engine::general_purpose::STANDARD.encode([7u8; 32]),
                "n_pk": base64::engine::general_purpose::STANDARD.encode([8u8; 32]),
                "alias": "living-room",
                "paired_at": 0,
                "revoked": false,
            }]
        });
        std::fs::write(
            dir.join("trusted.json"),
            serde_json::to_vec(&trust).unwrap(),
        )
        .unwrap();
        let node = Node::start(NodeConfig {
            data_dir: dir.clone(),
            loopback_only: true,
            ui_port: 0,
            ..NodeConfig::default()
        })
        .unwrap();
        let (port, _token) =
            tetherly_node::uihttp::bind_and_spawn_with_token(node.clone_handle(), 0)
                .await
                .unwrap();
        tetherly_node::set_port_api_token(
            port,
            Some(
                tetherly_node::apitoken::read(&dir)
                    .expect("api.token minted")
                    .to_string(),
            ),
        );

        // Add via alias (resolve path), then idempotent re-add via id.
        handle_tag_add("living-room", "media", false, port)
            .await
            .unwrap();
        handle_tag_add(id.as_str(), "Media", false, port)
            .await
            .unwrap();
        let (_, tags) = v1_patch_tags(port, id.as_str(), "proj", true)
            .await
            .unwrap();
        assert_eq!(tags, vec!["media".to_string(), "proj".to_string()]);

        // Remove (case-blind), then remove of a tag that is not there.
        let (_, tags) = v1_patch_tags(port, "living-room", "MEDIA", false)
            .await
            .unwrap();
        assert_eq!(tags, vec!["proj".to_string()]);
        let (_, tags) = v1_patch_tags(port, id.as_str(), "absent", false)
            .await
            .unwrap();
        assert_eq!(tags, vec!["proj".to_string()]);

        tetherly_node::set_port_api_token(port, None);
    }
}
