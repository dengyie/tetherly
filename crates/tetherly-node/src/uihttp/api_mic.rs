// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Microphone attach/detach, gain and mic policy.

use super::*;

pub(super) async fn get_mic(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.microphone_status())
}

pub(super) async fn post_mic_attach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MicAttachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<MicAttachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected MicAttachBody"),
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

    let mic_id = parsed_body
        .as_ref()
        .and_then(|b| b.mic_id.clone())
        .or_else(|| qmap.get("mic_id").cloned())
        .or_else(|| qmap.get("mic").cloned());

    let mode = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned())
        .map(|m| tetherly_core::MicMode::from_str_lenient(&m))
        .unwrap_or(tetherly_core::MicMode::VirtualMic);

    let codec = parsed_body
        .as_ref()
        .and_then(|b| b.codec.clone())
        .or_else(|| qmap.get("codec").cloned())
        .map(|c| match c.to_ascii_lowercase().as_str() {
            "pcm16" | "pcm" | "raw" => MicCodec::Pcm16,
            "aac_light" | "aac-light" | "aac" => MicCodec::AacLight,
            _ => MicCodec::Opus,
        });

    let sample_rate = parsed_body
        .as_ref()
        .and_then(|b| b.sample_rate)
        .or_else(|| qmap.get("sample_rate").and_then(|s| s.parse::<u32>().ok()));

    let channels = parsed_body
        .as_ref()
        .and_then(|b| b.channels)
        .or_else(|| qmap.get("channels").and_then(|s| s.parse::<u32>().ok()));

    let gain = parsed_body
        .as_ref()
        .and_then(|b| b.gain)
        .or_else(|| qmap.get("gain").and_then(|s| s.parse::<f32>().ok()));

    let muted = parsed_body
        .as_ref()
        .and_then(|b| b.muted)
        .or_else(|| qmap.get("muted").and_then(|s| s.parse::<bool>().ok()));

    let noise_suppression = parsed_body
        .as_ref()
        .and_then(|b| b.noise_suppression)
        .or_else(|| {
            qmap.get("noise_suppression")
                .or_else(|| qmap.get("ns"))
                .and_then(|s| s.parse::<bool>().ok())
        });

    let echo_cancellation = parsed_body
        .as_ref()
        .and_then(|b| b.echo_cancellation)
        .or_else(|| {
            qmap.get("echo_cancellation")
                .or_else(|| qmap.get("aec"))
                .and_then(|s| s.parse::<bool>().ok())
        });

    let auto_gain = parsed_body.as_ref().and_then(|b| b.auto_gain).or_else(|| {
        qmap.get("auto_gain")
            .or_else(|| qmap.get("agc"))
            .and_then(|s| s.parse::<bool>().ok())
    });

    match node
        .mic_attach(
            &peer,
            mic_id.as_deref(),
            mode,
            codec,
            sample_rate,
            channels,
            gain,
            muted,
            noise_suppression,
            echo_cancellation,
            auto_gain,
        )
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_mic_detach(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MicDetachBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<MicDetachBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected MicDetachBody"),
        }
    };

    let reason = parsed_body
        .as_ref()
        .and_then(|b| b.reason.clone())
        .or_else(|| qmap.get("reason").cloned());

    match node.mic_detach(reason.as_deref()).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_mic_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.microphone_policy_report())
}

pub(super) async fn post_mic_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MicPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<MicPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected MicPolicyBody"),
        }
    };

    let mut policy = node.microphone_policy();
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
        if let Some(ic) = b.allow_intercom {
            policy.allow_intercom = ic;
        }
        if let Some(r) = b.max_sample_rate {
            policy.max_sample_rate = r;
        }
        if let Some(d) = b.max_session_duration_secs {
            policy.max_session_duration_secs = d;
        }
        if let Some(p) = b.enforce_privacy_indicator {
            policy.enforce_privacy_indicator = p;
        }
        if let Some(m) = b.default_muted {
            policy.default_muted = m;
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
    if let Some(ic) = qmap
        .get("allow_intercom")
        .or_else(|| qmap.get("intercom"))
        .or_else(|| qmap.get("ptt"))
    {
        if let Ok(b) = ic.parse::<bool>() {
            policy.allow_intercom = b;
        }
    }
    if let Some(r_str) = qmap.get("max_sample_rate").or_else(|| qmap.get("max_rate")) {
        if let Ok(r) = r_str.parse::<u32>() {
            policy.max_sample_rate = r;
        }
    }
    if let Some(d_str) = qmap
        .get("max_session_duration_secs")
        .or_else(|| qmap.get("max_duration"))
    {
        if let Ok(d) = d_str.parse::<u32>() {
            policy.max_session_duration_secs = d;
        }
    }
    if let Some(p_str) = qmap
        .get("enforce_privacy_indicator")
        .or_else(|| qmap.get("privacy_indicator"))
        .or_else(|| qmap.get("privacy"))
    {
        if let Ok(b) = p_str.parse::<bool>() {
            policy.enforce_privacy_indicator = b;
        }
    }
    if let Some(m_str) = qmap
        .get("default_muted")
        .or_else(|| qmap.get("start_muted"))
    {
        if let Ok(b) = m_str.parse::<bool>() {
            policy.default_muted = b;
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

    match node.set_microphone_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
