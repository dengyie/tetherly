// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Security audit trail list/clear/watch.

use super::util::now_ms;
use super::*;

/// `tetherly audit` -- Audit arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn audit(
    action: Option<AuditCmd>,
    category: Option<String>,
    severity: Option<String>,
    limit: Option<usize>,
    since: Option<u64>,
    watch: bool,
    clear: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    handle_audit_cmd(
        action, category, severity, limit, since, watch, clear, json, ui_port,
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_audit_cmd(
    action: Option<AuditCmd>,
    default_cat: Option<String>,
    default_sev: Option<String>,
    default_lim: Option<usize>,
    default_since: Option<u64>,
    default_watch: bool,
    default_clear: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_audit_clear(default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_audit_watch(default_cat, default_sev, 0, default_json, default_ui_port)
            .await;
    }
    match action {
        None => {
            handle_audit_list(
                default_cat,
                default_sev,
                default_lim,
                default_since,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(AuditCmd::List {
            category,
            severity,
            limit,
            since,
            json,
            ui_port,
        }) => {
            let cat = category.or(default_cat);
            let sev = severity.or(default_sev);
            let lim = limit.or(default_lim);
            let snc = since.or(default_since);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_list(cat, sev, lim, snc, j, port).await
        }
        Some(AuditCmd::Clear { json, ui_port }) => {
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_clear(j, port).await
        }
        Some(AuditCmd::Watch {
            category,
            severity,
            count,
            json,
            ui_port,
        }) => {
            let cat = category.or(default_cat);
            let sev = severity.or(default_sev);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_watch(cat, sev, count, j, port).await
        }
    }
}

pub(crate) async fn handle_audit_list(
    category: Option<String>,
    severity: Option<String>,
    limit: Option<usize>,
    since: Option<u64>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(c) = &category {
        query_params.push(format!("category={c}"));
    }
    if let Some(s) = &severity {
        query_params.push(format!("severity={s}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    if let Some(snc) = since {
        query_params.push(format!("since={snc}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/audit{qs}");
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
            "node refused audit query (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_audit_report(&val);
    }
    Ok(())
}

pub(crate) async fn handle_audit_clear(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/audit/clear", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear audit log (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let msg = val["message"].as_str().unwrap_or("cleared");
        println!("Audit log cleared: {count} entries purged ({msg})");
    }
    Ok(())
}

pub(crate) async fn handle_audit_watch(
    category: Option<String>,
    severity: Option<String>,
    count: usize,
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
        let cat_str = category.as_deref().unwrap_or("all");
        let sev_str = severity.as_deref().unwrap_or("all");
        println!("Streaming live security audit events on 127.0.0.1:{ui_port} (category: {cat_str}, min-severity: {sev_str})...");
    }
    let cat_filter = category.map(|c| c.trim().to_ascii_lowercase());
    let min_sev = severity.and_then(|s| match s.trim().to_ascii_lowercase().as_str() {
        "info" => Some(1),
        "notice" => Some(2),
        "warning" | "warn" => Some(3),
        "alert" | "critical" => Some(4),
        _ => None,
    });

    let mut received = 0usize;
    loop {
        match sse.next_event().await {
            Ok(Some(raw)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if v["kind"].as_str() == Some("audit_event_recorded") {
                        if let Some(ref cf) = cat_filter {
                            let entry_cat =
                                v["category"].as_str().unwrap_or("").to_ascii_lowercase();
                            if !entry_cat.contains(cf) {
                                continue;
                            }
                        }
                        if let Some(target_sev_rank) = min_sev {
                            let sev_rank = match v["severity"]
                                .as_str()
                                .unwrap_or("")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "info" => 1,
                                "notice" => 2,
                                "warning" | "warn" => 3,
                                "alert" | "critical" => 4,
                                _ => 0,
                            };
                            if sev_rank < target_sev_rank {
                                continue;
                            }
                        }
                        received += 1;
                        if json {
                            println!("{raw}");
                        } else {
                            let id = v["id"].as_u64().unwrap_or(0);
                            let cat = v["category"].as_str().unwrap_or("general");
                            let sev = v["severity"].as_str().unwrap_or("info");
                            let action = v["action"].as_str().unwrap_or("?");
                            let target = v["target"]
                                .as_str()
                                .map(|t| format!(" target={t}"))
                                .unwrap_or_default();
                            let summary = v["summary"].as_str().unwrap_or("");
                            let success = v["success"].as_bool().unwrap_or(true);
                            let status = if success { "ok" } else { "failed" };
                            println!(
                                "[audit] #{id} [{sev}] [{cat}] {action}{target} -> {summary} ({status})"
                            );
                        }
                        if count > 0 && received >= count {
                            break;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => anyhow::bail!("event stream error: {e}"),
        }
    }
    Ok(())
}

pub(crate) fn print_audit_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Node Security Audit Trail (total: {total}, showing: {returned}):");
    let entries = match val["entries"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no audit entries recorded)");
            return;
        }
    };
    println!(
        "  {:<5} {:<12} {:<8} {:<10} {:<20} {:<14} {:<32} {:<6}",
        "ID", "TIME", "SEV", "CATEGORY", "ACTION", "TARGET", "SUMMARY", "STATUS"
    );
    println!("  {}", "-".repeat(115));
    let now = now_ms();
    for e in entries {
        let id = e["id"].as_u64().unwrap_or(0);
        let ts = e["timestamp_ms"].as_u64().unwrap_or(0);
        let ago_str = if now >= ts {
            let diff_s = (now - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        let sev = e["severity"].as_str().unwrap_or("info");
        let cat = e["category"].as_str().unwrap_or("-");
        let action = e["action"].as_str().unwrap_or("-");
        let target = e["target"].as_str().unwrap_or("-");
        let summary = e["summary"].as_str().unwrap_or("-");
        let success = e["success"].as_bool().unwrap_or(true);
        let status = if success { "ok" } else { "failed" };
        println!(
            "  #{:<4} {:<12} {:<8} {:<10} {:<20} {:<14} {:<32} {:<6}",
            id, ago_str, sev, cat, action, target, summary, status
        );
    }
}
