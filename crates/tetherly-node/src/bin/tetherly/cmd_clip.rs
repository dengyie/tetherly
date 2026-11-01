// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard policy, watch and poll.

use super::*;

/// `tetherly clip` -- Clip arm of the top-level dispatch.
pub(crate) async fn clip(action: ClipCmd) -> Result<()> {
    match action {
        ClipCmd::Status { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip/status")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "clipboard status query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let has_text = v["has_text"].as_bool().unwrap_or(false);
                let bytes = v["bytes"].as_u64().unwrap_or(0);
                let hash = v["hash"].as_str().unwrap_or("-");
                let last_seq = v["last_seq"].as_u64().unwrap_or(0);
                let inbound = v["inbound_count"].as_u64().unwrap_or(0);
                let outbound = v["outbound_count"].as_u64().unwrap_or(0);
                let history_count = v["history_count"].as_u64().unwrap_or(0);
                println!("Clipboard Status:");
                if !v["clipboard_available"].as_bool().unwrap_or(true) {
                    println!(
                        "  System Clipboard: unavailable ({})",
                        v["reason"].as_str().unwrap_or("no display server")
                    );
                }
                println!("  Has Content:     {has_text}");
                println!("  Bytes:           {bytes}");
                println!("  SHA-256 Digest:  {hash}");
                println!("  Last Sequence:   {last_seq}");
                println!("  Inbound Synced:  {inbound}");
                println!("  Outbound Synced:  {outbound}");
                println!("  History Count:   {history_count}");
            }
        }
        ClipCmd::History {
            limit,
            json,
            ui_port,
        } => {
            let path = if let Some(n) = limit {
                format!("/api/clip/history?limit={n}")
            } else {
                "/api/clip/history".to_string()
            };
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
                    "clipboard history query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let entries = v["entries"].as_array();
                match entries {
                    Some(items) if !items.is_empty() => {
                        println!(
                            "{:<6} {:<10} {:<34} {:<8} {:<16} TIME",
                            "SEQ", "DIR", "SOURCE", "BYTES", "SHA-256"
                        );
                        for item in items {
                            let seq = item["seq"].as_u64().unwrap_or(0);
                            let dir = item["direction"].as_str().unwrap_or("-");
                            let src = item["source"].as_str().unwrap_or("-");
                            let bytes = item["bytes"].as_u64().unwrap_or(0);
                            let hash = item["hash"].as_str().unwrap_or("-");
                            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                            let ts = item["timestamp_ms"].as_u64().unwrap_or(0);
                            println!(
                                "{:<6} {:<10} {:<34} {:<8} {:<16} {}ms",
                                seq, dir, src, bytes, short_hash, ts
                            );
                        }
                    }
                    _ => println!("(no clipboard sync history)"),
                }
            }
        }
        ClipCmd::Clear { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/clear", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "clipboard clear failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let cleared = v["cleared_history_count"].as_u64().unwrap_or(0);
                println!("clipboard cleared successfully (evicted {cleared} history entries)");
            }
        }
        ClipCmd::Sync { peer, ui_port } => {
            let payload = serde_json::json!({ "peer": peer });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "clipboard sync failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let sent_peers = v.get("sent_peers").and_then(|c| c.as_u64()).unwrap_or(0);
            let bytes = v.get("bytes").and_then(|c| c.as_u64()).unwrap_or(0);
            let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
            println!(
                "synced clipboard ({bytes} bytes, sha256:{short_hash}...) to {sent_peers} peer(s)"
            );
        }
        ClipCmd::Send {
            text,
            peer,
            ui_port,
        } => {
            let content = match text {
                Some(t) => t,
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let payload = serde_json::json!({
                "text": content,
                "peer": peer,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "clipboard send failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let sent_peers = v.get("sent_peers").and_then(|c| c.as_u64()).unwrap_or(0);
            let bytes = v.get("bytes").and_then(|c| c.as_u64()).unwrap_or(0);
            let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
            println!("sent {bytes} bytes to {sent_peers} peer(s) (sha256:{short_hash}...)");
        }
        ClipCmd::Get { ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "clipboard get failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                print!("{text}");
            } else if !v["clipboard_available"].as_bool().unwrap_or(true) {
                eprintln!(
                    "(no OS clipboard on this host: {})",
                    v["reason"].as_str().unwrap_or("no display server")
                );
            } else {
                eprintln!("(clipboard is empty)");
            }
        }
        ClipCmd::Policy {
            action,
            mode,
            sensitive_guard,
            no_sensitive_guard,
            group,
            tag,
            device_type,
            max_bytes,
            auto_clear,
            clear_filters,
            json,
            ui_port,
        } => {
            do_clip_policy(
                ui_port,
                action,
                mode,
                sensitive_guard,
                no_sensitive_guard,
                group,
                tag,
                device_type,
                max_bytes,
                auto_clear,
                clear_filters,
                json,
            )
            .await?;
        }
        ClipCmd::Watch { json, ui_port } => {
            do_clip_watch(ui_port, json).await?;
        }
        ClipCmd::Poll { json, ui_port } => {
            do_clip_poll(ui_port, json).await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_clip_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    sensitive_guard: bool,
    no_sensitive_guard: bool,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_bytes: Option<usize>,
    auto_clear: Option<u64>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    if sensitive_guard && no_sensitive_guard {
        anyhow::bail!("cannot specify both --sensitive-guard and --no-sensitive-guard");
    }
    let has_set_intent = action.as_deref() == Some("set")
        || mode.is_some()
        || sensitive_guard
        || no_sensitive_guard
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_bytes.is_some()
        || auto_clear.is_some()
        || clear_filters;

    if has_set_intent {
        let mut payload = serde_json::Map::new();
        if let Some(m) = mode {
            payload.insert("mode".into(), serde_json::Value::String(m));
        }
        if sensitive_guard {
            payload.insert("sensitive_guard".into(), serde_json::Value::Bool(true));
        } else if no_sensitive_guard {
            payload.insert("sensitive_guard".into(), serde_json::Value::Bool(false));
        }
        if let Some(grps) = group {
            payload.insert("allowed_groups".into(), serde_json::to_value(grps)?);
        }
        if let Some(tags) = tag {
            payload.insert("allowed_tags".into(), serde_json::to_value(tags)?);
        }
        if let Some(types) = device_type {
            payload.insert("allowed_types".into(), serde_json::to_value(types)?);
        }
        if let Some(sz) = max_bytes {
            payload.insert("max_payload_bytes".into(), serde_json::json!(sz));
        }
        if let Some(secs) = auto_clear {
            payload.insert("auto_clear_timeout_secs".into(), serde_json::json!(secs));
        }
        if clear_filters {
            payload.insert("clear_filters".into(), serde_json::Value::Bool(true));
        }
        let body_bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/policy", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to update clipboard policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)?;
            print_clipboard_policy(&v);
        }
    } else {
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip/policy")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to query clipboard policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)?;
            print_clipboard_policy(&v);
        }
    }
    Ok(())
}

pub(crate) fn print_clipboard_policy(v: &serde_json::Value) {
    let p = &v["policy"];
    let mode = p["mode"].as_str().unwrap_or("manual");
    let guard = p["sensitive_guard"].as_bool().unwrap_or(true);
    let max_bytes = p["max_payload_bytes"].as_u64().unwrap_or(65536);
    let auto_clear = p["auto_clear_timeout_secs"].as_u64();
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

    let auto_out = v["auto_synced_out_count"].as_u64().unwrap_or(0);
    let auto_in = v["auto_synced_in_count"].as_u64().unwrap_or(0);
    let blocked = v["sensitive_blocked_count"].as_u64().unwrap_or(0);

    println!("Clipboard Policy & Governance:");
    println!("  Mode:              {mode}");
    println!(
        "  Sensitive Guard:   {}",
        if guard {
            "enabled (fail-closed, blocks keys & credentials)"
        } else {
            "disabled (unrestricted)"
        }
    );
    println!(
        "  Allowed Groups:    {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:      {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:     {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    println!("  Max Payload Bytes: {max_bytes} bytes");
    println!(
        "  Auto-Clear Timer:  {}",
        match auto_clear {
            Some(secs) if secs > 0 => format!("{secs} seconds"),
            _ => "disabled".to_string(),
        }
    );
    println!("  Auto-Synced Out:   {auto_out}");
    println!("  Auto-Synced In:    {auto_in}");
    println!("  Sensitive Blocked: {blocked}");
}

pub(crate) async fn do_clip_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!(
            "Streaming live clipboard synchronization & security events on 127.0.0.1:{ui_port}..."
        );
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "clipboard_synced"
                || kind == "clipboard_policy_changed"
                || kind == "clipboard_sync_blocked"
                || kind == "clipboard_cleared"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "clipboard_synced" => {
                            let seq = v["seq"].as_u64().unwrap_or(0);
                            let dir = v["direction"].as_str().unwrap_or("");
                            let bytes = v["bytes"].as_u64().unwrap_or(0);
                            let hash = v["hash"].as_str().unwrap_or("");
                            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                            let src = v["source"].as_str().unwrap_or("");
                            let peers = v["sent_peers"].as_u64().unwrap_or(0);
                            if dir == "outbound" {
                                println!("[clipboard_synced] outbound seq={seq} from={src} ({bytes} bytes, sha256:{short_hash}...) to {peers} peer(s)");
                            } else {
                                println!("[clipboard_synced] inbound seq={seq} from={src} ({bytes} bytes, sha256:{short_hash}...)");
                            }
                        }
                        "clipboard_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let guard = v["sensitive_guard"].as_bool().unwrap_or(true);
                            let max_bytes = v["max_payload_bytes"].as_u64().unwrap_or(0);
                            println!("[clipboard_policy_changed] mode={mode} sensitive_guard={guard} max_payload_bytes={max_bytes}");
                        }
                        "clipboard_sync_blocked" => {
                            let reason = v["reason"].as_str().unwrap_or("");
                            let bytes = v["bytes"].as_u64().unwrap_or(0);
                            let hash = v["hash"].as_str().unwrap_or("");
                            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                            println!("[clipboard_sync_blocked] BLOCKED reason={reason} ({bytes} bytes, sha256:{short_hash}...)");
                        }
                        "clipboard_cleared" => {
                            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
                            println!("[clipboard_cleared] timestamp_ms={ts}");
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn do_clip_poll(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/auto-sync/poll", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "clipboard auto-sync poll failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&resp)?;
        let dispatched = v["dispatched_peers"].as_u64().unwrap_or(0);
        let blocked = v["stats"]["sensitive_blocked_count"].as_u64().unwrap_or(0);
        let out = v["stats"]["auto_synced_out_count"].as_u64().unwrap_or(0);
        println!("Clipboard Auto-Sync Poll Result:");
        println!("  Dispatched Peers:    {dispatched}");
        println!("  Total Auto-Synced:   {out}");
        println!("  Sensitive Blocked:   {blocked}");
    }
    Ok(())
}
