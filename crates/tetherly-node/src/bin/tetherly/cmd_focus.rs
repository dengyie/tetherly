// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Focus (DND) on/off/status/peers/sync/policy/watch.

use super::*;

/// `tetherly focus` -- Focus arm of the top-level dispatch.
pub(crate) async fn focus(action: FocusCmd) -> Result<()> {
    match action {
        FocusCmd::On {
            mode,
            duration,
            reason,
            no_broadcast,
            json,
            ui_port,
        } => {
            do_focus_on(ui_port, &mode, duration, reason, no_broadcast, json).await?;
        }
        FocusCmd::Off {
            no_broadcast,
            json,
            ui_port,
        } => {
            do_focus_off(ui_port, no_broadcast, json).await?;
        }
        FocusCmd::Status { json, ui_port } => {
            do_focus_status(ui_port, json).await?;
        }
        FocusCmd::Peers { json, ui_port } => {
            do_focus_peers(ui_port, json).await?;
        }
        FocusCmd::Sync { json, ui_port } => {
            do_focus_sync(ui_port, json).await?;
        }
        FocusCmd::Policy {
            action,
            enable,
            default_mode,
            mirror,
            breakthrough,
            quiet_hours,
            start,
            end,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_focus_policy(
                ui_port,
                action.as_deref(),
                enable,
                default_mode.as_deref(),
                mirror,
                breakthrough,
                quiet_hours,
                start.as_deref(),
                end.as_deref(),
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        FocusCmd::Watch { json, ui_port } => {
            do_focus_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_focus_on(
    ui_port: u16,
    mode: &str,
    duration: Option<u32>,
    reason: Option<String>,
    no_broadcast: bool,
    json: bool,
) -> Result<()> {
    let req = serde_json::json!({
        "enabled": true,
        "mode": mode,
        "duration": duration,
        "reason": reason,
        "broadcast": !no_broadcast,
    });
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/focus/state", &serde_json::to_vec(&req)?)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to activate focus mode (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let rep: serde_json::Value = serde_json::from_slice(&body)?;
    let rep_mode = rep["mode"].as_str().unwrap_or(mode);
    let active = rep["active"].as_bool().unwrap_or(true);
    let reason_str = rep["reason"].as_str().unwrap_or("none");
    let broadcast_count = rep["broadcast_count"].as_u64().unwrap_or(0);
    println!("Focus Mode Activated:");
    println!("  Mode:            {rep_mode}");
    println!("  Active:          {active}");
    if let Some(mins) = duration {
        println!("  Duration:        {mins} mins");
    }
    if let Some(exp) = rep["expires_ms"].as_u64() {
        println!("  Expires At (ms): {exp}");
    }
    println!("  Reason:          {reason_str}");
    println!("  Peers Notified:  {broadcast_count}");
    Ok(())
}

pub(crate) async fn do_focus_off(ui_port: u16, no_broadcast: bool, json: bool) -> Result<()> {
    let req = serde_json::json!({
        "enabled": false,
        "broadcast": !no_broadcast,
    });
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/focus/state", &serde_json::to_vec(&req)?)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to deactivate focus mode (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    println!("Focus mode deactivated successfully (normal notification delivery restored).");
    Ok(())
}

pub(crate) async fn do_focus_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/focus/state")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to get focus status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let rep: serde_json::Value = serde_json::from_slice(&body)?;
    let enabled = rep["enabled"].as_bool().unwrap_or(false);
    let active = rep["active"].as_bool().unwrap_or(false);
    let mode = rep["mode"].as_str().unwrap_or("dnd");
    let quiet_active = rep["quiet_hours_active"].as_bool().unwrap_or(false);
    let remaining = rep["remaining_secs"].as_u64();
    let reason = rep["reason"].as_str();

    println!("Focus Mode Status:");
    println!("  Active:              {active}");
    println!("  Manual Switch:       {enabled}");
    println!("  Effective Mode:      {mode}");
    if let Some(rem) = remaining {
        println!("  Remaining Time:      {}m {}s", rem / 60, rem % 60);
    }
    if let Some(r) = reason {
        println!("  Reason:              {r}");
    }
    println!("  Quiet Hours Active:  {quiet_active}");
    if let Some(pol) = rep.get("policy") {
        let qh_en = pol["quiet_hours_enabled"].as_bool().unwrap_or(false);
        let qh_start = pol["quiet_hours_start_minute"].as_u64().unwrap_or(0);
        let qh_end = pol["quiet_hours_end_minute"].as_u64().unwrap_or(0);
        let mirror = pol["mirror_inbound_focus"].as_bool().unwrap_or(false);
        let brk = pol["allow_emergency_breakthrough"]
            .as_bool()
            .unwrap_or(true);
        println!(
            "  Quiet Hours Config:  {} ({:02}:{:02} - {:02}:{:02})",
            if qh_en { "Enabled" } else { "Disabled" },
            qh_start / 60,
            qh_start % 60,
            qh_end / 60,
            qh_end % 60
        );
        println!("  Mirror Remote Focus: {mirror}");
        println!("  OTP Breakthrough:    {brk}");
    }
    Ok(())
}

pub(crate) async fn do_focus_peers(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/focus/peers")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query peers focus states (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let rep: serde_json::Value = serde_json::from_slice(&body)?;
    let peers = rep["peers"].as_array().cloned().unwrap_or_default();
    println!("Known Peers Focus States ({}):", peers.len());
    if peers.is_empty() {
        println!("  (no peer focus states recorded yet; run `tetherly focus sync` to refresh)");
        return Ok(());
    }
    for p in peers {
        let pid = p["peer"].as_str().unwrap_or("");
        let alias = p["peer_alias"].as_str().unwrap_or("unknown");
        let active = p["active"].as_bool().unwrap_or(false);
        let mode = p["state"]["mode"].as_str().unwrap_or("dnd");
        let reason = p["state"]["reason"].as_str().unwrap_or("none");
        let rem = p["remaining_secs"].as_u64();
        let rem_str = match rem {
            Some(s) => format!(" ({}m remaining)", s / 60),
            None => "".to_string(),
        };
        let status_str = if active {
            format!("ACTIVE [{mode}]")
        } else {
            "INACTIVE".to_string()
        };
        println!("  - [{status_str}] {alias} ({pid}): reason=\"{reason}\"{rem_str}");
    }
    Ok(())
}

pub(crate) async fn do_focus_sync(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = tetherly_node::loopback_post(ui_port, "/api/focus/sync", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to dispatch focus sync (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let rep: serde_json::Value = serde_json::from_slice(&body)?;
    let count = rep["sync_requests_sent"].as_u64().unwrap_or(0);
    println!("Dispatched focus sync request to {count} connected mesh peer(s).");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_focus_policy(
    ui_port: u16,
    action: Option<&str>,
    enable: Option<bool>,
    default_mode: Option<&str>,
    mirror: Option<bool>,
    breakthrough: Option<bool>,
    quiet_hours: Option<bool>,
    start: Option<&str>,
    end: Option<&str>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_set = action == Some("set")
        || enable.is_some()
        || default_mode.is_some()
        || mirror.is_some()
        || breakthrough.is_some()
        || quiet_hours.is_some()
        || start.is_some()
        || end.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_set {
        let mut req = serde_json::Map::new();
        if let Some(en) = enable {
            req.insert("enabled".to_string(), serde_json::json!(en));
        }
        if let Some(m) = default_mode {
            req.insert("default_mode".to_string(), serde_json::json!(m));
        }
        if let Some(m) = mirror {
            req.insert("mirror".to_string(), serde_json::json!(m));
        }
        if let Some(brk) = breakthrough {
            req.insert("breakthrough".to_string(), serde_json::json!(brk));
        }
        if let Some(qh) = quiet_hours {
            req.insert("quiet_hours".to_string(), serde_json::json!(qh));
        }
        if let Some(s) = start {
            req.insert("start".to_string(), serde_json::json!(s));
        }
        if let Some(e) = end {
            req.insert("end".to_string(), serde_json::json!(e));
        }
        if let Some(grps) = group {
            req.insert("allowed_groups".to_string(), serde_json::json!(grps));
        }
        if let Some(tags) = tag {
            req.insert("allowed_tags".to_string(), serde_json::json!(tags));
        }
        if let Some(types) = device_type {
            req.insert("allowed_types".to_string(), serde_json::json!(types));
        }
        if clear_filters {
            req.insert("clear_filters".to_string(), serde_json::json!(true));
        }

        let body_bytes = serde_json::to_vec(&req)?;
        let (code, body) = tetherly_node::loopback_post(ui_port, "/api/focus/policy", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update focus policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        println!("Focus Policy Updated Successfully:");
        print_focus_policy_json(&body)?;
    } else {
        let (code, body) = tetherly_node::loopback_get(ui_port, "/api/focus/policy")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to query focus policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
        println!("Focus Governance Policy:");
        print_focus_policy_json(&body)?;
    }
    Ok(())
}

pub(crate) fn print_focus_policy_json(body: &[u8]) -> Result<()> {
    let rep: serde_json::Value = serde_json::from_slice(body)?;
    let pol = &rep["policy"];
    let en = pol["enabled"].as_bool().unwrap_or(true);
    let def_m = pol["default_mode"].as_str().unwrap_or("dnd");
    let mirror = pol["mirror_inbound_focus"].as_bool().unwrap_or(false);
    let brk = pol["allow_emergency_breakthrough"]
        .as_bool()
        .unwrap_or(true);
    let qh = pol["quiet_hours_enabled"].as_bool().unwrap_or(false);
    let start_m = pol["quiet_hours_start_minute"].as_u64().unwrap_or(1380);
    let end_m = pol["quiet_hours_end_minute"].as_u64().unwrap_or(420);
    let grps: Vec<String> = pol["allowed_groups"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let tags: Vec<String> = pol["allowed_tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let types: Vec<String> = pol["allowed_types"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    println!("  Policy Enabled:             {en}");
    println!("  Default Mode:               {def_m}");
    println!("  Mirror Inbound Focus:       {mirror}");
    println!("  Emergency OTP Breakthrough: {brk}");
    println!(
        "  Quiet Hours Automation:     {}",
        if qh { "Enabled" } else { "Disabled" }
    );
    println!(
        "  Quiet Hours Schedule:       {:02}:{:02} - {:02}:{:02}",
        start_m / 60,
        start_m % 60,
        end_m / 60,
        end_m % 60
    );
    println!(
        "  Allowed Groups:             {}",
        if grps.is_empty() {
            "* (any)".into()
        } else {
            format!("[{}]", grps.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "* (any)".into()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Device Types:       {}",
        if types.is_empty() {
            "* (any)".into()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

pub(crate) async fn do_focus_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live focus mode events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "focus_state_changed"
                || kind == "peer_focus_state_changed"
                || kind == "focus_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "focus_state_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(false);
                            let mode = v["mode"].as_str().unwrap_or("dnd");
                            let active = v["active"].as_bool().unwrap_or(false);
                            let reason = v["reason"].as_str().unwrap_or("none");
                            println!("[focus_state_changed] enabled={en} active={active} mode={mode} reason=\"{reason}\"");
                        }
                        "peer_focus_state_changed" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            let en = v["enabled"].as_bool().unwrap_or(false);
                            let mode = v["mode"].as_str().unwrap_or("dnd");
                            let active = v["active"].as_bool().unwrap_or(false);
                            println!("[peer_focus_state_changed] peer={peer} ({pid}) enabled={en} active={active} mode={mode}");
                        }
                        "focus_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(false);
                            let mode = v["default_mode"].as_str().unwrap_or("dnd");
                            let qh = v["quiet_hours_enabled"].as_bool().unwrap_or(false);
                            println!("[focus_policy_changed] enabled={en} default_mode={mode} quiet_hours={qh}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
