// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Media command/state bridging and media policy.

use super::*;

pub(super) async fn post_media_command(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let req: MediaCommandBody = if body.is_empty() {
        MediaCommandBody {
            peer: qmap.get("peer").cloned(),
            target: qmap.get("target").cloned(),
            command: qmap.get("command").or_else(|| qmap.get("cmd")).cloned(),
            action: qmap.get("action").cloned(),
            volume: qmap
                .get("volume")
                .or_else(|| qmap.get("volume_percent"))
                .and_then(|v| v.parse::<u8>().ok()),
            volume_percent: None,
            offset_ms: qmap
                .get("offset_ms")
                .or_else(|| qmap.get("offset"))
                .and_then(|v| v.parse::<i64>().ok()),
            target_player: qmap
                .get("target_player")
                .or_else(|| qmap.get("player"))
                .cloned(),
            tag: qmap.get("tag").cloned(),
            group: qmap.get("group").cloned(),
            device_type: qmap
                .get("type")
                .or_else(|| qmap.get("device_type"))
                .cloned(),
            r#type: None,
            all: qmap.get("all").map(|s| s == "true" || s == "1"),
        }
    } else {
        match serde_json::from_slice::<MediaCommandBody>(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json, expected MediaCommandBody"),
        }
    };

    let cmd_str = req
        .command
        .or(req.action)
        .or_else(|| {
            qmap.get("command")
                .or_else(|| qmap.get("cmd"))
                .or_else(|| qmap.get("action"))
                .cloned()
        })
        .unwrap_or_default();
    let cmd_str = cmd_str.trim().to_ascii_lowercase();
    if cmd_str.is_empty() {
        return err(400, "missing required 'command' or 'action'");
    }

    let command = match cmd_str.as_str() {
        "play" => MediaCommand::Play,
        "pause" => MediaCommand::Pause,
        "play_pause" | "playpause" | "toggle" => MediaCommand::PlayPause,
        "stop" => MediaCommand::Stop,
        "next" => MediaCommand::Next,
        "prev" | "previous" => MediaCommand::Previous,
        "seek" => {
            let offset = req
                .offset_ms
                .or_else(|| {
                    qmap.get("offset_ms")
                        .or_else(|| qmap.get("offset"))
                        .and_then(|s| s.parse::<i64>().ok())
                })
                .unwrap_or(0);
            MediaCommand::Seek { offset_ms: offset }
        }
        "volume" | "set_volume" => {
            let vol = req
                .volume
                .or(req.volume_percent)
                .or_else(|| {
                    qmap.get("volume")
                        .or_else(|| qmap.get("volume_percent"))
                        .and_then(|s| s.parse::<u8>().ok())
                })
                .unwrap_or(0);
            MediaCommand::SetVolume {
                volume_percent: vol,
            }
        }
        "toggle_mute" | "mute" => MediaCommand::ToggleMute,
        other => return err(400, &format!("unsupported media command: '{other}'")),
    };

    let target_player = req.target_player.or_else(|| {
        qmap.get("target_player")
            .or_else(|| qmap.get("player"))
            .cloned()
    });

    let is_all = req.all.unwrap_or(false)
        || qmap
            .get("all")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false)
        || qmap.contains_key("all");
    let tag = req
        .tag
        .as_deref()
        .or_else(|| qmap.get("tag").map(String::as_str));
    let group = req
        .group
        .as_deref()
        .or_else(|| qmap.get("group").map(String::as_str));
    let dev_type = req
        .device_type
        .as_deref()
        .or(req.r#type.as_deref())
        .or_else(|| {
            qmap.get("type")
                .or_else(|| qmap.get("device_type"))
                .map(String::as_str)
        });
    let peer = req.peer.as_deref().or(req.target.as_deref()).or_else(|| {
        qmap.get("peer")
            .or_else(|| qmap.get("target"))
            .map(String::as_str)
    });

    if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
        let selector =
            match crate::store::TargetSelector::from_query(peer, tag, group, dev_type, is_all) {
                Ok(s) => s,
                Err(e) => return err(400, &e),
            };
        match node
            .media_send_command_multi(&selector, command, target_player)
            .await
        {
            Ok(report) => return json_ok(&report),
            Err(e) => return err(400, &e.to_string()),
        }
    }

    let target_peer = peer.unwrap_or("");
    match node
        .media_send_command(target_peer, command, target_player)
        .await
    {
        Ok(res) => json_ok(&res),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_media_state(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let peer = qmap
        .get("peer")
        .or_else(|| qmap.get("target"))
        .map(String::as_str);
    match node.media_get_state(peer) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_media_state(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let parsed_body = match serde_json::from_slice::<MediaStateUpdateBody>(body) {
        Ok(b) => b,
        Err(_) => return err(400, "bad json, expected MediaStateUpdateBody"),
    };
    let mut current = match node.media_get_state(None) {
        Ok(rep) => rep.state.unwrap_or_default(),
        Err(e) => return err(400, &e.to_string()),
    };

    if let Some(pid) = parsed_body.player_id {
        current.player_id = pid;
    }
    if let Some(ref st_str) = parsed_body.state {
        match st_str.parse::<PlaybackState>() {
            Ok(ps) => current.state = ps,
            Err(e) => return err(400, &e),
        }
    }
    if let Some(title) = parsed_body.title {
        current.title = title;
    }
    if let Some(artist) = parsed_body.artist {
        current.artist = artist;
    }
    if let Some(album) = parsed_body.album {
        current.album = album;
    }
    if let Some(pos) = parsed_body.position_ms {
        current.position_ms = pos;
    }
    if let Some(dur) = parsed_body.duration_ms {
        current.duration_ms = dur;
    }
    if let Some(vol) = parsed_body.volume_percent {
        current.volume_percent = vol;
    }
    if let Some(muted) = parsed_body.muted {
        current.muted = muted;
    }
    if let Some(cp) = parsed_body.can_play {
        current.can_play = cp;
    }
    if let Some(cp) = parsed_body.can_pause {
        current.can_pause = cp;
    }
    if let Some(cgn) = parsed_body.can_go_next {
        current.can_go_next = cgn;
    }
    if let Some(cgp) = parsed_body.can_go_previous {
        current.can_go_previous = cgp;
    }
    if let Some(cs) = parsed_body.can_seek {
        current.can_seek = cs;
    }

    match node.media_update_local_state(current) {
        Ok(st) => json_ok(&serde_json::json!({
            "ok": true,
            "state": st,
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_media_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.media_policy_report())
}

pub(super) async fn post_media_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MediaPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<MediaPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected MediaPolicyBody"),
        }
    };
    let mut policy = node.media_policy();

    if let Some(ref b) = parsed_body {
        if let Some(ref m) = b.mode {
            match m.parse::<tetherly_core::MediaControlMode>() {
                Ok(mode) => policy.mode = mode,
                Err(e) => return err(400, &format!("invalid media control mode '{m}': {e}")),
            }
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
        if let Some(limit) = b.max_volume_limit {
            policy.max_volume_limit = limit.min(100);
        }
        if let Some(seek) = b.allow_seek {
            policy.allow_seek = seek;
        }
        if b.clear_filters == Some(true) {
            policy.allowed_groups.clear();
            policy.allowed_tags.clear();
            policy.allowed_types.clear();
        }
    }

    if let Some(m_str) = qmap.get("mode") {
        match m_str.parse::<tetherly_core::MediaControlMode>() {
            Ok(mode) => policy.mode = mode,
            Err(e) => return err(400, &format!("invalid media control mode '{m_str}': {e}")),
        }
    }
    if let Some(g) = qmap.get("group") {
        if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
            policy.allowed_groups.push(g.clone());
        }
    }
    if let Some(t) = qmap.get("tag") {
        if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
            policy.allowed_tags.push(t.clone());
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
    if let Some(lim_str) = qmap
        .get("max_volume_limit")
        .or_else(|| qmap.get("max_volume"))
    {
        if let Ok(l) = lim_str.parse::<u8>() {
            policy.max_volume_limit = l.min(100);
        }
    }
    if let Some(seek_str) = qmap.get("allow_seek") {
        if let Ok(s) = seek_str.parse::<bool>() {
            policy.allow_seek = s;
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

    match node.set_media_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
