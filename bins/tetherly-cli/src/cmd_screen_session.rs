// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen session lifecycle: allow/revoke/start/stop/status/snapshot.

use super::util::{loopback_get, loopback_post, urlencode_path};
use super::*;

/// `tetherly-cli screen allow` -- Allow arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_allow(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_post(ui_port, "/api/screen/allow", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "screen allow failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
        let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(true);
        println!(
            "Screen viewing allowed (local consent granted). State: {state}, Allowed: {allowed}"
        );
    }
    Ok(())
}

/// `tetherly-cli screen revoke` -- Revoke arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_revoke(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_post(ui_port, "/api/screen/revoke", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "screen revoke failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
        let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(false);
        println!("Screen viewing consent revoked. State: {state}, Allowed: {allowed}");
    }
    Ok(())
}

/// `tetherly-cli screen start` -- Start arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_start(peer: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = loopback_post(ui_port, "/api/screen/start", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "screen start failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
        let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("none");
        println!("Screen streaming started. State: {state}, Target peer: {p}");
    }
    Ok(())
}

/// `tetherly-cli screen stop` -- Stop arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_stop(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_post(ui_port, "/api/screen/stop", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "screen stop failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
        println!("Screen streaming stopped. State: {state}");
    }
    Ok(())
}

/// `tetherly-cli screen status` -- Status arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_status(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/screen").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
            "screen status query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
        let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(false);
        let open = v
            .get("channel_open")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let active_peer = v.get("active_peer").and_then(|s| s.as_str());
        let sent = v.get("sent").and_then(|n| n.as_u64()).unwrap_or(0);
        let received = v.get("received").and_then(|n| n.as_u64()).unwrap_or(0);
        let acked = v.get("acked").and_then(|n| n.as_u64()).unwrap_or(0);
        let in_flight = v.get("in_flight").and_then(|n| n.as_u64()).unwrap_or(0);
        let dropped = v.get("dropped").and_then(|n| n.as_u64()).unwrap_or(0);
        let refused = v.get("refused").and_then(|n| n.as_u64()).unwrap_or(0);
        let bytes = v.get("bytes").and_then(|n| n.as_u64()).unwrap_or(0);
        let rx_bytes = v.get("rx_bytes").and_then(|n| n.as_u64()).unwrap_or(0);
        let p95 = v.get("p95_frame_interval_us").and_then(|n| n.as_u64());
        let rtt_latest = v.get("rtt_latest_us").and_then(|n| n.as_u64());
        let rtt_p95 = v.get("rtt_p95_us").and_then(|n| n.as_u64());
        let congested = v
            .get("is_congested")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let keyframes_requested = v
            .get("keyframes_requested")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let gaps = v.get("gaps").and_then(|n| n.as_u64()).unwrap_or(0);
        let bitrate_bps = v.get("bitrate_bps").and_then(|n| n.as_u64()).unwrap_or(0);
        let fps_target = v.get("fps_target").and_then(|n| n.as_u64()).unwrap_or(0);
        let scale_pct = v.get("scale_pct").and_then(|n| n.as_u64()).unwrap_or(0);
        let profile = v.get("profile").and_then(|s| s.as_str()).unwrap_or("");
        let qp_min = v.get("qp_min").and_then(|n| n.as_u64()).unwrap_or(0);
        let qp_max = v.get("qp_max").and_then(|n| n.as_u64()).unwrap_or(0);
        let matrix_mode = v.get("matrix_mode").and_then(|s| s.as_str()).unwrap_or("");
        let network_health = v
            .get("network_health")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let playout_mode = v.get("playout_mode").and_then(|s| s.as_str()).unwrap_or("");
        let playout_delay_ms = v
            .get("playout_delay_ms")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let playout_jitter_ms = v
            .get("playout_jitter_ms")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let playout_buffered = v
            .get("playout_buffered")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let cursor_mode = v.get("cursor_mode").and_then(|s| s.as_str()).unwrap_or("");
        let cursor_x = v.get("cursor_x").and_then(|n| n.as_i64()).unwrap_or(0);
        let cursor_y = v.get("cursor_y").and_then(|n| n.as_i64()).unwrap_or(0);
        let cursor_visible = v
            .get("cursor_visible")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let cursor_shape = v.get("cursor_shape").and_then(|s| s.as_str()).unwrap_or("");
        let cursor_updates_sent = v
            .get("cursor_updates_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let cursor_updates_received = v
            .get("cursor_updates_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let squelch_mode = v.get("squelch_mode").and_then(|s| s.as_str()).unwrap_or("");
        let is_squelched = v
            .get("is_squelched")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let squelched_frames = v
            .get("squelched_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let fec_mode = v.get("fec_mode").and_then(|s| s.as_str()).unwrap_or("");
        let fec_redundancy_pct = v
            .get("fec_redundancy_pct")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let fec_recovered_shards = v
            .get("fec_recovered_shards")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let fec_unrecoverable_blocks = v
            .get("fec_unrecoverable_blocks")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let pacer_mode = v.get("pacer_mode").and_then(|s| s.as_str()).unwrap_or("");
        let pacer_packets_paced = v
            .get("pacer_packets_paced")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let pacer_bursts_smoothed = v
            .get("pacer_bursts_smoothed")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let pacer_avg_delay_us = v
            .get("pacer_avg_delay_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let trendline_mode = v
            .get("trendline_mode")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let trendline_state = v
            .get("trendline_state")
            .and_then(|s| s.as_str())
            .unwrap_or("normal");
        let trendline_slope_us = v
            .get("trendline_slope_us")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let trendline_overuse_count = v
            .get("trendline_overuse_count")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let harq_mode = v.get("harq_mode").and_then(|s| s.as_str()).unwrap_or("");
        let harq_nacks_sent = v
            .get("harq_nacks_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let harq_nacks_received = v
            .get("harq_nacks_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let harq_shards_retransmitted = v
            .get("harq_shards_retransmitted")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let harq_recovered_frames = v
            .get("harq_recovered_frames")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let harq_buffer_occupancy = v
            .get("harq_buffer_occupancy")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let twcc_mode = v.get("twcc_mode").and_then(|s| s.as_str()).unwrap_or("");
        let twcc_feedbacks_sent = v
            .get("twcc_feedbacks_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let twcc_feedbacks_received = v
            .get("twcc_feedbacks_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let twcc_delivery_rate_bps = v
            .get("twcc_delivery_rate_bps")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let twcc_delay_gradient_us = v
            .get("twcc_delay_gradient_us")
            .and_then(|n| n.as_i64())
            .unwrap_or(0);
        let twcc_loss_ratio = v
            .get("twcc_loss_ratio")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let probe_mode = v.get("probe_mode").and_then(|s| s.as_str()).unwrap_or("");
        let probe_state = v.get("probe_state").and_then(|s| s.as_str()).unwrap_or("");
        let probes_triggered = v
            .get("probes_triggered")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let probes_completed = v
            .get("probes_completed")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let probes_failed = v.get("probes_failed").and_then(|n| n.as_u64()).unwrap_or(0);
        let probe_target_bps = v
            .get("probe_target_bps")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let probe_measured_bps = v
            .get("probe_measured_bps")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let sync_mode = v.get("sync_mode").and_then(|s| s.as_str()).unwrap_or("");
        let sync_beacons_sent = v
            .get("sync_beacons_sent")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let sync_beacons_received = v
            .get("sync_beacons_received")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let sync_skew_us = v.get("sync_skew_us").and_then(|n| n.as_i64()).unwrap_or(0);
        let sync_clock_drift_ppm = v
            .get("sync_clock_drift_ppm")
            .and_then(|n| n.as_f64())
            .unwrap_or(0.0);
        let sync_adjustments_applied = v
            .get("sync_adjustments_applied")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let in_sync = v.get("in_sync").and_then(|b| b.as_bool()).unwrap_or(true);
        let active_display_id = v
            .get("active_display_id")
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u8;
        let display_count = v.get("display_count").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
        let display_switches_count = v
            .get("display_switches_count")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let latency_mode = v.get("latency_mode").and_then(|s| s.as_str()).unwrap_or("");
        let latency_bottleneck = v
            .get("latency_bottleneck")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let latency_total_e2e_us = v
            .get("latency_total_e2e_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let latency_p95_e2e_us = v
            .get("latency_p95_e2e_us")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        let tuning_mode = v.get("tuning_mode").and_then(|s| s.as_str()).unwrap_or("");
        let tuning_actions_total = v
            .get("tuning_actions_total")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let tuning_recoveries = v
            .get("tuning_recoveries")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let tuning_last_stage = v
            .get("tuning_last_stage")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let tuning_last_action = v
            .get("tuning_last_action")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let recording_active = v
            .get("recording_active")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let peer_recording = v
            .get("peer_recording")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let recording_frames = v
            .get("recording_frames")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let playback_active = v
            .get("playback_active")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let playback_state = v
            .get("playback_state")
            .and_then(|x| x.as_str())
            .unwrap_or("idle");
        let playback_frame = v
            .get("playback_frame")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let snapshot_count = v
            .get("snapshot_count")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let watermark_enabled = v
            .get("watermark_enabled")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let watermark_enforced = v
            .get("watermark_enforced")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let watermark_frames = v
            .get("watermark_frames")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let annotations_count = v
            .get("annotations_count")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let laser_active = v
            .get("laser_active")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let annotations_total = v
            .get("annotations_total")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let laser_updates_total = v
            .get("laser_updates_total")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let whiteboard_undo_count = v
            .get("whiteboard_undo_count")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let whiteboard_redo_count = v
            .get("whiteboard_redo_count")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let latency_frames_profiled = v
            .get("latency_frames_profiled")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        println!("Screen Channel Status:");
        println!("  State:        {state}");
        println!("  Allowed:      {allowed}");
        println!("  Channel Open: {open}");
        if let Some(peer) = active_peer {
            println!("  Active Peer:  {peer}");
        }
        println!("  Frames Sent:  {sent}");
        println!("  Frames Recv:  {received}");
        println!("  Frames Acked: {acked}");
        println!("  In Flight:    {in_flight}");
        println!("  Dropped:      {dropped}");
        println!("  Refused:      {refused}");
        println!("  Bytes Sent:   {bytes}");
        println!("  Bytes Recv:   {rx_bytes}");
        match p95 {
            Some(us) => {
                println!("  p95 Interval: {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  p95 Interval: (not yet measurable)"),
        }
        match rtt_latest {
            Some(us) => {
                println!("  RTT (latest): {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  RTT (latest): (no ack yet)"),
        }
        match rtt_p95 {
            Some(us) => {
                println!("  RTT (p95):    {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  RTT (p95):    (no ack yet)"),
        }
        println!("  Congested:    {congested}");
        println!("  Keyframe Req: {keyframes_requested}");
        println!("  Gaps:         {gaps}");
        if bitrate_bps > 0 {
            println!(
                "  Bitrate:      {} kbps ({:.2} Mbps)",
                bitrate_bps / 1000,
                bitrate_bps as f64 / 1_000_000.0
            );
        }
        if fps_target > 0 {
            println!("  FPS:          {fps_target} FPS");
        }
        if scale_pct > 0 {
            println!("  Scale:        {scale_pct}%");
        }
        if !profile.is_empty() {
            println!("  Profile:      {profile} (QP {qp_min}-{qp_max})");
        }
        if !matrix_mode.is_empty() {
            println!("  Matrix Mode:  {matrix_mode}");
        }
        if !network_health.is_empty() {
            println!("  Net Health:   {network_health}");
        }
        if !playout_mode.is_empty() {
            println!(
                "  Playout Mode: {} ({:.1} ms delay, {:.2} ms jitter, {} buf)",
                playout_mode, playout_delay_ms, playout_jitter_ms, playout_buffered
            );
        }
        if !cursor_mode.is_empty() {
            println!(
                "  Cursor Mode:  {} (pos: {},{}, vis: {}, shape: {}, tx: {}, rx: {})",
                cursor_mode,
                cursor_x,
                cursor_y,
                cursor_visible,
                cursor_shape,
                cursor_updates_sent,
                cursor_updates_received
            );
        }
        if !squelch_mode.is_empty() {
            println!(
                "  Squelch Mode: {} (squelched={}, suppressed_frames={})",
                squelch_mode, is_squelched, squelched_frames
            );
        }
        if !fec_mode.is_empty() {
            println!(
                "  FEC Mode:     {} (redundancy: {}%, recovered: {}, unrecoverable: {})",
                fec_mode, fec_redundancy_pct, fec_recovered_shards, fec_unrecoverable_blocks
            );
        }
        if !pacer_mode.is_empty() {
            println!(
                "  Pacer Mode:   {} (paced_packets: {}, bursts_smoothed: {}, avg_delay: {} us)",
                pacer_mode, pacer_packets_paced, pacer_bursts_smoothed, pacer_avg_delay_us
            );
        }
        if !trendline_mode.is_empty() {
            println!(
                "  Trendline:    {} [state: {}] (slope: {:.2} us/frame, overuse_cnt: {})",
                trendline_mode, trendline_state, trendline_slope_us, trendline_overuse_count
            );
        }
        if !harq_mode.is_empty() {
            println!(
                "  HARQ:         {} (sent_nacks: {}, rcvd_nacks: {}, retx_shards: {}, recovered: {}, buf: {}/{})",
                harq_mode,
                harq_nacks_sent,
                harq_nacks_received,
                harq_shards_retransmitted,
                harq_recovered_frames,
                harq_buffer_occupancy,
                tetherly_core::SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY
            );
        }
        if !twcc_mode.is_empty() {
            println!(
                "  TWCC:         {} (sent_fb: {}, rcvd_fb: {}, rate: {:.2} Mbps, grad: {} us, loss: {:.2}%)",
                twcc_mode,
                twcc_feedbacks_sent,
                twcc_feedbacks_received,
                twcc_delivery_rate_bps as f64 / 1_000_000.0,
                twcc_delay_gradient_us,
                twcc_loss_ratio * 100.0
            );
        }
        if !probe_mode.is_empty() {
            println!(
                "  Probe:        {} (state: {}, trg: {}, ok: {}, fail: {}, target: {:.2} Mbps, emp: {:.2} Mbps)",
                probe_mode,
                probe_state,
                probes_triggered,
                probes_completed,
                probes_failed,
                probe_target_bps as f64 / 1_000_000.0,
                probe_measured_bps as f64 / 1_000_000.0
            );
        }
        if !sync_mode.is_empty() {
            println!(
                "  Sync:         {} (sent: {}, recv: {}, skew: {:.2} ms, drift: {:+.1} ppm, adj: {}, in-sync: {})",
                sync_mode,
                sync_beacons_sent,
                sync_beacons_received,
                sync_skew_us as f64 / 1_000.0,
                sync_clock_drift_ppm,
                sync_adjustments_applied,
                in_sync
            );
        }
        if display_count > 0 {
            println!(
                "  Display:      ID {} (total: {}, switches: {})",
                active_display_id, display_count, display_switches_count
            );
        }
        if !latency_mode.is_empty() {
            println!(
                "  Latency:      {} [bottleneck: {}] (total: {:.2} ms, P95: {:.2} ms, frames: {})",
                latency_mode,
                latency_bottleneck,
                latency_total_e2e_us as f64 / 1_000.0,
                latency_p95_e2e_us as f64 / 1_000.0,
                latency_frames_profiled
            );
        }
        if !tuning_mode.is_empty() {
            println!(
                "  Tuning:       {} [last: {} / {}] (actions: {}, recoveries: {})",
                tuning_mode,
                tuning_last_stage,
                tuning_last_action,
                tuning_actions_total,
                tuning_recoveries
            );
        }
        if recording_active || peer_recording {
            println!(
                "  Recording:    local: {} (frames: {}) · peer_recording: {}",
                recording_active, recording_frames, peer_recording
            );
        }
        if playback_active || playback_state != "idle" {
            println!(
                "  Playback:     state: {} (frame: {})",
                playback_state, playback_frame
            );
        }
        if snapshot_count > 0 {
            println!("  Snapshots:    {}", snapshot_count);
        }
        if watermark_enabled || watermark_enforced || watermark_frames > 0 {
            println!(
                "  Watermark:    enabled: {} · host_enforced: {} · frames: {}",
                watermark_enabled, watermark_enforced, watermark_frames
            );
        }
        if annotations_count > 0
            || laser_active
            || annotations_total > 0
            || laser_updates_total > 0
            || whiteboard_undo_count > 0
            || whiteboard_redo_count > 0
        {
            println!(
                "  Annotations:  active: {} · total_added: {} · laser_active: {} · laser_updates: {}",
                annotations_count, annotations_total, laser_active, laser_updates_total
            );
            if whiteboard_undo_count > 0
                || whiteboard_redo_count > 0
                || v.get("whiteboard_policy").is_some()
            {
                let pol_str = if let Some(p) = v.get("whiteboard_policy") {
                    format!(
                        " [read_only: {}, allow_clear: {}, max: {}]",
                        p.get("read_only")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(false),
                        p.get("allow_peer_clear")
                            .and_then(|b| b.as_bool())
                            .unwrap_or(true),
                        p.get("max_items").and_then(|m| m.as_u64()).unwrap_or(1000)
                    )
                } else {
                    String::new()
                };
                println!(
                    "  Whiteboard:   undo_events: {} · redo_events: {}{}",
                    whiteboard_undo_count, whiteboard_redo_count, pol_str
                );
            }
        }
        let roi_active = v
            .get("roi_active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let roi_x = v.get("roi_x").and_then(|v| v.as_u64()).unwrap_or(0);
        let roi_y = v.get("roi_y").and_then(|v| v.as_u64()).unwrap_or(0);
        let roi_w = v.get("roi_w").and_then(|v| v.as_u64()).unwrap_or(0);
        let roi_h = v.get("roi_h").and_then(|v| v.as_u64()).unwrap_or(0);
        let roi_zoom = v
            .get("roi_zoom_percent")
            .and_then(|v| v.as_u64())
            .unwrap_or(100);
        let roi_updates = v.get("roi_updates").and_then(|v| v.as_u64()).unwrap_or(0);
        if roi_active || roi_zoom != 100 || roi_updates > 0 {
            if roi_active {
                println!(
                    "  ROI Viewport: active [x: {roi_x}, y: {roi_y}, {roi_w}x{roi_h}, zoom: {roi_zoom}%, updates: {roi_updates}]"
                );
            } else {
                println!(
                    "  ROI Viewport: inactive (full-screen) [zoom: {roi_zoom}%, updates: {roi_updates}]"
                );
            }
        }
        let color_mode = v
            .get("color_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("full_color32");
        let color_threshold = v
            .get("color_threshold")
            .and_then(|v| v.as_u64())
            .unwrap_or(128);
        let color_updates = v.get("color_updates").and_then(|v| v.as_u64()).unwrap_or(0);
        if color_mode != "full_color32" || color_updates > 0 {
            println!(
                "  Color Depth:  mode: {color_mode} · threshold: {color_threshold} · updates: {color_updates}"
            );
        }
        let delta_enabled = v
            .get("delta_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let delta_keyframe_interval = v
            .get("delta_keyframe_interval")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);
        let delta_frames_sent = v
            .get("delta_frames_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let delta_keyframes_sent = v
            .get("delta_keyframes_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let delta_saved_bytes = v
            .get("delta_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if delta_enabled || delta_frames_sent > 0 || delta_saved_bytes > 0 {
            println!(
                "  Delta Patch:  enabled: {delta_enabled} · interval: {delta_keyframe_interval} · delta_sent: {delta_frames_sent} · keyframes: {delta_keyframes_sent} · saved: {delta_saved_bytes} B"
            );
        }
        let metric_enabled = v
            .get("metric_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let metric_sample_interval = v
            .get("metric_sample_interval")
            .and_then(|v| v.as_u64())
            .unwrap_or(10);
        let metric_evaluated_frames = v
            .get("metric_evaluated_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let metric_psnr_db = v
            .get("metric_psnr_db")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let metric_ssim = v.get("metric_ssim").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let metric_quality_warning = v
            .get("metric_quality_warning")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if metric_enabled || metric_evaluated_frames > 0 {
            let warn_str = if metric_quality_warning {
                " [QUALITY WARNING]"
            } else {
                ""
            };
            println!(
                "  Visual Metric:enabled: {metric_enabled} · interval: {metric_sample_interval} · evaluated: {metric_evaluated_frames} · psnr: {:.2} dB · ssim: {:.4}{warn_str}",
                metric_psnr_db, metric_ssim
            );
        }
        let qoe_enabled = v
            .get("qoe_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let qoe_mode = v.get("qoe_mode").and_then(|v| v.as_str()).unwrap_or("off");
        let qoe_target_psnr_db = v
            .get("qoe_target_psnr_db")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        let qoe_target_ssim_pct = v
            .get("qoe_target_ssim_pct")
            .and_then(|v| v.as_u64())
            .unwrap_or(85);
        let qoe_actions_total = v
            .get("qoe_actions_total")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let qoe_last_action = v
            .get("qoe_last_action")
            .and_then(|v| v.as_str())
            .unwrap_or("hold");
        if qoe_enabled || qoe_actions_total > 0 || qoe_mode != "off" {
            println!(
                "  QoE Closed-Loop: mode: {qoe_mode} · target: {qoe_target_psnr_db} dB / {qoe_target_ssim_pct}% · actions: {qoe_actions_total} · last: {qoe_last_action}"
            );
        }
        let pir_enabled = v
            .get("pir_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pir_mode = v
            .get("pir_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("disabled");
        let pir_bands = v.get("pir_bands").and_then(|v| v.as_u64()).unwrap_or(8);
        let pir_sweeps_completed = v
            .get("pir_sweeps_completed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let pir_slices_sent = v
            .get("pir_slices_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if pir_enabled || pir_sweeps_completed > 0 || pir_slices_sent > 0 || pir_mode != "disabled"
        {
            println!(
                "  PIR Anti-Burst: mode: {pir_mode} · bands: {pir_bands} · sweeps: {pir_sweeps_completed} · slices_sent: {pir_slices_sent}"
            );
        }
        let ltr_enabled = v
            .get("ltr_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ltr_auto_interval = v
            .get("ltr_auto_interval")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);
        let ltr_current_seq = v
            .get("ltr_current_seq")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ltr_anchors_established = v
            .get("ltr_anchors_established")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let ltr_recoveries_completed = v
            .get("ltr_recoveries_completed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if ltr_enabled || ltr_anchors_established > 0 || ltr_recoveries_completed > 0 {
            println!(
                "  LTR Recovery:   status: {} · auto_interval: {ltr_auto_interval} · anchor_seq: #{ltr_current_seq} · anchors: {ltr_anchors_established} · recoveries: {ltr_recoveries_completed}",
                if ltr_enabled { "enabled" } else { "disabled" }
            );
        }
        let svc_enabled = v
            .get("svc_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let svc_mode = v
            .get("svc_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("disabled");
        let svc_drop_t1 = v
            .get("svc_drop_t1")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let svc_t0_sent = v.get("svc_t0_sent").and_then(|v| v.as_u64()).unwrap_or(0);
        let svc_t1_sent = v.get("svc_t1_sent").and_then(|v| v.as_u64()).unwrap_or(0);
        let svc_t1_dropped = v
            .get("svc_t1_dropped")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if svc_enabled
            || svc_t0_sent > 0
            || svc_t1_sent > 0
            || svc_t1_dropped > 0
            || svc_mode != "disabled"
        {
            println!(
                "  SVC-T Stream:   status: {} · mode: {svc_mode} · drop_t1: {svc_drop_t1} · T0: {svc_t0_sent} · T1: {svc_t1_sent} · dropped: {svc_t1_dropped}",
                if svc_enabled { "enabled" } else { "disabled" }
            );
        }
        let motion_enabled = v
            .get("motion_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let motion_max_disp = v
            .get("motion_max_displacement")
            .and_then(|v| v.as_u64())
            .unwrap_or(64);
        let motion_frames_sent = v
            .get("motion_frames_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let motion_fallback = v
            .get("motion_fallback_frames_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let motion_saved_bytes = v
            .get("motion_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if motion_enabled || motion_frames_sent > 0 || motion_fallback > 0 || motion_saved_bytes > 0
        {
            println!(
                "  Motion Scroll:  status: {} · max_disp: {motion_max_disp}px · motion_frames: {motion_frames_sent} · fallback: {motion_fallback} · saved: {motion_saved_bytes}B",
                if motion_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_enabled = v
            .get("tile_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_size = v.get("tile_size").and_then(|v| v.as_u64()).unwrap_or(64);
        let tile_frames_sent = v
            .get("tile_frames_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let dirty_tiles_sent = v
            .get("dirty_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let skipped_tiles_count = v
            .get("skipped_tiles_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_saved_bytes = v
            .get("tile_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_enabled || tile_frames_sent > 0 || dirty_tiles_sent > 0 || tile_saved_bytes > 0 {
            println!(
                "  Spatial Tile-Grid: status: {} · tile_size: {tile_size}px · grid_frames: {tile_frames_sent} · dirty_tiles: {dirty_tiles_sent} · skipped: {skipped_tiles_count} · saved: {tile_saved_bytes}B",
                if tile_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_cache_enabled = v
            .get("tile_cache_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_cache_entries = v
            .get("tile_cache_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_cache_hits = v
            .get("tile_cache_hits")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_cache_misses = v
            .get("tile_cache_misses")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_cache_saved_bytes = v
            .get("tile_cache_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_cache_enabled
            || tile_cache_entries > 0
            || tile_cache_hits > 0
            || tile_cache_misses > 0
            || tile_cache_saved_bytes > 0
        {
            let total_lookups = tile_cache_hits + tile_cache_misses;
            let hit_pct = if total_lookups > 0 {
                (tile_cache_hits as f64 / total_lookups as f64) * 100.0
            } else {
                0.0
            };
            println!(
                "  Tile Dictionary:  status: {} · entries: {tile_cache_entries} · hits: {tile_cache_hits} · misses: {tile_cache_misses} · hit_ratio: {hit_pct:.1}% · saved: {tile_cache_saved_bytes}B",
                if tile_cache_enabled { "enabled" } else { "disabled" }
            );
        }
        let focus_enabled = v
            .get("focus_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let focus_mode = v
            .get("focus_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("cursor");
        let focus_radius = v
            .get("focus_radius")
            .and_then(|v| v.as_u64())
            .unwrap_or(256);
        let focus_skip_ratio = v
            .get("focus_peripheral_skip_ratio")
            .and_then(|v| v.as_u64())
            .unwrap_or(2);
        let focus_tiles_sent = v
            .get("focus_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let peripheral_tiles_sent = v
            .get("peripheral_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let peripheral_tiles_skipped = v
            .get("peripheral_tiles_skipped")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let focus_saved_bandwidth_bytes = v
            .get("focus_saved_bandwidth_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if focus_enabled
            || focus_tiles_sent > 0
            || peripheral_tiles_sent > 0
            || peripheral_tiles_skipped > 0
            || focus_saved_bandwidth_bytes > 0
        {
            println!(
                "  Foveated Focus:   status: {} · mode: {focus_mode} · radius: {focus_radius}px · skip_ratio: {focus_skip_ratio} · focus_tiles: {focus_tiles_sent} · peripheral_sent: {peripheral_tiles_sent} · peripheral_skipped: {peripheral_tiles_skipped} · saved: {focus_saved_bandwidth_bytes}B",
                if focus_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_palette_enabled = v
            .get("tile_palette_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_palette_max_colors = v
            .get("tile_palette_max_colors")
            .and_then(|v| v.as_u64())
            .unwrap_or(16);
        let tile_palette_solid_tiles = v
            .get("tile_palette_solid_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_palette_bilevel_tiles = v
            .get("tile_palette_bilevel_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_palette_palette_tiles = v
            .get("tile_palette_palette_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_palette_truecolor_tiles = v
            .get("tile_palette_truecolor_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_palette_saved_bytes = v
            .get("tile_palette_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_palette_enabled
            || tile_palette_solid_tiles > 0
            || tile_palette_bilevel_tiles > 0
            || tile_palette_palette_tiles > 0
            || tile_palette_truecolor_tiles > 0
            || tile_palette_saved_bytes > 0
        {
            println!(
                "  Tile Palette:     status: {} · max_colors: {tile_palette_max_colors} · solid: {tile_palette_solid_tiles} · bilevel: {tile_palette_bilevel_tiles} · palette: {tile_palette_palette_tiles} · truecolor: {tile_palette_truecolor_tiles} · saved: {tile_palette_saved_bytes}B",
                if tile_palette_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_predict_enabled = v
            .get("tile_predict_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_predict_filter_mode = v
            .get("tile_predict_filter_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let tile_predict_enable_rct = v
            .get("tile_predict_enable_rct")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let tile_predict_tiles_sent = v
            .get("tile_predict_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_predict_saved_bytes = v
            .get("tile_predict_saved_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_predict_enabled || tile_predict_tiles_sent > 0 || tile_predict_saved_bytes > 0 {
            println!(
                "  Tile Predict:     status: {} · filter: {tile_predict_filter_mode} · rct: {tile_predict_enable_rct} · predict_tiles: {tile_predict_tiles_sent} · saved: {tile_predict_saved_bytes}B",
                if tile_predict_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_present_enabled = v
            .get("tile_present_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_present_vsync_mode = v
            .get("tile_present_vsync_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("off");
        let tile_present_double_buffer = v
            .get("tile_present_double_buffer")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let tile_present_tear_prevention = v
            .get("tile_present_tear_prevention")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let tile_present_committed_transactions = v
            .get("tile_present_committed_transactions")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_present_swapped_frames = v
            .get("tile_present_swapped_frames")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_present_torn_frames_prevented = v
            .get("tile_present_torn_frames_prevented")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_present_avg_swap_latency_us = v
            .get("tile_present_avg_swap_latency_us")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_present_enabled
            || tile_present_committed_transactions > 0
            || tile_present_swapped_frames > 0
        {
            println!(
                "  Tile Present:     status: {} · vsync: {tile_present_vsync_mode} · double_buf: {tile_present_double_buffer} · anti_tear: {tile_present_tear_prevention} · committed_tx: {tile_present_committed_transactions} · swapped: {tile_present_swapped_frames} · torn_prevented: {tile_present_torn_frames_prevented} · avg_latency: {tile_present_avg_swap_latency_us}µs",
                if tile_present_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_nack_enabled = v
            .get("tile_nack_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_nack_timeout_ms = v
            .get("tile_nack_timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(20);
        let tile_nack_max_retries = v
            .get("tile_nack_max_retries")
            .and_then(|v| v.as_u64())
            .unwrap_or(2);
        let tile_nack_requests_sent = v
            .get("tile_nack_requests_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_nack_tiles_retransmitted = v
            .get("tile_nack_tiles_retransmitted")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_nack_degraded_commits = v
            .get("tile_nack_degraded_commits")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_nack_enabled
            || tile_nack_requests_sent > 0
            || tile_nack_tiles_retransmitted > 0
            || tile_nack_degraded_commits > 0
        {
            println!(
                "  Tile Micro-NACK:  status: {} · timeout: {tile_nack_timeout_ms}ms · max_retries: {tile_nack_max_retries} · nack_sent: {tile_nack_requests_sent} · retransmitted: {tile_nack_tiles_retransmitted} · degraded: {tile_nack_degraded_commits}",
                if tile_nack_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_saliency_enabled = v
            .get("tile_saliency_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_saliency_high_threshold = v
            .get("tile_saliency_high_threshold")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);
        let tile_saliency_low_threshold = v
            .get("tile_saliency_low_threshold")
            .and_then(|v| v.as_u64())
            .unwrap_or(40);
        let tile_saliency_drop_background_on_congestion = v
            .get("tile_saliency_drop_background_on_congestion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let tile_saliency_critical_tiles = v
            .get("tile_saliency_critical_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_saliency_normal_tiles = v
            .get("tile_saliency_normal_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_saliency_background_tiles = v
            .get("tile_saliency_background_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_saliency_background_skipped = v
            .get("tile_saliency_background_skipped")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_saliency_enabled
            || tile_saliency_critical_tiles > 0
            || tile_saliency_normal_tiles > 0
            || tile_saliency_background_tiles > 0
            || tile_saliency_background_skipped > 0
        {
            println!(
                "  Tile Saliency:    status: {} · high_thresh: {tile_saliency_high_threshold} · low_thresh: {tile_saliency_low_threshold} · drop_bg_congest: {tile_saliency_drop_background_on_congestion} · critical: {tile_saliency_critical_tiles} · normal: {tile_saliency_normal_tiles} · background: {tile_saliency_background_tiles} · bg_skipped: {tile_saliency_background_skipped}",
                if tile_saliency_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_lod_enabled = v
            .get("tile_lod_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_lod_max_lod = v
            .get("tile_lod_max_lod")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let tile_lod_auto_lod_on_congestion = v
            .get("tile_lod_auto_lod_on_congestion")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let tile_lod0_tiles = v
            .get("tile_lod0_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_lod1_tiles = v
            .get("tile_lod1_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_lod2_tiles = v
            .get("tile_lod2_tiles")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_lod_bytes_saved = v
            .get("tile_lod_bytes_saved")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_lod_enabled
            || tile_lod0_tiles > 0
            || tile_lod1_tiles > 0
            || tile_lod2_tiles > 0
            || tile_lod_bytes_saved > 0
        {
            println!(
                "  Tile LoD Downsample: status: {} · max_lod: Lod{tile_lod_max_lod} · auto_lod_congest: {tile_lod_auto_lod_on_congestion} · lod0: {tile_lod0_tiles} · lod1: {tile_lod1_tiles} · lod2: {tile_lod2_tiles} · bytes_saved: {tile_lod_bytes_saved} B",
                if tile_lod_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_batch_enabled = v
            .get("tile_batch_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_batch_target_chunk_bytes = v
            .get("tile_batch_target_chunk_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(1200);
        let tile_batch_pacing_interval_us = v
            .get("tile_batch_pacing_interval_us")
            .and_then(|v| v.as_u64())
            .unwrap_or(200);
        let tile_batches_sent = v
            .get("tile_batches_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_chunks_sent = v
            .get("tile_chunks_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_batch_avg_chunk_bytes = v
            .get("tile_batch_avg_chunk_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_batch_pacing_pauses = v
            .get("tile_batch_pacing_pauses")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_batch_enabled
            || tile_batches_sent > 0
            || tile_chunks_sent > 0
            || tile_batch_pacing_pauses > 0
        {
            println!(
                "  Tile Batching:    status: {} · target_chunk: {tile_batch_target_chunk_bytes} B · pacing: {tile_batch_pacing_interval_us} µs · batches: {tile_batches_sent} · chunks: {tile_chunks_sent} · avg_chunk: {tile_batch_avg_chunk_bytes} B · pauses: {tile_batch_pacing_pauses}",
                if tile_batch_enabled { "enabled" } else { "disabled" }
            );
        }
        let tile_motion_enabled = v
            .get("tile_motion_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tile_motion_max_displacement = v
            .get("tile_motion_max_displacement")
            .and_then(|v| v.as_u64())
            .unwrap_or(32);
        let tile_motion_min_match_ratio = v
            .get("tile_motion_min_match_ratio")
            .and_then(|v| v.as_u64())
            .unwrap_or(90);
        let tile_motion_tiles_sent = v
            .get("tile_motion_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_motion_fallback_tiles_sent = v
            .get("tile_motion_fallback_tiles_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let tile_motion_bytes_saved = v
            .get("tile_motion_bytes_saved")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if tile_motion_enabled
            || tile_motion_tiles_sent > 0
            || tile_motion_fallback_tiles_sent > 0
            || tile_motion_bytes_saved > 0
        {
            println!(
                "  Tile Motion Est:  status: {} · max_disp: {tile_motion_max_displacement} px · min_match: {tile_motion_min_match_ratio}% · motion_tiles: {tile_motion_tiles_sent} · fallback_tiles: {tile_motion_fallback_tiles_sent} · bytes_saved: {tile_motion_bytes_saved} B",
                if tile_motion_enabled { "enabled" } else { "disabled" }
            );
        }
    }
    Ok(())
}

/// `tetherly-cli screen keyframe` -- Keyframe arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_keyframe(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_post(ui_port, "/api/screen/keyframe", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "screen keyframe request failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let total = v
            .get("keyframes_requested")
            .and_then(|n| n.as_u64())
            .unwrap_or(0);
        println!("On-demand IDR keyframe requested from screen host. Total requested: {total}");
    }
    Ok(())
}

/// `tetherly-cli screen displays` -- Displays arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_displays(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/screen/displays")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen displays query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_core::ScreenDisplayReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!("Screen Multi-Display & Virtual Display Pipeline (ADR-097):");
        println!("  Active Display ID:    {}", rep.active_display_id);
        println!("  Total Displays:       {}", rep.displays.len());
        println!("  Dynamic Switches:     {}", rep.switches_count);
        if rep.last_switch_us > 0 {
            println!("  Last Switch (us):     {}", rep.last_switch_us);
        }
        println!("  Displays:");
        for d in &rep.displays {
            let tag = if d.display_id == rep.active_display_id {
                " [active]"
            } else {
                ""
            };
            let kind = if d.is_virtual {
                " (virtual)"
            } else if d.is_primary {
                " (primary)"
            } else {
                ""
            };
            println!(
                "    - ID {}: \"{}\" {}x{} @ {}% scale{}{}",
                d.display_id, d.title, d.width, d.height, d.scale_factor_pct, kind, tag
            );
        }
    }
    Ok(())
}

/// `tetherly-cli screen switch` -- Switch arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_switch(display_id: u8, json: bool, ui_port: u16) -> Result<()> {
    let req = serde_json::json!({ "display_id": display_id });
    let body = serde_json::to_vec(&req)?;
    let (code, resp) = loopback_post(ui_port, "/api/screen/switch", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "display switch failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_core::ScreenDisplayReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "Screen display switched successfully: active display is now ID {} (total switches: {})",
            rep.active_display_id, rep.switches_count
        );
    }
    Ok(())
}

/// `tetherly-cli screen snapshot` -- Snapshot arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_snapshot(
    out: Option<String>,
    format: String,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let query = match (&out, format.as_str()) {
        (Some(o), f) => format!("?format={f}&out={}", urlencode_path(o)),
        (None, f) => format!("?format={f}"),
    };
    let uri = format!("/api/screen/snapshot{query}");
    let (code, resp) = loopback_get(ui_port, &uri).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        anyhow::bail!(
            "screen snapshot failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        print_screen_snapshot_report(&rep);
    }
    Ok(())
}

/// `tetherly-cli screen laser` -- Laser arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_laser(
    x: Option<u16>,
    y: Option<u16>,
    color: Option<String>,
    radius: Option<u8>,
    off: bool,
    status: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    if status || (x.is_none() && y.is_none() && !off && color.is_none() && radius.is_none()) {
        let (code, resp) = loopback_get(ui_port, "/api/screen/laser")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen laser query failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let active = rep
                .get("laser_active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let updates = rep
                .get("laser_updates_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            println!("Screen Transient Virtual Laser Pointer (ADR-105):");
            println!(
                "  Laser State:          {}",
                if active { "ACTIVE" } else { "OFF / IDLE" }
            );
            println!("  Laser Updates Total:  {updates}");
            if let Some(l) = rep.get("laser").filter(|v| !v.is_null()) {
                let lx = l.get("x").and_then(|v| v.as_u64()).unwrap_or(0);
                let ly = l.get("y").and_then(|v| v.as_u64()).unwrap_or(0);
                let lr = l.get("radius").and_then(|v| v.as_u64()).unwrap_or(12);
                let col = l.get("color_rgb").and_then(|v| v.as_array());
                let col_str = if let Some(c) = col {
                    if c.len() == 3 {
                        format!(
                            "#{:02X}{:02X}{:02X}",
                            c[0].as_u64().unwrap_or(0),
                            c[1].as_u64().unwrap_or(0),
                            c[2].as_u64().unwrap_or(0)
                        )
                    } else {
                        "#FF0000".to_string()
                    }
                } else {
                    "#FF0000".to_string()
                };
                println!("  Spot Position:        ({lx}, {ly}) [radius: {lr}px, color: {col_str}]");
            }
        }
    } else {
        let mut payload = serde_json::Map::new();
        if off {
            payload.insert("active".to_string(), serde_json::json!(false));
        } else {
            payload.insert("active".to_string(), serde_json::json!(true));
        }
        if let Some(px) = x {
            payload.insert("x".to_string(), serde_json::json!(px));
        }
        if let Some(py) = y {
            payload.insert("y".to_string(), serde_json::json!(py));
        }
        if let Some(c) = color {
            payload.insert("color".to_string(), serde_json::json!(c));
        }
        if let Some(r) = radius {
            payload.insert("radius".to_string(), serde_json::json!(r));
        }
        let bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = loopback_post(ui_port, "/api/screen/laser", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen laser update failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let active = rep
                .get("laser_active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if active {
                println!("Laser pointer activated / updated.");
            } else {
                println!("Laser pointer deactivated / turned off.");
            }
        }
    }
    Ok(())
}

pub(crate) fn print_screen_snapshot_report(rep: &serde_json::Value) {
    println!("Screen Snapshot (Zero-Dependency BMP Pipeline, ADR-103):");
    let ok = rep.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let source = rep
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let seq = rep.get("frame_seq").and_then(|v| v.as_u64()).unwrap_or(0);
    let pts = rep.get("pts_us").and_then(|v| v.as_u64()).unwrap_or(0);
    let width = rep.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = rep.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
    let format = rep.get("format").and_then(|v| v.as_str()).unwrap_or("bmp");
    let bytes_len = rep.get("bytes_len").and_then(|v| v.as_u64()).unwrap_or(0);
    let blanked = rep
        .get("privacy_blanked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let watermarked = rep
        .get("watermarked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let path = rep.get("path").and_then(|v| v.as_str());

    println!(
        "  Status:               {}",
        if ok { "ok" } else { "failed" }
    );
    println!("  Source:               {source}");
    println!("  Frame Seq:            #{seq}");
    println!(
        "  PTS:                  {pts} us ({:.2} s)",
        pts as f64 / 1_000_000.0
    );
    println!("  Resolution:           {width}x{height}");
    println!("  Format:               {format}");
    println!("  Payload Size:         {bytes_len} bytes");
    println!(
        "  Privacy Blanked:      {}",
        if blanked { "yes (blackout mask)" } else { "no" }
    );
    println!(
        "  Watermarked:          {}",
        if watermarked {
            "yes (forensic attribution overlay)"
        } else {
            "no"
        }
    );
    if let Some(p) = path {
        println!("  Saved To:             {p}");
    }
}
