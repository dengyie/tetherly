// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen session lifecycle: allow/revoke/start/stop/status/snapshot.

use super::util::urlencode_path;
use super::*;

/// `tetherly screen allow` -- Allow arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_allow(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/allow", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen allow failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenControlReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "Screen viewing allowed (local consent granted). State: {}, Allowed: {}",
            rep.state, rep.allowed
        );
    }
    Ok(())
}

/// `tetherly screen revoke` -- Revoke arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_revoke(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/revoke", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen revoke failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenControlReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "Screen viewing consent revoked. State: {}, Allowed: {}",
            rep.state, rep.allowed
        );
    }
    Ok(())
}

/// `tetherly screen start` -- Start arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_start(peer: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({ "peer": peer });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/start", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen start failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenControlReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "Screen streaming started. State: {}, Target peer: {}",
            rep.state,
            rep.peer.as_deref().unwrap_or("none")
        );
    }
    Ok(())
}

/// `tetherly screen stop` -- Stop arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_stop(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/stop", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen stop failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenControlReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!("Screen streaming stopped. State: {}", rep.state);
    }
    Ok(())
}

/// `tetherly screen keyframe` -- Keyframe arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_keyframe(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/keyframe", b"")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen keyframe request failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenControlReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "Keyframe requested from host. State: {}, Target peer: {}",
            rep.state,
            rep.peer.as_deref().unwrap_or("none")
        );
    }
    Ok(())
}

/// `tetherly screen status` -- Status arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_status(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "screen status query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let rep: tetherly_node::ScreenStatusReport =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!("Screen Channel Status:");
        println!("  State:        {}", rep.state);
        println!("  Allowed:      {}", rep.allowed);
        println!("  Channel Open: {}", rep.channel_open);
        if let Some(peer) = &rep.active_peer {
            println!("  Active Peer:  {peer}");
        }
        println!("  Frames Sent:  {}", rep.sent);
        println!("  Frames Recv:  {}", rep.received);
        println!("  Frames Acked: {}", rep.acked);
        println!("  In Flight:    {}", rep.in_flight);
        println!("  Dropped:      {}", rep.dropped);
        println!("  Refused:      {}", rep.refused);
        println!("  Bytes Sent:   {}", rep.bytes);
        println!("  Bytes Recv:   {}", rep.rx_bytes);
        match rep.p95_frame_interval_us {
            Some(us) => {
                println!("  p95 Interval: {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  p95 Interval: (not yet measurable)"),
        }
        match rep.rtt_latest_us {
            Some(us) => {
                println!("  RTT (latest): {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  RTT (latest): (no ack yet)"),
        }
        match rep.rtt_p95_us {
            Some(us) => {
                println!("  RTT (p95):    {:.2} ms ({us} us)", us as f64 / 1000.0);
            }
            None => println!("  RTT (p95):    (no ack yet)"),
        }
        println!("  Congested:    {}", rep.is_congested);
        println!("  Keyframe Req: {}", rep.keyframes_requested);
        println!("  Gaps:         {}", rep.gaps);
        if rep.bitrate_bps > 0 {
            println!(
                "  Bitrate:      {} kbps ({:.2} Mbps)",
                rep.bitrate_bps / 1000,
                rep.bitrate_bps as f64 / 1_000_000.0
            );
        }
        if rep.fps_target > 0 {
            println!("  FPS:          {} FPS", rep.fps_target);
        }
        if rep.scale_pct > 0 {
            println!("  Scale:        {}%", rep.scale_pct);
        }
        if !rep.profile.is_empty() {
            println!(
                "  Profile:      {} (QP {}-{})",
                rep.profile, rep.qp_min, rep.qp_max
            );
        }
        if !rep.matrix_mode.is_empty() {
            println!("  Matrix Mode:  {}", rep.matrix_mode);
        }
        if !rep.network_health.is_empty() {
            println!("  Net Health:   {}", rep.network_health);
        }
        if !rep.playout_mode.is_empty() {
            println!(
                "  Playout Mode: {} ({:.1} ms delay, {:.2} ms jitter, {} buf)",
                rep.playout_mode, rep.playout_delay_ms, rep.playout_jitter_ms, rep.playout_buffered
            );
        }
        if !rep.cursor_mode.is_empty() {
            println!(
                "  Cursor Mode:  {} (pos: {},{}, vis: {}, shape: {}, tx: {}, rx: {})",
                rep.cursor_mode,
                rep.cursor_x,
                rep.cursor_y,
                rep.cursor_visible,
                rep.cursor_shape,
                rep.cursor_updates_sent,
                rep.cursor_updates_received
            );
        }
        if !rep.squelch_mode.is_empty() {
            println!(
                "  Squelch Mode: {} (squelched={}, suppressed_frames={})",
                rep.squelch_mode, rep.is_squelched, rep.squelched_frames
            );
        }
        if !rep.fec_mode.is_empty() {
            println!(
                "  FEC Mode:     {} (redundancy: {}%, recovered: {}, unrecoverable: {})",
                rep.fec_mode,
                rep.fec_redundancy_pct,
                rep.fec_recovered_shards,
                rep.fec_unrecoverable_blocks
            );
        }
        if !rep.pacer_mode.is_empty() {
            println!(
                "  Pacer Mode:   {} (paced_packets: {}, bursts_smoothed: {}, avg_delay: {} us)",
                rep.pacer_mode,
                rep.pacer_packets_paced,
                rep.pacer_bursts_smoothed,
                rep.pacer_avg_delay_us
            );
        }
        if !rep.trendline_mode.is_empty() {
            println!(
                "  Trendline:    {} [state: {}] (slope: {:.2} us/frame, overuse_cnt: {})",
                rep.trendline_mode,
                rep.trendline_state,
                rep.trendline_slope_us,
                rep.trendline_overuse_count
            );
        }
        if !rep.harq_mode.is_empty() {
            println!(
                "  HARQ:         {} (sent_nacks: {}, rcvd_nacks: {}, retx_shards: {}, recovered: {}, buf: {}/{})",
                rep.harq_mode,
                rep.harq_nacks_sent,
                rep.harq_nacks_received,
                rep.harq_shards_retransmitted,
                rep.harq_recovered_frames,
                rep.harq_buffer_occupancy,
                tetherly_core::SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY
            );
        }
        if !rep.twcc_mode.is_empty() {
            println!(
                "  TWCC:         {} (sent_fb: {}, rcvd_fb: {}, rate: {:.2} Mbps, grad: {} us, loss: {:.2}%)",
                rep.twcc_mode,
                rep.twcc_feedbacks_sent,
                rep.twcc_feedbacks_received,
                rep.twcc_delivery_rate_bps as f64 / 1_000_000.0,
                rep.twcc_delay_gradient_us,
                rep.twcc_loss_ratio * 100.0
            );
        }
        if !rep.probe_mode.is_empty() {
            println!(
                "  Probe:        {} (state: {}, trg: {}, ok: {}, fail: {}, target: {:.2} Mbps, emp: {:.2} Mbps)",
                rep.probe_mode,
                rep.probe_state,
                rep.probes_triggered,
                rep.probes_completed,
                rep.probes_failed,
                rep.probe_target_bps as f64 / 1_000_000.0,
                rep.probe_measured_bps as f64 / 1_000_000.0
            );
        }
        if !rep.sync_mode.is_empty() {
            println!(
                "  Sync:         {} (sent: {}, recv: {}, skew: {:.2} ms, drift: {:+.1} ppm, adj: {}, in-sync: {})",
                rep.sync_mode,
                rep.sync_beacons_sent,
                rep.sync_beacons_received,
                rep.sync_skew_us as f64 / 1_000.0,
                rep.sync_clock_drift_ppm,
                rep.sync_adjustments_applied,
                rep.in_sync
            );
        }
        if rep.display_count > 0 {
            println!(
                "  Display:      ID {} (total: {}, switches: {})",
                rep.active_display_id, rep.display_count, rep.display_switches_count
            );
        }
        if !rep.latency_mode.is_empty() {
            println!(
                "  Latency:      {} [bottleneck: {}] (total: {:.2} ms, P95: {:.2} ms, frames: {})",
                rep.latency_mode,
                rep.latency_bottleneck,
                rep.latency_total_e2e_us as f64 / 1_000.0,
                rep.latency_p95_e2e_us as f64 / 1_000.0,
                rep.latency_frames_profiled
            );
        }
        if !rep.tuning_mode.is_empty() {
            println!(
                "  Tuning:       {} [last: {} / {}] (actions: {}, recoveries: {})",
                rep.tuning_mode,
                rep.tuning_last_stage,
                rep.tuning_last_action,
                rep.tuning_actions_total,
                rep.tuning_recoveries
            );
        }
        if rep.recording_active || rep.peer_recording {
            println!(
                "  Recording:    local: {} (frames: {}) · peer_recording: {}",
                rep.recording_active, rep.recording_frames, rep.peer_recording
            );
        }
        if rep.playback_active || rep.playback_state != "idle" {
            println!(
                "  Playback:     state: {} (frame: {})",
                rep.playback_state, rep.playback_frame
            );
        }
        if rep.snapshot_count > 0 {
            println!("  Snapshots:    {}", rep.snapshot_count);
        }
        if rep.watermark_enabled || rep.watermark_enforced || rep.watermark_frames > 0 {
            println!(
                "  Watermark:    enabled: {} · host_enforced: {} · frames: {}",
                rep.watermark_enabled, rep.watermark_enforced, rep.watermark_frames
            );
        }
        if rep.annotations_count > 0
            || rep.laser_active
            || rep.annotations_total > 0
            || rep.laser_updates_total > 0
            || rep.whiteboard_undo_count > 0
            || rep.whiteboard_redo_count > 0
        {
            println!(
                "  Annotations:  active: {} · total_added: {} · laser_active: {} · laser_updates: {}",
                rep.annotations_count, rep.annotations_total, rep.laser_active, rep.laser_updates_total
            );
            if rep.whiteboard_undo_count > 0
                || rep.whiteboard_redo_count > 0
                || rep.whiteboard_read_only
                || rep.whiteboard_max_items != 1000
            {
                println!(
                    "  Whiteboard:   undo_events: {} · redo_events: {} [read_only: {}, max: {}]",
                    rep.whiteboard_undo_count,
                    rep.whiteboard_redo_count,
                    rep.whiteboard_read_only,
                    rep.whiteboard_max_items
                );
            }
        }
        if rep.roi_active || rep.roi_zoom_percent != 100 || rep.roi_updates > 0 {
            if rep.roi_active {
                println!(
                    "  ROI Viewport: active [x: {}, y: {}, {}x{}, zoom: {}%, updates: {}]",
                    rep.roi_x,
                    rep.roi_y,
                    rep.roi_w,
                    rep.roi_h,
                    rep.roi_zoom_percent,
                    rep.roi_updates
                );
            } else {
                println!(
                    "  ROI Viewport: inactive (full-screen) [zoom: {}%, updates: {}]",
                    rep.roi_zoom_percent, rep.roi_updates
                );
            }
        }
        if rep.color_mode != "full_color32" || rep.color_updates > 0 {
            println!(
                "  Color Depth:  mode: {} · threshold: {} · updates: {}",
                rep.color_mode, rep.color_threshold, rep.color_updates
            );
        }
        if rep.delta_enabled || rep.delta_frames_sent > 0 || rep.delta_saved_bytes > 0 {
            println!(
                "  Delta Patch:  enabled: {} · interval: {} · delta_sent: {} · keyframes: {} · saved: {} B",
                rep.delta_enabled, rep.delta_keyframe_interval, rep.delta_frames_sent, rep.delta_keyframes_sent, rep.delta_saved_bytes
            );
        }
        if rep.metric_enabled || rep.metric_evaluated_frames > 0 {
            let warn_str = if rep.metric_quality_warning {
                " [QUALITY WARNING]"
            } else {
                ""
            };
            println!(
                "  Visual Metric:enabled: {} · interval: {} · evaluated: {} · psnr: {:.2} dB · ssim: {:.4}{warn_str}",
                rep.metric_enabled, rep.metric_sample_interval, rep.metric_evaluated_frames,
                rep.metric_psnr_db, rep.metric_ssim
            );
        }
        if rep.qoe_enabled || rep.qoe_actions_total > 0 || rep.qoe_mode != "off" {
            println!(
                "  QoE Closed-Loop: mode: {} · target: {} dB / {}% · actions: {} · last: {}",
                rep.qoe_mode,
                rep.qoe_target_psnr_db,
                rep.qoe_target_ssim_pct,
                rep.qoe_actions_total,
                rep.qoe_last_action
            );
        }
        if rep.pir_enabled
            || rep.pir_sweeps_completed > 0
            || rep.pir_slices_sent > 0
            || rep.pir_mode != "disabled"
        {
            println!(
                "  PIR Anti-Burst: mode: {} · bands: {} · sweeps: {} · slices_sent: {}",
                rep.pir_mode, rep.pir_bands, rep.pir_sweeps_completed, rep.pir_slices_sent
            );
        }
        if rep.ltr_enabled || rep.ltr_anchors_established > 0 || rep.ltr_recoveries_completed > 0 {
            println!(
                "  LTR Recovery:   status: {} · auto_interval: {} · anchor_seq: #{} · anchors: {} · recoveries: {}",
                if rep.ltr_enabled { "enabled" } else { "disabled" },
                rep.ltr_auto_interval,
                rep.ltr_current_seq,
                rep.ltr_anchors_established,
                rep.ltr_recoveries_completed
            );
        }
        if rep.svc_enabled
            || rep.svc_t0_sent > 0
            || rep.svc_t1_sent > 0
            || rep.svc_t1_dropped > 0
            || rep.svc_mode != "disabled"
        {
            println!(
                "  SVC-T Stream:   status: {} · mode: {} · drop_t1: {} · T0: {} · T1: {} · dropped: {}",
                if rep.svc_enabled { "enabled" } else { "disabled" },
                rep.svc_mode,
                rep.svc_drop_t1,
                rep.svc_t0_sent,
                rep.svc_t1_sent,
                rep.svc_t1_dropped
            );
        }
        if rep.motion_enabled
            || rep.motion_frames_sent > 0
            || rep.motion_fallback_frames_sent > 0
            || rep.motion_saved_bytes > 0
        {
            println!(
                "  Motion Scroll:  status: {} · max_disp: {}px · motion_frames: {} · fallback: {} · saved: {}B",
                if rep.motion_enabled { "enabled" } else { "disabled" },
                rep.motion_max_displacement,
                rep.motion_frames_sent,
                rep.motion_fallback_frames_sent,
                rep.motion_saved_bytes
            );
        }
        if rep.tile_enabled
            || rep.tile_frames_sent > 0
            || rep.dirty_tiles_sent > 0
            || rep.tile_saved_bytes > 0
        {
            println!(
                "  Spatial Tile-Grid: status: {} · tile_size: {}px · grid_frames: {} · dirty_tiles: {} · skipped: {} · saved: {}B",
                if rep.tile_enabled { "enabled" } else { "disabled" },
                rep.tile_size,
                rep.tile_frames_sent,
                rep.dirty_tiles_sent,
                rep.skipped_tiles_count,
                rep.tile_saved_bytes
            );
        }
        if rep.tile_cache_enabled
            || rep.tile_cache_entries > 0
            || rep.tile_cache_hits > 0
            || rep.tile_cache_misses > 0
            || rep.tile_cache_saved_bytes > 0
        {
            let total_lookups = rep.tile_cache_hits + rep.tile_cache_misses;
            let hit_pct = if total_lookups > 0 {
                (rep.tile_cache_hits as f64 / total_lookups as f64) * 100.0
            } else {
                0.0
            };
            println!(
                "  Tile Dictionary:  status: {} · entries: {} · hits: {} · misses: {} · hit_ratio: {:.1}% · saved: {}B",
                if rep.tile_cache_enabled { "enabled" } else { "disabled" },
                rep.tile_cache_entries,
                rep.tile_cache_hits,
                rep.tile_cache_misses,
                hit_pct,
                rep.tile_cache_saved_bytes
            );
        }
        if rep.focus_enabled
            || rep.focus_tiles_sent > 0
            || rep.peripheral_tiles_sent > 0
            || rep.peripheral_tiles_skipped > 0
            || rep.focus_saved_bandwidth_bytes > 0
        {
            println!(
                "  Foveated Focus:   status: {} · mode: {} · radius: {}px · skip_ratio: {} · focus_tiles: {} · peripheral_sent: {} · peripheral_skipped: {} · saved: {}B",
                if rep.focus_enabled { "enabled" } else { "disabled" },
                rep.focus_mode,
                rep.focus_radius,
                rep.focus_peripheral_skip_ratio,
                rep.focus_tiles_sent,
                rep.peripheral_tiles_sent,
                rep.peripheral_tiles_skipped,
                rep.focus_saved_bandwidth_bytes
            );
        }
        if rep.tile_palette_enabled
            || rep.tile_palette_solid_tiles > 0
            || rep.tile_palette_bilevel_tiles > 0
            || rep.tile_palette_palette_tiles > 0
            || rep.tile_palette_truecolor_tiles > 0
            || rep.tile_palette_saved_bytes > 0
        {
            println!(
                "  Tile Palette:     status: {} · max_colors: {} · solid: {} · bilevel: {} · palette: {} · truecolor: {} · saved: {}B",
                if rep.tile_palette_enabled { "enabled" } else { "disabled" },
                rep.tile_palette_max_colors,
                rep.tile_palette_solid_tiles,
                rep.tile_palette_bilevel_tiles,
                rep.tile_palette_palette_tiles,
                rep.tile_palette_truecolor_tiles,
                rep.tile_palette_saved_bytes
            );
        }
        if rep.tile_predict_enabled
            || rep.tile_predict_tiles_sent > 0
            || rep.tile_predict_saved_bytes > 0
        {
            println!(
                "  Tile Predict:     status: {} · filter: {} · rct: {} · predict_tiles: {} · saved: {}B",
                if rep.tile_predict_enabled { "enabled" } else { "disabled" },
                rep.tile_predict_filter_mode,
                rep.tile_predict_enable_rct,
                rep.tile_predict_tiles_sent,
                rep.tile_predict_saved_bytes
            );
        }
        if rep.tile_present_enabled
            || rep.tile_present_committed_transactions > 0
            || rep.tile_present_swapped_frames > 0
        {
            println!(
                "  Tile Present:     status: {} · vsync: {} · double_buf: {} · anti_tear: {} · committed_tx: {} · swapped: {} · torn_prevented: {} · avg_latency: {}µs",
                if rep.tile_present_enabled { "enabled" } else { "disabled" },
                rep.tile_present_vsync_mode,
                rep.tile_present_double_buffer,
                rep.tile_present_tear_prevention,
                rep.tile_present_committed_transactions,
                rep.tile_present_swapped_frames,
                rep.tile_present_torn_frames_prevented,
                rep.tile_present_avg_swap_latency_us
            );
        }
        if rep.tile_nack_enabled
            || rep.tile_nack_requests_sent > 0
            || rep.tile_nack_tiles_retransmitted > 0
            || rep.tile_nack_degraded_commits > 0
        {
            println!(
                "  Tile Micro-NACK:  status: {} · timeout: {}ms · max_retries: {} · nack_sent: {} · retransmitted: {} · degraded: {}",
                if rep.tile_nack_enabled { "enabled" } else { "disabled" },
                rep.tile_nack_timeout_ms,
                rep.tile_nack_max_retries,
                rep.tile_nack_requests_sent,
                rep.tile_nack_tiles_retransmitted,
                rep.tile_nack_degraded_commits
            );
        }
        if rep.tile_saliency_enabled
            || rep.tile_saliency_critical_tiles > 0
            || rep.tile_saliency_normal_tiles > 0
            || rep.tile_saliency_background_tiles > 0
            || rep.tile_saliency_background_skipped > 0
        {
            println!(
                "  Tile Saliency:    status: {} · high_thresh: {} · low_thresh: {} · drop_bg_congest: {} · critical: {} · normal: {} · background: {} · bg_skipped: {}",
                if rep.tile_saliency_enabled { "enabled" } else { "disabled" },
                rep.tile_saliency_high_threshold,
                rep.tile_saliency_low_threshold,
                rep.tile_saliency_drop_background_on_congestion,
                rep.tile_saliency_critical_tiles,
                rep.tile_saliency_normal_tiles,
                rep.tile_saliency_background_tiles,
                rep.tile_saliency_background_skipped
            );
        }
        if rep.tile_lod_enabled
            || rep.tile_lod0_tiles > 0
            || rep.tile_lod1_tiles > 0
            || rep.tile_lod2_tiles > 0
            || rep.tile_lod_bytes_saved > 0
        {
            println!(
                "  Tile LoD Downsample: status: {} · max_lod: Lod{} · auto_lod_congest: {} · lod0: {} · lod1: {} · lod2: {} · bytes_saved: {} B",
                if rep.tile_lod_enabled { "enabled" } else { "disabled" },
                rep.tile_lod_max_lod,
                rep.tile_lod_auto_lod_on_congestion,
                rep.tile_lod0_tiles,
                rep.tile_lod1_tiles,
                rep.tile_lod2_tiles,
                rep.tile_lod_bytes_saved
            );
        }
        if rep.tile_batch_enabled
            || rep.tile_batches_sent > 0
            || rep.tile_chunks_sent > 0
            || rep.tile_batch_pacing_pauses > 0
        {
            println!(
                "  Tile Batching:    status: {} · target_chunk: {} B · pacing: {} µs · batches: {} · chunks: {} · avg_chunk: {} B · pauses: {}",
                if rep.tile_batch_enabled { "enabled" } else { "disabled" },
                rep.tile_batch_target_chunk_bytes,
                rep.tile_batch_pacing_interval_us,
                rep.tile_batches_sent,
                rep.tile_chunks_sent,
                rep.tile_batch_avg_chunk_bytes,
                rep.tile_batch_pacing_pauses
            );
        }
        if rep.tile_motion_enabled
            || rep.tile_motion_tiles_sent > 0
            || rep.tile_motion_fallback_tiles_sent > 0
            || rep.tile_motion_bytes_saved > 0
        {
            println!(
                "  Tile Motion Est:  status: {} · max_disp: {} px · min_match: {}% · motion_tiles: {} · fallback_tiles: {} · bytes_saved: {} B",
                if rep.tile_motion_enabled { "enabled" } else { "disabled" },
                rep.tile_motion_max_displacement,
                rep.tile_motion_min_match_ratio,
                rep.tile_motion_tiles_sent,
                rep.tile_motion_fallback_tiles_sent,
                rep.tile_motion_bytes_saved
            );
        }
    }
    Ok(())
}

/// `tetherly screen displays` -- Displays arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_displays(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/displays")
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

/// `tetherly screen switch` -- Switch arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_switch(display_id: u8, json: bool, ui_port: u16) -> Result<()> {
    let req = serde_json::json!({ "display_id": display_id });
    let body = serde_json::to_vec(&req)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/switch", &body)
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

/// `tetherly screen snapshot` -- Snapshot arm of the screen dispatcher.
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
    let (code, resp) = tetherly_node::loopback_get(ui_port, &uri)
        .await
        .map_err(|e| {
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

/// `tetherly screen laser` -- Laser arm of the screen dispatcher.
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
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/laser")
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
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/laser", &bytes)
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
