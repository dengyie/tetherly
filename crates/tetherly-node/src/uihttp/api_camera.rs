// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Camera attach/detach, frames and camera policy.

use super::*;

pub(super) async fn get_camera(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.camera_status())
}

pub(super) async fn post_camera_attach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<CameraAttachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<CameraAttachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected CameraAttachBody"),
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

    let lens_id = parsed_body
        .as_ref()
        .and_then(|b| b.lens_id.clone())
        .or_else(|| qmap.get("lens_id").cloned())
        .or_else(|| qmap.get("lens").cloned());

    let width = parsed_body
        .as_ref()
        .and_then(|b| b.width)
        .or_else(|| qmap.get("width").and_then(|s| s.parse::<u32>().ok()));

    let height = parsed_body
        .as_ref()
        .and_then(|b| b.height)
        .or_else(|| qmap.get("height").and_then(|s| s.parse::<u32>().ok()));

    let fps = parsed_body.as_ref().and_then(|b| b.fps).or_else(|| {
        qmap.get("target_fps")
            .or_else(|| qmap.get("fps"))
            .and_then(|s| s.parse::<u32>().ok())
    });

    let codec = parsed_body
        .as_ref()
        .and_then(|b| b.codec.clone())
        .or_else(|| qmap.get("codec").cloned())
        .and_then(|c| match c.to_ascii_lowercase().as_str() {
            "mjpeg" => Some(CameraCodec::Mjpeg),
            "zstd_rgb" | "zstd-rgb" | "zstd" => Some(CameraCodec::ZstdRgb),
            "raw_nv12" | "raw-nv12" | "nv12" => Some(CameraCodec::RawNv12),
            "h264_light" | "h264-light" | "h264" => Some(CameraCodec::H264Light),
            _ => None,
        });

    let flashlight = parsed_body.as_ref().and_then(|b| b.flashlight).or_else(|| {
        qmap.get("flashlight")
            .or_else(|| qmap.get("torch"))
            .or_else(|| qmap.get("flashlight_on"))
            .and_then(|s| s.parse::<bool>().ok())
    });

    let zoom = parsed_body.as_ref().and_then(|b| b.zoom).or_else(|| {
        qmap.get("zoom")
            .or_else(|| qmap.get("zoom_ratio"))
            .and_then(|s| s.parse::<f32>().ok())
    });

    match node
        .camera_attach(
            &peer,
            lens_id.as_deref(),
            width,
            height,
            fps,
            codec,
            flashlight,
            zoom,
        )
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_camera_detach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<CameraDetachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<CameraDetachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected CameraDetachBody"),
        }
    };

    let reason = parsed_body
        .as_ref()
        .and_then(|b| b.reason.clone())
        .or_else(|| qmap.get("reason").cloned());

    match node.camera_detach(reason.as_deref()).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_camera_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.camera_policy_report())
}

pub(super) async fn post_camera_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<CameraPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<CameraPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected CameraPolicyBody"),
        }
    };

    let mut policy = node.camera_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled {
            policy.enabled = en;
        }
        if let Some(inbound) = b.allow_inbound_stream {
            policy.allow_inbound_stream = inbound;
        }
        if let Some(capture) = b.allow_remote_capture {
            policy.allow_remote_capture = capture;
        }
        if let Some(w) = b.max_resolution_width {
            policy.max_resolution_width = w;
        }
        if let Some(h) = b.max_resolution_height {
            policy.max_resolution_height = h;
        }
        if let Some(fps) = b.max_fps {
            policy.max_fps = fps;
        }
        if let Some(shutter) = b.enforce_privacy_shutter_indicator {
            policy.enforce_privacy_shutter_indicator = shutter;
        }
        if let Some(to) = b.idle_timeout_secs {
            policy.idle_timeout_secs = to;
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
        .get("allow_inbound_stream")
        .or_else(|| qmap.get("allow_inbound"))
        .or_else(|| qmap.get("inbound"))
    {
        if let Ok(b) = inbound.parse::<bool>() {
            policy.allow_inbound_stream = b;
        }
    }
    if let Some(capture) = qmap
        .get("allow_remote_capture")
        .or_else(|| qmap.get("allow_capture"))
        .or_else(|| qmap.get("capture"))
    {
        if let Ok(b) = capture.parse::<bool>() {
            policy.allow_remote_capture = b;
        }
    }
    if let Some(w_str) = qmap
        .get("max_resolution_width")
        .or_else(|| qmap.get("max_width"))
    {
        if let Ok(w) = w_str.parse::<u32>() {
            policy.max_resolution_width = w;
        }
    }
    if let Some(h_str) = qmap
        .get("max_resolution_height")
        .or_else(|| qmap.get("max_height"))
    {
        if let Ok(h) = h_str.parse::<u32>() {
            policy.max_resolution_height = h;
        }
    }
    if let Some(fps_str) = qmap.get("max_fps").or_else(|| qmap.get("fps")) {
        if let Ok(f) = fps_str.parse::<u32>() {
            policy.max_fps = f;
        }
    }
    if let Some(shutter_str) = qmap
        .get("enforce_privacy_shutter_indicator")
        .or_else(|| qmap.get("privacy_shutter"))
        .or_else(|| qmap.get("shutter"))
    {
        if let Ok(b) = shutter_str.parse::<bool>() {
            policy.enforce_privacy_shutter_indicator = b;
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

    match node.set_camera_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
