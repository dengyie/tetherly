// SPDX-License-Identifier: Apache-2.0 OR MIT
//! File send/transfer/offer/accept flows plus storage and inbox/outbox.

use super::cmd_core::print_event;
use super::util::{format_bytes, format_speed, format_timestamp_ago, url_encode};
use super::*;

/// `tetherly send` -- Send arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send(
    paths: Vec<PathBuf>,
    name: Option<String>,
    peer: Option<String>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    all: bool,
    json: bool,
    watch: bool,
    ui_port: u16,
) -> Result<()> {
    if paths.is_empty() {
        anyhow::bail!("no file paths provided");
    }
    let is_multi = all || tag.is_some() || group.is_some() || device_type.is_some();
    if is_multi {
        for path in &paths {
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?;
            let payload = serde_json::json!({
                "path": path_str,
                "name": name,
                "peer": peer,
                "tag": tag,
                "group": group,
                "type": device_type,
                "all": all,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/file/send-multi", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused targeted file send (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let fname = rep["filename"].as_str().unwrap_or("file");
                let fsize = rep["size"].as_u64().unwrap_or(0);
                let fhash = rep["sha256"].as_str().unwrap_or("");
                let short_hash = if fhash.len() > 12 {
                    &fhash[..12]
                } else {
                    fhash
                };
                let selector = rep["selector"].as_str().unwrap_or("target");
                let total = rep["target_count"].as_u64().unwrap_or(0);
                let succ = rep["success_count"].as_u64().unwrap_or(0);
                let skipped = rep["skipped_offline_count"].as_u64().unwrap_or(0);
                let failed = rep["failed_count"].as_u64().unwrap_or(0);
                println!(
                    "Multi-Dispatch File [{fname}] ({}, sha256:{short_hash}...) to [{selector}]: {succ}/{total} offered, {skipped} skipped offline, {failed} failed",
                    format_bytes(fsize)
                );
                if let Some(dispatches) = rep["dispatches"].as_array() {
                    if !dispatches.is_empty() {
                        println!(
                            "  {:<20} {:<16} {:<10} {:<20} {:<20} {:<20}",
                            "TARGET", "ALIAS", "STATUS", "TRANSFER ID", "OUTBOX ID", "ERROR"
                        );
                        println!("  {}", "-".repeat(108));
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
                                "offered"
                            } else {
                                "failed"
                            };
                            let tid = d["transfer_id"].as_str().unwrap_or("-");
                            let tid_short = if tid.len() > 18 { &tid[..18] } else { tid };
                            let obid = d["outbox_id"].as_str().unwrap_or("-");
                            let obid_short = if obid.len() > 18 { &obid[..18] } else { obid };
                            let err = d["error"].as_str().unwrap_or("-");
                            let err_short = if err.len() > 18 { &err[..18] } else { err };
                            println!(
                                "  {:<20} {:<16} {:<10} {:<20} {:<20} {:<20}",
                                dev_short, alias_short, status, tid_short, obid_short, err_short
                            );
                        }
                    }
                }
            }
        }
        return Ok(());
    }
    if paths.len() == 1 {
        let path = &paths[0];
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?;
        let payload = serde_json::json!({
            "path": path_str,
            "name": name,
            "peer": peer,
        });
        let bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/send", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused file send (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let transfer_id = v.get("transfer_id").and_then(|p| p.as_str()).unwrap_or("?");
        let target_peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
        let file_name = v.get("name").and_then(|p| p.as_str()).unwrap_or("?");
        let size = v.get("size").and_then(|p| p.as_u64()).unwrap_or(0);
        println!("offered {file_name} ({size} bytes) to {target_peer} [{transfer_id}]");

        if watch {
            println!("watching transfer progress for [{transfer_id}]...");
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                let (status_code, state_resp) =
                    tetherly_node::loopback_get(ui_port, "/api/state").await?;
                if status_code != 200 {
                    continue;
                }
                let state: serde_json::Value = match serde_json::from_slice(&state_resp) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let transfers = state.get("transfers").and_then(|t| t.as_array());
                let current = transfers.and_then(|list| {
                    list.iter().find(|t| {
                        t.get("transfer_id").and_then(|id| id.as_str()) == Some(transfer_id)
                    })
                });
                if let Some(t) = current {
                    let transferred = t.get("transferred").and_then(|v| v.as_u64()).unwrap_or(0);
                    let total = t.get("total").and_then(|v| v.as_u64()).unwrap_or(size);
                    let speed_bps = t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                    let phase = t
                        .get("phase")
                        .and_then(|v| v.as_str())
                        .unwrap_or("transferring");
                    let pct = if total > 0 {
                        (transferred as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    print!(
                        "\r[{:>5.1}%] {} / {} ({}) [{}]      ",
                        pct,
                        format_bytes(transferred),
                        format_bytes(total),
                        format_speed(speed_bps),
                        phase
                    );
                    use std::io::Write;
                    let _ = std::io::stdout().flush();

                    if phase == "completed" {
                        println!(
                            "\n✓ Transfer completed: {file_name} ({})",
                            format_bytes(total)
                        );
                        break;
                    } else if phase == "failed" {
                        let err_msg = t
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("transfer failed");
                        anyhow::bail!("\n✗ Transfer failed: {err_msg}");
                    }
                }
            }
        }
    } else {
        if name.is_some() {
            anyhow::bail!("--name cannot be specified when sending multiple files in batch");
        }
        let path_strs: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let payload = serde_json::json!({
            "paths": path_strs,
            "peer": peer,
        });
        let bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/send-batch", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused batch file send (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: tetherly_node::BatchSendReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let target = rep.peer_alias.as_deref().unwrap_or(&rep.peer);
        println!(
            "Batch [{}] to {}: {}/{} succeeded, {} failed ({} total)",
            rep.batch_id,
            target,
            rep.successful_files,
            rep.total_files,
            rep.failed_files,
            format_bytes(rep.total_bytes)
        );
        for item in rep.items {
            if item.status == "sent" || item.status == "queued" {
                let tid = item.transfer_id.as_deref().unwrap_or("?");
                println!("  ✓ {} ({}) -> [{tid}]", item.name, format_bytes(item.size));
            } else {
                let err = item.error.as_deref().unwrap_or("unknown error");
                println!("  ✗ {} -> failed: {err}", item.name);
            }
        }
    }
    Ok(())
}

/// `tetherly resume` -- Resume arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn resume(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/file/resume")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node returned HTTP {code} querying resume markers: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }
    let rep: tetherly_node::ResumeMarkersReport =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if rep.markers.is_empty() {
        println!("No resumable transfer markers found in inbox.");
        return Ok(());
    }
    println!(
        "Found {} resumable marker(s) ({} total partial data):",
        rep.count,
        format_bytes(rep.total_partial_bytes)
    );
    println!(
        "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
        "TRANSFER ID", "SOURCE", "OFFSET / TOTAL", "PROGRESS", "ALIGNED", ".PART FILE"
    );
    for m in rep.markers {
        let src = m.source_alias.as_deref().unwrap_or(&m.source);
        let off_tot = format!(
            "{} / {}",
            format_bytes(m.offset_bytes),
            format_bytes(m.total_bytes)
        );
        let prog = format!("{:.1}%", m.progress_percent);
        let align = if m.chunk_aligned { "yes" } else { "no" };
        let part = if m.part_file_exists {
            format!("yes ({})", format_bytes(m.part_file_bytes))
        } else {
            "no".to_string()
        };
        println!(
            "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
            m.transfer_id, src, off_tot, prog, align, part
        );
    }
    Ok(())
}

/// `tetherly cancel` -- Cancel arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn cancel(id: String, ui_port: u16) -> Result<()> {
    let body = serde_json::to_vec(&serde_json::json!({ "id": id }))?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/cancel", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "cancel failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    println!("cancelled transfer [{id}]");
    Ok(())
}

/// `tetherly transfers` -- Transfers arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn transfers(
    id: Option<String>,
    active: bool,
    terminal: bool,
    direction: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    do_transfers(ui_port, id, active, terminal, direction, json).await?;
    Ok(())
}

/// `tetherly clear transfers` -- ClearTransfers arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn clear_transfers(ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/clear", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "clear failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let cleared = v.get("cleared").and_then(|c| c.as_u64()).unwrap_or(0);
    println!("cleared {cleared} finished transfer(s) from history");
    Ok(())
}

/// `tetherly offers` -- Offers arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn offers(
    peer: Option<String>,
    id: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    do_file_offers(ui_port, peer, id, json).await?;
    Ok(())
}

/// `tetherly accept` -- Accept arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn accept(
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    do_file_accept(ui_port, id, all, peer, json).await?;
    Ok(())
}

/// `tetherly reject` -- Reject arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn reject(
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    do_file_reject(ui_port, id, all, peer, json).await?;
    Ok(())
}

/// `tetherly auto accept` -- AutoAccept arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn auto_accept(
    enable: bool,
    disable: bool,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_size: Option<u64>,
    clear_filters: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    do_file_auto_accept(
        ui_port,
        enable,
        disable,
        group,
        tag,
        device_type,
        max_size,
        clear_filters,
        json,
    )
    .await?;
    Ok(())
}

/// `tetherly storage` -- Storage arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn storage(action: StorageCmd) -> Result<()> {
    match action {
        StorageCmd::Status { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/storage")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "storage status query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::StorageReport = serde_json::from_slice(&resp)?;
                println!("Node Storage Status:");
                println!("  Data Directory:    {}", rep.data_dir);
                println!("  Inbox Directory:   {}", rep.inbox_dir);
                println!(
                    "  Inbox Files:       {} ({})",
                    rep.inbox_file_count,
                    format_bytes(rep.inbox_total_bytes)
                );
                println!(
                    "  Partial In-flight: {} ({})",
                    rep.partial_count,
                    format_bytes(rep.partial_bytes)
                );
                println!(
                    "  Orphaned Partials: {} ({})",
                    rep.orphaned_partial_count,
                    format_bytes(rep.orphaned_partial_bytes)
                );
                println!("  Metadata Storage:  {}", format_bytes(rep.meta_bytes));
            }
        }
        StorageCmd::Clean { all, ui_port } => {
            let payload = serde_json::json!({ "orphaned_only": !all });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/storage/clean", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "storage clean failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: tetherly_node::StorageCleanReport = serde_json::from_slice(&resp)?;
            println!("Storage cleanup complete:");
            println!("  Freed Space:     {}", format_bytes(rep.freed_bytes));
            println!("  Deleted Files:   {}", rep.deleted_files_count);
            if rep.cleared_resume_entries > 0 {
                println!("  Cleared Resumes: {}", rep.cleared_resume_entries);
            }
        }
    }
    Ok(())
}

/// `tetherly inbox` -- Inbox arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inbox(action: InboxCmd) -> Result<()> {
    match action {
        InboxCmd::List {
            search,
            limit,
            json,
            ui_port,
        } => {
            let mut path = "/api/file/inbox".to_string();
            let mut query_params = Vec::new();
            if let Some(s) = search {
                query_params.push(format!("search={s}"));
            }
            if let Some(l) = limit {
                query_params.push(format!("limit={l}"));
            }
            if !query_params.is_empty() {
                path.push('?');
                path.push_str(&query_params.join("&"));
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
                    "inbox list query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::InboxReport = serde_json::from_slice(&resp)?;
                if rep.files.is_empty() {
                    println!("Inbox is empty (0 files).");
                } else {
                    println!(
                        "Inbox Files ({}, total {}):",
                        rep.count,
                        format_bytes(rep.total_bytes)
                    );
                    println!(
                        "{:<32} {:<12} {:<16} {:<6}",
                        "NAME", "SIZE", "MODIFIED", "SAFE"
                    );
                    println!("{:-<70}", "");
                    for f in &rep.files {
                        let mod_str = format_timestamp_ago(f.modified_at_secs);
                        let safe_str = if f.is_safe { "yes" } else { "no" };
                        let name_display = if f.name.len() > 30 {
                            format!("{}...", &f.name[..27])
                        } else {
                            f.name.clone()
                        };
                        println!(
                            "{:<32} {:<12} {:<16} {:<6}",
                            name_display,
                            format_bytes(f.size),
                            mod_str,
                            safe_str,
                        );
                    }
                }
            }
        }
        InboxCmd::Delete {
            name,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "name": name });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/file/inbox/delete", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "inbox delete failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::InboxDeleteReport = serde_json::from_slice(&resp)?;
                println!(
                    "Deleted inbox file '{}' (freed {}).",
                    rep.name,
                    format_bytes(rep.freed_bytes)
                );
            }
        }
        InboxCmd::Clear {
            yes,
            before_secs,
            json,
            ui_port,
        } => {
            if !yes && !json {
                eprint!("Are you sure you want to clear completed files from the inbox? [y/N] ");
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Inbox clear aborted.");
                    return Ok(());
                }
            }
            let payload = serde_json::json!({ "before_secs": before_secs });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/file/inbox/clear", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "inbox clear failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::InboxClearReport = serde_json::from_slice(&resp)?;
                println!(
                    "Inbox cleared: deleted {} files (freed {}).",
                    rep.deleted_count,
                    format_bytes(rep.freed_bytes)
                );
            }
        }
        InboxCmd::Get {
            name,
            dest,
            ui_port,
        } => {
            let encoded_name = url_encode(&name);
            let path = format!("/api/file/inbox/download?name={encoded_name}");
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
                    "inbox get failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let target_path = match dest {
                Some(p) => {
                    if p.is_dir() {
                        p.join(&name)
                    } else {
                        p
                    }
                }
                None => PathBuf::from(&name),
            };
            if let Some(parent) = target_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(&target_path, &resp)?;
            println!(
                "Extracted '{}' ({} bytes) -> {}",
                name,
                resp.len(),
                target_path.display()
            );
        }
        InboxCmd::Open {
            name,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "name": name });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/file/inbox/open", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "inbox open failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::InboxOpenReport = serde_json::from_slice(&resp)?;
                if let Some(f) = rep.name {
                    println!(
                        "Opened file '{}' in system default handler (path: {}).",
                        f, rep.path
                    );
                } else {
                    println!(
                        "Opened inbox directory in system file manager (path: {}).",
                        rep.path
                    );
                }
            }
        }
    }
    Ok(())
}

/// `tetherly outbox` -- Outbox arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn outbox(
    action: Option<OutboxCmd>,
    peer: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
    clear: bool,
    resend: Option<String>,
    watch: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    handle_outbox_cmd(
        action, peer, status, limit, clear, resend, watch, json, ui_port,
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_outbox_cmd(
    action: Option<OutboxCmd>,
    default_peer: Option<String>,
    default_status: Option<String>,
    default_limit: Option<usize>,
    default_clear: bool,
    default_resend: Option<String>,
    default_watch: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_outbox_clear(default_peer, default_status, default_json, default_ui_port)
            .await;
    }
    if let Some(resend_id) = default_resend {
        return handle_outbox_resend(&resend_id, default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_outbox_watch(default_peer, default_json, default_ui_port).await;
    }
    match action {
        None => {
            handle_outbox_list(
                default_peer,
                default_status,
                default_limit,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(OutboxCmd::List {
            peer,
            status,
            limit,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let s = status.or(default_status);
            let lim = limit.or(default_limit);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_list(p, s, lim, j, port).await
        }
        Some(OutboxCmd::Clear {
            peer,
            status,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let s = status.or(default_status);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_clear(p, s, j, port).await
        }
        Some(OutboxCmd::Resend { id, json, ui_port }) => {
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_resend(&id, j, port).await
        }
        Some(OutboxCmd::Watch { peer, ui_port }) => {
            let p = peer.or(default_peer);
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_watch(p, default_json, port).await
        }
    }
}

pub(crate) async fn handle_outbox_list(
    peer: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(p) = &peer {
        query_params.push(format!("peer={p}"));
    }
    if let Some(s) = &status {
        query_params.push(format!("status={s}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/file/outbox{qs}");
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
            "failed to query outbox history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_outbox_report(&val);
    }
    Ok(())
}

pub(crate) fn print_outbox_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Outbox & Sent Files History (total: {total}, showing: {returned}):");
    let entries = match val["entries"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no outbox entries found)");
            return;
        }
    };
    println!(
        "  {:<20} {:<24} {:<12} {:<12} {:<12} {:<12} {:<24}",
        "ID", "FILENAME", "SIZE", "PEER", "STATUS", "SPEED", "STARTED"
    );
    println!("  {}", "-".repeat(110));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    for e in entries {
        let id = e["id"].as_str().unwrap_or("-");
        let filename = e["filename"].as_str().unwrap_or("-");
        let clean_filename: String = filename.chars().take(22).collect();
        let size = e["size"].as_u64().unwrap_or(0);
        let peer = e["peer"].as_str().unwrap_or("-");
        let short_peer = if peer.len() > 10 { &peer[..10] } else { peer };
        let status = e["status"].as_str().unwrap_or("unknown");
        let speed = e["speed_bps"].as_u64().unwrap_or(0);
        let speed_str = if speed > 0 {
            format!("{}/s", format_bytes(speed))
        } else {
            "-".to_string()
        };
        let ts = e["started_ms"].as_u64().unwrap_or(0);
        let ago_str = if now_ms >= ts {
            let diff_s = (now_ms - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        println!(
            "  {:<20} {:<24} {:<12} {:<12} {:<12} {:<12} {:<24}",
            id,
            clean_filename,
            format_bytes(size),
            short_peer,
            status,
            speed_str,
            ago_str
        );
    }
}

pub(crate) async fn handle_outbox_clear(
    peer: Option<String>,
    status: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "status": status,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/outbox/clear", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear outbox records (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let target_p = val["target_peer"].as_str().unwrap_or("all");
        let target_s = val["target_status"].as_str().unwrap_or("all");
        println!("Outbox purged: {count} entries cleared (peer: {target_p}, status: {target_s})");
    }
    Ok(())
}

pub(crate) async fn handle_outbox_resend(id: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "id": id,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/outbox/resend", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to resend outbox file (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let orig = val["original_id"].as_str().unwrap_or(id);
        let new_id = val["new_transfer_id"].as_str().unwrap_or("?");
        let msg = val["message"].as_str().unwrap_or("success");
        println!("File re-offered successfully!");
        println!("  Original: {orig}");
        println!("  New TxID: {new_id}");
        println!("  Status:   {msg}");
    }
    Ok(())
}

pub(crate) async fn handle_outbox_watch(
    peer: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
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
        println!("Streaming live outbox events on 127.0.0.1:{ui_port}{filter_info}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = val["kind"].as_str().unwrap_or("");
            if kind == "outbox_updated" || kind == "outbox_cleared" {
                if let Some(target_p) = &peer {
                    let ev_p = val["peer"].as_str().unwrap_or("");
                    if !ev_p.contains(target_p) {
                        continue;
                    }
                }
                if json {
                    println!("{raw}");
                } else {
                    print_event(&val);
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn do_transfers(
    ui_port: u16,
    id: Option<String>,
    active: bool,
    terminal: bool,
    direction: Option<String>,
    json: bool,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(ref target_id) = id {
        query_params.push(format!("id={target_id}"));
    }
    if active {
        query_params.push("active=true".to_string());
    }
    if terminal {
        query_params.push("terminal=true".to_string());
    }
    if let Some(ref dir) = direction {
        query_params.push(format!("direction={dir}"));
    }

    let path = if query_params.is_empty() {
        "/api/file/transfers".to_string()
    } else {
        format!("/api/file/transfers?{}", query_params.join("&"))
    };

    let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code == 404 {
        if let Some(ref target_id) = id {
            anyhow::bail!("transfer not found: {target_id}");
        } else {
            anyhow::bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        anyhow::bail!(
            "query transfers failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    if id.is_some() {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let dir = v.get("direction").and_then(|s| s.as_str()).unwrap_or("?");
        let peer = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        let total = v.get("total").and_then(|s| s.as_u64()).unwrap_or(0);
        let transferred = v.get("transferred").and_then(|s| s.as_u64()).unwrap_or(0);
        let speed = v.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
        let phase = v.get("phase").and_then(|s| s.as_str()).unwrap_or("?");
        let started_ms = v.get("started_ms").and_then(|s| s.as_u64()).unwrap_or(0);
        let error = v.get("error").and_then(|s| s.as_str());
        let pct = if total > 0 {
            (transferred as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        println!("Transfer {tid}:");
        println!("  File:      {filename}");
        println!("  Direction: {dir}");
        println!("  Peer:      {peer}");
        println!("  Phase:     {phase}");
        println!(
            "  Progress:  {:>5.1}% ({} / {})",
            pct,
            format_bytes(transferred),
            format_bytes(total)
        );
        if speed > 0 {
            println!("  Speed:     {}", format_speed(speed));
        }
        if started_ms > 0 {
            println!("  Started:   {started_ms} ms");
        }
        if let Some(err_msg) = error {
            println!("  Error:     {err_msg}");
        }
        return Ok(());
    }

    let active_count = v.get("active_count").and_then(|s| s.as_u64()).unwrap_or(0);
    let terminal_count = v
        .get("terminal_count")
        .and_then(|s| s.as_u64())
        .unwrap_or(0);
    let total_count = v.get("total_count").and_then(|s| s.as_u64()).unwrap_or(0);

    println!("Transfers: {total_count} total ({active_count} active, {terminal_count} terminal)");

    if let Some(stats) = v.get("stats") {
        let sent = stats
            .get("bytes_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let recv = stats
            .get("bytes_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let completed = stats
            .get("completed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let failed = stats
            .get("failed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  Cumulative Metrics: sent: {}, received: {}, completed: {}, failed: {}",
            format_bytes(sent),
            format_bytes(recv),
            completed,
            failed
        );
    }

    let transfers = v.get("transfers").and_then(|t| t.as_array());
    match transfers {
        Some(list) if !list.is_empty() => {
            println!("\n  Transfers List:");
            for t in list {
                let id = t.get("transfer_id").and_then(|v| v.as_str()).unwrap_or("?");
                let filename = t.get("filename").and_then(|v| v.as_str()).unwrap_or("?");
                let transferred = t.get("transferred").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = t.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let speed_bps = t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                let phase = t.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
                let dir = t.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
                let pct = if total > 0 {
                    (transferred as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                let speed_str = if speed_bps > 0 {
                    format!("{} ", format_speed(speed_bps))
                } else {
                    "".to_string()
                };
                println!(
                    "    - [{dir}] {filename} ({:>5.1}%, {} / {}) {speed_str}[{phase}] ({id})",
                    pct,
                    format_bytes(transferred),
                    format_bytes(total)
                );
            }
        }
        _ => {
            println!("  No matching transfers recorded.");
        }
    }

    Ok(())
}

pub(crate) async fn do_file_offers(
    ui_port: u16,
    peer: Option<String>,
    id: Option<String>,
    json: bool,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(ref target_id) = id {
        query_params.push(format!("id={target_id}"));
    }
    if let Some(ref target_peer) = peer {
        query_params.push(format!("peer={target_peer}"));
    }
    let path = if query_params.is_empty() {
        "/api/file/offers".to_string()
    } else {
        format!("/api/file/offers?{}", query_params.join("&"))
    };
    let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code == 404 {
        if let Some(ref target_id) = id {
            anyhow::bail!("file offer not found: {target_id}");
        } else {
            anyhow::bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        anyhow::bail!(
            "query file offers failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    if id.is_some() {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let source = v.get("source").and_then(|s| s.as_str()).unwrap_or("?");
        let total_bytes = v.get("total_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
        let resume_from = v.get("resume_from").and_then(|s| s.as_u64()).unwrap_or(0);
        let ts = v.get("timestamp_ms").and_then(|s| s.as_u64()).unwrap_or(0);

        println!("File Offer {tid}:");
        println!("  File:        {filename}");
        println!("  Source:      {source}");
        println!(
            "  Total Bytes: {total_bytes} ({})",
            format_bytes(total_bytes)
        );
        println!(
            "  Resume From: {resume_from} ({})",
            format_bytes(resume_from)
        );
        println!("  Offered At:  {ts}ms");
        return Ok(());
    }

    let count = v.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
    let auto_accept = v
        .get("auto_accept_trusted")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    println!("Pending File Offers (count: {count}, auto_accept_trusted: {auto_accept}):");
    if let Some(offers) = v.get("offers").and_then(|o| o.as_array()) {
        if offers.is_empty() {
            println!("  (no pending file offers)");
        } else {
            for o in offers {
                let tid = o.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = o.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let source = o.get("source").and_then(|s| s.as_str()).unwrap_or("?");
                let total_bytes = o.get("total_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                let resume_from = o.get("resume_from").and_then(|s| s.as_u64()).unwrap_or(0);
                let resume_str = if resume_from > 0 {
                    format!(" [resuming from {}]", format_bytes(resume_from))
                } else {
                    "".to_string()
                };
                println!(
                    "  - [{tid}] {filename} ({}) from {source}{resume_str}",
                    format_bytes(total_bytes)
                );
            }
        }
    }
    Ok(())
}

pub(crate) async fn do_file_accept(
    ui_port: u16,
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
) -> Result<()> {
    if !all && id.is_none() {
        anyhow::bail!("specify a transfer ID to accept or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/accept", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "accept file offer failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if all {
        let count = v
            .get("accepted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let total = v
            .get("total_evaluated")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        println!("Accepted {count} of {total} pending file offer(s)");
        if let Some(decisions) = v.get("decisions").and_then(|d| d.as_array()) {
            for d in decisions {
                let tid = d.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = d.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let p = d.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
                let bytes = d.get("total_bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                println!(
                    "  * accepted [{tid}] {filename} ({}) from {p}",
                    format_bytes(bytes)
                );
            }
        }
    } else {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        let bytes = v.get("total_bytes").and_then(|b| b.as_u64()).unwrap_or(0);
        println!(
            "Accepted file offer [{tid}] {filename} ({}) from {p}",
            format_bytes(bytes)
        );
    }
    Ok(())
}

pub(crate) async fn do_file_reject(
    ui_port: u16,
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
) -> Result<()> {
    if !all && id.is_none() {
        anyhow::bail!("specify a transfer ID to reject or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/reject", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "reject file offer failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if all {
        let count = v
            .get("rejected_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let total = v
            .get("total_evaluated")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        println!("Rejected {count} of {total} pending file offer(s)");
        if let Some(decisions) = v.get("decisions").and_then(|d| d.as_array()) {
            for d in decisions {
                let tid = d.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = d.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let p = d.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
                println!("  * rejected [{tid}] {filename} from {p}");
            }
        }
    } else {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        println!("Rejected file offer [{tid}] {filename} from {p}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_file_auto_accept(
    ui_port: u16,
    enable: bool,
    disable: bool,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_size: Option<u64>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    if enable && disable {
        anyhow::bail!("cannot specify both --enable and --disable");
    }
    let has_update = enable
        || disable
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_size.is_some()
        || clear_filters;

    if has_update {
        let mut body_map = serde_json::Map::new();
        if enable {
            body_map.insert("enabled".to_string(), serde_json::json!(true));
        } else if disable {
            body_map.insert("enabled".to_string(), serde_json::json!(false));
        }
        if clear_filters {
            body_map.insert("allowed_groups".to_string(), serde_json::json!([]));
            body_map.insert("allowed_tags".to_string(), serde_json::json!([]));
            body_map.insert("allowed_types".to_string(), serde_json::json!([]));
            body_map.insert("max_file_size".to_string(), serde_json::Value::Null);
        } else {
            if let Some(g) = group {
                body_map.insert("allowed_groups".to_string(), serde_json::json!(g));
            }
            if let Some(t) = tag {
                body_map.insert("allowed_tags".to_string(), serde_json::json!(t));
            }
            if let Some(dt) = device_type {
                body_map.insert("allowed_types".to_string(), serde_json::json!(dt));
            }
            if let Some(ms) = max_size {
                if ms == 0 {
                    body_map.insert("max_file_size".to_string(), serde_json::Value::Null);
                } else {
                    body_map.insert("max_file_size".to_string(), serde_json::json!(ms));
                }
            }
        }
        let body = serde_json::to_vec(&serde_json::Value::Object(body_map))?;
        let (code, resp) =
            tetherly_node::loopback_post(ui_port, "/api/file/auto-accept/policy", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
        if code != 200 {
            anyhow::bail!(
                "configure auto-accept policy failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let pol = &v["policy"];
            let enabled = pol["enabled"].as_bool().unwrap_or(false);
            let count = v
                .get("cumulative_auto_accepted_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let grps = pol["allowed_groups"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let tags = pol["allowed_tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let types = pol["allowed_types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let max_sz = pol["max_file_size"]
                .as_u64()
                .map(format_bytes)
                .unwrap_or_else(|| "unlimited".to_string());
            println!("File auto-accept policy updated:");
            println!("  Master Enabled:      {enabled}");
            println!("  Allowed Groups:      [{grps}] (empty = all trusted)");
            println!("  Allowed Tags:        [{tags}] (empty = all trusted)");
            println!("  Allowed Types:       [{types}] (empty = all trusted)");
            println!("  Max File Size:       {max_sz}");
            println!("  Cumulative Accepted: {count} file(s)");
        }
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/file/auto-accept/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "query auto-accept policy failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        let pol = &v["policy"];
        let enabled = pol["enabled"].as_bool().unwrap_or(false);
        let count = v
            .get("cumulative_auto_accepted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let grps = pol["allowed_groups"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let tags = pol["allowed_tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let types = pol["allowed_types"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let max_sz = pol["max_file_size"]
            .as_u64()
            .map(format_bytes)
            .unwrap_or_else(|| "unlimited".to_string());
        println!("File Auto-Accept Policy:");
        println!("  Master Enabled:      {enabled}");
        println!("  Allowed Groups:      [{grps}] (empty = all trusted)");
        println!("  Allowed Tags:        [{tags}] (empty = all trusted)");
        println!("  Allowed Types:       [{types}] (empty = all trusted)");
        println!("  Max File Size:       {max_sz}");
        println!("  Cumulative Accepted: {count} file(s)");
        println!("  Target:              Trusted Noise-authenticated peers matching policy");
    }
    Ok(())
}
