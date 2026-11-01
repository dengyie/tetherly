// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen quality/transport knobs: bitrate/fps/fec/twcc/probe/sync.

use super::util::{loopback_get, loopback_post};
use super::*;

/// `tetherly-cli screen bitrate` -- Bitrate arm of the screen dispatcher.
pub(crate) async fn screen_bitrate(kbps: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match kbps {
        Some(k) => {
            let req = serde_json::json!({ "kbps": k });
            let body = serde_json::to_vec(&req)?;
            loopback_post(ui_port, "/api/screen/bitrate", &body).await
        }
        None => loopback_get(ui_port, "/api/screen/bitrate").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen bitrate query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let bitrate_kbps = v.get("bitrate_kbps").and_then(|n| n.as_u64()).unwrap_or(0);
        let bitrate_bps = v.get("bitrate_bps").and_then(|n| n.as_u64()).unwrap_or(0);
        let min_bps = v.get("min_bps").and_then(|n| n.as_u64()).unwrap_or(0);
        let max_bps = v.get("max_bps").and_then(|n| n.as_u64()).unwrap_or(0);
        match kbps {
            Some(_) => {
                println!(
                    "Screen target bitrate set to {} kbps ({} bps)",
                    bitrate_kbps, bitrate_bps
                );
            }
            None => {
                println!("Screen Streaming Bitrate:");
                println!(
                    "  Current: {} kbps ({:.2} Mbps)",
                    bitrate_kbps,
                    bitrate_bps as f64 / 1_000_000.0
                );
                println!(
                    "  Range:   {} kbps - {} kbps ({:.1} Mbps - {:.1} Mbps)",
                    min_bps / 1000,
                    max_bps / 1000,
                    min_bps as f64 / 1_000_000.0,
                    max_bps as f64 / 1_000_000.0
                );
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen fps` -- Fps arm of the screen dispatcher.
pub(crate) async fn screen_fps(fps: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match fps {
        Some(f) => {
            let req = serde_json::json!({ "fps": f });
            let body = serde_json::to_vec(&req)?;
            loopback_post(ui_port, "/api/screen/fps", &body).await
        }
        None => loopback_get(ui_port, "/api/screen/fps").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen fps query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let cur_fps = v.get("fps").and_then(|n| n.as_u64()).unwrap_or(0);
        let interval_us = v.get("interval_us").and_then(|n| n.as_u64()).unwrap_or(0);
        let min_fps = v.get("min_fps").and_then(|n| n.as_u64()).unwrap_or(0);
        let max_fps = v.get("max_fps").and_then(|n| n.as_u64()).unwrap_or(0);
        let auto_profile = v
            .get("auto_profile")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        match fps {
            Some(_) => {
                println!(
                    "Screen target frame rate set to {} FPS (pacing interval: {:.2} ms)",
                    cur_fps,
                    interval_us as f64 / 1000.0
                );
            }
            None => {
                println!("Screen Streaming Frame Rate:");
                println!(
                    "  Current:   {} FPS (pacing interval: {:.2} ms)",
                    cur_fps,
                    interval_us as f64 / 1000.0
                );
                println!("  Range:     {} FPS - {} FPS", min_fps, max_fps);
                println!(
                    "  Auto-tier: {}",
                    if auto_profile {
                        "enabled (derived from link bandwidth)"
                    } else {
                        "manual override"
                    }
                );
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen scale` -- Scale arm of the screen dispatcher.
pub(crate) async fn screen_scale(scale: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match scale {
        Some(s) => {
            let req = serde_json::json!({ "scale": s });
            let body = serde_json::to_vec(&req)?;
            loopback_post(ui_port, "/api/screen/scale", &body).await
        }
        None => loopback_get(ui_port, "/api/screen/scale").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen scale query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let scale_pct = v.get("scale_pct").and_then(|n| n.as_u64()).unwrap_or(0);
        let min_scale_pct = v.get("min_scale_pct").and_then(|n| n.as_u64()).unwrap_or(0);
        let max_scale_pct = v.get("max_scale_pct").and_then(|n| n.as_u64()).unwrap_or(0);
        let auto_profile = v
            .get("auto_profile")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        match scale {
            Some(_) => {
                println!(
                    "Screen target resolution scale set to {}% (allowed: {}%..={}%)",
                    scale_pct, min_scale_pct, max_scale_pct
                );
            }
            None => {
                println!("Screen Streaming Resolution Scale:");
                println!("  Current:   {}%", scale_pct);
                println!("  Range:     {}% - {}%", min_scale_pct, max_scale_pct);
                println!(
                    "  Auto-tier: {}",
                    if auto_profile {
                        "enabled (derived from link bandwidth)"
                    } else {
                        "manual override"
                    }
                );
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen profile` -- Profile arm of the screen dispatcher.
pub(crate) async fn screen_profile(
    profile: Option<String>,
    min_qp: Option<u8>,
    max_qp: Option<u8>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let has_update = profile.is_some() || min_qp.is_some() || max_qp.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(p) = profile {
            req.insert("profile".into(), serde_json::json!(p));
        }
        if let Some(min) = min_qp {
            req.insert("min_qp".into(), serde_json::json!(min));
        }
        if let Some(max) = max_qp {
            req.insert("max_qp".into(), serde_json::json!(max));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/profile", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/profile").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen profile query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let p = v.get("profile").and_then(|s| s.as_str()).unwrap_or("");
        let min = v.get("min_qp").and_then(|n| n.as_u64()).unwrap_or(0);
        let max = v.get("max_qp").and_then(|n| n.as_u64()).unwrap_or(0);
        let is_custom_qp = v
            .get("is_custom_qp")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let auto_profile = v
            .get("auto_profile")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        if has_update {
            println!(
                "Screen content profile updated: {} (QP range: {}..={}, custom={})",
                p, min, max, is_custom_qp
            );
        } else {
            println!("Screen Streaming Content Profile & Quality:");
            println!("  Profile:     {}", p);
            println!("  QP Range:    {} - {}", min, max);
            println!(
                "  Custom QP:   {}",
                if is_custom_qp {
                    "yes"
                } else {
                    "no (profile default)"
                }
            );
            println!(
                "  Auto-tier:   {}",
                if auto_profile {
                    "enabled (auto-selects profile on stream start)"
                } else {
                    "manual override"
                }
            );
        }
    }
    Ok(())
}

/// `tetherly-cli screen matrix` -- Matrix arm of the screen dispatcher.
pub(crate) async fn screen_matrix(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/matrix", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/matrix").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen matrix query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("");
        let health = v.get("health").and_then(|s| s.as_str()).unwrap_or("");
        let bitrate_bps = v.get("bitrate_bps").and_then(|n| n.as_u64()).unwrap_or(0);
        let fps_target = v.get("fps_target").and_then(|n| n.as_u64()).unwrap_or(0);
        let scale_pct = v.get("scale_pct").and_then(|n| n.as_u64()).unwrap_or(0);
        let profile = v.get("profile").and_then(|s| s.as_str()).unwrap_or("");
        let qp_min = v.get("qp_min").and_then(|n| n.as_u64()).unwrap_or(0);
        let qp_max = v.get("qp_max").and_then(|n| n.as_u64()).unwrap_or(0);
        let healthy_acks = v
            .get("consecutive_healthy_acks")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let drops = v
            .get("consecutive_drops")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let in_flight = v.get("in_flight").and_then(|n| n.as_u64()).unwrap_or(0);
        let rtt_us = v.get("rtt_us").and_then(|n| n.as_u64());

        if has_update {
            println!(
                "Screen 4D matrix coordinator mode updated: {} (health: {})",
                m, health
            );
        } else {
            println!("Screen Streaming 4D Autonomous Adaptation Matrix (ADR-086):");
            println!("  Coordinator Mode: {}", m);
            println!("  Network Health:   {}", health);
            println!("  Bitrate (kbps):   {}", bitrate_bps / 1000);
            println!("  FPS Target:       {}", fps_target);
            println!("  Resolution Scale: {}%", scale_pct);
            println!("  Content Profile:  {}", profile);
            println!("  QP Range:         {} - {}", qp_min, qp_max);
            println!("  Healthy Acks:     {}", healthy_acks);
            println!("  Consec. Drops:    {}", drops);
            println!("  In Flight:        {}", in_flight);
            if let Some(rtt) = rtt_us {
                println!(
                    "  RTT (latest):     {:.2} ms ({rtt} us)",
                    rtt as f64 / 1000.0
                );
            } else {
                println!("  RTT (latest):     (no ack yet)");
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen playout` -- Playout arm of the screen dispatcher.
pub(crate) async fn screen_playout(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/playout", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/playout").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen playout query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("");
        let target_delay_ms = v
            .get("target_delay_ms")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let current_jitter_ms = v
            .get("current_jitter_ms")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let target_delay_us = v
            .get("target_delay_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let current_jitter_us = v
            .get("current_jitter_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let buffered_frames = v
            .get("buffered_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let played_frames = v.get("played_frames").and_then(|n| n.as_u64()).unwrap_or(0);
        let dropped_late = v
            .get("dropped_late_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let dropped_overflow = v
            .get("dropped_overflow_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let custom_delay_us = v.get("custom_delay_us").and_then(|n| n.as_u64());

        if has_update {
            println!(
                "Screen receiver playout mode updated: {} (target delay: {:.1} ms)",
                m, target_delay_ms
            );
        } else {
            println!("Screen Streaming Receiver Playout Buffer & Dejitterizer (ADR-087):");
            println!("  Playout Mode:        {}", m);
            println!(
                "  Target Delay:        {:.1} ms ({} us)",
                target_delay_ms, target_delay_us
            );
            println!(
                "  Interarrival Jitter: {:.2} ms ({} us)",
                current_jitter_ms, current_jitter_us
            );
            println!("  Buffered Frames:     {}", buffered_frames);
            println!("  Played Frames:       {}", played_frames);
            println!("  Late Dropped:        {}", dropped_late);
            println!("  Overflow Dropped:    {}", dropped_overflow);
            if let Some(c) = custom_delay_us {
                println!(
                    "  Custom Delay:        {:.1} ms ({c} us)",
                    c as f64 / 1000.0
                );
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen cursor` -- Cursor arm of the screen dispatcher.
pub(crate) async fn screen_cursor(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/cursor", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/cursor").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen cursor query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("");
        let x = v.get("x").and_then(|n| n.as_i64()).unwrap_or(0);
        let y = v.get("y").and_then(|n| n.as_i64()).unwrap_or(0);
        let visible = v.get("visible").and_then(|b| b.as_bool()).unwrap_or(false);
        let shape = v.get("shape").and_then(|s| s.as_str()).unwrap_or("");
        let sent = v.get("updates_sent").and_then(|n| n.as_u64()).unwrap_or(0);
        let received = v
            .get("updates_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let dedup = v
            .get("updates_dropped_dedup")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let rate = v
            .get("updates_dropped_rate")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        if has_update {
            println!("Screen hardware cursor decoupling mode updated: {m}");
        } else {
            println!("Screen Streaming Hardware Cursor Decoupling Controller (ADR-088):");
            println!("  Cursor Mode:         {m}");
            println!("  Current Position:    ({x}, {y})");
            println!("  Cursor Visible:      {visible}");
            println!("  Cursor Shape:        {shape}");
            println!("  Updates Sent:        {sent}");
            println!("  Updates Received:    {received}");
            println!("  Dropped (Dedup):     {dedup}");
            println!("  Dropped (Rate):      {rate}");
        }
    }
    Ok(())
}

/// `tetherly-cli screen squelch` -- Squelch arm of the screen dispatcher.
pub(crate) async fn screen_squelch(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/squelch", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/squelch").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen squelch query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("");
        let is_squelched = v
            .get("is_squelched")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let frames_evaluated = v
            .get("frames_evaluated")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let frames_passed = v.get("frames_passed").and_then(|n| n.as_u64()).unwrap_or(0);
        let frames_squelched = v
            .get("frames_squelched")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let keepalive_frames = v
            .get("keepalive_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let last_dirty_ratio = v
            .get("last_dirty_ratio")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let consecutive_static_frames = v
            .get("consecutive_static_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        if has_update {
            println!("Screen static frame adaptive squelch mode updated: {m}");
        } else {
            println!(
                "Screen Streaming Dirty-Rect Detection & Adaptive Squelch Controller (ADR-089):"
            );
            println!("  Squelch Mode:        {m}");
            println!("  Currently Squelched: {is_squelched}");
            println!("  Frames Evaluated:    {frames_evaluated}");
            println!("  Frames Passed:       {frames_passed}");
            println!("  Frames Squelched:    {frames_squelched}");
            println!("  Keepalive Frames:    {keepalive_frames}");
            println!("  Last Dirty Ratio:    {:.2}%", last_dirty_ratio * 100.0);
            println!("  Static Frames Run:   {consecutive_static_frames}");
        }
    }
    Ok(())
}

/// `tetherly-cli screen fec` -- Fec arm of the screen dispatcher.
pub(crate) async fn screen_fec(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/fec", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/fec").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen fec query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("unknown");
        let redundancy_pct = v
            .get("redundancy_pct")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let blocks_encoded = v
            .get("blocks_encoded")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let data_shards_sent = v
            .get("data_shards_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let parity_shards_sent = v
            .get("parity_shards_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let blocks_decoded = v
            .get("blocks_decoded")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let recovered_shards = v
            .get("recovered_shards")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let unrecoverable_blocks = v
            .get("unrecoverable_blocks")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        if has_update {
            println!(
                "Screen forward error correction mode updated: {m} (redundancy: {redundancy_pct}%)"
            );
        } else {
            println!(
                "Screen Forward Error Correction (FEC) & Adaptive Redundancy Protection (ADR-090):"
            );
            println!("  FEC Mode:             {m}");
            println!("  Redundancy:           {redundancy_pct}%");
            println!("  Blocks Encoded:       {blocks_encoded}");
            println!("  Data Shards Sent:     {data_shards_sent}");
            println!("  Parity Shards Sent:   {parity_shards_sent}");
            println!("  Blocks Decoded:       {blocks_decoded}");
            println!("  Recovered Shards:     {recovered_shards}");
            println!("  Unrecoverable Blocks: {unrecoverable_blocks}");
        }
    }
    Ok(())
}

/// `tetherly-cli screen pacer` -- Pacer arm of the screen dispatcher.
pub(crate) async fn screen_pacer(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/pacer", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/pacer").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen pacer query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("unknown");
        let packets_paced = v.get("packets_paced").and_then(|n| n.as_u64()).unwrap_or(0);
        let bursts_smoothed = v
            .get("bursts_smoothed")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let avg_delay_us = v.get("avg_delay_us").and_then(|n| n.as_u64()).unwrap_or(0);
        let last_delay_us = v.get("last_delay_us").and_then(|n| n.as_u64()).unwrap_or(0);

        if has_update {
            println!("Screen microsecond packet pacer mode updated: {m}");
        } else {
            println!("Screen Microsecond Packet Pacing & Burst Shaper (ADR-091):");
            println!("  Pacer Mode:           {m}");
            println!("  Packets Paced:        {packets_paced}");
            println!("  Bursts Smoothed:      {bursts_smoothed}");
            println!("  Average Delay:        {avg_delay_us} us");
            println!("  Last Packet Delay:    {last_delay_us} us");
        }
    }
    Ok(())
}

/// `tetherly-cli screen trendline` -- Trendline arm of the screen dispatcher.
pub(crate) async fn screen_trendline(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/trendline", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/trendline").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen trendline query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("unknown");
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("normal");
        let current_slope = v
            .get("current_slope_us_per_frame")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let threshold = v
            .get("threshold_us_per_frame")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let accumulated_delay = v
            .get("accumulated_delay_us")
            .and_then(|n| n.as_i64())
            .unwrap_or(0);
        let overuse_count = v.get("overuse_count").and_then(|n| n.as_u64()).unwrap_or(0);
        let underuse_count = v
            .get("underuse_count")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        if has_update {
            println!("Screen delay trendline filter mode updated: {m}");
        } else {
            println!("Screen Delay-Based Congestion Controller & Trendline Filter (ADR-092):");
            println!("  Trendline Mode:       {m}");
            println!("  Usage State:          {state}");
            println!("  Current Slope:        {current_slope:.2} us/frame");
            println!("  Threshold:            {threshold:.2} us/frame");
            println!("  Accumulated Delay:    {accumulated_delay} us");
            println!("  Overuse Events:       {overuse_count}");
            println!("  Underuse Events:      {underuse_count}");
        }
    }
    Ok(())
}

/// `tetherly-cli screen harq` -- Harq arm of the screen dispatcher.
pub(crate) async fn screen_harq(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/harq", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/harq").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen harq query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("unknown");
        let nacks_sent = v.get("nacks_sent").and_then(|n| n.as_u64()).unwrap_or(0);
        let nacks_received = v
            .get("nacks_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let shards_retransmitted = v
            .get("shards_retransmitted")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let recovered_frames = v
            .get("recovered_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let retransmit_timeouts = v
            .get("retransmit_timeouts")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let buffer_occupancy = v
            .get("buffer_occupancy")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let buffer_capacity = v
            .get("buffer_capacity")
            .and_then(|n| n.as_u64())
            .unwrap_or(tetherly_core::SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY as u64);

        if has_update {
            println!("Screen HARQ selective retransmission mode updated: {m}");
        } else {
            println!("Screen Selective NACK & Hybrid ARQ (HARQ) Pipeline (ADR-093):");
            println!("  HARQ Mode:            {m}");
            println!("  NACKs Sent:           {nacks_sent}");
            println!("  NACKs Received:       {nacks_received}");
            println!("  Shards Retransmitted: {shards_retransmitted}");
            println!("  Recovered Frames:     {recovered_frames}");
            println!("  Retransmit Timeouts:  {retransmit_timeouts}");
            println!("  Buffer Occupancy:     {buffer_occupancy}/{buffer_capacity}");
        }
    }
    Ok(())
}

/// `tetherly-cli screen twcc` -- Twcc arm of the screen dispatcher.
pub(crate) async fn screen_twcc(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/twcc", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/twcc").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen twcc query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let m = v.get("mode").and_then(|s| s.as_str()).unwrap_or("unknown");
        let feedbacks_sent = v
            .get("feedbacks_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let feedbacks_received = v
            .get("feedbacks_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let packets_tracked = v
            .get("packets_tracked")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let packets_acknowledged = v
            .get("packets_acknowledged")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let packets_lost = v.get("packets_lost").and_then(|n| n.as_u64()).unwrap_or(0);
        let loss_ratio = v.get("loss_ratio").and_then(|n| n.as_f64()).unwrap_or(0.0);
        let delivery_rate_bps = v
            .get("delivery_rate_bps")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let delay_gradient_us = v
            .get("delay_gradient_us")
            .and_then(|n| n.as_i64())
            .unwrap_or(0);
        let smoothed_rtt_us = v
            .get("smoothed_rtt_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        if has_update {
            println!("Screen TWCC feedback mode updated: {m}");
        } else {
            println!("Screen Transport-Wide Congestion Control (TWCC) Pipeline (ADR-094):");
            println!("  TWCC Mode:            {m}");
            println!("  Feedbacks Sent:       {feedbacks_sent}");
            println!("  Feedbacks Received:   {feedbacks_received}");
            println!("  Packets Tracked:      {packets_tracked}");
            println!("  Packets Acknowledged: {packets_acknowledged}");
            println!("  Packets Lost:         {packets_lost}");
            println!("  Loss Ratio:           {:.2}%", loss_ratio * 100.0);
            println!(
                "  Delivery Rate:        {:.2} Mbps ({} bps)",
                delivery_rate_bps as f64 / 1_000_000.0,
                delivery_rate_bps
            );
            println!("  Delay Gradient:       {delay_gradient_us} us");
            println!(
                "  Smoothed RTT:         {} us ({:.2} ms)",
                smoothed_rtt_us,
                smoothed_rtt_us as f64 / 1_000.0
            );
        }
    }
    Ok(())
}

/// `tetherly-cli screen probe` -- Probe arm of the screen dispatcher.
pub(crate) async fn screen_probe(
    mode: Option<String>,
    target_kbps: Option<u32>,
    trigger: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let has_update = mode.is_some() || target_kbps.is_some() || trigger;
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        if let Some(kbps) = target_kbps {
            req.insert("target_bps".into(), serde_json::json!(kbps * 1000));
            req.insert("trigger".into(), serde_json::json!(true));
        } else if trigger {
            req.insert("trigger".into(), serde_json::json!(true));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/probe", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/probe").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen probe query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_core::ScreenProbeReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if trigger || target_kbps.is_some() {
            println!(
                "Screen active bandwidth probe burst triggered (state: {}, target: {:.2} Mbps)",
                rep.state,
                rep.last_target_bps as f64 / 1_000_000.0
            );
        } else if has_update {
            println!("Screen active bandwidth probe mode updated: {}", rep.mode);
        } else {
            println!("Screen Active Bandwidth Probing & Fast Ramp-Up Pipeline (ADR-095):");
            println!("  Probe Mode:           {}", rep.mode);
            println!("  Probe State:          {}", rep.state);
            println!("  Probes Triggered:     {}", rep.probes_triggered);
            println!("  Probes Completed:     {}", rep.probes_completed);
            println!("  Probes Failed:        {}", rep.probes_failed);
            println!(
                "  Last Target Rate:     {:.2} Mbps ({} bps)",
                rep.last_target_bps as f64 / 1_000_000.0,
                rep.last_target_bps
            );
            println!(
                "  Last Measured Rate:   {:.2} Mbps ({} bps)",
                rep.last_measured_bps as f64 / 1_000_000.0,
                rep.last_measured_bps
            );
            println!(
                "  Last RTT:             {} us ({:.2} ms)",
                rep.last_rtt_us,
                rep.last_rtt_us as f64 / 1_000.0
            );
            println!(
                "  Last Loss Ratio:      {:.2}%",
                rep.last_loss_ratio * 100.0
            );
            println!("  Last Delay Gradient:  {} us", rep.last_delay_gradient_us);
            if let Some(cid) = rep.active_cluster_id {
                println!("  Active Cluster ID:    {}", cid);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen sync` -- Sync arm of the screen dispatcher.
pub(crate) async fn screen_sync(
    mode: Option<String>,
    trigger: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let has_update = mode.is_some() || trigger;
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        if trigger {
            req.insert("trigger".into(), serde_json::json!(true));
        }
        let body = serde_json::to_vec(&req)?;
        loopback_post(ui_port, "/api/screen/sync", &body).await
    } else {
        loopback_get(ui_port, "/api/screen/sync").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen sync query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_core::ScreenAudioSyncReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if trigger {
            println!(
                "Screen & audio sync beacon triggered (skew: {:.2} ms, drift: {:+.1} ppm, in-sync: {})",
                rep.current_skew_us as f64 / 1_000.0,
                rep.clock_drift_ppm,
                rep.in_sync
            );
        } else if has_update {
            println!("Screen & audio sync mode updated: {}", rep.mode);
        } else {
            println!("Screen & Audio Playout Lip-Sync & Clock Synchronization Pipeline (ADR-096):");
            println!("  Sync Mode:            {}", rep.mode);
            println!("  Beacons Sent:         {}", rep.beacons_sent);
            println!("  Beacons Received:     {}", rep.beacons_received);
            println!(
                "  Current Skew:         {:.2} ms ({} us)",
                rep.current_skew_us as f64 / 1_000.0,
                rep.current_skew_us
            );
            println!("  Clock Drift (PPM):    {:+.1} ppm", rep.clock_drift_ppm);
            println!("  Playout Adjustments:  {}", rep.adjustments_applied);
            println!("  In Lip-Sync:          {}", rep.in_sync);
        }
    }
    Ok(())
}
