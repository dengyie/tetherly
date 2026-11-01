// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop notifications, OTP candidates, banners and reply inbox.

use super::util::now_ms;
use super::*;

/// `tetherly notify` -- Notify arm of the top-level dispatch.
pub(crate) async fn notify(
    title: String,
    body: String,
    app_id: String,
    peer: Option<String>,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "title": title,
        "body": body,
        "app_id": app_id,
        "peer": peer,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/notify", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused notify (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
    println!("notified {peer}");
    Ok(())
}

/// `tetherly candidates` -- Candidates arm of the top-level dispatch.
pub(crate) async fn candidates(
    peer: Option<String>,
    app_id: Option<String>,
    has_otp: Option<bool>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut path = "/api/notify/candidates".to_string();
    let mut query = Vec::new();
    if let Some(ref p) = peer {
        query.push(format!("peer={p}"));
    }
    if let Some(ref a) = app_id {
        query.push(format!("app_id={a}"));
    }
    if let Some(h) = has_otp {
        query.push(format!("has_otp={h}"));
    }
    if !query.is_empty() {
        path = format!("{path}?{}", query.join("&"));
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
            "candidates query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let list = v.get("candidates").and_then(|c| c.as_array());
        match list {
            Some(items) if !items.is_empty() => {
                println!(
                    "{:<32} {:<24} {:<6} {:<10} TITLE",
                    "ID", "APP", "OTP", "EXPIRES"
                );
                let now = now_ms();
                for item in items {
                    let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("-");
                    let app = item.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
                    let has_otp = item
                        .get("has_otp")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let exp = item.get("expires_ms").and_then(|n| n.as_u64()).unwrap_or(0);
                    let title = item.get("title").and_then(|s| s.as_str()).unwrap_or("");
                    let rem_sec = exp.saturating_sub(now) / 1000;
                    println!(
                        "{:<32} {:<24} {:<6} {:<10} {}",
                        id,
                        app,
                        if has_otp { "YES" } else { "NO" },
                        format!("{}s", rem_sec),
                        title
                    );
                }
            }
            _ => println!("(no active notification candidates)"),
        }
    }
    Ok(())
}

/// `tetherly otp` -- Otp arm of the top-level dispatch.
pub(crate) async fn otp(id: String, copy: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    if copy {
        let (c_code, c_resp) = tetherly_node::loopback_post(ui_port, "/api/copy", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if c_code != 200 {
            anyhow::bail!(
                "copy OTP failed (HTTP {c_code}): {}",
                String::from_utf8_lossy(&c_resp)
            );
        }
    }
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/otp/get", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "get OTP failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let otp = v
        .get("otp")
        .and_then(|o| o.as_str())
        .ok_or_else(|| anyhow::anyhow!("no OTP in response"))?;
    println!("{otp}");
    if copy {
        eprintln!("(copied to system clipboard)");
    }
    Ok(())
}

/// `tetherly dismiss` -- Dismiss arm of the top-level dispatch.
pub(crate) async fn dismiss(
    id: Option<String>,
    peer: Option<String>,
    app_id: Option<String>,
    all: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    if id.is_none() && peer.is_none() && app_id.is_none() && !all {
        anyhow::bail!(
            "dismiss target required: specify candidate ID, or use --all, --peer <peer>, or --app-id <app>"
        );
    }
    let payload = serde_json::json!({
        "id": id,
        "peer": peer,
        "app_id": app_id,
        "all": if all { Some(true) } else { None },
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/notify/dismiss", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "dismiss failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else if let Ok(batch) = serde_json::from_slice::<serde_json::Value>(&resp) {
        if let Some(count) = batch.get("count").and_then(|c| c.as_u64()) {
            let synced = batch
                .get("notified_remote_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            println!("dismissed {count} candidate(s) (remote peer synced: {synced})");
        } else if let Some(cand_id) = batch.get("id").and_then(|c| c.as_str()) {
            let remote = batch
                .get("notified_remote")
                .and_then(|c| c.as_bool())
                .unwrap_or(false);
            println!("dismissed candidate {cand_id} (remote synced: {remote})");
        } else {
            println!("dismissed successfully");
        }
    } else {
        println!("dismissed successfully");
    }
    Ok(())
}

/// `tetherly reply` -- Reply arm of the top-level dispatch.
pub(crate) async fn reply(id: String, text: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "id": id, "text": text });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/reply", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "reply failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    println!("reply sent for candidate {id}");
    Ok(())
}

/// `tetherly replies` -- Replies arm of the top-level dispatch.
pub(crate) async fn replies(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/notify/replies")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "replies query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let list = v.get("replies").and_then(|r| r.as_array());
        match list {
            Some(items) if !items.is_empty() => {
                println!("{:<32} {:<24} UID", "ID", "APP");
                for item in items {
                    let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("-");
                    let app = item.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
                    let uid = item.get("uid").and_then(|s| s.as_str()).unwrap_or("-");
                    println!("{:<32} {:<24} {}", id, app, uid);
                }
            }
            _ => println!("(no queued inbound replies)"),
        }
    }
    Ok(())
}

/// `tetherly take reply` -- TakeReply arm of the top-level dispatch.
pub(crate) async fn take_reply(id: String, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/reply/take", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "take reply failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let app = v.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
    let uid = v.get("uid").and_then(|s| s.as_str()).unwrap_or("-");
    let text = v.get("text").and_then(|s| s.as_str()).unwrap_or("");
    println!("app_id: {app}, uid: {uid}, text: {text}");
    Ok(())
}

/// `tetherly banner` -- Banner arm of the top-level dispatch.
pub(crate) async fn banner(action: BannerCmd) -> Result<()> {
    match action {
        BannerCmd::Config {
            banners,
            dnd,
            sound,
            mask_otp,
            allow_otp_dnd,
            json,
            ui_port,
        } => {
            let has_update = banners.is_some()
                || dnd.is_some()
                || sound.is_some()
                || mask_otp.is_some()
                || allow_otp_dnd.is_some();
            let (code, resp) = if has_update {
                let mut payload = serde_json::Map::new();
                if let Some(b) = banners {
                    payload.insert("banners_enabled".into(), serde_json::Value::Bool(b));
                }
                if let Some(d) = dnd {
                    payload.insert("dnd_mode".into(), serde_json::Value::String(d));
                }
                if let Some(s) = sound {
                    payload.insert("sound_enabled".into(), serde_json::Value::Bool(s));
                }
                if let Some(m) = mask_otp {
                    payload.insert("mask_otp".into(), serde_json::Value::Bool(m));
                }
                if let Some(a) = allow_otp_dnd {
                    payload.insert("allow_otp_in_dnd".into(), serde_json::Value::Bool(a));
                }
                let bytes = serde_json::to_vec(&payload)?;
                tetherly_node::loopback_post(ui_port, "/api/notify/config", &bytes).await?
            } else {
                tetherly_node::loopback_get(ui_port, "/api/notify/config").await?
            };

            if code != 200 {
                anyhow::bail!(
                    "notify config failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::NotifyConfigReport = serde_json::from_slice(&resp)?;
                println!("Desktop Notification Configuration:");
                println!("  Banners Enabled:  {}", rep.config.banners_enabled);
                println!("  DND Mode:         {:?}", rep.config.dnd_mode);
                println!("  Sound Enabled:    {}", rep.config.sound_enabled);
                println!("  Mask OTP Code:    {}", rep.config.mask_otp);
                println!("  Allow OTP in DND: {}", rep.config.allow_otp_in_dnd);
                println!("  Total Dispatched: {}", rep.total_dispatched);
                println!("  Total Suppressed: {}", rep.total_suppressed);
            }
        }
        BannerCmd::Test {
            title,
            body,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "title": title,
                "body": body,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/notify/test", &bytes).await?;
            if code != 200 {
                anyhow::bail!(
                    "notify test failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::NotifyTestReport = serde_json::from_slice(&resp)?;
                println!("Test notification dispatched: '{}'", rep.title);
            }
        }
    }
    Ok(())
}
