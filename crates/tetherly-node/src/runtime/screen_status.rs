// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen status sampling, profiling and congestion state.

use super::*;
use std::sync::atomic::Ordering;
use tetherly_core::ScreenState;

impl Node {
    /// Comprehensive telemetry and lifecycle status report for remote screen streaming.
    pub async fn screen_status_report(&self) -> ScreenStatusReport {
        let stats = self.screen_stats();
        let rx_stats = self.screen_rx_stats();
        let p95 = self.screen_latency_p95_us();
        let channel_open =
            self.screen_channel_open().await || self.shared.screen_out.lock().await.is_some();
        let active_peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let is_congested = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_congested();
        let playout_mode = *self
            .shared
            .screen_playout_mode
            .lock()
            .expect("playout mode");
        let playout_stats = *self
            .shared
            .screen_playout_stats
            .lock()
            .expect("playout stats");
        let cursor_mode = *self.shared.screen_cursor_mode.lock().expect("cursor mode");
        let cursor_stats = *self
            .shared
            .screen_cursor_stats
            .lock()
            .expect("cursor stats");
        let fec_mode = *self.shared.screen_fec_mode.lock().expect("fec mode");
        let harq_mode = *self.shared.screen_harq_mode.lock().expect("harq mode");
        let harq_report = self.screen_harq_report();
        let twcc_mode = *self.shared.screen_twcc_mode.lock().expect("twcc mode");
        let twcc_report = self.screen_twcc_report();
        let probe_mode = *self.shared.screen_probe_mode.lock().expect("probe mode");
        let probe_report = self.screen_probe_report();
        let sync_report = self.screen_sync_report();
        let latency_report = self.screen_latency_report();
        let tuning_report = self.screen_tuning_report();
        ScreenStatusReport {
            ok: true,
            state: state.as_str().to_string(),
            channel_open,
            allowed: matches!(state, ScreenState::Allowed | ScreenState::Streaming),
            sent: stats.sent,
            received: rx_stats.received,
            dropped: stats.dropped,
            refused: stats.refused,
            bytes: stats.bytes,
            rx_bytes: rx_stats.bytes,
            p95_frame_interval_us: p95,
            active_peer,
            acked: stats.acked,
            in_flight: stats.in_flight,
            rtt_latest_us: stats.rtt_latest_us,
            rtt_p95_us: stats.rtt_p95_us,
            is_congested,
            keyframes_requested: stats.keyframes_requested,
            gaps: rx_stats.gaps,
            bitrate_bps: stats.bitrate_bps,
            fps_target: stats.fps_target,
            scale_pct: stats.scale_pct,
            profile: stats.content_profile().as_str().to_string(),
            qp_min: stats.qp_min,
            qp_max: stats.qp_max,
            matrix_mode: stats.matrix_mode().as_str().to_string(),
            network_health: stats.network_health().as_str().to_string(),
            playout_mode: playout_mode.as_str().to_string(),
            playout_delay_ms: (playout_stats.target_delay_us as f64 / 1000.0 * 10.0).round() / 10.0,
            playout_jitter_ms: (playout_stats.current_jitter_us as f64 / 1000.0 * 100.0).round()
                / 100.0,
            playout_buffered: playout_stats.buffered_frames,
            playout_dropped_late: playout_stats.dropped_late_frames,
            cursor_mode: cursor_mode.as_str().to_string(),
            cursor_x: cursor_stats.current_x,
            cursor_y: cursor_stats.current_y,
            cursor_visible: cursor_stats.current_visible,
            cursor_shape: cursor_stats.current_shape.as_str().to_string(),
            cursor_updates_sent: cursor_stats.updates_sent,
            cursor_updates_received: cursor_stats.updates_received,
            squelch_mode: stats.squelch_mode().as_str().to_string(),
            is_squelched: stats.is_squelched,
            squelched_frames: stats.squelched_frames,
            fec_mode: fec_mode.as_str().to_string(),
            fec_redundancy_pct: stats.fec_redundancy_pct,
            fec_recovered_shards: rx_stats.fec_recovered_shards,
            fec_unrecoverable_blocks: rx_stats.fec_unrecoverable_blocks,
            pacer_mode: stats.pacer_mode().as_str().to_string(),
            pacer_packets_paced: stats.pacer_packets_paced,
            pacer_bursts_smoothed: stats.pacer_bursts_smoothed,
            pacer_avg_delay_us: stats.pacer_avg_delay_us,
            trendline_mode: stats.trendline_mode().to_string(),
            trendline_state: stats.trendline_state().to_string(),
            trendline_slope_us: stats.trendline_slope_us,
            trendline_overuse_count: stats.trendline_overuse_count,
            harq_mode: harq_mode.as_str().to_string(),
            harq_nacks_sent: rx_stats.harq_nacks_sent,
            harq_nacks_received: stats.harq_nacks_received,
            harq_shards_retransmitted: stats.harq_shards_retransmitted,
            harq_recovered_frames: rx_stats.harq_recovered_frames,
            harq_retransmit_timeouts: harq_report.retransmit_timeouts,
            harq_buffer_occupancy: harq_report.buffer_occupancy,
            twcc_mode: twcc_mode.as_str().to_string(),
            twcc_feedbacks_sent: rx_stats.twcc_feedbacks_sent,
            twcc_feedbacks_received: stats.twcc_feedbacks_received,
            twcc_delivery_rate_bps: twcc_report.delivery_rate_bps,
            twcc_delay_gradient_us: twcc_report.delay_gradient_us,
            twcc_loss_ratio: twcc_report.loss_ratio,
            probe_mode: probe_mode.as_str().to_string(),
            probe_state: probe_report.state.tag().to_string(),
            probes_triggered: probe_report.probes_triggered,
            probes_completed: probe_report.probes_completed,
            probes_failed: probe_report.probes_failed,
            probe_target_bps: probe_report.last_target_bps,
            probe_measured_bps: probe_report.last_measured_bps,
            sync_mode: sync_report.mode.as_str().to_string(),
            sync_beacons_sent: sync_report.beacons_sent,
            sync_beacons_received: sync_report.beacons_received,
            sync_skew_us: sync_report.current_skew_us,
            sync_clock_drift_ppm: sync_report.clock_drift_ppm,
            sync_adjustments_applied: sync_report.adjustments_applied,
            in_sync: sync_report.in_sync,
            active_display_id: self.shared.screen_active_display_id.load(Ordering::Relaxed),
            display_count: self.screen_displays().len(),
            display_switches_count: self
                .shared
                .screen_display_switches_count
                .load(Ordering::Relaxed),
            privacy_enabled: self.shared.screen_privacy_enabled.load(Ordering::Relaxed),
            privacy_input_locked: self
                .shared
                .screen_privacy_lock_input
                .load(Ordering::Relaxed),
            privacy_blanking_method: self
                .shared
                .screen_privacy_blanking_method
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            latency_mode: latency_report.mode,
            latency_bottleneck: latency_report.bottleneck,
            latency_total_e2e_us: latency_report.current.total_e2e_us,
            latency_p95_e2e_us: latency_report.p95_e2e_us,
            latency_frames_profiled: latency_report.frames_profiled,
            tuning_mode: tuning_report.mode,
            tuning_actions_total: tuning_report.actions_applied,
            tuning_recoveries: tuning_report.recoveries_applied,
            tuning_last_stage: tuning_report.last_stage,
            tuning_last_action: tuning_report.last_action,
            recording_active: self.screen_recording_report().active,
            peer_recording: self
                .shared
                .screen_peer_recording
                .load(std::sync::atomic::Ordering::Relaxed),
            recording_frames: self.screen_recording_report().frames_recorded,
            playback_active: matches!(
                self.screen_playback_report().state.as_str(),
                "playing" | "paused"
            ),
            playback_state: self.screen_playback_report().state,
            playback_frame: self.screen_playback_report().current_frame,
            snapshot_count: self
                .shared
                .screen_snapshot_count
                .load(std::sync::atomic::Ordering::Relaxed),
            watermark_enabled: self.shared.screen_watermark.read().expect("lock").enabled,
            watermark_enforced: self
                .shared
                .screen_watermark
                .read()
                .expect("lock")
                .host_enforced
                || self
                    .shared
                    .screen_peer_watermark_enforced
                    .load(std::sync::atomic::Ordering::Relaxed),
            watermark_frames: self
                .shared
                .screen_watermark_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            annotations_count: self.shared.screen_annotations.read().expect("lock").len(),
            laser_active: self
                .shared
                .screen_laser_pointer
                .read()
                .unwrap()
                .as_ref()
                .map(|l| l.active)
                .unwrap_or(false),
            annotations_total: self
                .shared
                .screen_annotation_count
                .load(std::sync::atomic::Ordering::Relaxed),
            laser_updates_total: self
                .shared
                .screen_laser_updates
                .load(std::sync::atomic::Ordering::Relaxed),
            whiteboard_undo_count: self
                .shared
                .screen_whiteboard_undo_count
                .load(std::sync::atomic::Ordering::Relaxed),
            whiteboard_redo_count: self
                .shared
                .screen_whiteboard_redo_count
                .load(std::sync::atomic::Ordering::Relaxed),
            whiteboard_read_only: self
                .shared
                .screen_whiteboard
                .read()
                .unwrap()
                .policy()
                .read_only,
            whiteboard_max_items: self
                .shared
                .screen_whiteboard
                .read()
                .unwrap()
                .policy()
                .max_items,
            roi_active: self.shared.screen_roi.read().expect("lock").active,
            roi_x: self.shared.screen_roi.read().expect("lock").x,
            roi_y: self.shared.screen_roi.read().expect("lock").y,
            roi_w: self.shared.screen_roi.read().expect("lock").w,
            roi_h: self.shared.screen_roi.read().expect("lock").h,
            roi_zoom_percent: self.shared.screen_roi.read().expect("lock").zoom_percent,
            roi_updates: self
                .shared
                .screen_roi_updates
                .load(std::sync::atomic::Ordering::Relaxed),
            color_mode: self
                .shared
                .screen_color_mode
                .read()
                .unwrap()
                .mode
                .as_str()
                .to_string(),
            color_threshold: self
                .shared
                .screen_color_mode
                .read()
                .expect("lock")
                .threshold,
            color_updates: self
                .shared
                .screen_color_updates
                .load(std::sync::atomic::Ordering::Relaxed),
            delta_enabled: self
                .shared
                .screen_delta_config
                .read()
                .expect("lock")
                .enabled,
            delta_keyframe_interval: self
                .shared
                .screen_delta_config
                .read()
                .unwrap()
                .keyframe_interval,
            delta_frames_sent: self
                .shared
                .screen_delta_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            delta_keyframes_sent: self
                .shared
                .screen_delta_keyframes_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            delta_frames_received: self
                .shared
                .screen_delta_frames_received
                .load(std::sync::atomic::Ordering::Relaxed),
            delta_keyframes_received: self
                .shared
                .screen_delta_keyframes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            delta_saved_bytes: self
                .shared
                .screen_delta_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            metric_enabled: self
                .shared
                .screen_quality_metric_config
                .read()
                .unwrap()
                .enabled,
            metric_sample_interval: self
                .shared
                .screen_quality_metric_config
                .read()
                .unwrap()
                .sample_interval,
            metric_evaluated_frames: self
                .shared
                .screen_quality_metric_state
                .read()
                .unwrap()
                .evaluated_frames,
            metric_psnr_db: self
                .shared
                .screen_quality_metric_state
                .read()
                .unwrap()
                .last_psnr_db,
            metric_ssim: self
                .shared
                .screen_quality_metric_state
                .read()
                .unwrap()
                .last_ssim,
            metric_quality_warning: {
                let cfg = *self
                    .shared
                    .screen_quality_metric_config
                    .read()
                    .expect("lock");
                let state = self
                    .shared
                    .screen_quality_metric_state
                    .read()
                    .expect("lock");
                cfg.enabled
                    && state.evaluated_frames > 0
                    && (state.last_psnr_db < 30.0 || state.last_ssim < 0.85)
            },
            qoe_enabled: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .config()
                .is_active(),
            qoe_mode: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .config()
                .mode
                .as_str()
                .to_string(),
            qoe_target_psnr_db: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .config()
                .target_psnr_db,
            qoe_target_ssim_pct: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .config()
                .target_ssim_pct,
            qoe_actions_total: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .actions_total(),
            qoe_last_action: self
                .shared
                .screen_qoe_controller
                .read()
                .unwrap()
                .last_action()
                .as_str()
                .to_string(),
            pir_enabled: self
                .shared
                .screen_pir_config
                .read()
                .unwrap()
                .mode
                .is_active(),
            pir_mode: self
                .shared
                .screen_pir_config
                .read()
                .unwrap()
                .mode
                .as_str()
                .to_string(),
            pir_bands: self.shared.screen_pir_config.read().expect("lock").bands,
            pir_sweeps_completed: self
                .shared
                .screen_pir_sweeps_completed
                .load(std::sync::atomic::Ordering::Relaxed),
            pir_slices_sent: self
                .shared
                .screen_pir_slices_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            ltr_enabled: self.shared.screen_ltr_config.read().expect("lock").enabled,
            ltr_auto_interval: self
                .shared
                .screen_ltr_config
                .read()
                .expect("lock")
                .auto_interval,
            ltr_current_seq: self
                .shared
                .screen_ltr_current_seq
                .load(std::sync::atomic::Ordering::Relaxed),
            ltr_anchors_established: self
                .shared
                .screen_ltr_anchors_established
                .load(std::sync::atomic::Ordering::Relaxed),
            ltr_recoveries_completed: self
                .shared
                .screen_ltr_recoveries_completed
                .load(std::sync::atomic::Ordering::Relaxed),
            svc_enabled: self
                .shared
                .screen_svc_config
                .read()
                .expect("lock")
                .is_enabled(),
            svc_mode: self
                .shared
                .screen_svc_config
                .read()
                .unwrap()
                .mode
                .as_str()
                .to_string(),
            svc_drop_t1: self.shared.screen_svc_config.read().expect("lock").drop_t1,
            svc_t0_sent: self
                .shared
                .screen_svc_t0_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            svc_t1_sent: self
                .shared
                .screen_svc_t1_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            svc_t1_dropped: self
                .shared
                .screen_svc_t1_frames_dropped
                .load(std::sync::atomic::Ordering::Relaxed),
            motion_enabled: self
                .shared
                .screen_motion_config
                .read()
                .expect("lock")
                .enabled,
            motion_max_displacement: self
                .shared
                .screen_motion_config
                .read()
                .unwrap()
                .max_displacement,
            motion_frames_sent: self
                .shared
                .screen_motion_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            motion_fallback_frames_sent: self
                .shared
                .screen_motion_fallback_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            motion_saved_bytes: self
                .shared
                .screen_motion_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_enabled: self.shared.screen_tile_config.read().expect("lock").enabled,
            tile_size: self
                .shared
                .screen_tile_config
                .read()
                .expect("lock")
                .tile_size,
            tile_frames_sent: self
                .shared
                .screen_tile_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            dirty_tiles_sent: self
                .shared
                .screen_tile_dirty_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            skipped_tiles_count: self
                .shared
                .screen_tile_skipped_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_saved_bytes: self
                .shared
                .screen_tile_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_cache_enabled: self
                .shared
                .screen_tile_cache_config
                .read()
                .expect("lock")
                .enabled,
            tile_cache_max_entries: self
                .shared
                .screen_tile_cache_config
                .read()
                .unwrap()
                .max_entries,
            tile_cache_entries: self
                .shared
                .screen_tile_cache_dict
                .lock()
                .unwrap()
                .current_entries(),
            tile_cache_hits: self
                .shared
                .screen_tile_cache_hits
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_cache_misses: self
                .shared
                .screen_tile_cache_misses
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_cache_hit_ratio_pct: {
                let hits = self
                    .shared
                    .screen_tile_cache_hits
                    .load(std::sync::atomic::Ordering::Relaxed);
                let misses = self
                    .shared
                    .screen_tile_cache_misses
                    .load(std::sync::atomic::Ordering::Relaxed);
                let total = hits + misses;
                if total > 0 {
                    (hits as f32 / total as f32) * 100.0
                } else {
                    0.0
                }
            },
            tile_cache_saved_bytes: self
                .shared
                .screen_tile_cache_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            focus_enabled: self
                .shared
                .screen_focus_config
                .read()
                .expect("lock")
                .enabled,
            focus_mode: self
                .shared
                .screen_focus_config
                .read()
                .unwrap()
                .mode
                .as_str()
                .to_string(),
            focus_radius: self.shared.screen_focus_config.read().expect("lock").radius,
            focus_peripheral_skip_ratio: self
                .shared
                .screen_focus_config
                .read()
                .unwrap()
                .peripheral_skip_ratio,
            focus_box_x: self.shared.screen_focus_config.read().expect("lock").box_x,
            focus_box_y: self.shared.screen_focus_config.read().expect("lock").box_y,
            focus_box_w: self.shared.screen_focus_config.read().expect("lock").box_w,
            focus_box_h: self.shared.screen_focus_config.read().expect("lock").box_h,
            focus_tiles_sent: self
                .shared
                .screen_focus_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            peripheral_tiles_sent: self
                .shared
                .screen_focus_peripheral_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            peripheral_tiles_skipped: self
                .shared
                .screen_focus_peripheral_skipped
                .load(std::sync::atomic::Ordering::Relaxed),
            focus_saved_bandwidth_bytes: self
                .shared
                .screen_focus_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_palette_enabled: self
                .shared
                .screen_tile_palette_config
                .read()
                .unwrap()
                .enabled,
            tile_palette_max_colors: self
                .shared
                .screen_tile_palette_config
                .read()
                .unwrap()
                .max_colors,
            tile_palette_enable_rle: self
                .shared
                .screen_tile_palette_config
                .read()
                .unwrap()
                .enable_rle,
            tile_palette_solid_tiles: self
                .shared
                .screen_tile_palette_solid_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_palette_bilevel_tiles: self
                .shared
                .screen_tile_palette_bilevel_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_palette_palette_tiles: self
                .shared
                .screen_tile_palette_palette_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_palette_truecolor_tiles: self
                .shared
                .screen_tile_palette_truecolor_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_palette_saved_bytes: self
                .shared
                .screen_tile_palette_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_predict_enabled: self
                .shared
                .screen_tile_predict_config
                .read()
                .unwrap()
                .enabled,
            tile_predict_filter_mode: self
                .shared
                .screen_tile_predict_config
                .read()
                .unwrap()
                .filter_mode
                .as_str()
                .to_string(),
            tile_predict_enable_rct: self
                .shared
                .screen_tile_predict_config
                .read()
                .unwrap()
                .enable_rct,
            tile_predict_tiles_sent: self
                .shared
                .screen_tile_predict_tiles_count
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_predict_saved_bytes: self
                .shared
                .screen_tile_predict_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_present_enabled: self
                .shared
                .screen_tile_present_config
                .read()
                .unwrap()
                .enabled,
            tile_present_vsync_mode: self
                .shared
                .screen_tile_present_config
                .read()
                .unwrap()
                .vsync_mode
                .as_str()
                .to_string(),
            tile_present_double_buffer: self
                .shared
                .screen_tile_present_config
                .read()
                .unwrap()
                .double_buffer,
            tile_present_tear_prevention: self
                .shared
                .screen_tile_present_config
                .read()
                .unwrap()
                .tear_prevention,
            tile_present_committed_transactions: self
                .shared
                .screen_tile_present_committed_txs
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_present_swapped_frames: self
                .shared
                .screen_tile_present_swapped_frames
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_present_torn_frames_prevented: self
                .shared
                .screen_tile_present_torn_prevented
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_present_avg_swap_latency_us: {
                let swaps = self
                    .shared
                    .screen_tile_present_swapped_frames
                    .load(std::sync::atomic::Ordering::Relaxed);
                let total_lat = self
                    .shared
                    .screen_tile_present_total_latency_us
                    .load(std::sync::atomic::Ordering::Relaxed);
                total_lat.checked_div(swaps).unwrap_or(0)
            },
            tile_nack_enabled: self
                .shared
                .screen_tile_nack_config
                .read()
                .expect("lock")
                .enabled,
            tile_nack_timeout_ms: self
                .shared
                .screen_tile_nack_config
                .read()
                .unwrap()
                .timeout_ms,
            tile_nack_max_retries: self
                .shared
                .screen_tile_nack_config
                .read()
                .unwrap()
                .max_retries,
            tile_nack_requests_sent: self
                .shared
                .screen_tile_nack_requests_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_nack_tiles_retransmitted: self
                .shared
                .screen_tile_nack_tiles_retransmitted
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_nack_degraded_commits: self
                .shared
                .screen_tile_nack_degraded_commits
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_saliency_enabled: self
                .shared
                .screen_tile_saliency_config
                .read()
                .unwrap()
                .enabled,
            tile_saliency_high_threshold: self
                .shared
                .screen_tile_saliency_config
                .read()
                .unwrap()
                .high_threshold,
            tile_saliency_low_threshold: self
                .shared
                .screen_tile_saliency_config
                .read()
                .unwrap()
                .low_threshold,
            tile_saliency_drop_background_on_congestion: self
                .shared
                .screen_tile_saliency_config
                .read()
                .unwrap()
                .drop_background_on_congestion,
            tile_saliency_critical_tiles: self
                .shared
                .screen_tile_saliency_critical_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_saliency_normal_tiles: self
                .shared
                .screen_tile_saliency_normal_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_saliency_background_tiles: self
                .shared
                .screen_tile_saliency_background_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_saliency_background_skipped: self
                .shared
                .screen_tile_saliency_background_skipped
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_lod_enabled: self
                .shared
                .screen_tile_lod_config
                .read()
                .expect("lock")
                .enabled,
            tile_lod_max_lod: self
                .shared
                .screen_tile_lod_config
                .read()
                .expect("lock")
                .max_lod,
            tile_lod_auto_lod_on_congestion: self
                .shared
                .screen_tile_lod_config
                .read()
                .unwrap()
                .auto_lod_on_congestion,
            tile_lod0_tiles: self
                .shared
                .screen_tile_lod0_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_lod1_tiles: self
                .shared
                .screen_tile_lod1_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_lod2_tiles: self
                .shared
                .screen_tile_lod2_tiles
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_lod_bytes_saved: self
                .shared
                .screen_tile_lod_bytes_saved
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_batch_enabled: self
                .shared
                .screen_tile_batch_config
                .read()
                .expect("lock")
                .enabled,
            tile_batch_target_chunk_bytes: self
                .shared
                .screen_tile_batch_config
                .read()
                .unwrap()
                .target_chunk_bytes,
            tile_batch_pacing_interval_us: self
                .shared
                .screen_tile_batch_config
                .read()
                .unwrap()
                .pacing_interval_us,
            tile_batches_sent: self
                .shared
                .screen_tile_batches_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_chunks_sent: self
                .shared
                .screen_tile_chunks_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_batch_avg_chunk_bytes: {
                let chunks = self
                    .shared
                    .screen_tile_chunks_sent
                    .load(std::sync::atomic::Ordering::Relaxed);
                let bytes = self
                    .shared
                    .screen_tile_batch_bytes_total
                    .load(std::sync::atomic::Ordering::Relaxed);
                bytes.checked_div(chunks).unwrap_or(0) as u32
            },
            tile_batch_pacing_pauses: self
                .shared
                .screen_tile_batch_pacing_pauses
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_motion_enabled: self
                .shared
                .screen_tile_motion_config
                .read()
                .unwrap()
                .enabled,
            tile_motion_max_displacement: self
                .shared
                .screen_tile_motion_config
                .read()
                .unwrap()
                .max_displacement,
            tile_motion_min_match_ratio: self
                .shared
                .screen_tile_motion_config
                .read()
                .unwrap()
                .min_match_ratio,
            tile_motion_tiles_sent: self
                .shared
                .screen_tile_motion_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_motion_fallback_tiles_sent: self
                .shared
                .screen_tile_motion_fallback_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            tile_motion_bytes_saved: self
                .shared
                .screen_tile_motion_bytes_saved
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}
