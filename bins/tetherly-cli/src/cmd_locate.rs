// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Locate/ring/stop/status/policy/watch.

use super::util::{loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli locate` -- Locate arm of the top-level dispatch.
pub(crate) async fn locate(action: LocateCmd) -> Result<()> {
    match action {
        LocateCmd::Ring {
            peer,
            duration_secs,
            pattern,
            override_silent,
            message,
            json,
            ui_port,
        } => {
            do_locate_ring(
                ui_port,
                &peer,
                duration_secs,
                pattern.as_deref(),
                override_silent,
                message.as_deref(),
                json,
            )
            .await?;
        }
        LocateCmd::Stop {
            peer,
            request_id,
            reason,
            json,
            ui_port,
        } => {
            do_locate_stop(
                ui_port,
                peer.as_deref(),
                request_id.as_deref(),
                reason.as_deref(),
                json,
            )
            .await?;
        }
        LocateCmd::Status { json, ui_port } => {
            do_locate_status(ui_port, json).await?;
        }
        LocateCmd::Policy {
            action,
            enabled,
            override_silent,
            max_duration,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_locate_policy(
                ui_port,
                action,
                enabled,
                override_silent,
                max_duration,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        LocateCmd::Watch { json, ui_port } => {
            do_locate_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_locate_ring(
    ui_port: u16,
    peer: &str,
    duration_secs: Option<u32>,
    pattern: Option<&str>,
    override_silent: Option<bool>,
    message: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "peer": peer,
    });
    if let Some(dur) = duration_secs {
        payload["duration_secs"] = serde_json::json!(dur);
    }
    if let Some(pat) = pattern {
        payload["pattern"] = serde_json::json!(pat);
    }
    if let Some(ov) = override_silent {
        payload["override_silent"] = serde_json::json!(ov);
    }
    if let Some(msg) = message {
        payload["message"] = serde_json::json!(msg);
    }

    let (code, resp) = loopback_post(ui_port, "/api/locate/ring", &serde_json::to_vec(&payload)?)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to trigger locate ring (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target = v["target_peer"].as_str().unwrap_or(peer);
    let req_id = v["request_id"].as_str().unwrap_or("unknown");
    let dur = v["duration_secs"].as_u64().unwrap_or(15);
    let pat = v["pattern"].as_str().unwrap_or("chime");
    let ov = v["override_silent"].as_bool().unwrap_or(true);
    let msg = v["message"].as_str().unwrap_or("");

    println!("Locate Acoustic Alarm Triggered:");
    println!("  Target Peer:       {target}");
    println!("  Request ID:        {req_id}");
    println!("  Duration:          {dur}s");
    println!("  Pattern:           {pat}");
    println!("  Override Silent:   {ov}");
    if !msg.is_empty() {
        println!("  Message:           {msg}");
    }
    Ok(())
}

pub(crate) async fn do_locate_stop(
    ui_port: u16,
    peer: Option<&str>,
    request_id: Option<&str>,
    reason: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({});
    if let Some(p) = peer {
        payload["peer"] = serde_json::json!(p);
    }
    if let Some(id) = request_id {
        payload["request_id"] = serde_json::json!(id);
    }
    if let Some(r) = reason {
        payload["reason"] = serde_json::json!(r);
    }

    let (code, resp) = loopback_post(ui_port, "/api/locate/stop", &serde_json::to_vec(&payload)?)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to stop locate ring (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target = v["target_peer"].as_str().unwrap_or("local");
    let req_id = v["request_id"].as_str().unwrap_or("active");
    let rsn = v["reason"].as_str().unwrap_or("unspecified");

    println!("Locate Ring Stopped:");
    println!("  Target Peer:       {target}");
    println!("  Request ID:        {req_id}");
    println!("  Reason:            {rsn}");
    Ok(())
}

pub(crate) async fn do_locate_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/locate/status")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to query locate status (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let is_ringing = v["is_ringing"].as_bool().unwrap_or(false);

    println!("Device Locator Status:");
    println!("  Is Ringing:        {is_ringing}");
    if let Some(ar) = v.get("active_ring").filter(|r| !r.is_null()) {
        let req_id = ar["request_id"].as_str().unwrap_or("");
        let init_peer = ar["initiator_peer"].as_str().unwrap_or("");
        let init_alias = ar["initiator_alias"].as_str().unwrap_or("");
        let pat = ar["pattern"].as_str().unwrap_or("chime");
        let dur = ar["duration_secs"].as_u64().unwrap_or(0);
        let rem = ar["remaining_secs"].as_u64().unwrap_or(0);
        let ov = ar["override_silent"].as_bool().unwrap_or(true);
        let msg = ar["message"].as_str().unwrap_or("");

        println!("  Active Ringing Alarm:");
        println!("    Request ID:      {req_id}");
        println!("    Initiator Peer:  {init_peer} ({init_alias})");
        println!("    Pattern:         {pat}");
        println!("    Duration:        {dur}s (remaining: {rem}s)");
        println!("    Override Silent: {ov}");
        if !msg.is_empty() {
            println!("    Message:         {msg}");
        }
    }

    if let Some(p) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = p["enabled"].as_bool().unwrap_or(true);
        let allow_ov = p["allow_override_silent"].as_bool().unwrap_or(true);
        let max_dur = p["max_duration_secs"].as_u64().unwrap_or(60);
        println!("  Policy Summary:");
        println!("    Enabled:         {enabled}");
        println!("    Override Silent: {allow_ov}");
        println!("    Max Duration:    {max_dur}s");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_locate_policy(
    ui_port: u16,
    action: Option<String>,
    enabled: Option<bool>,
    override_silent: Option<bool>,
    max_duration: Option<u32>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || enabled.is_some()
        || override_silent.is_some()
        || max_duration.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(en) = enabled {
            payload["enabled"] = serde_json::json!(en);
        }
        if let Some(ov) = override_silent {
            payload["allow_override_silent"] = serde_json::json!(ov);
        }
        if let Some(dur) = max_duration {
            payload["max_duration_secs"] = serde_json::json!(dur);
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

        let (code, resp) = loopback_post(
            ui_port,
            "/api/locate/policy",
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
            bail!(
                "failed to update locate policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
    }

    let (code, resp) = loopback_get(ui_port, "/api/locate/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to query locate policy (HTTP {code}): {}",
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
    let en = p["enabled"].as_bool().unwrap_or(true);
    let allow_ov = p["allow_override_silent"].as_bool().unwrap_or(true);
    let max_dur = p["max_duration_secs"].as_u64().unwrap_or(60);
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

    println!("Device Locator Policy:");
    println!("  Enabled:                    {en}");
    println!("  Allow Silent Override:      {allow_ov}");
    println!("  Max Duration:               {max_dur}s");
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

pub(crate) async fn do_locate_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if !json {
        println!("Streaming live device locator events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "locate_ring_triggered"
                || kind == "locate_ring_stopped"
                || kind == "locate_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "locate_ring_triggered" => {
                            let req_id = v["request_id"].as_str().unwrap_or("");
                            let peer = v["initiator_alias"].as_str().unwrap_or("unknown");
                            let pat = v["pattern"].as_str().unwrap_or("chime");
                            let dur = v["duration_secs"].as_u64().unwrap_or(0);
                            let ov = v["override_silent"].as_bool().unwrap_or(true);
                            let msg = v["message"].as_str().unwrap_or("");
                            let msg_str = if !msg.is_empty() {
                                format!(" msg=\"{msg}\"")
                            } else {
                                String::new()
                            };
                            println!("[locate_ring_triggered] req={req_id} peer={peer} pattern={pat} duration={dur}s override={ov}{msg_str}");
                        }
                        "locate_ring_stopped" => {
                            let req_id = v["request_id"].as_str().unwrap_or("");
                            let peer = v["stopped_by_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("unspecified");
                            println!("[locate_ring_stopped] req={req_id} stopped_by={peer} reason={reason}");
                        }
                        "locate_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let ov = v["allow_override_silent"].as_bool().unwrap_or(true);
                            let max_d = v["max_duration_secs"].as_u64().unwrap_or(60);
                            println!("[locate_policy_changed] enabled={en} override_silent={ov} max_duration={max_d}s");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
