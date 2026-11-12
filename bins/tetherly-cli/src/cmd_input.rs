// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Input seat attach/release/status/policy/watch.

use super::util::{loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli input` -- Input arm of the top-level dispatch.
pub(crate) async fn input(action: InputCmd) -> Result<()> {
    match action {
        InputCmd::Start {
            peer,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "peer": peer });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/input/start", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "input start failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("unknown");
                let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("none");
                println!("Input channel connected. Cursor seat: {seat}, Peer: {p}");
            }
        }
        InputCmd::Stop { json, ui_port } => {
            let (code, resp) = loopback_post(ui_port, "/api/input/stop", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "input stop failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("local");
                println!("Input channel disconnected. Cursor seat: {seat}");
            }
        }
        InputCmd::Status { json, ui_port } => {
            do_input_status(ui_port, json).await?;
        }
        InputCmd::Attach {
            peer,
            edge,
            x,
            y,
            json,
            ui_port,
        } => {
            do_input_attach(ui_port, &peer, edge, x, y, json).await?;
        }
        InputCmd::Release {
            reason,
            json,
            ui_port,
        } => {
            do_input_release(ui_port, reason, json).await?;
        }
        InputCmd::Policy {
            action,
            enable,
            allow_inbound,
            keyboard,
            mouse,
            hotkey,
            timeout,
            group,
            tag,
            device_type,
            clear_filters,
            keymap,
            edge,
            wheel_speed,
            json,
            ui_port,
        } => {
            do_input_policy(
                ui_port,
                action,
                enable,
                allow_inbound,
                keyboard,
                mouse,
                hotkey,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                keymap,
                edge,
                wheel_speed,
                json,
            )
            .await?;
        }
        InputCmd::Watch { json, ui_port } => {
            do_input_watch(ui_port, json).await?;
        }
        InputCmd::Trace {
            limit,
            json,
            ui_port,
        } => {
            do_input_trace(ui_port, limit, json).await?;
        }
    }
    Ok(())
}

/// `tetherly-cli input trace` -- dump the node's input-engine decision ring.
pub(crate) async fn do_input_trace(ui_port: u16, limit: usize, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/input/trace")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query input trace (HTTP {code}): {}",
            String::from_utf8_lossy(&body)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body).trim());
        return Ok(());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    print_input_trace(&v, limit);
    Ok(())
}

pub(crate) fn print_input_trace(v: &serde_json::Value, limit: usize) {
    let entries = v["entries"].as_array().cloned().unwrap_or_default();
    println!(
        "Input engine trace: seat={} channel_open={} entries={}/{}",
        v["seat"].as_str().unwrap_or("unknown"),
        v["channel_open"].as_bool().unwrap_or(false),
        entries.len(),
        v["capacity"].as_u64().unwrap_or(0)
    );
    let skip = if limit == 0 {
        0
    } else {
        entries.len().saturating_sub(limit)
    };
    if skip > 0 {
        println!("  ... {skip} older entries omitted (--limit 0 shows all)");
    }
    let mut prev_ts: Option<u64> = None;
    for e in entries.iter().skip(skip) {
        let ts = e["ts_ms"].as_u64().unwrap_or(0);
        let gap = match prev_ts {
            Some(p) => format!("+{:>6}ms", ts.saturating_sub(p)),
            None => "         ".to_string(),
        };
        println!(
            "  {ts:>13} {gap}  {:<11} {}",
            e["kind"].as_str().unwrap_or(""),
            e["detail"].as_str().unwrap_or("")
        );
        prev_ts = Some(ts);
    }
}

pub(crate) async fn do_input_attach(
    ui_port: u16,
    peer: &str,
    edge: Option<String>,
    x: Option<i32>,
    y: Option<i32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
    });
    if let Some(e) = edge {
        req["edge"] = serde_json::json!(e);
    }
    if let Some(xv) = x {
        req["x"] = serde_json::json!(xv);
    }
    if let Some(yv) = y {
        req["y"] = serde_json::json!(yv);
    }
    let (code, body) = loopback_post(
        ui_port,
        "/api/input/tunnel/attach",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to attach input tunnel (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer"].as_str().unwrap_or(peer);
        let accepted = v["accepted"]
            .as_bool()
            .or_else(|| v["ok"].as_bool())
            .unwrap_or(false);
        if accepted {
            println!("Virtual peripheral input tunnel attached successfully.");
            println!("  Session ID: {sid}");
            println!("  Peer:       {peer_val}");
            if let Some(w) = v["capture_width"].as_u64() {
                if let Some(h) = v["capture_height"].as_u64() {
                    println!("  Resolution: {w}x{h}");
                }
            }
        } else {
            let reason = v["reject_reason"].as_str().unwrap_or("rejected");
            println!("Input tunnel rejected by target: {reason}");
        }
    }
    Ok(())
}

pub(crate) async fn do_input_release(
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
        "/api/input/tunnel/release",
        &serde_json::to_vec(&req)?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to release input tunnel (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("none");
        let reason_val = v["reason"].as_str().unwrap_or("user requested");
        println!("Virtual peripheral input tunnel released ({reason_val}). Active session: {sid}");
    }
    Ok(())
}

pub(crate) async fn do_input_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/input/tunnel").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query input tunnel status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Virtual Peripheral Input Tunnel Status:");
    println!("  Active Tunnel: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer = sess["peer_id"]
            .as_str()
            .or_else(|| sess["peer"].as_str())
            .unwrap_or("unknown");
        let role = sess["role"].as_str().unwrap_or("unknown");
        println!("  Session ID:    {sid}");
        println!("  Peer:          {peer}");
        println!("  Role:          {role}");
        if let Some(w) = sess["capture_width"].as_u64() {
            if let Some(h) = sess["capture_height"].as_u64() {
                println!("  Resolution:    {w}x{h}");
            }
        }
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let inbound = pol["allow_inbound_control"].as_bool().unwrap_or(true);
        let kb = pol["forward_keyboard"].as_bool().unwrap_or(true);
        let mouse = pol["forward_mouse"].as_bool().unwrap_or(true);
        let hotkey = pol["forward_hotkeys"].as_bool().unwrap_or(true);
        let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
        println!("Policy:");
        println!("  Enabled:       {enabled}");
        println!("  Inbound:       {inbound}");
        println!("  Forwarding:    keyboard={kb}, mouse={mouse}, hotkeys={hotkey}");
        println!("  Idle Timeout:  {timeout}s");
        let km = pol["keymap"].as_str().unwrap_or("none (identity)");
        println!("  Keymap:        {km}");
        let edge = pol["edge"].as_str().unwrap_or("right (default)");
        println!("  Edge:          {edge}");
        let ws = pol["wheel_speed"].as_f64().unwrap_or(1.0);
        println!("  Wheel Speed:   {ws}x");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_input_policy(
    ui_port: u16,
    action: Option<InputPolicySubCmd>,
    enable: Option<bool>,
    allow_inbound: Option<bool>,
    keyboard: Option<bool>,
    mouse: Option<bool>,
    hotkey: Option<bool>,
    timeout: Option<u64>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    keymap: Option<String>,
    edge: Option<String>,
    wheel_speed: Option<f64>,
    json: bool,
) -> Result<()> {
    let is_mutation = match &action {
        Some(InputPolicySubCmd::Set { .. }) => true,
        Some(InputPolicySubCmd::Show { .. }) => false,
        None => {
            enable.is_some()
                || allow_inbound.is_some()
                || keyboard.is_some()
                || mouse.is_some()
                || hotkey.is_some()
                || timeout.is_some()
                || group.is_some()
                || tag.is_some()
                || device_type.is_some()
                || clear_filters
                || keymap.is_some()
                || edge.is_some()
                || wheel_speed.is_some()
        }
    };

    if is_mutation {
        let mut req = serde_json::Map::new();
        match action {
            Some(InputPolicySubCmd::Set {
                enable: e,
                allow_inbound: ai,
                keyboard: kb,
                mouse: m,
                hotkey: hk,
                timeout: to,
                group: g,
                tag: t,
                device_type: dt,
                clear_filters: cf,
                keymap: km,
                edge: ed,
                wheel_speed: ws,
                ..
            }) => {
                if let Some(v) = e {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = ai {
                    req.insert("allow_inbound_control".into(), serde_json::json!(v));
                }
                if let Some(v) = kb {
                    req.insert("forward_keyboard".into(), serde_json::json!(v));
                }
                if let Some(v) = m {
                    req.insert("forward_mouse".into(), serde_json::json!(v));
                }
                if let Some(v) = hk {
                    req.insert("forward_hotkeys".into(), serde_json::json!(v));
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
                if let Some(v) = km {
                    req.insert("keymap".into(), serde_json::json!(v));
                }
                if let Some(v) = ed {
                    req.insert("edge".into(), serde_json::json!(v));
                }
                if let Some(v) = ws {
                    req.insert("wheel_speed".into(), serde_json::json!(v));
                }
            }
            _ => {
                if let Some(v) = enable {
                    req.insert("enabled".into(), serde_json::json!(v));
                }
                if let Some(v) = allow_inbound {
                    req.insert("allow_inbound_control".into(), serde_json::json!(v));
                }
                if let Some(v) = keyboard {
                    req.insert("forward_keyboard".into(), serde_json::json!(v));
                }
                if let Some(v) = mouse {
                    req.insert("forward_mouse".into(), serde_json::json!(v));
                }
                if let Some(v) = hotkey {
                    req.insert("forward_hotkeys".into(), serde_json::json!(v));
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
                if let Some(v) = keymap {
                    req.insert("keymap".into(), serde_json::json!(v));
                }
                if let Some(v) = edge {
                    req.insert("edge".into(), serde_json::json!(v));
                }
                if let Some(v) = wheel_speed {
                    req.insert("wheel_speed".into(), serde_json::json!(v));
                }
            }
        }
        let body_bytes = serde_json::to_vec(&req)?;
        let (code, body) = loopback_post(ui_port, "/api/input/policy", &body_bytes).await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update input policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        println!("Successfully updated virtual peripheral sharing policy.");
        print_input_policy_json(&body)?;
    } else {
        let (code, body) = loopback_get(ui_port, "/api/input/policy").await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to query input policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        print_input_policy_json(&body)?;
    }
    Ok(())
}

pub(crate) fn print_input_policy_json(body: &[u8]) -> Result<()> {
    let val: serde_json::Value = serde_json::from_slice(body)?;
    let pol = if val.get("policy").is_some() {
        &val["policy"]
    } else {
        &val
    };

    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_inbound = pol["allow_inbound_control"].as_bool().unwrap_or(true);
    let kb = pol["forward_keyboard"].as_bool().unwrap_or(true);
    let mouse = pol["forward_mouse"].as_bool().unwrap_or(true);
    let hotkey = pol["forward_hotkeys"].as_bool().unwrap_or(true);
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

    println!("Virtual Peripheral & Input Tunnel Governance Policy:");
    println!("  Enabled:              {enabled}");
    println!("  Allow Inbound:        {allow_inbound}");
    println!("  Forward Keyboard:     {kb}");
    println!("  Forward Mouse:        {mouse}");
    println!("  Forward Hotkeys:      {hotkey}");
    println!("  Idle Timeout (s):     {timeout}");
    let km = pol["keymap"].as_str().unwrap_or("none (identity)");
    println!("  Keymap Profile:       {km}");
    let edge_str = pol["edge"].as_str().unwrap_or("right (default)");
    println!("  Edge Roaming:         {edge_str}");
    let ws = pol["wheel_speed"].as_f64().unwrap_or(1.0);
    println!("  Wheel Speed:          {ws}x");
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

pub(crate) async fn do_input_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
    })?;
    if !json {
        println!("Streaming live input tunnel events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "input_tunnel_session_started"
                || kind == "input_tunnel_session_closed"
                || kind == "input_tunnel_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "input_tunnel_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let role = v["role"].as_str().unwrap_or("unknown");
                            println!("[input_tunnel_session_started] session={sid} peer={peer} role={role}");
                        }
                        "input_tunnel_session_closed" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let reason = v["reason"].as_str().unwrap_or("");
                            println!("[input_tunnel_session_closed] session={sid} reason={reason}");
                        }
                        "input_tunnel_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let inbound = v["allow_inbound"].as_bool().unwrap_or(true);
                            println!("[input_tunnel_policy_changed] enabled={en} allow_inbound={inbound}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
