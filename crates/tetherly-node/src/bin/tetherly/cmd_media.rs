// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Media transport status, policy and watch.

use super::*;

/// `tetherly media` -- Media arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn media(action: MediaCmd) -> Result<()> {
    match action {
        MediaCmd::Play {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "play",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Pause {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "pause",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Toggle {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "toggle",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Stop {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "stop",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Next {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "next",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Prev {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "prev",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Seek {
            offset_ms,
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "seek",
                None,
                Some(offset_ms),
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Volume {
            percent,
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "volume",
                Some(percent),
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Mute {
            peer,
            tag,
            group,
            device_type,
            all,
            player,
            json,
            ui_port,
        } => {
            do_media_command(
                ui_port,
                "mute",
                None,
                None,
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                player.as_deref(),
                json,
            )
            .await?;
        }
        MediaCmd::Status {
            peer,
            json,
            ui_port,
        } => {
            do_media_status(ui_port, peer.as_deref(), json).await?;
        }
        MediaCmd::Policy {
            action,
            mode,
            group,
            tag,
            device_type,
            max_volume,
            allow_seek,
            clear_filters,
            json,
            ui_port,
        } => {
            do_media_policy(
                ui_port,
                action,
                mode,
                group,
                tag,
                device_type,
                max_volume,
                allow_seek,
                clear_filters,
                json,
            )
            .await?;
        }
        MediaCmd::Watch { json, ui_port } => {
            do_media_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_media_command(
    ui_port: u16,
    cmd: &str,
    volume: Option<u8>,
    offset_ms: Option<i64>,
    peer: Option<&str>,
    tag: Option<&str>,
    group: Option<&str>,
    device_type: Option<&str>,
    all: bool,
    player: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "command": cmd,
    });
    if let Some(vol) = volume {
        payload["volume"] = serde_json::json!(vol);
    }
    if let Some(off) = offset_ms {
        payload["offset_ms"] = serde_json::json!(off);
    }
    if let Some(p) = peer {
        payload["peer"] = serde_json::json!(p);
    }
    if let Some(t) = tag {
        payload["tag"] = serde_json::json!(t);
    }
    if let Some(g) = group {
        payload["group"] = serde_json::json!(g);
    }
    if let Some(dt) = device_type {
        payload["device_type"] = serde_json::json!(dt);
    }
    if all {
        payload["all"] = serde_json::json!(true);
    }
    if let Some(pl) = player {
        payload["target_player"] = serde_json::json!(pl);
    }

    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/media/command", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "media command failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if let Some(target_cnt) = v.get("target_count").and_then(|c| c.as_u64()) {
            let succ_cnt = v["success_count"].as_u64().unwrap_or(0);
            let fail_cnt = v["failed_count"].as_u64().unwrap_or(0);
            let skip_cnt = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let sel_desc = v["selector"].to_string();
            let cmd_label = v["command"].as_str().unwrap_or(cmd);
            println!("Multi-Peer Media Command Dispatched:");
            println!("  Command:              {cmd_label}");
            println!("  Target Selector:      {sel_desc}");
            println!("  Targets / Dispatched: {succ_cnt} / {target_cnt} (failed: {fail_cnt}, skipped: {skip_cnt})");
        } else {
            let id = v["id"].as_str().unwrap_or("-");
            let target_peer = v["peer"].as_str().unwrap_or("-");
            let alias_desc = v["peer_alias"]
                .as_str()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            let cmd_label = v["command"].as_str().unwrap_or(cmd);
            let succ = v["success"].as_bool().unwrap_or(true);
            println!("Media Command Dispatched:");
            println!("  Command ID:           {id}");
            println!("  Command:              {cmd_label}");
            println!("  Target Peer:          {target_peer}{alias_desc}");
            println!("  Delivered:            {succ}");
        }
    }
    Ok(())
}

pub(crate) async fn do_media_status(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/media/state".to_string();
    if let Some(p) = peer {
        path = format!("{path}?peer={p}");
    }
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
            "failed to query media status (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target_peer = v["peer"].as_str().unwrap_or("local");
    if let Some(st) = v.get("state").filter(|s| !s.is_null()) {
        let player_id = st["player_id"].as_str().unwrap_or("default");
        let state = st["state"].as_str().unwrap_or("stopped");
        let title = st["title"].as_str().unwrap_or("");
        let artist = st["artist"].as_str().unwrap_or("");
        let album = st["album"].as_str().unwrap_or("");
        let pos_ms = st["position_ms"].as_u64().unwrap_or(0);
        let dur_ms = st["duration_ms"].as_u64().unwrap_or(0);
        let vol = st["volume_percent"].as_u64().unwrap_or(100);
        let muted = st["muted"].as_bool().unwrap_or(false);

        let pos_sec = pos_ms / 1000;
        let dur_sec = dur_ms / 1000;
        let pos_str = format!("{:02}:{:02}", pos_sec / 60, pos_sec % 60);
        let dur_str = if dur_ms > 0 {
            format!("{:02}:{:02}", dur_sec / 60, dur_sec % 60)
        } else {
            "--:--".to_string()
        };

        println!("Media Playback Status ({target_peer}):");
        println!("  Player:     {player_id}");
        println!("  State:      {state}");
        if !title.is_empty() {
            println!("  Title:      {title}");
        }
        if !artist.is_empty() {
            println!("  Artist:     {artist}");
        }
        if !album.is_empty() {
            println!("  Album:      {album}");
        }
        println!("  Position:   {pos_str} / {dur_str}");
        println!(
            "  Volume:     {vol}%{}",
            if muted { " (muted)" } else { "" }
        );
    } else {
        println!("No active playback on {target_peer}.");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_media_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_volume: Option<u8>,
    allow_seek: Option<bool>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || mode.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_volume.is_some()
        || allow_seek.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(m) = mode {
            payload["mode"] = serde_json::json!(m);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if let Some(lim) = max_volume {
            payload["max_volume_limit"] = serde_json::json!(lim);
        }
        if let Some(seek) = allow_seek {
            payload["allow_seek"] = serde_json::json!(seek);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let body = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/media/policy", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to update media policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/media/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query media policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let p = &v["policy"];
    let mode = p["mode"].as_str().unwrap_or("allow");
    let max_vol = p["max_volume_limit"].as_u64().unwrap_or(100);
    let allow_seek = p["allow_seek"].as_bool().unwrap_or(true);
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    println!("Media Remote Control Ingress Policy:");
    println!("  Control Mode:      {mode}");
    println!("  Max Volume Limit:  {max_vol}%");
    println!("  Allow Remote Seek: {allow_seek}");
    println!(
        "  Allowed Groups:    {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:      {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:     {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

pub(crate) async fn do_media_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live media playback & control events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "media_command_received"
                || kind == "media_command_dispatched"
                || kind == "media_state_changed"
                || kind == "media_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "media_command_received" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let sender = v["sender_alias"].as_str().unwrap_or("unknown");
                            let cmd = v["command"].as_str().unwrap_or("");
                            println!(
                                "[media_command_received] id={id} from={sender} command={cmd}"
                            );
                        }
                        "media_command_dispatched" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let target = v["target_peer"].as_str().unwrap_or("");
                            let cmd = v["command"].as_str().unwrap_or("");
                            let succ = v["success"].as_bool().unwrap_or(true);
                            println!("[media_command_dispatched] id={id} to={target} command={cmd} success={succ}");
                        }
                        "media_state_changed" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let state = v["state"].as_str().unwrap_or("");
                            let title = v["title"].as_str().unwrap_or("");
                            let vol = v["volume_percent"].as_u64().unwrap_or(0);
                            let title_disp = if !title.is_empty() {
                                format!(" \"{title}\"")
                            } else {
                                String::new()
                            };
                            println!("[media_state_changed] peer={peer} state={state}{title_disp} vol={vol}%");
                        }
                        "media_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let max_vol = v["max_volume_limit"].as_u64().unwrap_or(100);
                            println!("[media_policy_changed] mode={mode} max_volume={max_vol}%");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
