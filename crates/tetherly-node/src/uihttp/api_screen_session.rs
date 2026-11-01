// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen session lifecycle: start/stop, keyframe, capture, LTR, SVC, switch.

use super::*;

pub(super) async fn get_screen(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_status_report().await;
    json_ok(&rep)
}

pub(super) async fn post_screen_allow(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_allow_reported();
    json_ok(&rep)
}

pub(super) async fn post_screen_revoke(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_revoke_reported();
    json_ok(&rep)
}

pub(super) async fn post_screen_start(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let target_opt = if !body.is_empty() {
        match serde_json::from_slice::<PeerTargetBody>(body) {
            Ok(b) => b.peer,
            Err(_) => return err(400, "bad json"),
        }
    } else {
        None
    };
    match node.screen_start_reported(target_opt.as_deref()).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_screen_stop(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_stop_reported().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_screen_keyframe(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_keyframe_request_reported().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_screen_displays(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_display_report();
    json_ok(&rep)
}

// Screen privacy mode, display blanking & host input lockout pipeline (v2.12 / ADR-098).
pub(super) async fn post_screen_switch(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct SwitchBody {
        display_id: Option<u8>,
        display: Option<u8>,
        id: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<SwitchBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let display_id_opt = parsed_body
        .as_ref()
        .and_then(|b| b.display_id.or(b.display).or(b.id))
        .or_else(|| {
            qmap.get("display_id")
                .or_else(|| qmap.get("display"))
                .or_else(|| qmap.get("id"))
                .and_then(|v| v.parse::<u8>().ok())
        });

    let Some(display_id) = display_id_opt else {
        return err(
            400,
            "missing required field: display_id (expected integer 0-255)",
        );
    };

    match node.screen_switch_display(display_id).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => {
            let status = match &e {
                tetherly_core::CoreError::ScreenRefused(_) => 404,
                _ => 400,
            };
            err(status, &format!("failed to switch display: {e}"))
        }
    }
}

pub(super) async fn post_screen_recording_start(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct RecordingStartBody {
        path: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<RecordingStartBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let path_opt = parsed_body
        .as_ref()
        .and_then(|b| b.path.clone())
        .or_else(|| qmap.get("path").cloned());
    match node.screen_start_recording(path_opt).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("failed to start screen recording: {e}")),
    }
}

pub(super) async fn post_screen_recording_stop(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_stop_recording().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to stop screen recording: {e}")),
    }
}

pub(super) async fn get_screen_recording(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_recording_report();
    json_ok(&rep)
}

pub(super) async fn get_screen_recording_replay(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let Some(path) = qmap.get("path").cloned() else {
        return err(400, "recording replay requires a ?path= parameter");
    };
    let limit = qmap
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(32);
    match node.screen_recording_replay(path, limit) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("recording replay failed: {e}")),
    }
}

pub(super) async fn post_screen_playback_start(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct PlaybackStartBody {
        path: Option<String>,
        speed: Option<f32>,
        #[serde(rename = "loop")]
        loop_enabled: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PlaybackStartBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let path_opt = parsed_body
        .as_ref()
        .and_then(|b| b.path.clone())
        .or_else(|| qmap.get("path").cloned());
    let speed_opt = parsed_body
        .as_ref()
        .and_then(|b| b.speed)
        .or_else(|| qmap.get("speed").and_then(|v| v.parse::<f32>().ok()));
    let loop_opt = parsed_body
        .as_ref()
        .and_then(|b| b.loop_enabled)
        .or_else(|| qmap.get("loop").and_then(|v| v.parse::<bool>().ok()));

    let Some(path) = path_opt else {
        return err(400, "playback requires a path parameter or JSON body");
    };

    match node.screen_playback_start(path, speed_opt, loop_opt).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("failed to start screen playback: {e}")),
    }
}

pub(super) async fn post_screen_playback_pause(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_playback_pause() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("failed to pause screen playback: {e}")),
    }
}

pub(super) async fn post_screen_playback_resume(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_playback_resume() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("failed to resume screen playback: {e}")),
    }
}

pub(super) async fn post_screen_playback_stop(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_playback_stop() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to stop screen playback: {e}")),
    }
}

pub(super) async fn get_screen_playback(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_playback_report();
    json_ok(&rep)
}

pub(super) async fn get_screen_snapshot(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let format_param = qmap
        .get("format")
        .cloned()
        .unwrap_or_else(|| "json".to_string());
    let out_param = qmap.get("out").cloned();

    if format_param.eq_ignore_ascii_case("bmp") {
        match node.screen_snapshot_data(Some("bmp".to_string()), out_param) {
            Ok((bmp_bytes, _rep)) => UiResponse::new(200, "image/bmp", bmp_bytes),
            Err(tetherly_core::CoreError::ScreenRefused(msg))
                if msg.contains("no screen frame available") =>
            {
                err(503, &format!("screen snapshot failed: {msg}"))
            }
            Err(e) => err(400, &format!("screen snapshot failed: {e}")),
        }
    } else {
        match node.screen_snapshot(Some(format_param), out_param) {
            Ok(rep) => json_ok(&rep),
            Err(tetherly_core::CoreError::ScreenRefused(msg))
                if msg.contains("no screen frame available") =>
            {
                err(503, &format!("screen snapshot failed: {msg}"))
            }
            Err(e) => err(400, &format!("screen snapshot failed: {e}")),
        }
    }
}

pub(super) async fn post_screen_recording_snapshot(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct RecordingSnapshotBody {
        path: Option<String>,
        frame: Option<u64>,
        pts: Option<u64>,
        pts_us: Option<u64>,
        out: Option<String>,
        format: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<RecordingSnapshotBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let path_opt = parsed_body
        .as_ref()
        .and_then(|b| b.path.clone())
        .or_else(|| qmap.get("path").cloned());
    let frame_opt = parsed_body
        .as_ref()
        .and_then(|b| b.frame)
        .or_else(|| qmap.get("frame").and_then(|v| v.parse::<u64>().ok()));
    let pts_opt = parsed_body
        .as_ref()
        .and_then(|b| b.pts.or(b.pts_us))
        .or_else(|| {
            qmap.get("pts")
                .or_else(|| qmap.get("pts_us"))
                .and_then(|v| v.parse::<u64>().ok())
        });
    let out_opt = parsed_body
        .as_ref()
        .and_then(|b| b.out.clone())
        .or_else(|| qmap.get("out").cloned());
    let format_opt = parsed_body
        .as_ref()
        .and_then(|b| b.format.clone())
        .or_else(|| qmap.get("format").cloned());

    let Some(path) = path_opt else {
        return err(
            400,
            "recording snapshot requires a path parameter or JSON body",
        );
    };

    let is_bmp_direct = format_opt
        .as_deref()
        .map(|f| f.eq_ignore_ascii_case("bmp"))
        .unwrap_or(false);

    if is_bmp_direct {
        match node.screen_recording_snapshot_data(&path, frame_opt, pts_opt, out_opt) {
            Ok((bmp_bytes, _rep)) => UiResponse::new(200, "image/bmp", bmp_bytes),
            Err(e) => err(400, &format!("screen recording snapshot failed: {e}")),
        }
    } else {
        match node.screen_recording_snapshot(&path, frame_opt, pts_opt, out_opt) {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &format!("screen recording snapshot failed: {e}")),
        }
    }
}

pub(super) async fn get_screen_ltr(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_ltr_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_ltr(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct LtrBody {
        enabled: Option<bool>,
        auto_interval: Option<u16>,
        interval: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<LtrBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let interval = parsed_body
        .as_ref()
        .and_then(|b| b.auto_interval.or(b.interval))
        .or_else(|| {
            qmap.get("auto_interval")
                .or_else(|| qmap.get("interval"))
                .and_then(|v| v.parse::<u16>().ok())
        });

    match node.screen_set_ltr_mode(enabled, interval).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen ltr mode: {e}")),
    }
}

pub(super) async fn post_screen_ltr_anchor(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_anchor_ltr().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to anchor screen ltr reference: {e}")),
    }
}

pub(super) async fn post_screen_ltr_recover(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_request_ltr_recovery().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to request screen ltr recovery: {e}")),
    }
}

// Screen streaming 2-Layer Temporal Scalability (SVC-T) hierarchical reference pipeline (v2.28 / ADR-114)
pub(super) async fn post_screen_ltr_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_ltr_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen ltr mode: {e}")),
    }
}

pub(super) async fn get_screen_svc(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_svc_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_svc(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct SvcBody {
        mode: Option<String>,
        drop_t1: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<SvcBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned())
        .unwrap_or_else(|| "adaptive".to_string());
    let mode = tetherly_core::ScreenSvcMode::parse_mode(&mode_str);
    let drop_t1 = parsed_body
        .as_ref()
        .and_then(|b| b.drop_t1)
        .or_else(|| qmap.get("drop_t1").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    match node.screen_set_svc_mode(mode, drop_t1).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen svc mode: {e}")),
    }
}

pub(super) async fn post_screen_svc_drop_t1(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct DropT1Body {
        drop_t1: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<DropT1Body> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let drop_t1 = parsed_body
        .as_ref()
        .and_then(|b| b.drop_t1)
        .or_else(|| qmap.get("drop_t1").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);

    match node.screen_set_svc_drop_t1(drop_t1).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen svc drop-t1: {e}")),
    }
}

// Screen streaming 2D motion vector estimation & scroll acceleration (v2.29 / ADR-115)
pub(super) async fn post_screen_svc_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_svc_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen svc mode: {e}")),
    }
}
