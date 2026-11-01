// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Virtual display status/attach/detach/policy/watch.

use super::util::{loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli display` -- Display arm of the top-level dispatch.
pub(crate) async fn display(action: DisplayCmd) -> Result<()> {
    match action {
        DisplayCmd::Virtual { action } => match action {
            VirtualDisplaySubCmd::Status { json, ui_port } => {
                do_virtual_display_status(ui_port, json).await?;
            }
            VirtualDisplaySubCmd::Attach {
                peer,
                edge,
                width,
                height,
                fps,
                json,
                ui_port,
            } => {
                do_virtual_display_attach(ui_port, &peer, edge, width, height, fps, json).await?;
            }
            VirtualDisplaySubCmd::Detach {
                reason,
                json,
                ui_port,
            } => {
                do_virtual_display_detach(ui_port, reason, json).await?;
            }
            VirtualDisplaySubCmd::Policy {
                action,
                enable,
                allow_host,
                allow_extend,
                max_width,
                max_height,
                max_fps,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_virtual_display_policy(
                    ui_port,
                    action,
                    enable,
                    allow_host,
                    allow_extend,
                    max_width,
                    max_height,
                    max_fps,
                    timeout,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            VirtualDisplaySubCmd::Watch { json, ui_port } => {
                do_virtual_display_watch(ui_port, json).await?;
            }
        },
        DisplayCmd::Status { json, ui_port } => {
            do_virtual_display_status(ui_port, json).await?;
        }
        DisplayCmd::Attach {
            peer,
            edge,
            width,
            height,
            fps,
            json,
            ui_port,
        } => {
            do_virtual_display_attach(ui_port, &peer, edge, width, height, fps, json).await?;
        }
        DisplayCmd::Detach {
            reason,
            json,
            ui_port,
        } => {
            do_virtual_display_detach(ui_port, reason, json).await?;
        }
        DisplayCmd::Policy {
            action,
            enable,
            allow_host,
            allow_extend,
            max_width,
            max_height,
            max_fps,
            timeout,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_virtual_display_policy(
                ui_port,
                action,
                enable,
                allow_host,
                allow_extend,
                max_width,
                max_height,
                max_fps,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        DisplayCmd::Watch { json, ui_port } => {
            do_virtual_display_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_virtual_display_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/display/virtual").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual display status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Virtual Secondary Display Status:");
    println!("  Active Session: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer = sess["peer"].as_str().unwrap_or("unknown");
        let role = sess["role"].as_str().unwrap_or("unknown");
        let edge = sess["assigned_edge"].as_str().unwrap_or("unknown");
        let cur_fps = sess["current_fps"].as_u64().unwrap_or(0);
        let tgt_fps = sess["target_fps"].as_u64().unwrap_or(0);
        let rtt = sess["rtt_us"].as_u64().unwrap_or(0);
        let dropped = sess["dropped_frames"].as_u64().unwrap_or(0);
        println!("  Session ID:    {sid}");
        println!("  Peer:          {peer}");
        println!("  Role:          {role}");
        println!("  Assigned Edge: {edge}");
        if let Some(mode) = sess.get("mode") {
            let w = mode["width"].as_u64().unwrap_or(0);
            let h = mode["height"].as_u64().unwrap_or(0);
            let fmt = mode["pixel_format"].as_str().unwrap_or("Bgra8");
            println!("  Resolution:    {w}x{h} ({fmt})");
        }
        println!("  Framerate:     {cur_fps}/{tgt_fps} FPS (dropped: {dropped})");
        println!("  RTT:           {rtt} µs");
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let allow_host = pol["allow_hosting"].as_bool().unwrap_or(true);
        let allow_extend = pol["allow_remote_extend"].as_bool().unwrap_or(true);
        let max_w = pol["max_resolution_width"].as_u64().unwrap_or(3840);
        let max_h = pol["max_resolution_height"].as_u64().unwrap_or(2160);
        let max_fps = pol["max_fps"].as_u64().unwrap_or(60);
        let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
        println!("Policy:");
        println!("  Enabled:       {enabled}");
        println!("  Allow Hosting: {allow_host} (as secondary screen)");
        println!("  Allow Extend:  {allow_extend} (extend desktop)");
        println!("  Max Resolution:{max_w}x{max_h} @ {max_fps} FPS");
        println!("  Idle Timeout:  {timeout}s");
    }
    Ok(())
}

pub(crate) async fn do_virtual_display_attach(
    ui_port: u16,
    peer: &str,
    edge: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
    });
    if let Some(e) = edge {
        req["edge"] = serde_json::json!(e);
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
    let (code, body) = loopback_post(
        ui_port,
        "/api/display/virtual/attach",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to attach virtual display (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer"].as_str().unwrap_or(peer);
        let accepted = v["accepted"].as_bool().unwrap_or(false);
        if accepted {
            println!("Virtual secondary display attached successfully.");
            println!("  Session ID: {sid}");
            println!("  Peer:       {peer_val}");
            if let Some(e) = v["assigned_edge"].as_str() {
                println!("  Edge:       {e}");
            }
            if let Some(w) = v["width"].as_u64() {
                if let Some(h) = v["height"].as_u64() {
                    let fps_val = v["fps"].as_u64().unwrap_or(60);
                    println!("  Resolution: {w}x{h} @ {fps_val} FPS");
                }
            }
        } else {
            let reason = v["reject_reason"].as_str().unwrap_or("rejected");
            println!("Virtual display rejected by target: {reason}");
        }
    }
    Ok(())
}

pub(crate) async fn do_virtual_display_detach(
    ui_port: u16,
    reason: Option<String>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({});
    if let Some(r) = reason {
        req["reason"] = serde_json::json!(r);
    }
    let (code, body) = loopback_post(
        ui_port,
        "/api/display/virtual/detach",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to detach virtual display (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("none");
        let reason_val = v["reason"].as_str().unwrap_or("user requested");
        println!("Virtual secondary display detached ({reason_val}). Active session: {sid}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_virtual_display_policy(
    ui_port: u16,
    action: Option<VirtualDisplayPolicySubCmd>,
    enable: Option<bool>,
    allow_host: Option<bool>,
    allow_extend: Option<bool>,
    max_width: Option<u32>,
    max_height: Option<u32>,
    max_fps: Option<u32>,
    timeout: Option<u32>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = match &action {
        Some(VirtualDisplayPolicySubCmd::Set { .. }) => true,
        Some(VirtualDisplayPolicySubCmd::Show { .. }) => false,
        None => {
            enable.is_some()
                || allow_host.is_some()
                || allow_extend.is_some()
                || max_width.is_some()
                || max_height.is_some()
                || max_fps.is_some()
                || timeout.is_some()
                || group.is_some()
                || tag.is_some()
                || device_type.is_some()
                || clear_filters
        }
    };

    if is_mutation {
        let mut req = serde_json::Map::new();
        match action {
            Some(VirtualDisplayPolicySubCmd::Set {
                enable: e,
                allow_host: ah,
                allow_extend: ae,
                max_width: mw,
                max_height: mh,
                max_fps: mf,
                timeout: to,
                group: g,
                tag: t,
                device_type: dt,
                clear_filters: cf,
                ..
            }) => {
                if let Some(v) = e {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = ah {
                    req.insert("allow_hosting".into(), serde_json::json!(v));
                }
                if let Some(v) = ae {
                    req.insert("allow_remote_extend".into(), serde_json::json!(v));
                }
                if let Some(v) = mw {
                    req.insert("max_resolution_width".into(), serde_json::json!(v));
                }
                if let Some(v) = mh {
                    req.insert("max_resolution_height".into(), serde_json::json!(v));
                }
                if let Some(v) = mf {
                    req.insert("max_fps".into(), serde_json::json!(v));
                }
                if let Some(v) = to {
                    req.insert("idle_timeout_secs".into(), serde_json::json!(v));
                }
                if let Some(v) = g {
                    req.insert("allowed_groups".into(), serde_json::json!(v));
                }
                if let Some(v) = t {
                    req.insert("allowed_tags".into(), serde_json::json!(v));
                }
                if let Some(v) = dt {
                    req.insert("allowed_types".into(), serde_json::json!(v));
                }
                if cf {
                    req.insert("clear_filters".into(), serde_json::json!(true));
                }
            }
            _ => {
                if let Some(v) = enable {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = allow_host {
                    req.insert("allow_hosting".into(), serde_json::json!(v));
                }
                if let Some(v) = allow_extend {
                    req.insert("allow_remote_extend".into(), serde_json::json!(v));
                }
                if let Some(v) = max_width {
                    req.insert("max_resolution_width".into(), serde_json::json!(v));
                }
                if let Some(v) = max_height {
                    req.insert("max_resolution_height".into(), serde_json::json!(v));
                }
                if let Some(v) = max_fps {
                    req.insert("max_fps".into(), serde_json::json!(v));
                }
                if let Some(v) = timeout {
                    req.insert("idle_timeout_secs".into(), serde_json::json!(v));
                }
                if let Some(v) = group {
                    req.insert("allowed_groups".into(), serde_json::json!(v));
                }
                if let Some(v) = tag {
                    req.insert("allowed_tags".into(), serde_json::json!(v));
                }
                if let Some(v) = device_type {
                    req.insert("allowed_types".into(), serde_json::json!(v));
                }
                if clear_filters {
                    req.insert("clear_filters".into(), serde_json::json!(true));
                }
            }
        }
        let (code, body) = loopback_post(
            ui_port,
            "/api/display/virtual/policy",
            &serde_json::to_vec(&req)?,
        )
        .await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update virtual display policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
    }

    let (code, body) = loopback_get(ui_port, "/api/display/virtual/policy").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual display policy (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let pol = v.get("policy").unwrap_or(&v);
    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_host = pol["allow_hosting"].as_bool().unwrap_or(true);
    let allow_extend = pol["allow_remote_extend"].as_bool().unwrap_or(true);
    let max_w = pol["max_resolution_width"].as_u64().unwrap_or(3840);
    let max_h = pol["max_resolution_height"].as_u64().unwrap_or(2160);
    let max_fps = pol["max_fps"].as_u64().unwrap_or(60);
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

    println!("Virtual Secondary Display Governance Policy:");
    println!("  Enabled:              {enabled}");
    println!("  Allow Hosting:        {allow_host} (as secondary screen)");
    println!("  Allow Extend:         {allow_extend} (extend desktop)");
    println!("  Max Resolution:       {max_w}x{max_h}");
    println!("  Max FPS:              {max_fps}");
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

pub(crate) async fn do_virtual_display_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
    })?;
    if !json {
        println!("Streaming live virtual display events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "virtual_display_session_started"
                || kind == "virtual_display_session_stopped"
                || kind == "virtual_display_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "virtual_display_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let role = v["role"].as_str().unwrap_or("unknown");
                            let w = v["width"].as_u64().unwrap_or(0);
                            let h = v["height"].as_u64().unwrap_or(0);
                            let fps = v["fps"].as_u64().unwrap_or(0);
                            let edge = v["edge"].as_str().unwrap_or("unknown");
                            println!("[virtual_display_session_started] session={sid} peer={peer} role={role} {w}x{h}@{fps}fps edge={edge}");
                        }
                        "virtual_display_session_stopped" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("none");
                            println!("[virtual_display_session_stopped] session={sid} peer={peer} reason={reason}");
                        }
                        "virtual_display_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let host = v["allow_hosting"].as_bool().unwrap_or(true);
                            let ext = v["allow_remote_extend"].as_bool().unwrap_or(true);
                            println!("[virtual_display_policy_changed] enabled={en} allow_hosting={host} allow_remote_extend={ext}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
