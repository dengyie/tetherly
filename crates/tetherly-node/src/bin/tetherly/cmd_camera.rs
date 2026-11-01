// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Camera stream status/attach/detach/policy/watch.

use super::*;

/// `tetherly camera` -- Camera arm of the top-level dispatch.
pub(crate) async fn camera(action: CameraCmd) -> Result<()> {
    match action {
        CameraCmd::Stream { action } => match action {
            CameraStreamSubCmd::Status {
                peer,
                json,
                ui_port,
            } => {
                do_camera_status(ui_port, peer.as_deref(), json).await?;
            }
            CameraStreamSubCmd::Attach {
                peer,
                lens,
                width,
                height,
                fps,
                codec,
                torch,
                zoom,
                json,
                ui_port,
            } => {
                do_camera_attach(
                    ui_port, &peer, lens, width, height, fps, codec, torch, zoom, json,
                )
                .await?;
            }
            CameraStreamSubCmd::Detach {
                reason,
                json,
                ui_port,
            } => {
                do_camera_detach(ui_port, reason, json).await?;
            }
            CameraStreamSubCmd::Policy {
                action,
                enable,
                allow_inbound,
                allow_capture,
                max_width,
                max_height,
                max_fps,
                privacy_shutter,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_camera_policy(
                    ui_port,
                    action,
                    enable,
                    allow_inbound,
                    allow_capture,
                    max_width,
                    max_height,
                    max_fps,
                    privacy_shutter,
                    timeout,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            CameraStreamSubCmd::Watch { json, ui_port } => {
                do_camera_watch(ui_port, json).await?;
            }
        },
        CameraCmd::Status {
            peer,
            json,
            ui_port,
        } => {
            do_camera_status(ui_port, peer.as_deref(), json).await?;
        }
        CameraCmd::Attach {
            peer,
            lens,
            width,
            height,
            fps,
            codec,
            torch,
            zoom,
            json,
            ui_port,
        } => {
            do_camera_attach(
                ui_port, &peer, lens, width, height, fps, codec, torch, zoom, json,
            )
            .await?;
        }
        CameraCmd::Detach {
            reason,
            json,
            ui_port,
        } => {
            do_camera_detach(ui_port, reason, json).await?;
        }
        CameraCmd::Policy {
            action,
            enable,
            allow_inbound,
            allow_capture,
            max_width,
            max_height,
            max_fps,
            privacy_shutter,
            timeout,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_camera_policy(
                ui_port,
                action,
                enable,
                allow_inbound,
                allow_capture,
                max_width,
                max_height,
                max_fps,
                privacy_shutter,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        CameraCmd::Watch { json, ui_port } => {
            do_camera_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_camera_status(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/camera".to_string();
    if let Some(p) = peer {
        path = format!("/api/camera?peer={p}");
    }
    let (code, body) = tetherly_node::loopback_get(ui_port, &path).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual camera status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Virtual Camera Status:");
    println!("  Active Session: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer_id = sess["peer_id"].as_str().unwrap_or("unknown");
        let lens_id = sess["lens_id"].as_str().unwrap_or("default");
        let facing = sess["facing"].as_str().unwrap_or("unknown");
        let w = sess["width"].as_u64().unwrap_or(1280);
        let h = sess["height"].as_u64().unwrap_or(720);
        let target_fps = sess["target_fps"].as_u64().unwrap_or(30);
        let current_fps = sess["current_fps"].as_u64().unwrap_or(target_fps);
        let codec = sess["codec"].as_str().unwrap_or("mjpeg");
        let torch = sess["flashlight"].as_bool().unwrap_or(false);
        let zoom = sess["zoom_ratio"].as_f64().unwrap_or(1.0);
        let shutter = sess["privacy_shutter_active"].as_bool().unwrap_or(true);
        let frames_tx = sess["frames_sent"].as_u64().unwrap_or(0);
        let frames_rx = sess["frames_received"].as_u64().unwrap_or(0);
        let bytes = sess["bytes_transferred"].as_u64().unwrap_or(0);
        println!("  Session ID:      {sid}");
        println!("  Peer:            {peer_id}");
        println!("  Lens:            {lens_id} ({facing})");
        println!("  Resolution:      {w}x{h} (target: {target_fps}fps, current: {current_fps}fps)");
        println!("  Codec:           {codec}");
        println!("  Flashlight:      {}", if torch { "ON" } else { "OFF" });
        println!("  Zoom Ratio:      {zoom:.1}x");
        println!(
            "  Privacy Shutter: {}",
            if shutter {
                "ACTIVE (Indicator On)"
            } else {
                "OFF"
            }
        );
        println!("  Transferred:     {bytes} bytes (tx: {frames_tx}, rx: {frames_rx} frames)");
    }
    if let Some(lenses) = v
        .get("discovered_lenses")
        .and_then(|l| l.as_array())
        .filter(|l| !l.is_empty())
    {
        println!("  Discovered Lenses:");
        for lens in lenses {
            let id = lens["lens_id"].as_str().unwrap_or("unknown");
            let name = lens["name"].as_str().unwrap_or("Camera");
            let facing = lens["facing"].as_str().unwrap_or("unknown");
            let max_fps = lens["max_fps"].as_u64().unwrap_or(30);
            let torch = lens["has_flashlight"].as_bool().unwrap_or(false);
            let min_z = lens["min_zoom"].as_f64().unwrap_or(1.0);
            let max_z = lens["max_zoom"].as_f64().unwrap_or(1.0);
            println!("    - [{id}] {name} ({facing}, max {max_fps}fps, torch: {torch}, zoom: {min_z:.1}x-{max_z:.1}x)");
        }
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let allow_in = pol["allow_inbound_stream"].as_bool().unwrap_or(true);
        let allow_cap = pol["allow_remote_capture"].as_bool().unwrap_or(true);
        let max_w = pol["max_resolution_width"].as_u64().unwrap_or(3840);
        let max_h = pol["max_resolution_height"].as_u64().unwrap_or(2160);
        let max_fps = pol["max_fps"].as_u64().unwrap_or(60);
        let shutter = pol["enforce_privacy_shutter_indicator"]
            .as_bool()
            .unwrap_or(true);
        let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
        println!("Policy:");
        println!("  Enabled:         {enabled}");
        println!("  Allow Inbound:   {allow_in} (stream from remote device)");
        println!("  Allow Capture:   {allow_cap} (remote capture enabled)");
        println!("  Max Resolution:  {max_w}x{max_h}");
        println!("  Max FPS:         {max_fps}");
        println!(
            "  Privacy Shutter: {}",
            if shutter { "Enforced" } else { "Optional" }
        );
        println!("  Idle Timeout:    {timeout}s");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_camera_attach(
    ui_port: u16,
    peer: &str,
    lens_id: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    codec: Option<String>,
    flashlight: bool,
    zoom: Option<f32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
    });
    if let Some(l) = lens_id {
        req["lens_id"] = serde_json::json!(l);
    }
    if let Some(w) = width {
        req["width"] = serde_json::json!(w);
    }
    if let Some(h) = height {
        req["height"] = serde_json::json!(h);
    }
    if let Some(f) = fps {
        req["fps"] = serde_json::json!(f);
    }
    if let Some(c) = codec {
        req["codec"] = serde_json::json!(c);
    }
    if flashlight {
        req["flashlight"] = serde_json::json!(true);
    }
    if let Some(z) = zoom {
        req["zoom"] = serde_json::json!(z);
    }
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/camera/attach", &serde_json::to_vec(&req)?)
            .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to attach virtual camera (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer_id"].as_str().unwrap_or(peer);
        let msg = v["message"]
            .as_str()
            .unwrap_or("virtual camera stream attached");
        println!("Virtual camera stream attached successfully.");
        println!("  Session ID: {sid}");
        println!("  Peer:       {peer_val}");
        println!("  Message:    {msg}");
    }
    Ok(())
}

pub(crate) async fn do_camera_detach(
    ui_port: u16,
    reason: Option<String>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({});
    if let Some(r) = reason {
        req["reason"] = serde_json::json!(r);
    }
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/camera/detach", &serde_json::to_vec(&req)?)
            .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to detach virtual camera (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let msg = v["message"].as_str().unwrap_or("detached");
        println!("Virtual camera detached: {msg}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_camera_policy(
    ui_port: u16,
    action: Option<CameraPolicySubCmd>,
    enable: Option<bool>,
    allow_inbound: Option<bool>,
    allow_capture: Option<bool>,
    max_width: Option<u32>,
    max_height: Option<u32>,
    max_fps: Option<u32>,
    privacy_shutter: Option<bool>,
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
            CameraPolicySubCmd::Show {
                json: j,
                ui_port: p,
            } => {
                let (code, body) = tetherly_node::loopback_get(p, "/api/camera/policy").await?;
                if code != 200 {
                    let err_msg = String::from_utf8_lossy(&body);
                    anyhow::bail!("failed to query virtual camera policy (HTTP {code}): {err_msg}");
                }
                if j {
                    println!("{}", String::from_utf8_lossy(&body));
                } else {
                    print_camera_policy_value(&body)?;
                }
                return Ok(());
            }
            CameraPolicySubCmd::Set {
                enable: e,
                allow_inbound: ai,
                allow_capture: ac,
                max_width: mw,
                max_height: mh,
                max_fps: mf,
                privacy_shutter: ps,
                timeout: t,
                group: g,
                tag: tg,
                device_type: dt,
                clear_filters: cf,
                json: j,
                ui_port: p,
            } => {
                return Box::pin(do_camera_policy(
                    p, None, e, ai, ac, mw, mh, mf, ps, t, g, tg, dt, cf, j,
                ))
                .await;
            }
        }
    } else {
        if let Some(en) = enable {
            req.insert("enabled".into(), serde_json::json!(en));
            update_needed = true;
        }
        if let Some(ai) = allow_inbound {
            req.insert("allow_inbound_stream".into(), serde_json::json!(ai));
            update_needed = true;
        }
        if let Some(ac) = allow_capture {
            req.insert("allow_remote_capture".into(), serde_json::json!(ac));
            update_needed = true;
        }
        if let Some(mw) = max_width {
            req.insert("max_resolution_width".into(), serde_json::json!(mw));
            update_needed = true;
        }
        if let Some(mh) = max_height {
            req.insert("max_resolution_height".into(), serde_json::json!(mh));
            update_needed = true;
        }
        if let Some(mf) = max_fps {
            req.insert("max_fps".into(), serde_json::json!(mf));
            update_needed = true;
        }
        if let Some(ps) = privacy_shutter {
            req.insert(
                "enforce_privacy_shutter_indicator".into(),
                serde_json::json!(ps),
            );
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
        let (code, body) =
            tetherly_node::loopback_post(ui_port, "/api/camera/policy", &serde_json::to_vec(&req)?)
                .await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update virtual camera policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
    }

    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/camera/policy").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual camera policy (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    print_camera_policy_value(&body)
}

pub(crate) fn print_camera_policy_value(body: &[u8]) -> Result<()> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let pol = v.get("policy").unwrap_or(&v);
    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_in = pol["allow_inbound_stream"].as_bool().unwrap_or(true);
    let allow_cap = pol["allow_remote_capture"].as_bool().unwrap_or(true);
    let max_w = pol["max_resolution_width"].as_u64().unwrap_or(3840);
    let max_h = pol["max_resolution_height"].as_u64().unwrap_or(2160);
    let max_fps = pol["max_fps"].as_u64().unwrap_or(60);
    let shutter = pol["enforce_privacy_shutter_indicator"]
        .as_bool()
        .unwrap_or(true);
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

    println!("Virtual Camera Governance Policy:");
    println!("  Enabled:                 {enabled}");
    println!("  Allow Inbound Stream:    {allow_in} (act as camera sink)");
    println!("  Allow Remote Capture:    {allow_cap} (remote lens activation)");
    println!("  Max Resolution:          {max_w}x{max_h}");
    println!("  Max FPS:                 {max_fps}");
    println!(
        "  Privacy Shutter Guard:   {}",
        if shutter { "Enforced" } else { "Optional" }
    );
    println!("  Idle Timeout (s):        {timeout}");
    println!(
        "  Allowed Groups:          {}",
        if groups.is_empty() {
            "(all groups allowed)".to_string()
        } else {
            groups.join(", ")
        }
    );
    println!(
        "  Allowed Tags:            {}",
        if tags.is_empty() {
            "(all tags allowed)".to_string()
        } else {
            tags.join(", ")
        }
    );
    println!(
        "  Allowed Types:           {}",
        if types.is_empty() {
            "(all types allowed)".to_string()
        } else {
            types.join(", ")
        }
    );
    Ok(())
}

pub(crate) async fn do_camera_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
        })?;
    if !json {
        println!("Streaming live virtual camera events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "camera_session_started"
                || kind == "camera_session_stopped"
                || kind == "camera_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "camera_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let lens = v["lens_id"].as_str().unwrap_or("default");
                            let w = v["width"].as_u64().unwrap_or(1280);
                            let h = v["height"].as_u64().unwrap_or(720);
                            let fps = v["target_fps"].as_u64().unwrap_or(30);
                            let codec = v["codec"].as_str().unwrap_or("mjpeg");
                            let shutter = v["privacy_shutter_active"].as_bool().unwrap_or(true);
                            println!("[camera_session_started] session={sid} peer={peer} lens={lens} {w}x{h}@{fps}fps codec={codec} shutter={shutter}");
                        }
                        "camera_session_stopped" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("none");
                            println!("[camera_session_stopped] session={sid} peer={peer} reason={reason}");
                        }
                        "camera_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let allow_in = v["allow_inbound_stream"].as_bool().unwrap_or(true);
                            let shutter = v["enforce_privacy_shutter_indicator"]
                                .as_bool()
                                .unwrap_or(true);
                            println!("[camera_policy_changed] enabled={en} allow_inbound={allow_in} shutter_enforced={shutter}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
