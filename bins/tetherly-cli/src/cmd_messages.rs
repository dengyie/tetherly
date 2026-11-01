// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer text message send/history/clear/watch.

use super::cmd_core::print_event;
use super::util::{loopback_events_connect, loopback_get, loopback_post, now_ms};
use super::*;

/// `tetherly-cli message` -- Message arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn message(
    action: Option<MessageCmd>,
    peer: Option<String>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    all: bool,
    text: Option<String>,
    limit: Option<usize>,
    unread: bool,
    mark_read: bool,
    clear: bool,
    watch: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    handle_message_cmd(
        action,
        peer,
        tag,
        group,
        device_type,
        all,
        text,
        limit,
        unread,
        mark_read,
        clear,
        watch,
        json,
        ui_port,
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message_cmd(
    action: Option<MessageCmd>,
    default_peer: Option<String>,
    default_tag: Option<String>,
    default_group: Option<String>,
    default_type: Option<String>,
    default_all: bool,
    default_text: Option<String>,
    default_limit: Option<usize>,
    default_unread: bool,
    default_mark_read: bool,
    default_clear: bool,
    default_watch: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_message_clear(default_peer, default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_message_watch(default_peer, 0, default_json, default_ui_port).await;
    }
    if let Some(txt) = default_text {
        return handle_message_send(
            default_peer,
            default_tag,
            default_group,
            default_type,
            default_all,
            &txt,
            default_json,
            default_ui_port,
        )
        .await;
    }
    match action {
        None => {
            handle_message_history(
                default_peer,
                default_limit,
                default_unread,
                default_mark_read,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(MessageCmd::Send {
            peer,
            tag,
            group,
            device_type,
            all,
            text,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let tg = tag.or(default_tag);
            let grp = group.or(default_group);
            let dt = device_type.or(default_type);
            let is_all = all || default_all;
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_send(p, tg, grp, dt, is_all, &text, j, port).await
        }
        Some(MessageCmd::History {
            peer,
            limit,
            unread,
            mark_read,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let lim = limit.or(default_limit);
            let un = unread || default_unread;
            let mr = mark_read || default_mark_read;
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_history(p, lim, un, mr, j, port).await
        }
        Some(MessageCmd::Clear {
            peer,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_clear(p, j, port).await
        }
        Some(MessageCmd::Watch {
            peer,
            count,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_watch(p, count, j, port).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_message_send(
    peer: Option<String>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    all: bool,
    text: &str,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let is_multi = all || tag.is_some() || group.is_some() || device_type.is_some();
    if is_multi {
        let payload = serde_json::json!({
            "peer": peer,
            "tag": tag,
            "group": group,
            "type": device_type,
            "all": all,
            "text": text,
        });
        let bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = loopback_post(ui_port, "/api/message/send-multi", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
                "failed to send targeted message (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let val: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let selector = val["selector"].as_str().unwrap_or("target");
            let total = val["target_count"].as_u64().unwrap_or(0);
            let succ = val["success_count"].as_u64().unwrap_or(0);
            let skipped = val["skipped_offline_count"].as_u64().unwrap_or(0);
            let failed = val["failed_count"].as_u64().unwrap_or(0);
            println!(
                "Multi-Dispatch Message Report [{selector}]: {succ}/{total} sent, {skipped} skipped offline, {failed} failed"
            );
            if let Some(dispatches) = val["dispatches"].as_array() {
                if !dispatches.is_empty() {
                    println!(
                        "  {:<20} {:<16} {:<10} {:<20} {:<20}",
                        "TARGET", "ALIAS", "STATUS", "MESSAGE ID", "ERROR"
                    );
                    println!("  {}", "-".repeat(88));
                    for d in dispatches {
                        let dev = d["peer"].as_str().unwrap_or("?");
                        let dev_short = if dev.len() > 18 { &dev[..18] } else { dev };
                        let alias = d["peer_alias"].as_str().unwrap_or("-");
                        let alias_short = if alias.len() > 14 {
                            &alias[..14]
                        } else {
                            alias
                        };
                        let status = if d["success"].as_bool().unwrap_or(false) {
                            "sent"
                        } else {
                            "failed"
                        };
                        let msg_id = d["message_id"].as_str().unwrap_or("-");
                        let msg_id_short = if msg_id.len() > 18 {
                            &msg_id[..18]
                        } else {
                            msg_id
                        };
                        let err = d["error"].as_str().unwrap_or("-");
                        let err_short = if err.len() > 18 { &err[..18] } else { err };
                        println!(
                            "  {:<20} {:<16} {:<10} {:<20} {:<20}",
                            dev_short, alias_short, status, msg_id_short, err_short
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    let payload = serde_json::json!({
        "peer": peer.unwrap_or_default(),
        "text": text,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/message/send", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to send message (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let msg = &val["message"];
        let id = msg["id"].as_str().unwrap_or("?");
        let to = msg["peer"].as_str().unwrap_or("?");
        let txt = msg["text"].as_str().unwrap_or("");
        println!("Message sent successfully!");
        println!("  ID:   {id}");
        println!("  To:   {to}");
        println!("  Text: {txt}");
    }
    Ok(())
}

pub(crate) async fn handle_message_history(
    peer: Option<String>,
    limit: Option<usize>,
    unread_only: bool,
    mark_read: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(p) = &peer {
        query_params.push(format!("peer={p}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    if unread_only {
        query_params.push("unread_only=true".to_string());
    }
    if mark_read {
        query_params.push("mark_read=true".to_string());
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/message/history{qs}");
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "failed to query message history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_message_history_report(&val);
    }
    Ok(())
}

pub(crate) fn print_message_history_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let unread = val["unread"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Direct Message History (total: {total}, unread: {unread}, showing: {returned}):");
    let messages = match val["messages"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no messages found)");
            return;
        }
    };
    println!(
        "  {:<16} {:<10} {:<10} {:<24} {:<6} {:<30}",
        "ID", "DIR", "TIME", "PEER", "READ", "CONTENT"
    );
    println!("  {}", "-".repeat(100));
    let now = now_ms();
    for m in messages {
        let id = m["id"].as_str().unwrap_or("-");
        let dir = m["direction"].as_str().unwrap_or("in");
        let ts = m["timestamp_ms"].as_u64().unwrap_or(0);
        let ago_str = if now >= ts {
            let diff_s = (now - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        let peer = m["peer"].as_str().unwrap_or("-");
        let read = if m["read"].as_bool().unwrap_or(true) {
            "yes"
        } else {
            "NO"
        };
        let text = m["text"].as_str().unwrap_or("");
        let clean_text: String = text.chars().take(40).collect();
        println!(
            "  {:<16} {:<10} {:<10} {:<24} {:<6} {:<30}",
            id, dir, ago_str, peer, read, clean_text
        );
    }
}

pub(crate) async fn handle_message_clear(
    peer: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/message/clear", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to clear messages (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let target = val["target"].as_str().unwrap_or("all");
        println!("Direct messages cleared: {count} messages purged (target: {target})");
    }
    Ok(())
}

pub(crate) async fn handle_message_watch(
    peer: Option<String>,
    count: usize,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if !json {
        let filter_info = match &peer {
            Some(p) => format!(" (filter peer: {p})"),
            None => String::new(),
        };
        println!("Streaming live direct message events on 127.0.0.1:{ui_port}{filter_info}...");
    }
    let mut received = 0usize;
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = val["kind"].as_str().unwrap_or("");
            if kind == "message_received" || kind == "message_sent" {
                if let Some(target_p) = &peer {
                    let ev_p = val["peer"].as_str().unwrap_or("");
                    if !ev_p.contains(target_p) {
                        continue;
                    }
                }
                received += 1;
                if json {
                    println!("{raw}");
                } else {
                    print_event(&val);
                }
                if count > 0 && received >= count {
                    break;
                }
            }
        }
    }
    Ok(())
}
