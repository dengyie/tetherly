// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Audio routing status/attach/detach/policy/watch.

use super::*;

/// `tetherly audio` -- Audio arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn audio(action: AudioCmd) -> Result<()> {
    match action {
        AudioCmd::Routing { action } => match action {
            AudioRoutingSubCmd::Status { json, ui_port } => {
                do_audio_routing_status(ui_port, json).await?;
            }
            AudioRoutingSubCmd::Attach {
                peer,
                layout,
                sample_rate,
                channels,
                x,
                y,
                z,
                gain,
                json,
                ui_port,
            } => {
                do_audio_routing_attach(
                    ui_port,
                    &peer,
                    layout,
                    sample_rate,
                    channels,
                    x,
                    y,
                    z,
                    gain,
                    json,
                )
                .await?;
            }
            AudioRoutingSubCmd::Detach {
                reason,
                json,
                ui_port,
            } => {
                do_audio_routing_detach(ui_port, reason, json).await?;
            }
            AudioRoutingSubCmd::Policy {
                action,
                enable,
                allow_sink,
                allow_outbound,
                max_volume,
                allow_spatial,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_audio_routing_policy(
                    ui_port,
                    action,
                    enable,
                    allow_sink,
                    allow_outbound,
                    max_volume,
                    allow_spatial,
                    timeout,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            AudioRoutingSubCmd::Watch { json, ui_port } => {
                do_audio_routing_watch(ui_port, json).await?;
            }
        },
        AudioCmd::Status { json, ui_port } => {
            do_audio_routing_status(ui_port, json).await?;
        }
        AudioCmd::Attach {
            peer,
            layout,
            sample_rate,
            channels,
            x,
            y,
            z,
            gain,
            json,
            ui_port,
        } => {
            do_audio_routing_attach(
                ui_port,
                &peer,
                layout,
                sample_rate,
                channels,
                x,
                y,
                z,
                gain,
                json,
            )
            .await?;
        }
        AudioCmd::Detach {
            reason,
            json,
            ui_port,
        } => {
            do_audio_routing_detach(ui_port, reason, json).await?;
        }
        AudioCmd::Policy {
            action,
            enable,
            allow_sink,
            allow_outbound,
            max_volume,
            allow_spatial,
            timeout,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_audio_routing_policy(
                ui_port,
                action,
                enable,
                allow_sink,
                allow_outbound,
                max_volume,
                allow_spatial,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        AudioCmd::Watch { json, ui_port } => {
            do_audio_routing_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_audio_routing_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/audio/routing").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query audio routing status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Spatial Audio Routing Status:");
    println!("  Active Session: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer = sess["peer_id"].as_str().unwrap_or("unknown");
        let role = sess["role"].as_str().unwrap_or("unknown");
        let layout = sess["layout"].as_str().unwrap_or("stereo");
        let rate = sess["sample_rate"].as_u64().unwrap_or(48000);
        let ch = sess["channels"].as_u64().unwrap_or(2);
        let fmt = sess["format"].as_str().unwrap_or("s16le");
        let codec = sess["codec"].as_str().unwrap_or("zstd_pcm");
        let gain = sess["volume_gain"].as_f64().unwrap_or(1.0);
        let chunks_tx = sess["chunks_sent"].as_u64().unwrap_or(0);
        let chunks_rx = sess["chunks_received"].as_u64().unwrap_or(0);
        let bytes = sess["bytes_transferred"].as_u64().unwrap_or(0);
        println!("  Session ID:    {sid}");
        println!("  Peer:          {peer}");
        println!("  Role:          {role}");
        println!("  Layout:        {layout} ({rate}Hz, {ch}ch, {fmt}, {codec})");
        println!("  Volume Gain:   {gain:.2}");
        if let Some(pos) = sess.get("spatial_pos").filter(|p| !p.is_null()) {
            let x = pos["x"].as_f64().unwrap_or(0.0);
            let y = pos["y"].as_f64().unwrap_or(0.0);
            let z = pos["z"].as_f64().unwrap_or(0.0);
            println!("  Spatial Pos:   [{x:.2}, {y:.2}, {z:.2}]");
        }
        println!("  Transferred:   {bytes} bytes (tx: {chunks_tx}, rx: {chunks_rx} chunks)");
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let allow_sink = pol["allow_inbound_sink"].as_bool().unwrap_or(true);
        let allow_outbound = pol["allow_outbound_routing"].as_bool().unwrap_or(true);
        let max_vol = pol["max_volume_limit"].as_u64().unwrap_or(100);
        let spatial = pol["allow_spatial_rendering"].as_bool().unwrap_or(true);
        let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
        println!("Policy:");
        println!("  Enabled:        {enabled}");
        println!("  Allow Sink:     {allow_sink} (inbound playback)");
        println!("  Allow Outbound: {allow_outbound} (route audio to peers)");
        println!("  Max Volume:     {max_vol}%");
        println!("  Spatial Render: {spatial}");
        println!("  Idle Timeout:   {timeout}s");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_audio_routing_attach(
    ui_port: u16,
    peer: &str,
    layout: Option<String>,
    sample_rate: Option<u32>,
    channels: Option<u8>,
    x: Option<f32>,
    y: Option<f32>,
    z: Option<f32>,
    gain: Option<f32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
    });
    if let Some(l) = layout {
        req["layout"] = serde_json::json!(l);
    }
    if let Some(r) = sample_rate {
        req["sample_rate"] = serde_json::json!(r);
    }
    if let Some(c) = channels {
        req["channels"] = serde_json::json!(c);
    }
    if let Some(vx) = x {
        req["x"] = serde_json::json!(vx);
    }
    if let Some(vy) = y {
        req["y"] = serde_json::json!(vy);
    }
    if let Some(vz) = z {
        req["z"] = serde_json::json!(vz);
    }
    if let Some(g) = gain {
        req["gain"] = serde_json::json!(g);
    }
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/audio/routing/attach",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to attach audio routing (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer_id"].as_str().unwrap_or(peer);
        let msg = v["message"].as_str().unwrap_or("audio routing attached");
        println!("Spatial audio routing attached successfully.");
        println!("  Session ID: {sid}");
        println!("  Peer:       {peer_val}");
        println!("  Message:    {msg}");
    }
    Ok(())
}

pub(crate) async fn do_audio_routing_detach(
    ui_port: u16,
    reason: Option<String>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({});
    if let Some(r) = reason {
        req["reason"] = serde_json::json!(r);
    }
    let (code, body) = tetherly_node::loopback_post(
        ui_port,
        "/api/audio/routing/detach",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to detach audio routing (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let msg = v["message"].as_str().unwrap_or("detached");
        println!("Spatial audio routing detached: {msg}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_audio_routing_policy(
    ui_port: u16,
    action: Option<AudioRoutingPolicySubCmd>,
    enable: Option<bool>,
    allow_sink: Option<bool>,
    allow_outbound: Option<bool>,
    max_volume: Option<u8>,
    allow_spatial: Option<bool>,
    timeout: Option<u32>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let mut update_needed = false;
    let mut req = serde_json::Map::new();

    if let Some(action) = action {
        match action {
            AudioRoutingPolicySubCmd::Show {
                json: j,
                ui_port: p,
            } => {
                let (code, body) =
                    tetherly_node::loopback_get(p, "/api/audio/routing/policy").await?;
                if code != 200 {
                    let err_msg = String::from_utf8_lossy(&body);
                    anyhow::bail!("failed to query audio routing policy (HTTP {code}): {err_msg}");
                }
                if j {
                    println!("{}", String::from_utf8_lossy(&body));
                } else {
                    print_audio_routing_policy_value(&body)?;
                }
                return Ok(());
            }
            AudioRoutingPolicySubCmd::Set {
                enable: e,
                allow_sink: s,
                allow_outbound: o,
                max_volume: mv,
                allow_spatial: sp,
                timeout: t,
                group: g,
                tag: tg,
                device_type: dt,
                clear_filters: cf,
                json: j,
                ui_port: p,
            } => {
                return Box::pin(do_audio_routing_policy(
                    p, None, e, s, o, mv, sp, t, g, tg, dt, cf, j,
                ))
                .await;
            }
        }
    } else {
        if let Some(en) = enable {
            req.insert("enabled".into(), serde_json::json!(en));
            update_needed = true;
        }
        if let Some(s) = allow_sink {
            req.insert("allow_inbound_sink".into(), serde_json::json!(s));
            update_needed = true;
        }
        if let Some(o) = allow_outbound {
            req.insert("allow_outbound_routing".into(), serde_json::json!(o));
            update_needed = true;
        }
        if let Some(mv) = max_volume {
            req.insert("max_volume_limit".into(), serde_json::json!(mv));
            update_needed = true;
        }
        if let Some(sp) = allow_spatial {
            req.insert("allow_spatial_rendering".into(), serde_json::json!(sp));
            update_needed = true;
        }
        if let Some(to) = timeout {
            req.insert("idle_timeout_secs".into(), serde_json::json!(to));
            update_needed = true;
        }
        if let Some(grps) = group {
            req.insert("allowed_groups".into(), serde_json::json!(grps));
            update_needed = true;
        }
        if let Some(tags) = tag {
            req.insert("allowed_tags".into(), serde_json::json!(tags));
            update_needed = true;
        }
        if let Some(types) = device_type {
            req.insert("allowed_types".into(), serde_json::json!(types));
            update_needed = true;
        }
        if clear_filters {
            req.insert("clear_filters".into(), serde_json::json!(true));
            update_needed = true;
        }
    }

    if update_needed {
        let (code, body) = tetherly_node::loopback_post(
            ui_port,
            "/api/audio/routing/policy",
            &serde_json::to_vec(&req)?,
        )
        .await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update audio routing policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
    }

    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/audio/routing/policy").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query audio routing policy (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    print_audio_routing_policy_value(&body)
}

pub(crate) fn print_audio_routing_policy_value(body: &[u8]) -> Result<()> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let pol = v.get("policy").unwrap_or(&v);
    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_sink = pol["allow_inbound_sink"].as_bool().unwrap_or(true);
    let allow_outbound = pol["allow_outbound_routing"].as_bool().unwrap_or(true);
    let max_vol = pol["max_volume_limit"].as_u64().unwrap_or(100);
    let spatial = pol["allow_spatial_rendering"].as_bool().unwrap_or(true);
    let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
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

    println!("Spatial Audio Routing Governance Policy:");
    println!("  Enabled:              {enabled}");
    println!("  Allow Inbound Sink:   {allow_sink} (act as speaker)");
    println!("  Allow Outbound Route: {allow_outbound} (route to peers)");
    println!("  Max Volume Limit:     {max_vol}%");
    println!("  Spatial Rendering:    {spatial}");
    println!("  Idle Timeout (s):     {timeout}");
    println!(
        "  Allowed Groups:       {}",
        if groups.is_empty() {
            "(all groups allowed)".to_string()
        } else {
            groups.join(", ")
        }
    );
    println!(
        "  Allowed Tags:         {}",
        if tags.is_empty() {
            "(all tags allowed)".to_string()
        } else {
            tags.join(", ")
        }
    );
    println!(
        "  Allowed Types:        {}",
        if types.is_empty() {
            "(all types allowed)".to_string()
        } else {
            types.join(", ")
        }
    );
    Ok(())
}

pub(crate) async fn do_audio_routing_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
        })?;
    if !json {
        println!("Streaming live spatial audio routing events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "audio_routing_session_started"
                || kind == "audio_routing_session_stopped"
                || kind == "audio_routing_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "audio_routing_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let role = v["role"].as_str().unwrap_or("unknown");
                            let layout = v["layout"].as_str().unwrap_or("stereo");
                            let rate = v["sample_rate"].as_u64().unwrap_or(0);
                            let ch = v["channels"].as_u64().unwrap_or(0);
                            let gain = v["volume_gain"].as_f64().unwrap_or(1.0);
                            println!("[audio_routing_session_started] session={sid} peer={peer} role={role} layout={layout} {rate}Hz/{ch}ch gain={gain:.2}");
                        }
                        "audio_routing_session_stopped" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("none");
                            println!("[audio_routing_session_stopped] session={sid} peer={peer} reason={reason}");
                        }
                        "audio_routing_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let sink = v["allow_inbound_sink"].as_bool().unwrap_or(true);
                            let outbound = v["allow_outbound_routing"].as_bool().unwrap_or(true);
                            let max_vol = v["max_volume_limit"].as_u64().unwrap_or(100);
                            let spatial = v["allow_spatial_rendering"].as_bool().unwrap_or(true);
                            println!("[audio_routing_policy_changed] enabled={en} allow_sink={sink} allow_outbound={outbound} max_volume={max_vol} spatial={spatial}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
