// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Microphone status/attach/detach/policy/watch.

use super::*;

/// `tetherly mic` -- Mic arm of the top-level dispatch.
pub(crate) async fn mic(action: MicCmd) -> Result<()> {
    match action {
        MicCmd::Status { json, ui_port } => {
            do_mic_status(ui_port, json).await?;
        }
        MicCmd::Attach {
            peer,
            mic,
            mode,
            codec,
            sample_rate,
            channels,
            gain,
            muted,
            noise_suppression,
            echo_cancellation,
            auto_gain,
            json,
            ui_port,
        } => {
            do_mic_attach(
                ui_port,
                &peer,
                mic,
                mode,
                codec,
                sample_rate,
                channels,
                gain,
                muted,
                noise_suppression,
                echo_cancellation,
                auto_gain,
                json,
            )
            .await?;
        }
        MicCmd::Detach {
            reason,
            json,
            ui_port,
        } => {
            do_mic_detach(ui_port, reason, json).await?;
        }
        MicCmd::Policy {
            action,
            enable,
            allow_inbound,
            allow_capture,
            allow_intercom,
            max_rate,
            max_duration,
            privacy_indicator,
            default_muted,
            timeout,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_mic_policy(
                ui_port,
                action,
                enable,
                allow_inbound,
                allow_capture,
                allow_intercom,
                max_rate,
                max_duration,
                privacy_indicator,
                default_muted,
                timeout,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        MicCmd::Watch { json, ui_port } => {
            do_mic_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_mic_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/mic").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual microphone status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Virtual Microphone Status:");
    println!("  Active Session: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer_id = sess["peer_id"].as_str().unwrap_or("unknown");
        let role = sess["role"].as_str().unwrap_or("unknown");
        let mode = sess["mode"].as_str().unwrap_or("virtual_mic");
        let mic_id = sess["mic_id"].as_str().unwrap_or("default");
        let codec = sess["codec"].as_str().unwrap_or("opus");
        let rate = sess["sample_rate"].as_u64().unwrap_or(48000);
        let ch = sess["channels"].as_u64().unwrap_or(1);
        let gain = sess["gain"].as_f64().unwrap_or(1.0);
        let muted = sess["muted"].as_bool().unwrap_or(false);
        let ns = sess["noise_suppression"].as_bool().unwrap_or(false);
        let aec = sess["echo_cancellation"].as_bool().unwrap_or(false);
        let agc = sess["auto_gain"].as_bool().unwrap_or(false);
        let privacy = sess["privacy_indicator_active"].as_bool().unwrap_or(true);
        let frames_tx = sess["frames_sent"].as_u64().unwrap_or(0);
        let frames_rx = sess["frames_received"].as_u64().unwrap_or(0);
        let bytes = sess["bytes_transferred"].as_u64().unwrap_or(0);
        println!("  Session ID:      {sid}");
        println!("  Peer:            {peer_id}");
        println!("  Role:            {role} (mode: {mode})");
        println!("  Microphone:      {mic_id}");
        println!("  Audio:           {rate}Hz x{ch}ch codec={codec} gain={gain:.2}");
        println!("  Processing:      ns={ns} aec={aec} agc={agc}");
        println!("  Muted:           {}", if muted { "YES" } else { "NO" });
        println!(
            "  Privacy Indicator: {}",
            if privacy {
                "ACTIVE (Indicator On)"
            } else {
                "OFF"
            }
        );
        println!("  Transferred:     {bytes} bytes (tx: {frames_tx}, rx: {frames_rx} frames)");
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let allow_in = pol["allow_inbound_stream"].as_bool().unwrap_or(true);
        let allow_cap = pol["allow_remote_capture"].as_bool().unwrap_or(true);
        let allow_ic = pol["allow_intercom"].as_bool().unwrap_or(true);
        let max_rate = pol["max_sample_rate"].as_u64().unwrap_or(48000);
        let max_dur = pol["max_session_duration_secs"].as_u64().unwrap_or(3600);
        let privacy = pol["enforce_privacy_indicator"].as_bool().unwrap_or(true);
        let def_muted = pol["default_muted"].as_bool().unwrap_or(false);
        let timeout = pol["idle_timeout_secs"].as_u64().unwrap_or(300);
        println!("Policy:");
        println!("  Enabled:         {enabled}");
        println!("  Allow Inbound:   {allow_in} (consume remote mic feed)");
        println!("  Allow Capture:   {allow_cap} (remote capture enabled)");
        println!("  Allow Intercom:  {allow_ic} (push-to-talk)");
        println!("  Max Sample Rate: {max_rate}Hz");
        println!("  Max Duration:    {max_dur}s");
        println!(
            "  Privacy Indicator: {}",
            if privacy { "Enforced" } else { "Optional" }
        );
        println!("  Default Muted:   {def_muted}");
        println!("  Idle Timeout:    {timeout}s");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_mic_attach(
    ui_port: u16,
    peer: &str,
    mic_id: Option<String>,
    mode: Option<String>,
    codec: Option<String>,
    sample_rate: Option<u32>,
    channels: Option<u32>,
    gain: Option<f32>,
    muted: bool,
    noise_suppression: Option<bool>,
    echo_cancellation: Option<bool>,
    auto_gain: Option<bool>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
    });
    if let Some(m) = mic_id {
        req["mic_id"] = serde_json::json!(m);
    }
    if let Some(m) = mode {
        req["mode"] = serde_json::json!(m);
    }
    if let Some(c) = codec {
        req["codec"] = serde_json::json!(c);
    }
    if let Some(r) = sample_rate {
        req["sample_rate"] = serde_json::json!(r);
    }
    if let Some(c) = channels {
        req["channels"] = serde_json::json!(c);
    }
    if let Some(g) = gain {
        req["gain"] = serde_json::json!(g);
    }
    if muted {
        req["muted"] = serde_json::json!(true);
    }
    if let Some(ns) = noise_suppression {
        req["noise_suppression"] = serde_json::json!(ns);
    }
    if let Some(aec) = echo_cancellation {
        req["echo_cancellation"] = serde_json::json!(aec);
    }
    if let Some(agc) = auto_gain {
        req["auto_gain"] = serde_json::json!(agc);
    }
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/mic/attach", &serde_json::to_vec(&req)?)
            .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to attach virtual microphone (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer_id"].as_str().unwrap_or(peer);
        let msg = v["message"]
            .as_str()
            .unwrap_or("virtual microphone stream attached");
        println!("Virtual microphone stream attached successfully.");
        println!("  Session ID: {sid}");
        println!("  Peer:       {peer_val}");
        println!("  Message:    {msg}");
    }
    Ok(())
}

pub(crate) async fn do_mic_detach(ui_port: u16, reason: Option<String>, json: bool) -> Result<()> {
    let mut req = serde_json::json!({});
    if let Some(r) = reason {
        req["reason"] = serde_json::json!(r);
    }
    let (code, body) =
        tetherly_node::loopback_post(ui_port, "/api/mic/detach", &serde_json::to_vec(&req)?)
            .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to detach virtual microphone (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let msg = v["message"].as_str().unwrap_or("detached");
        println!("Virtual microphone detached: {msg}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_mic_policy(
    ui_port: u16,
    action: Option<MicPolicySubCmd>,
    enable: Option<bool>,
    allow_inbound: Option<bool>,
    allow_capture: Option<bool>,
    allow_intercom: Option<bool>,
    max_rate: Option<u32>,
    max_duration: Option<u32>,
    privacy_indicator: Option<bool>,
    default_muted: Option<bool>,
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
            MicPolicySubCmd::Show {
                json: j,
                ui_port: p,
            } => {
                let (code, body) = tetherly_node::loopback_get(p, "/api/mic/policy").await?;
                if code != 200 {
                    let err_msg = String::from_utf8_lossy(&body);
                    anyhow::bail!(
                        "failed to query virtual microphone policy (HTTP {code}): {err_msg}"
                    );
                }
                if j {
                    println!("{}", String::from_utf8_lossy(&body));
                } else {
                    print_mic_policy_value(&body)?;
                }
                return Ok(());
            }
            MicPolicySubCmd::Set {
                enable: e,
                allow_inbound: ai,
                allow_capture: ac,
                allow_intercom: ic,
                max_rate: mr,
                max_duration: md,
                privacy_indicator: pi,
                default_muted: dm,
                timeout: t,
                group: g,
                tag: tg,
                device_type: dt,
                clear_filters: cf,
                json: j,
                ui_port: p,
            } => {
                return Box::pin(do_mic_policy(
                    p, None, e, ai, ac, ic, mr, md, pi, dm, t, g, tg, dt, cf, j,
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
        if let Some(ic) = allow_intercom {
            req.insert("allow_intercom".into(), serde_json::json!(ic));
            update_needed = true;
        }
        if let Some(mr) = max_rate {
            req.insert("max_sample_rate".into(), serde_json::json!(mr));
            update_needed = true;
        }
        if let Some(md) = max_duration {
            req.insert("max_session_duration_secs".into(), serde_json::json!(md));
            update_needed = true;
        }
        if let Some(pi) = privacy_indicator {
            req.insert("enforce_privacy_indicator".into(), serde_json::json!(pi));
            update_needed = true;
        }
        if let Some(dm) = default_muted {
            req.insert("default_muted".into(), serde_json::json!(dm));
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
            tetherly_node::loopback_post(ui_port, "/api/mic/policy", &serde_json::to_vec(&req)?)
                .await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update virtual microphone policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
    }

    let (code, body) = tetherly_node::loopback_get(ui_port, "/api/mic/policy").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query virtual microphone policy (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    print_mic_policy_value(&body)
}

pub(crate) fn print_mic_policy_value(body: &[u8]) -> Result<()> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let pol = v.get("policy").unwrap_or(&v);
    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let allow_in = pol["allow_inbound_stream"].as_bool().unwrap_or(true);
    let allow_cap = pol["allow_remote_capture"].as_bool().unwrap_or(true);
    let allow_ic = pol["allow_intercom"].as_bool().unwrap_or(true);
    let max_rate = pol["max_sample_rate"].as_u64().unwrap_or(48000);
    let max_dur = pol["max_session_duration_secs"].as_u64().unwrap_or(3600);
    let privacy = pol["enforce_privacy_indicator"].as_bool().unwrap_or(true);
    let def_muted = pol["default_muted"].as_bool().unwrap_or(false);
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

    println!("Virtual Microphone Governance Policy:");
    println!("  Enabled:                 {enabled}");
    println!("  Allow Inbound Stream:    {allow_in} (act as mic sink)");
    println!("  Allow Remote Capture:    {allow_cap} (remote mic activation)");
    println!("  Allow Intercom (PTT):    {allow_ic}");
    println!("  Max Sample Rate:         {max_rate}Hz");
    println!("  Max Session Duration:    {max_dur}s");
    println!(
        "  Privacy Indicator Guard: {}",
        if privacy { "Enforced" } else { "Optional" }
    );
    println!("  Default Muted:           {def_muted}");
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

pub(crate) async fn do_mic_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
        })?;
    if !json {
        println!("Streaming live virtual microphone events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "microphone_session_started"
                || kind == "microphone_session_stopped"
                || kind == "microphone_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "microphone_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let mode = v["mode"].as_str().unwrap_or("virtual_mic");
                            let codec = v["codec"].as_str().unwrap_or("opus");
                            let rate = v["sample_rate"].as_u64().unwrap_or(48000);
                            let muted = v["muted"].as_bool().unwrap_or(false);
                            let privacy = v["privacy_indicator_active"].as_bool().unwrap_or(true);
                            println!("[microphone_session_started] session={sid} peer={peer} mode={mode} {rate}Hz codec={codec} muted={muted} privacy={privacy}");
                        }
                        "microphone_session_stopped" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("none");
                            println!("[microphone_session_stopped] session={sid} peer={peer} reason={reason}");
                        }
                        "microphone_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let allow_in = v["allow_inbound_stream"].as_bool().unwrap_or(true);
                            let ic = v["allow_intercom"].as_bool().unwrap_or(true);
                            let privacy = v["enforce_privacy_indicator"].as_bool().unwrap_or(true);
                            println!("[microphone_policy_changed] enabled={en} allow_inbound={allow_in} allow_intercom={ic} privacy_enforced={privacy}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
