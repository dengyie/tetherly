// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ANCS (iPhone notification bridge) control.

use super::*;

/// `tetherly ancs` -- Ancs arm of the top-level dispatch.
pub(crate) async fn ancs(action: AncsCmd) -> Result<()> {
    match action {
        AncsCmd::Status { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/ancs")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS status query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let attached = v.get("attached").and_then(|b| b.as_bool()).unwrap_or(false);
                let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                let retry = v.get("retry_at_ms").and_then(|n| n.as_u64());
                let rules = v.get("rules").and_then(|r| r.as_array());
                let allowlist = v.get("allowlist").and_then(|a| a.as_array());

                println!("Apple Notification Center Service (ANCS):");
                println!("  Transport Attached: {attached}");
                println!("  Ingress State:      {state}");
                match retry {
                    Some(ms) => println!("  Retry Backoff At:   {ms} ms"),
                    None => println!("  Retry Backoff At:   none"),
                }
                if let Some(r_arr) = rules {
                    if !r_arr.is_empty() {
                        println!("  Open Allowlist Rules ({}):", r_arr.len());
                        for r in r_arr {
                            let app = r["app_id"].as_str().unwrap_or("?");
                            let url = r["url"].as_str().unwrap_or("?");
                            println!("    * {app} -> {url}");
                        }
                    } else {
                        println!("  Open Allowlist Rules: none");
                    }
                } else {
                    println!("  Open Allowlist:");
                    match allowlist {
                        Some(arr) if !arr.is_empty() => {
                            for item in arr {
                                if let Some(app) = item.as_str() {
                                    println!("    * {app}");
                                }
                            }
                        }
                        _ => println!("    (no applications in allowlist)"),
                    }
                }
            }
        }
        AncsCmd::Connect { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/ancs/connect", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS connect failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let state = v["state"].as_str().unwrap_or("unknown");
                let msg = v["message"]
                    .as_str()
                    .unwrap_or("ANCS connection triggered successfully.");
                println!("{msg} (state: {state})");
            }
        }
        AncsCmd::Disconnect { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/ancs/disconnect", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS disconnect failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let state = v["state"].as_str().unwrap_or("unknown");
                let msg = v["message"].as_str().unwrap_or("ANCS disconnected.");
                println!("{msg} (state: {state})");
            }
        }
        AncsCmd::Allow {
            app_id,
            url,
            json,
            ui_port,
        } => {
            let payload = match url {
                Some(u) => serde_json::json!({ "app_id": app_id, "url": u }),
                None => serde_json::json!({ "app_id": app_id }),
            };
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/ancs/allow", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS allow rule failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let total = v["total_rules"].as_u64().unwrap_or(0);
                println!("Allowed open action for '{app_id}' (total rules: {total}).");
            }
        }
        AncsCmd::Remove {
            app_id,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "app_id": app_id });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/ancs/rules/remove", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS remove rule failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let action = v["action"].as_str().unwrap_or("removed");
                let total = v["total_rules"].as_u64().unwrap_or(0);
                if action == "removed" {
                    println!("Removed rule for '{app_id}' (total rules: {total}).");
                } else {
                    println!("Rule for '{app_id}' not found (total rules: {total}).");
                }
            }
        }
        AncsCmd::Clear { json, ui_port } => {
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/ancs/rules/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "ANCS clear rules failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let cleared = v["cleared_count"].as_u64().unwrap_or(0);
                let total = v["total_rules"].as_u64().unwrap_or(0);
                println!("Cleared {cleared} allowlist rule(s) (remaining: {total}).");
            }
        }
    }
    Ok(())
}
