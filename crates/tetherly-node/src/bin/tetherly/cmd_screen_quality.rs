// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen quality/transport knobs: bitrate/fps/fec/twcc/probe/sync.

use super::*;

/// `tetherly screen bitrate` -- Bitrate arm of the screen dispatcher.
pub(crate) async fn screen_bitrate(kbps: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match kbps {
        Some(k) => {
            let req = serde_json::json!({ "kbps": k });
            let body = serde_json::to_vec(&req)?;
            tetherly_node::loopback_post(ui_port, "/api/screen/bitrate", &body).await
        }
        None => tetherly_node::loopback_get(ui_port, "/api/screen/bitrate").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen bitrate query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenBitrateReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        match kbps {
            Some(_) => {
                println!(
                    "Screen target bitrate set to {} kbps ({} bps)",
                    rep.bitrate_kbps, rep.bitrate_bps
                );
            }
            None => {
                println!("Screen Streaming Bitrate:");
                println!(
                    "  Current: {} kbps ({:.2} Mbps)",
                    rep.bitrate_kbps,
                    rep.bitrate_bps as f64 / 1_000_000.0
                );
                println!(
                    "  Range:   {} kbps - {} kbps ({:.1} Mbps - {:.1} Mbps)",
                    rep.min_bps / 1000,
                    rep.max_bps / 1000,
                    rep.min_bps as f64 / 1_000_000.0,
                    rep.max_bps as f64 / 1_000_000.0
                );
            }
        }
    }
    Ok(())
}

/// `tetherly screen fps` -- Fps arm of the screen dispatcher.
pub(crate) async fn screen_fps(fps: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match fps {
        Some(f) => {
            let req = serde_json::json!({ "fps": f });
            let body = serde_json::to_vec(&req)?;
            tetherly_node::loopback_post(ui_port, "/api/screen/fps", &body).await
        }
        None => tetherly_node::loopback_get(ui_port, "/api/screen/fps").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen fps query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenFpsReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        match fps {
            Some(_) => {
                println!(
                    "Screen target frame rate set to {} FPS (pacing interval: {:.2} ms)",
                    rep.fps,
                    rep.interval_us as f64 / 1000.0
                );
            }
            None => {
                println!("Screen Streaming Frame Rate:");
                println!(
                    "  Current:   {} FPS (pacing interval: {:.2} ms)",
                    rep.fps,
                    rep.interval_us as f64 / 1000.0
                );
                println!("  Range:     {} FPS - {} FPS", rep.min_fps, rep.max_fps);
                println!(
                    "  Auto-tier: {}",
                    if rep.auto_profile {
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

/// `tetherly screen scale` -- Scale arm of the screen dispatcher.
pub(crate) async fn screen_scale(scale: Option<u32>, json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = match scale {
        Some(s) => {
            let req = serde_json::json!({ "scale": s });
            let body = serde_json::to_vec(&req)?;
            tetherly_node::loopback_post(ui_port, "/api/screen/scale", &body).await
        }
        None => tetherly_node::loopback_get(ui_port, "/api/screen/scale").await,
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen scale query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenScaleReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        match scale {
            Some(_) => {
                println!(
                    "Screen target resolution scale set to {}% (allowed: {}%..={}%)",
                    rep.scale_pct, rep.min_scale_pct, rep.max_scale_pct
                );
            }
            None => {
                println!("Screen Streaming Resolution Scale:");
                println!("  Current:   {}%", rep.scale_pct);
                println!(
                    "  Range:     {}% - {}%",
                    rep.min_scale_pct, rep.max_scale_pct
                );
                println!(
                    "  Auto-tier: {}",
                    if rep.auto_profile {
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

/// `tetherly screen profile` -- Profile arm of the screen dispatcher.
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
        tetherly_node::loopback_post(ui_port, "/api/screen/profile", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/profile").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen profile query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenProfileReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen content profile updated: {} (QP range: {}..={}, custom={})",
                rep.profile, rep.min_qp, rep.max_qp, rep.is_custom_qp
            );
        } else {
            println!("Screen Streaming Content Profile & Quality:");
            println!("  Profile:     {}", rep.profile);
            println!("  QP Range:    {} - {}", rep.min_qp, rep.max_qp);
            println!(
                "  Custom QP:   {}",
                if rep.is_custom_qp {
                    "yes"
                } else {
                    "no (profile default)"
                }
            );
            println!(
                "  Auto-tier:   {}",
                if rep.auto_profile {
                    "enabled (auto-selects profile on stream start)"
                } else {
                    "manual override"
                }
            );
        }
    }
    Ok(())
}

/// `tetherly screen matrix` -- Matrix arm of the screen dispatcher.
pub(crate) async fn screen_matrix(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/matrix", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/matrix").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen matrix query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenMatrixReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen 4D matrix coordinator mode updated: {} (health: {})",
                rep.mode, rep.health
            );
        } else {
            println!("Screen Streaming 4D Autonomous Adaptation Matrix (ADR-086):");
            println!("  Coordinator Mode: {}", rep.mode);
            println!("  Network Health:   {}", rep.health);
            println!("  Bitrate (kbps):   {}", rep.bitrate_bps / 1000);
            println!("  FPS Target:       {}", rep.fps_target);
            println!("  Resolution Scale: {}%", rep.scale_pct);
            println!("  Content Profile:  {}", rep.profile);
            println!("  QP Range:         {} - {}", rep.qp_min, rep.qp_max);
            println!("  Healthy Acks:     {}", rep.consecutive_healthy_acks);
            println!("  Consec. Drops:    {}", rep.consecutive_drops);
            println!("  In Flight:        {}", rep.in_flight);
            if let Some(rtt) = rep.rtt_us {
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

/// `tetherly screen playout` -- Playout arm of the screen dispatcher.
pub(crate) async fn screen_playout(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/playout", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/playout").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen playout query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenPlayoutReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen receiver playout mode updated: {} (target delay: {:.1} ms)",
                rep.mode, rep.target_delay_ms
            );
        } else {
            println!("Screen Streaming Receiver Playout Buffer & Dejitterizer (ADR-087):");
            println!("  Playout Mode:        {}", rep.mode);
            println!(
                "  Target Delay:        {:.1} ms ({} us)",
                rep.target_delay_ms, rep.target_delay_us
            );
            println!(
                "  Interarrival Jitter: {:.2} ms ({} us)",
                rep.current_jitter_ms, rep.current_jitter_us
            );
            println!("  Buffered Frames:     {}", rep.buffered_frames);
            println!("  Played Frames:       {}", rep.played_frames);
            println!("  Late Dropped:        {}", rep.dropped_late_frames);
            println!("  Overflow Dropped:    {}", rep.dropped_overflow_frames);
            if let Some(c) = rep.custom_delay_us {
                println!(
                    "  Custom Delay:        {:.1} ms ({c} us)",
                    c as f64 / 1000.0
                );
            }
        }
    }
    Ok(())
}

/// `tetherly screen cursor` -- Cursor arm of the screen dispatcher.
pub(crate) async fn screen_cursor(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/cursor", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/cursor").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen cursor query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenCursorReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen hardware cursor decoupling mode updated: {}",
                rep.mode
            );
        } else {
            println!("Screen Streaming Hardware Cursor Decoupling Controller (ADR-088):");
            println!("  Cursor Mode:         {}", rep.mode);
            println!("  Current Position:    ({}, {})", rep.x, rep.y);
            println!("  Cursor Visible:      {}", rep.visible);
            println!("  Cursor Shape:        {}", rep.shape);
            println!("  Updates Sent:        {}", rep.updates_sent);
            println!("  Updates Received:    {}", rep.updates_received);
            println!("  Dropped (Dedup):     {}", rep.updates_dropped_dedup);
            println!("  Dropped (Rate):      {}", rep.updates_dropped_rate);
        }
    }
    Ok(())
}

/// `tetherly screen squelch` -- Squelch arm of the screen dispatcher.
pub(crate) async fn screen_squelch(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/squelch", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/squelch").await
    }
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen squelch query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenSquelchReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen static frame adaptive squelch mode updated: {}",
                rep.mode
            );
        } else {
            println!(
                "Screen Streaming Dirty-Rect Detection & Adaptive Squelch Controller (ADR-089):"
            );
            println!("  Squelch Mode:        {}", rep.mode);
            println!("  Currently Squelched: {}", rep.is_squelched);
            println!("  Frames Evaluated:    {}", rep.frames_evaluated);
            println!("  Frames Passed:       {}", rep.frames_passed);
            println!("  Frames Squelched:    {}", rep.frames_squelched);
            println!("  Keepalive Frames:    {}", rep.keepalive_frames);
            println!(
                "  Last Dirty Ratio:    {:.2}%",
                rep.last_dirty_ratio * 100.0
            );
            println!("  Static Frames Run:   {}", rep.consecutive_static_frames);
        }
    }
    Ok(())
}

/// `tetherly screen fec` -- Fec arm of the screen dispatcher.
pub(crate) async fn screen_fec(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/fec", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/fec").await
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
        let rep: tetherly_core::ScreenFecReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen forward error correction mode updated: {} (redundancy: {}%)",
                rep.mode, rep.redundancy_pct
            );
        } else {
            println!(
                "Screen Forward Error Correction (FEC) & Adaptive Redundancy Protection (ADR-090):"
            );
            println!("  FEC Mode:             {}", rep.mode);
            println!("  Redundancy:           {}%", rep.redundancy_pct);
            println!("  Blocks Encoded:       {}", rep.blocks_encoded);
            println!("  Data Shards Sent:     {}", rep.data_shards_sent);
            println!("  Parity Shards Sent:   {}", rep.parity_shards_sent);
            println!("  Blocks Decoded:       {}", rep.blocks_decoded);
            println!("  Recovered Shards:     {}", rep.recovered_shards);
            println!("  Unrecoverable Blocks: {}", rep.unrecoverable_blocks);
        }
    }
    Ok(())
}

/// `tetherly screen pacer` -- Pacer arm of the screen dispatcher.
pub(crate) async fn screen_pacer(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/pacer", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/pacer").await
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
        let rep: tetherly_core::ScreenPacerReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!("Screen microsecond packet pacer mode updated: {}", rep.mode);
        } else {
            println!("Screen Microsecond Packet Pacing & Burst Shaper (ADR-091):");
            println!("  Pacer Mode:           {}", rep.mode);
            println!("  Packets Paced:        {}", rep.packets_paced);
            println!("  Bursts Smoothed:      {}", rep.bursts_smoothed);
            println!("  Average Delay:        {} us", rep.avg_delay_us);
            println!("  Last Packet Delay:    {} us", rep.last_delay_us);
        }
    }
    Ok(())
}

/// `tetherly screen trendline` -- Trendline arm of the screen dispatcher.
pub(crate) async fn screen_trendline(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/trendline", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/trendline").await
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
        let rep: tetherly_core::ScreenTrendlineReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!("Screen delay trendline filter mode updated: {}", rep.mode);
        } else {
            println!("Screen Delay-Based Congestion Controller & Trendline Filter (ADR-092):");
            println!("  Trendline Mode:       {}", rep.mode);
            println!("  Usage State:          {}", rep.state);
            println!(
                "  Current Slope:        {:.2} us/frame",
                rep.current_slope_us_per_frame
            );
            println!(
                "  Threshold:            {:.2} us/frame",
                rep.threshold_us_per_frame
            );
            println!("  Accumulated Delay:    {} us", rep.accumulated_delay_us);
            println!("  Overuse Events:       {}", rep.overuse_count);
            println!("  Underuse Events:      {}", rep.underuse_count);
        }
    }
    Ok(())
}

/// `tetherly screen harq` -- Harq arm of the screen dispatcher.
pub(crate) async fn screen_harq(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/harq", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/harq").await
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
        let rep: tetherly_core::ScreenHarqReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!(
                "Screen HARQ selective retransmission mode updated: {}",
                rep.mode
            );
        } else {
            println!("Screen Selective NACK & Hybrid ARQ (HARQ) Pipeline (ADR-093):");
            println!("  HARQ Mode:            {}", rep.mode);
            println!("  NACKs Sent:           {}", rep.nacks_sent);
            println!("  NACKs Received:       {}", rep.nacks_received);
            println!("  Shards Retransmitted: {}", rep.shards_retransmitted);
            println!("  Recovered Frames:     {}", rep.recovered_frames);
            println!("  Retransmit Timeouts:  {}", rep.retransmit_timeouts);
            println!(
                "  Buffer Occupancy:     {}/{}",
                rep.buffer_occupancy, rep.buffer_capacity
            );
        }
    }
    Ok(())
}

/// `tetherly screen twcc` -- Twcc arm of the screen dispatcher.
pub(crate) async fn screen_twcc(mode: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let has_update = mode.is_some();
    let (code, resp) = if has_update {
        let mut req = serde_json::Map::new();
        if let Some(m) = mode {
            req.insert("mode".into(), serde_json::json!(m));
        }
        let body = serde_json::to_vec(&req)?;
        tetherly_node::loopback_post(ui_port, "/api/screen/twcc", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/twcc").await
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
        let rep: tetherly_core::ScreenTwccReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if has_update {
            println!("Screen TWCC feedback mode updated: {}", rep.mode);
        } else {
            println!("Screen Transport-Wide Congestion Control (TWCC) Pipeline (ADR-094):");
            println!("  TWCC Mode:            {}", rep.mode);
            println!("  Feedbacks Sent:       {}", rep.feedbacks_sent);
            println!("  Feedbacks Received:   {}", rep.feedbacks_received);
            println!("  Packets Tracked:      {}", rep.packets_tracked);
            println!("  Packets Acknowledged: {}", rep.packets_acknowledged);
            println!("  Packets Lost:         {}", rep.packets_lost);
            println!("  Loss Ratio:           {:.2}%", rep.loss_ratio * 100.0);
            println!(
                "  Delivery Rate:        {:.2} Mbps ({} bps)",
                rep.delivery_rate_bps as f64 / 1_000_000.0,
                rep.delivery_rate_bps
            );
            println!("  Delay Gradient:       {} us", rep.delay_gradient_us);
            println!(
                "  Smoothed RTT:         {} us ({:.2} ms)",
                rep.smoothed_rtt_us,
                rep.smoothed_rtt_us as f64 / 1_000.0
            );
        }
    }
    Ok(())
}

/// `tetherly screen probe` -- Probe arm of the screen dispatcher.
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
        tetherly_node::loopback_post(ui_port, "/api/screen/probe", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/probe").await
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

/// `tetherly screen sync` -- Sync arm of the screen dispatcher.
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
        tetherly_node::loopback_post(ui_port, "/api/screen/sync", &body).await
    } else {
        tetherly_node::loopback_get(ui_port, "/api/screen/sync").await
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
