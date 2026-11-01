// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Battery/power status, report, policy and watch.

use super::*;

/// `tetherly power` -- Power arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn power(action: PowerCmd) -> Result<()> {
    match action {
        PowerCmd::Status {
            peer,
            json,
            ui_port,
        } => {
            do_power_status(ui_port, peer.as_deref(), json).await?;
        }
        PowerCmd::Report {
            level,
            charging,
            power_source,
            battery_saver,
            health_percent,
            json,
            ui_port,
        } => {
            do_power_report(
                ui_port,
                level,
                charging,
                power_source.as_deref(),
                battery_saver,
                health_percent,
                json,
            )
            .await?;
        }
        PowerCmd::Policy {
            action,
            low_threshold,
            critical_threshold,
            alert,
            throttle,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_power_policy(
                ui_port,
                action,
                low_threshold,
                critical_threshold,
                alert,
                throttle,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        PowerCmd::Watch { json, ui_port } => {
            do_power_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_power_status(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/battery/state".to_string();
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
            "failed to query battery status (HTTP {code}): {}",
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
        let level = st["battery_level"].as_u64().unwrap_or(0);
        let charging = st["is_charging"].as_bool().unwrap_or(false);
        let source = st["power_source"].as_str().unwrap_or("unknown");
        let saver = st["battery_saver"].as_bool().unwrap_or(false);
        let health = st.get("health_percent").and_then(|h| h.as_u64());

        let bar_len = (level as usize * 20) / 100;
        let bar = format!("[{}{}]", "=".repeat(bar_len), " ".repeat(20 - bar_len));

        println!("Battery & Power Status ({target_peer}):");
        println!("  Battery Level:     {level}% {bar}");
        println!("  Charging:          {charging}");
        println!("  Power Source:      {source}");
        println!("  Battery Saver:     {saver}");
        if let Some(hp) = health {
            println!("  Battery Health:    {hp}%");
        }
    } else {
        println!("No battery state available for {target_peer}.");
    }
    Ok(())
}

pub(crate) async fn do_power_report(
    ui_port: u16,
    level: Option<u8>,
    charging: Option<bool>,
    power_source: Option<&str>,
    battery_saver: Option<bool>,
    health_percent: Option<u8>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({});
    if let Some(lvl) = level {
        payload["battery_level"] = serde_json::json!(lvl);
    }
    if let Some(chg) = charging {
        payload["is_charging"] = serde_json::json!(chg);
    }
    if let Some(src) = power_source {
        payload["power_source"] = serde_json::json!(src);
    }
    if let Some(saver) = battery_saver {
        payload["battery_saver"] = serde_json::json!(saver);
    }
    if let Some(hp) = health_percent {
        payload["health_percent"] = serde_json::json!(hp);
    }

    let (code, resp) = tetherly_node::loopback_post(
        ui_port,
        "/api/battery/state",
        &serde_json::to_vec(&payload)?,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;

    if code != 200 {
        anyhow::bail!(
            "failed to update battery state (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let st = &v["state"];
    let level = st["battery_level"].as_u64().unwrap_or(0);
    let charging = st["is_charging"].as_bool().unwrap_or(false);
    let source = st["power_source"].as_str().unwrap_or("unknown");
    let saver = st["battery_saver"].as_bool().unwrap_or(false);
    let bcast_cnt = v["broadcast_count"].as_u64().unwrap_or(0);

    println!("Local Battery State Updated & Broadcasted:");
    println!("  Battery Level:     {level}%");
    println!("  Charging:          {charging}");
    println!("  Power Source:      {source}");
    println!("  Battery Saver:     {saver}");
    println!("  Broadcast To:      {bcast_cnt} online peer(s)");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_power_policy(
    ui_port: u16,
    action: Option<String>,
    low_threshold: Option<u8>,
    critical_threshold: Option<u8>,
    alert: Option<bool>,
    throttle: Option<bool>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || low_threshold.is_some()
        || critical_threshold.is_some()
        || alert.is_some()
        || throttle.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(low) = low_threshold {
            payload["low_battery_threshold"] = serde_json::json!(low);
        }
        if let Some(crit) = critical_threshold {
            payload["critical_battery_threshold"] = serde_json::json!(crit);
        }
        if let Some(al) = alert {
            payload["alert_on_low_battery"] = serde_json::json!(al);
        }
        if let Some(th) = throttle {
            payload["auto_throttle_on_battery_saver"] = serde_json::json!(th);
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
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/battery/policy",
            &serde_json::to_vec(&payload)?,
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

        if code != 200 {
            anyhow::bail!(
                "failed to update battery policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/battery/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query battery policy (HTTP {code}): {}",
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
    let low = p["low_battery_threshold"].as_u64().unwrap_or(20);
    let crit = p["critical_battery_threshold"].as_u64().unwrap_or(10);
    let alert_enabled = p["alert_on_low_battery"].as_bool().unwrap_or(true);
    let throttle_enabled = p["auto_throttle_on_battery_saver"]
        .as_bool()
        .unwrap_or(false);
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

    println!("Battery & Energy Governance Policy:");
    println!("  Low Warning Threshold:      {low}%");
    println!("  Critical Alert Threshold:   {crit}%");
    println!("  Alert on Low Battery:       {alert_enabled}");
    println!("  Throttle on Battery Saver:  {throttle_enabled}");
    println!(
        "  Allowed Groups:             {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:              {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

pub(crate) async fn do_power_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live battery & power events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "battery_state_changed"
                || kind == "battery_low_alert"
                || kind == "battery_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "battery_state_changed" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let lvl = v["battery_level"].as_u64().unwrap_or(0);
                            let chg = v["is_charging"].as_bool().unwrap_or(false);
                            let src = v["power_source"].as_str().unwrap_or("unknown");
                            let saver = v["battery_saver"].as_bool().unwrap_or(false);
                            let chg_str = if chg { " [charging]" } else { "" };
                            let saver_str = if saver { " (saver)" } else { "" };
                            println!("[battery_state_changed] peer={peer} level={lvl}%{chg_str} source={src}{saver_str}");
                        }
                        "battery_low_alert" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let lvl = v["battery_level"].as_u64().unwrap_or(0);
                            let is_crit = v["is_critical"].as_bool().unwrap_or(false);
                            let alert_tag = if is_crit { "CRITICAL" } else { "LOW" };
                            println!("[battery_low_alert] [{alert_tag}] peer={peer} battery={lvl}%! Please connect to power.");
                        }
                        "battery_policy_changed" => {
                            let low = v["low_battery_threshold"].as_u64().unwrap_or(20);
                            let crit = v["critical_battery_threshold"].as_u64().unwrap_or(10);
                            let alert = v["alert_on_low_battery"].as_bool().unwrap_or(true);
                            println!("[battery_policy_changed] low={low}% critical={crit}% alert={alert}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
