// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Benchmark and recommendation commands.

use super::util::{format_bytes, loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli bench` -- Bench arm of the top-level dispatch.
pub(crate) async fn bench(action: BenchCmd) -> Result<()> {
    match action {
        BenchCmd::Status { json, ui_port } => {
            do_bench_status(ui_port, json).await?;
        }
        BenchCmd::Start {
            peer,
            direction,
            duration,
            chunk,
            json,
            ui_port,
        } => {
            do_bench_start(ui_port, &peer, &direction, duration, chunk, json).await?;
        }
        BenchCmd::Stop { json, ui_port } => {
            do_bench_stop(ui_port, json).await?;
        }
        BenchCmd::Policy {
            action,
            enable,
            allow_inbound,
            max_duration,
            max_bandwidth,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_bench_policy(
                ui_port,
                action,
                enable,
                allow_inbound,
                max_duration,
                max_bandwidth,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        BenchCmd::Watch { json, ui_port } => {
            do_bench_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

/// `tetherly-cli recommend` -- Recommend arm of the top-level dispatch.
pub(crate) async fn recommend(peer: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    do_recommend(ui_port, peer.as_deref(), json).await?;
    Ok(())
}

pub(crate) async fn do_bench_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_get(ui_port, "/api/bench").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query link benchmark status (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let active = v["active"].as_bool().unwrap_or(false);
    println!("Link Benchmark Status:");
    println!("  Active Session: {active}");
    if let Some(sess) = v.get("session").filter(|s| !s.is_null()) {
        let sid = sess["session_id"].as_str().unwrap_or("unknown");
        let peer_id = sess["peer_id"].as_str().unwrap_or("unknown");
        let role = sess["role"].as_str().unwrap_or("unknown");
        let direction = sess["direction"].as_str().unwrap_or("push");
        let duration = sess["duration_secs"].as_u64().unwrap_or(0);
        let chunk = sess["chunk_size"].as_u64().unwrap_or(0);
        let chunks = sess["chunks_processed"].as_u64().unwrap_or(0);
        let bytes = sess["bytes_transferred"].as_u64().unwrap_or(0);
        println!("  Session ID:      {sid}");
        println!("  Peer:            {peer_id}");
        println!("  Role:            {role} (direction: {direction})");
        println!("  Plan:            {duration}s x {chunk}B chunks");
        println!(
            "  Progress:        {chunks} chunks · {} bytes",
            format_bytes(bytes)
        );
    }
    if let Some(rep) = v.get("last_report").filter(|r| !r.is_null()) {
        let sid = rep["session_id"].as_str().unwrap_or("unknown");
        let direction = rep["direction"].as_str().unwrap_or("push");
        let mbps = rep["throughput_mbps"].as_f64().unwrap_or(0.0);
        let dur = rep["duration_ms"].as_u64().unwrap_or(0);
        let bytes = rep["bytes_transferred"].as_u64().unwrap_or(0);
        let chunks = rep["chunks"].as_u64().unwrap_or(0);
        let latency = rep["avg_chunk_latency_us"].as_u64().unwrap_or(0);
        let jitter = rep["jitter_us"].as_u64().unwrap_or(0);
        println!("Last Report:");
        println!("  Session ID:      {sid}");
        println!("  Direction:       {direction}");
        println!("  Throughput:      {mbps:.2} Mbps");
        println!(
            "  Transferred:     {} ({chunks} chunks in {dur} ms)",
            format_bytes(bytes)
        );
        println!("  Chunk Latency:   {latency}us avg · jitter {jitter}us");
        if rep["stopped_early"].as_bool().unwrap_or(false) {
            println!("  Note:            stopped early");
        }
    }
    if let Some(pol) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = pol["enabled"].as_bool().unwrap_or(true);
        let inbound = pol["allow_inbound_bench"].as_bool().unwrap_or(true);
        let max_dur = pol["max_duration_secs"].as_u64().unwrap_or(30);
        let max_bw = pol["max_bandwidth_mbps"].as_u64().unwrap_or(1000);
        println!("Policy:");
        println!("  Enabled:         {enabled}");
        println!("  Allow Inbound:   {inbound} (serve remote benchmarks)");
        println!("  Max Duration:    {max_dur}s");
        println!("  Max Bandwidth:   {max_bw} Mbps pacing cap");
    }
    Ok(())
}

pub(crate) async fn do_bench_start(
    ui_port: u16,
    peer: &str,
    direction: &str,
    duration: Option<u32>,
    chunk: Option<u32>,
    json: bool,
) -> Result<()> {
    let mut req = serde_json::json!({
        "peer": peer,
        "direction": direction,
    });
    if let Some(d) = duration {
        req["duration_secs"] = serde_json::json!(d);
    }
    if let Some(c) = chunk {
        req["chunk_size"] = serde_json::json!(c);
    }
    let (code, body) =
        loopback_post(ui_port, "/api/bench/start", &serde_json::to_vec(&req)?).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to start link benchmark (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let sid = v["session_id"].as_str().unwrap_or("unknown");
        let peer_val = v["peer_id"].as_str().unwrap_or(peer);
        let msg = v["message"].as_str().unwrap_or("benchmark requested");
        println!("Link benchmark session requested.");
        println!("  Session ID: {sid}");
        println!("  Peer:       {peer_val}");
        println!("  Message:    {msg}");
        println!("  Hint:       poll `tetherly bench status` for the final report.");
    }
    Ok(())
}

pub(crate) async fn do_bench_stop(ui_port: u16, json: bool) -> Result<()> {
    let (code, body) = loopback_post(
        ui_port,
        "/api/bench/stop",
        &serde_json::to_vec(&serde_json::json!({}))?,
    )
    .await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to stop link benchmark (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&body)?;
        let msg = v["message"].as_str().unwrap_or("stopped");
        println!("Link benchmark: {msg}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_bench_policy(
    ui_port: u16,
    action: Option<BenchPolicySubCmd>,
    enable: Option<bool>,
    allow_inbound: Option<bool>,
    max_duration: Option<u32>,
    max_bandwidth: Option<u32>,
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
            BenchPolicySubCmd::Show {
                json: j,
                ui_port: p,
            } => {
                let (code, body) = loopback_get(p, "/api/bench/policy").await?;
                if code != 200 {
                    let err_msg = String::from_utf8_lossy(&body);
                    anyhow::bail!("failed to query bench policy (HTTP {code}): {err_msg}");
                }
                if j {
                    println!("{}", String::from_utf8_lossy(&body));
                } else {
                    print_bench_policy_value(&body)?;
                }
                return Ok(());
            }
            BenchPolicySubCmd::Set {
                enable: e,
                allow_inbound: ai,
                max_duration: md,
                max_bandwidth: mb,
                group: g,
                tag: tg,
                device_type: dt,
                clear_filters: cf,
                json: j,
                ui_port: p,
            } => {
                return Box::pin(do_bench_policy(p, None, e, ai, md, mb, g, tg, dt, cf, j)).await;
            }
        }
    } else {
        if let Some(en) = enable {
            req.insert("enabled".into(), serde_json::json!(en));
            update_needed = true;
        }
        if let Some(ai) = allow_inbound {
            req.insert("allow_inbound_bench".into(), serde_json::json!(ai));
            update_needed = true;
        }
        if let Some(md) = max_duration {
            req.insert("max_duration_secs".into(), serde_json::json!(md));
            update_needed = true;
        }
        if let Some(mb) = max_bandwidth {
            req.insert("max_bandwidth_mbps".into(), serde_json::json!(mb));
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
            loopback_post(ui_port, "/api/bench/policy", &serde_json::to_vec(&req)?).await?;
        if code != 200 {
            let err_msg = String::from_utf8_lossy(&body);
            anyhow::bail!("failed to update bench policy (HTTP {code}): {err_msg}");
        }
        if json {
            println!("{}", String::from_utf8_lossy(&body));
            return Ok(());
        }
    }

    let (code, body) = loopback_get(ui_port, "/api/bench/policy").await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query bench policy (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    print_bench_policy_value(&body)
}

pub(crate) fn print_bench_policy_value(body: &[u8]) -> Result<()> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let pol = v.get("policy").unwrap_or(&v);
    let enabled = pol["enabled"].as_bool().unwrap_or(true);
    let inbound = pol["allow_inbound_bench"].as_bool().unwrap_or(true);
    let max_dur = pol["max_duration_secs"].as_u64().unwrap_or(30);
    let max_bw = pol["max_bandwidth_mbps"].as_u64().unwrap_or(1000);
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

    println!("Link Benchmark Governance Policy:");
    println!("  Enabled:                 {enabled}");
    println!("  Allow Inbound Serving:   {inbound}");
    println!("  Max Duration (s):        {max_dur}");
    println!("  Max Bandwidth (Mbps):    {max_bw} (pacing cap)");
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

pub(crate) async fn do_bench_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!("failed to connect to loopback events stream (is node running?): {e}")
    })?;
    if !json {
        println!("Streaming live link benchmark events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "bench_session_started"
                || kind == "bench_session_finished"
                || kind == "bench_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "bench_session_started" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let role = v["role"].as_str().unwrap_or("unknown");
                            let direction = v["direction"].as_str().unwrap_or("push");
                            let duration = v["duration_secs"].as_u64().unwrap_or(0);
                            println!("[bench_session_started] session={sid} peer={peer} role={role} direction={direction} duration={duration}s");
                        }
                        "bench_session_finished" => {
                            let sid = v["session_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let mbps = v["throughput_mbps"].as_f64().unwrap_or(0.0);
                            let bytes = v["bytes_transferred"].as_u64().unwrap_or(0);
                            let dur = v["duration_ms"].as_u64().unwrap_or(0);
                            println!("[bench_session_finished] session={sid} peer={peer} throughput={mbps:.2}Mbps ({bytes}B in {dur}ms)");
                        }
                        "bench_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let inbound = v["allow_inbound_bench"].as_bool().unwrap_or(true);
                            let max_bw = v["max_bandwidth_mbps"].as_u64().unwrap_or(1000);
                            println!("[bench_policy_changed] enabled={en} allow_inbound={inbound} max_bandwidth={max_bw}Mbps");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn do_recommend(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let path = match peer {
        Some(p) => format!("/api/recommendation?peer={p}"),
        None => "/api/recommendation".to_string(),
    };
    let (code, body) = loopback_get(ui_port, &path).await?;
    if code != 200 {
        let err_msg = String::from_utf8_lossy(&body);
        anyhow::bail!("failed to query streaming recommendation (HTTP {code}): {err_msg}");
    }
    if json {
        println!("{}", String::from_utf8_lossy(&body));
        return Ok(());
    }
    let v: serde_json::Value = serde_json::from_slice(&body)?;

    if let Some(rec) = v.get("recommendation") {
        print_recommendation_value(&v, rec);
        return Ok(());
    }

    println!("Streaming Recommendations (aggregate):");
    match v.get("peers").and_then(|p| p.as_array()) {
        Some(peers) if !peers.is_empty() => {
            for p in peers {
                let alias = p["peer_alias"].as_str().unwrap_or("unknown");
                let mbps = p["measured_mbps"].as_f64().unwrap_or(0.0);
                let tier = p["tier"].as_str().unwrap_or("minimal");
                let direction = p["direction"].as_str().unwrap_or("-");
                println!("  - {alias}: {mbps:.2} Mbps ({direction}) -> tier {tier}");
            }
        }
        _ => {
            println!("  (no measured peers yet; run `tetherly bench start` first)");
        }
    }
    Ok(())
}

pub(crate) fn print_recommendation_value(rep: &serde_json::Value, rec: &serde_json::Value) {
    let alias = rep["peer_alias"].as_str().unwrap_or("unknown");
    let has_profile = rep["has_profile"].as_bool().unwrap_or(false);
    let mbps = rep["measured_mbps"].as_f64().unwrap_or(0.0);
    let tier = rec["tier"].as_str().unwrap_or("minimal");
    println!("Streaming Recommendation for {alias}:");
    if has_profile {
        println!("  Measured:        {mbps:.2} Mbps (receiver-side)");
    } else {
        println!("  Measured:        none yet (conservative fallback)");
    }
    println!("  Tier:            {tier}");
    if let Some(d) = rec.get("display") {
        println!(
            "  Virtual Display: {}x{} @ {}fps",
            d["width"].as_u64().unwrap_or(0),
            d["height"].as_u64().unwrap_or(0),
            d["fps"].as_u64().unwrap_or(0)
        );
    }
    if let Some(c) = rec.get("camera") {
        println!(
            "  Virtual Camera:  {}x{} @ {}fps codec={}",
            c["width"].as_u64().unwrap_or(0),
            c["height"].as_u64().unwrap_or(0),
            c["fps"].as_u64().unwrap_or(0),
            c["codec"].as_str().unwrap_or("-")
        );
    }
    if let Some(a) = rec.get("audio") {
        println!(
            "  Spatial Audio:   {}Hz x{}ch codec={}",
            a["sample_rate"].as_u64().unwrap_or(0),
            a["channels"].as_u64().unwrap_or(0),
            a["codec"].as_str().unwrap_or("-")
        );
    }
    if let Some(m) = rec.get("mic") {
        println!(
            "  Virtual Mic:     {}Hz x{}ch codec={}",
            m["sample_rate"].as_u64().unwrap_or(0),
            m["channels"].as_u64().unwrap_or(0),
            m["codec"].as_str().unwrap_or("-")
        );
    }
}
