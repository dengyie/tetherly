// SPDX-License-Identifier: Apache-2.0 OR MIT
//! URL handoff send/list/open/dismiss/clear/policy.

use super::util::{loopback_delete, loopback_events_connect, loopback_get, loopback_post};
use super::*;

/// `tetherly-cli handoff` -- Handoff arm of the top-level dispatch.
pub(crate) async fn handoff(action: HandoffCmd) -> Result<()> {
    match action {
        HandoffCmd::Send {
            url,
            title,
            peer,
            tag,
            group,
            device_type,
            all,
            auto_open,
            json,
            ui_port,
        } => {
            do_handoff_send(
                ui_port,
                &url,
                title.as_deref(),
                peer.as_deref(),
                tag.as_deref(),
                group.as_deref(),
                device_type.as_deref(),
                all,
                auto_open,
                json,
            )
            .await?;
        }
        HandoffCmd::List {
            status,
            json,
            ui_port,
        } => {
            do_handoff_list(ui_port, status.as_deref(), json).await?;
        }
        HandoffCmd::Open { id, json, ui_port } => {
            do_handoff_open(ui_port, &id, json).await?;
        }
        HandoffCmd::Dismiss { id, json, ui_port } => {
            do_handoff_dismiss(ui_port, &id, json).await?;
        }
        HandoffCmd::Clear { json, ui_port } => {
            do_handoff_clear(ui_port, json).await?;
        }
        HandoffCmd::Policy {
            action,
            mode,
            group,
            tag,
            device_type,
            max_entries,
            clear_filters,
            json,
            ui_port,
        } => {
            do_handoff_policy(
                ui_port,
                action,
                mode,
                group,
                tag,
                device_type,
                max_entries,
                clear_filters,
                json,
            )
            .await?;
        }
        HandoffCmd::Watch { json, ui_port } => {
            do_handoff_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_handoff_send(
    ui_port: u16,
    url: &str,
    title: Option<&str>,
    peer: Option<&str>,
    tag: Option<&str>,
    group: Option<&str>,
    device_type: Option<&str>,
    all: bool,
    auto_open: bool,
    json: bool,
) -> Result<()> {
    let payload = serde_json::json!({
        "url": url,
        "title": title,
        "peer": peer,
        "tag": tag,
        "group": group,
        "device_type": device_type,
        "all": all,
        "auto_open": auto_open,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/handoff/send", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "resource handoff failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if let Some(target_cnt) = v.get("target_count").and_then(|c| c.as_u64()) {
            let succ_cnt = v["success_count"].as_u64().unwrap_or(0);
            let fail_cnt = v["failed_count"].as_u64().unwrap_or(0);
            let skip_cnt = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let sel_desc = v["selector"].to_string();
            let sent_url = v["url"].as_str().unwrap_or(url);
            let sent_title = v["title"].as_str().unwrap_or("");
            println!("Multi-Peer Resource Handoff Dispatched:");
            println!("  Target Selector:      {sel_desc}");
            println!("  Targets / Dispatched: {succ_cnt} / {target_cnt} (failed: {fail_cnt}, skipped: {skip_cnt})");
            println!("  URL:                  {sent_url}");
            if !sent_title.is_empty() {
                println!("  Title:                {sent_title}");
            }
        } else {
            let id = v["id"].as_str().unwrap_or("-");
            let target_peer = v["peer"].as_str().unwrap_or("-");
            let alias_desc = v["peer_alias"]
                .as_str()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            let sent_url = v["url"].as_str().unwrap_or(url);
            let sent_title = v["title"].as_str().unwrap_or("");
            let auto_req = v["auto_open_requested"].as_bool().unwrap_or(false);
            println!("Resource Handoff Dispatched:");
            println!("  Handoff ID:           {id}");
            println!("  Target Peer:          {target_peer}{alias_desc}");
            println!("  URL:                  {sent_url}");
            if !sent_title.is_empty() {
                println!("  Title:                {sent_title}");
            }
            println!("  Auto-Open Requested:  {auto_req}");
        }
    }
    Ok(())
}

pub(crate) async fn do_handoff_list(ui_port: u16, status: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/handoff/inbox".to_string();
    if let Some(s) = status {
        path = format!("{path}?status={s}");
    }
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "failed to list handoff inbox (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let entries = v["inbox"].as_array();
        match entries {
            Some(list) if !list.is_empty() => {
                println!("Resource Handoff Inbox ({} entries):", list.len());
                println!(
                    "  {:<24} {:<10} {:<24} TITLE / URL",
                    "ID", "STATUS", "SENDER"
                );
                println!("  {}", "-".repeat(84));
                for entry in list {
                    let id = entry["id"].as_str().unwrap_or("-");
                    let st = entry["status"].as_str().unwrap_or("-");
                    let sender_alias = entry["sender_alias"].as_str().unwrap_or("");
                    let sender_id = entry["sender_id"].as_str().unwrap_or("");
                    let sender_disp = if !sender_alias.is_empty() {
                        let short_id = if sender_id.len() >= 12 {
                            &sender_id[..12]
                        } else {
                            sender_id
                        };
                        format!("{sender_alias} ({short_id}...)")
                    } else if sender_id.len() >= 20 {
                        format!("{}...", &sender_id[..20])
                    } else {
                        sender_id.to_string()
                    };
                    let url = entry["url"].as_str().unwrap_or("");
                    let title = entry["title"].as_str().unwrap_or("");
                    let resource_disp = if !title.is_empty() {
                        format!("{title} ({url})")
                    } else {
                        url.to_string()
                    };
                    println!(
                        "  {:<24} {:<10} {:<24} {}",
                        id, st, sender_disp, resource_disp
                    );
                }
            }
            _ => println!("(handoff inbox is empty)"),
        }
    }
    Ok(())
}

pub(crate) async fn do_handoff_open(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/handoff/inbox/open", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to open handoff resource (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let entry = &v["entry"];
        let url = entry["url"].as_str().unwrap_or("-");
        let title = entry["title"].as_str().unwrap_or("");
        println!("Opened handoff resource '{id}':");
        if !title.is_empty() {
            println!("  Title:   {title}");
        }
        println!("  URL:     {url}");
        println!("  Status:  opened");
    }
    Ok(())
}

pub(crate) async fn do_handoff_dismiss(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/handoff/inbox/dismiss", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to dismiss handoff resource (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        println!("Dismissed handoff resource '{id}'.");
    }
    Ok(())
}

pub(crate) async fn do_handoff_clear(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = loopback_delete(ui_port, "/api/handoff/inbox")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to clear handoff inbox (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let cleared = v["cleared"].as_u64().unwrap_or(0);
        println!("Cleared {cleared} handoff entries from inbox.");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_handoff_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_entries: Option<usize>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let has_set_intent = action.as_deref() == Some("set")
        || mode.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_entries.is_some()
        || clear_filters;

    if has_set_intent {
        let mut payload = serde_json::Map::new();
        if let Some(m) = mode {
            payload.insert("action".into(), serde_json::Value::String(m));
        }
        if let Some(grps) = group {
            payload.insert(
                "allowed_groups".into(),
                serde_json::Value::Array(grps.into_iter().map(serde_json::Value::String).collect()),
            );
        }
        if let Some(tags) = tag {
            payload.insert(
                "allowed_tags".into(),
                serde_json::Value::Array(tags.into_iter().map(serde_json::Value::String).collect()),
            );
        }
        if let Some(types) = device_type {
            payload.insert(
                "allowed_types".into(),
                serde_json::Value::Array(
                    types.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }
        if let Some(max_e) = max_entries {
            payload.insert("max_inbox_entries".into(), serde_json::json!(max_e));
        }
        if clear_filters {
            payload.insert("clear_filters".into(), serde_json::Value::Bool(true));
        }

        let body = serde_json::to_vec(&payload)?;
        let (code, resp) = loopback_post(ui_port, "/api/handoff/policy", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
                "failed to update handoff policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
    }

    let (code, resp) = loopback_get(ui_port, "/api/handoff/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to query handoff policy (HTTP {code}): {}",
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
    let act = p["action"].as_str().unwrap_or("prompt_only");
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
    let max_entries = p["max_inbox_entries"].as_u64().unwrap_or(100);

    println!("Resource Handoff Ingress Policy:");
    println!("  Action Mode:       {act}");
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
    println!("  Max Inbox Entries: {max_entries}");
    Ok(())
}

pub(crate) async fn do_handoff_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if !json {
        println!("Streaming live resource handoff & policy events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "resource_handoff_received"
                || kind == "resource_handoff_dispatched"
                || kind == "handoff_policy_changed"
                || kind == "handoff_inbox_updated"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "resource_handoff_received" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let sender = v["sender_alias"].as_str().unwrap_or("unknown");
                            let host = v["host"].as_str().unwrap_or("");
                            let title = v["title"].as_str().unwrap_or("");
                            let auto_opened = v["auto_opened"].as_bool().unwrap_or(false);
                            let title_disp = if !title.is_empty() {
                                format!(" \"{title}\"")
                            } else {
                                String::new()
                            };
                            println!("[resource_handoff_received] id={id} from={sender}{title_disp} ({host}) auto_open={auto_opened}");
                        }
                        "resource_handoff_dispatched" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let host = v["host"].as_str().unwrap_or("");
                            let succ = v["success_count"].as_u64().unwrap_or(0);
                            let targets = v["target_count"].as_u64().unwrap_or(0);
                            println!("[resource_handoff_dispatched] id={id} dispatched to {succ}/{targets} targets ({host})");
                        }
                        "handoff_policy_changed" => {
                            let action = v["action"].as_str().unwrap_or("");
                            println!("[handoff_policy_changed] action={action}");
                        }
                        "handoff_inbox_updated" => {
                            let total = v["total_entries"].as_u64().unwrap_or(0);
                            let pending = v["pending_count"].as_u64().unwrap_or(0);
                            println!("[handoff_inbox_updated] total={total} pending={pending}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
