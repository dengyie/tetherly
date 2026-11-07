// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Controlled-side sender: capture ingest, compression and frame pacing.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use crate::frame::INNER_PAYLOAD_MAX;
use std::collections::VecDeque;

/// Sender side. Owns the monotonic sequence so a reconnect cannot replay old
/// frames, and so the controller can detect a gap.
pub struct ScreenSender {
    pub(super) next_seq: u64,
    pub(super) state: ScreenState,
    pub(super) stats: ScreenStats,
    pub(super) pending: VecDeque<ScreenFrame>,
    /// Track (seq, sent_time_us) of frames in-flight (max SCREEN_IN_FLIGHT_MAX).
    pub(super) in_flight_records: VecDeque<(u64, u64)>,
    pub(super) rtt_tracker: ScreenRttTracker,
    pub(super) rate_controller: ScreenRateController,
    pub(super) fps_pacer: ScreenFpsPacer,
    pub(super) scale_controller: ScreenScaleController,
    pub(super) quality_controller: ScreenQualityController,
    pub(super) matrix_coordinator: ScreenMatrixCoordinator,
    pub(super) cursor_controller: ScreenCursorController,
    pub(super) squelch_controller: ScreenSquelchController,
    pub(super) fec_controller: ScreenFecController,
    pub(super) packet_pacer: ScreenPacketPacer,
    pub(super) trendline_filter: ScreenTrendlineFilter,
    pub(super) retransmit_buffer: ScreenRetransmitBuffer,
    pub(super) twcc_sender: ScreenTwccSenderTracker,
    pub(super) probe_controller: ScreenProbeController,
    pub(super) sync_coordinator: ScreenAudioSyncCoordinator,
    pub(super) active_display_id: u8,
    pub(super) displays: Vec<ScreenDisplayInfo>,
    pub(super) display_switches_count: u64,
    pub(super) last_display_switch_us: u64,
    pub(super) privacy_enabled: bool,
    pub(super) privacy_lock_input: bool,
    pub(super) privacy_blanking_method: String,
    pub(super) privacy_last_change_us: u64,
    pub(super) frame_profiler: ScreenFrameProfiler,
    pub(super) tuner: ScreenTuner,
    pub(super) congested_override: Option<bool>,
}

impl Default for ScreenSender {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenSender {
    pub fn new() -> Self {
        let rate_controller = ScreenRateController::default();
        let fps_pacer = ScreenFpsPacer::default();
        let scale_controller = ScreenScaleController::default();
        let quality_controller = ScreenQualityController::default();
        let matrix_coordinator = ScreenMatrixCoordinator::default();
        let cursor_controller = ScreenCursorController::default();
        let squelch_controller = ScreenSquelchController::default();
        let fec_controller = ScreenFecController::default();
        let packet_pacer = ScreenPacketPacer::default();
        let trendline_filter = ScreenTrendlineFilter::default();
        let retransmit_buffer = ScreenRetransmitBuffer::new(ScreenHarqMode::Auto);
        let twcc_sender = ScreenTwccSenderTracker::new(ScreenTwccMode::Auto);
        let probe_controller = ScreenProbeController::default();
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let active_display_id = 0;
        let display_switches_count = 0;
        let last_display_switch_us = 0;
        let privacy_enabled = false;
        let privacy_lock_input = false;
        let privacy_blanking_method = "virtual_overlay".to_string();
        let privacy_last_change_us = 0;
        let frame_profiler = ScreenFrameProfiler::new(ScreenProfilerMode::Auto);
        let tuner = ScreenTuner::default();
        let stats = ScreenStats {
            bitrate_bps: rate_controller.current_bps(),
            fps_target: fps_pacer.fps_target(),
            scale_pct: scale_controller.scale_pct(),
            profile_tag: quality_controller.profile().tag(),
            qp_min: quality_controller.min_qp(),
            qp_max: quality_controller.max_qp(),
            matrix_mode: matrix_coordinator.mode().tag(),
            network_health: matrix_coordinator.health().tag(),
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            squelch_mode: squelch_controller.mode().tag(),
            is_squelched: squelch_controller.is_squelched(),
            squelched_frames: 0,
            fec_mode: fec_controller.mode().tag(),
            fec_redundancy_pct: fec_controller.stats().redundancy_pct,
            fec_recovered_shards: 0,
            fec_unrecoverable_blocks: 0,
            pacer_mode: packet_pacer.mode().tag(),
            pacer_packets_paced: 0,
            pacer_bursts_smoothed: 0,
            pacer_avg_delay_us: 0,
            trendline_mode: trendline_filter.mode().tag(),
            trendline_state: ScreenBandwidthUsageState::Normal.tag(),
            trendline_slope_us: 0.0,
            trendline_overuse_count: 0,
            harq_mode: retransmit_buffer.mode().tag(),
            harq_nacks_sent: 0,
            harq_nacks_received: 0,
            harq_shards_retransmitted: 0,
            harq_recovered_frames: 0,
            twcc_mode: twcc_sender.mode().tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_controller.mode().tag(),
            probe_state: probe_controller.state().tag(),
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: sync_coordinator.mode().tag(),
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            privacy_enabled,
            privacy_input_locked: privacy_lock_input,
            ..Default::default()
        };
        Self {
            next_seq: 1,
            state: ScreenState::Idle,
            stats,
            pending: VecDeque::new(),
            in_flight_records: VecDeque::new(),
            rtt_tracker: ScreenRttTracker::new(SCREEN_IN_FLIGHT_MAX),
            rate_controller,
            fps_pacer,
            scale_controller,
            quality_controller,
            matrix_coordinator,
            cursor_controller,
            squelch_controller,
            fec_controller,
            packet_pacer,
            trendline_filter,
            retransmit_buffer,
            twcc_sender,
            probe_controller,
            sync_coordinator,
            active_display_id,
            displays,
            display_switches_count,
            last_display_switch_us,
            privacy_enabled,
            privacy_lock_input,
            privacy_blanking_method,
            privacy_last_change_us,
            frame_profiler,
            tuner,
            congested_override: None,
        }
    }

    pub fn with_profiler(mode: ScreenProfilerMode) -> Self {
        let mut sender = Self::new();
        sender.set_profiler_mode(mode);
        sender
    }

    pub fn with_display(display_id: u8) -> Self {
        let mut sender = Self::new();
        let _ = sender.switch_display(display_id, 0);
        sender
    }

    pub fn with_sync(sync: ScreenAudioSyncMode) -> Self {
        let mut sender = Self::new();
        sender.set_sync_mode(sync);
        sender
    }

    pub fn with_probe(probe: ScreenProbeMode) -> Self {
        let mut sender = Self::new();
        sender.set_probe_mode(probe);
        sender
    }

    pub fn with_twcc(twcc: ScreenTwccMode) -> Self {
        let mut sender = Self::new();
        sender.set_twcc_mode(twcc);
        sender
    }

    pub fn with_harq(harq: ScreenHarqMode) -> Self {
        let mut sender = Self::new();
        sender.set_harq_mode(harq);
        sender
    }

    pub fn with_trendline(trendline: ScreenTrendlineMode) -> Self {
        let mut sender = Self::new();
        sender.set_trendline_mode(trendline);
        sender
    }

    pub fn with_pacer(pacer: ScreenPacerMode) -> Self {
        let mut sender = Self::new();
        sender.set_pacer_mode(pacer);
        sender
    }

    pub fn with_fec(fec: ScreenFecMode) -> Self {
        let mut sender = Self::new();
        sender.set_fec_mode(fec);
        sender
    }

    pub fn with_fps(fps: u32) -> Self {
        let mut sender = Self::new();
        sender.set_fps(fps);
        sender
    }

    pub fn with_scale(scale_pct: u32) -> Self {
        let mut sender = Self::new();
        sender.set_scale_pct(scale_pct);
        sender
    }

    pub fn with_profile(profile: ScreenContentProfile) -> Self {
        let mut sender = Self::new();
        sender.set_content_profile(profile);
        sender
    }

    pub fn state(&self) -> ScreenState {
        self.state
    }

    pub fn stats(&self) -> ScreenStats {
        let mut s = self.stats;
        s.fec_mode = self.fec_controller.mode().tag();
        s.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;
        s.trendline_mode = self.trendline_filter.mode().tag();
        s.trendline_state = self.trendline_filter.state().tag();
        s.trendline_slope_us = self.trendline_filter.stats().current_slope_us_per_frame;
        s.trendline_overuse_count = self.trendline_filter.stats().overuse_count;
        let h_stats = self.retransmit_buffer.stats();
        s.harq_mode = self.retransmit_buffer.mode().tag();
        s.harq_nacks_sent = h_stats.nacks_sent;
        s.harq_nacks_received = h_stats.nacks_received;
        s.harq_shards_retransmitted = h_stats.shards_retransmitted;
        s.harq_recovered_frames = h_stats.recovered_frames;
        s.twcc_mode = self.twcc_sender.mode().tag();
        let twcc_st = self.twcc_sender.stats();
        s.twcc_feedbacks_sent = twcc_st.feedbacks_sent;
        s.twcc_feedbacks_received = twcc_st.feedbacks_received;
        s.twcc_delivery_rate_bps = twcc_st.delivery_rate_bps;
        s.twcc_delay_gradient_us = twcc_st.delay_gradient_us;
        let p_stats = self.probe_controller.stats();
        s.probe_mode = p_stats.mode.tag();
        s.probe_state = p_stats.state.tag();
        s.probes_triggered = p_stats.probes_triggered;
        s.probes_completed = p_stats.probes_completed;
        s.probes_failed = p_stats.probes_failed;
        s.probe_target_bps = p_stats.last_target_bps;
        s.probe_measured_bps = p_stats.last_measured_bps;
        let sync_st = self.sync_coordinator.stats();
        s.sync_mode = sync_st.mode.tag();
        s.sync_beacons_sent = sync_st.beacons_sent;
        s.sync_beacons_received = sync_st.beacons_received;
        s.sync_skew_us = sync_st.current_skew_us;
        s.sync_clock_drift_ppm = sync_st.clock_drift_ppm;
        s.sync_adjustments_applied = sync_st.adjustments_applied;
        s.in_sync = sync_st.in_sync;
        let lstats = self.frame_profiler.report();
        s.latency_mode = self.frame_profiler.mode().tag();
        s.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        s.latency_total_e2e_us = lstats.avg_e2e_us;
        s.latency_p95_e2e_us = lstats.p95_e2e_us;
        s.latency_frames_profiled = lstats.frames_profiled;
        s
    }

    /// Microsecond-precision multi-stage latency profiler mode (v2.13 / ADR-099).
    pub fn profiler_mode(&self) -> ScreenProfilerMode {
        self.frame_profiler.mode()
    }

    /// Alias for profiler_mode.
    pub fn latency_mode(&self) -> ScreenProfilerMode {
        self.profiler_mode()
    }

    /// Set microsecond-precision multi-stage latency profiler mode.
    pub fn set_profiler_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.frame_profiler.set_mode(mode);
        self.sync_profiler_stats();
        mode
    }

    /// Alias for set_profiler_mode.
    pub fn set_latency_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.set_profiler_mode(mode)
    }

    /// Microsecond-precision multi-stage latency report.
    pub fn latency_report(&self) -> ScreenLatencyReport {
        self.frame_profiler.report()
    }

    /// Reset latency profiler statistics and percentiles ring buffer.
    pub fn reset_latency(&mut self) -> ScreenLatencyReport {
        let rep = self.frame_profiler.reset();
        self.sync_profiler_stats();
        rep
    }

    /// Ingest a measured frame latency breakdown.
    pub fn record_latency_breakdown(&mut self, breakdown: ScreenLatencyBreakdown) {
        self.frame_profiler.record_frame(breakdown);
        self.sync_profiler_stats();
    }

    /// Ingest an inbound profile frame (TYPE_SCREEN_PROFILE).
    pub fn on_profile_frame(&mut self, frame: &ScreenProfileFrame) {
        self.frame_profiler.record_profile_frame(frame);
        self.sync_profiler_stats();
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.run_bottleneck_tuner(now_us, frame.frame_seq);
    }

    /// Bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    pub fn tuning_mode(&self) -> ScreenTunerMode {
        self.tuner.mode()
    }

    /// Alias for tuning_mode.
    pub fn tuner_mode(&self) -> ScreenTunerMode {
        self.tuning_mode()
    }

    /// Configure the bottleneck-driven autonomous tuner mode.
    pub fn set_tuning_mode(&mut self, mode: ScreenTunerMode) -> ScreenTunerMode {
        self.tuner.set_mode(mode);
        self.sync_tuner_stats();
        self.tuner.mode()
    }

    /// Structured autonomous tuning report for REST API and CLI.
    pub fn tuning_report(&self) -> ScreenTuningReport {
        self.tuner.report()
    }

    /// Reset autonomous tuning counters, hysteresis windows and recovery stack.
    pub fn reset_tuning(&mut self) -> ScreenTuningReport {
        self.tuner.reset();
        self.sync_tuner_stats();
        self.tuner.report()
    }

    /// Reference to internal autonomous tuner.
    pub fn tuner(&self) -> &ScreenTuner {
        &self.tuner
    }

    /// Mutable reference to internal autonomous tuner.
    pub fn tuner_mut(&mut self) -> &mut ScreenTuner {
        &mut self.tuner
    }

    /// Run the bottleneck-driven tuner over the freshly ingested profile frame and
    /// apply sender-side knob updates (bitrate / fps / scale / QP) (v2.14 / ADR-100).
    /// Only real cross-end profile envelopes drive the tuner; the local synthetic
    /// Ack-path breakdown stays observation-only so it cannot fight AIMD/TWCC.
    pub(super) fn run_bottleneck_tuner(&mut self, now_us: u64, frame_seq: u64) {
        let bottleneck = self.frame_profiler.bottleneck();
        let (qp_min, qp_max) = self.quality_controller.qp_range();
        let ctx = ScreenTuningContext {
            bitrate_bps: self.rate_controller.current_bps(),
            bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
            bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
            fps: self.stats.fps_target,
            scale_pct: self.stats.scale_pct,
            qp_min,
            qp_max,
            playout_delay_us: 0,
        };
        let outcome = self.tuner.evaluate(now_us, frame_seq, bottleneck, &ctx);
        if let Some(bps) = outcome.bitrate_bps {
            self.rate_controller.set_manual_bps(bps);
            self.stats.bitrate_bps = self.rate_controller.current_bps();
        }
        if let Some(fps) = outcome.fps {
            self.stats.fps_target = self.fps_pacer.set_fps(fps);
        }
        if let Some(scale) = outcome.scale_pct {
            self.stats.scale_pct = self.scale_controller.set_scale_pct(scale);
        }
        if let Some((qmin, qmax)) = outcome.qp_range {
            let (amin, amax) = self.quality_controller.set_qp_range(qmin, qmax);
            self.stats.qp_min = amin;
            self.stats.qp_max = amax;
        }
        self.sync_tuner_stats();
    }

    /// Synchronize autonomous tuner telemetry into ScreenStats (v2.14 / ADR-100).
    pub fn sync_tuner_stats(&mut self) {
        let report = self.tuner.report();
        self.stats.tuning_mode = self.tuner.mode().tag();
        self.stats.tuning_actions_total = report.actions_applied;
        self.stats.tuning_recoveries = report.recoveries_applied;
        self.stats.tuning_last_stage = self.tuner.last_stage_tag();
        self.stats.tuning_last_action = self.tuner.last_action_tag();
    }

    /// Access reference to internal frame profiler.
    pub fn frame_profiler(&self) -> &ScreenFrameProfiler {
        &self.frame_profiler
    }

    /// Access mutable reference to internal frame profiler.
    pub fn frame_profiler_mut(&mut self) -> &mut ScreenFrameProfiler {
        &mut self.frame_profiler
    }

    /// Synchronize internal profiler telemetry to ScreenStats.
    pub fn sync_profiler_stats(&mut self) {
        let lstats = self.frame_profiler.report();
        self.stats.latency_mode = self.frame_profiler.mode().tag();
        self.stats.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        self.stats.latency_total_e2e_us = lstats.avg_e2e_us;
        self.stats.latency_p95_e2e_us = lstats.p95_e2e_us;
        self.stats.latency_frames_profiled = lstats.frames_profiled;
    }

    /// Active bandwidth probing mode (v2.09 / ADR-095).
    pub fn probe_mode(&self) -> ScreenProbeMode {
        self.probe_controller.mode()
    }

    /// Set active bandwidth probing mode.
    pub fn set_probe_mode(&mut self, mode: ScreenProbeMode) {
        self.probe_controller.set_mode(mode);
        self.sync_probe_stats();
    }

    /// Telemetry statistics for Screen Active Bandwidth Probing.
    pub fn probe_stats(&self) -> ScreenProbeStats {
        self.probe_controller.stats()
    }

    /// Structured probe status report for REST API and CLI.
    pub fn probe_report(&self) -> ScreenProbeReport {
        self.probe_controller.report()
    }

    /// Reference to internal probe controller.
    pub fn probe_controller(&self) -> &ScreenProbeController {
        &self.probe_controller
    }

    /// Mutable reference to internal probe controller.
    pub fn probe_controller_mut(&mut self) -> &mut ScreenProbeController {
        &mut self.probe_controller
    }

    /// Manually trigger an active bandwidth probe burst to target_bps.
    pub fn trigger_probe(&mut self, target_bps: u32, now_us: u64) -> bool {
        if self.probe_controller.mode() == ScreenProbeMode::Disabled {
            return false;
        }
        let cluster_size = self.probe_controller.mode().cluster_size().max(4);
        let mut seqs = Vec::with_capacity(cluster_size as usize);
        for _ in 0..cluster_size {
            let seq = self
                .twcc_sender
                .allocate_seq(now_us, SCREEN_PROBE_DEFAULT_PACKET_LEN);
            seqs.push(seq);
        }
        let res = self
            .probe_controller
            .trigger_probe_with_seqs(target_bps, now_us, &seqs);
        self.sync_probe_stats();
        res
    }

    /// Automatically schedule a bandwidth probe if network conditions and intervals allow.
    pub fn maybe_schedule_probe(&mut self, now_us: u64) -> bool {
        if self.probe_controller.mode() == ScreenProbeMode::Disabled {
            return false;
        }
        let cur_bps = self.rate_controller.current_bps();
        let mut next_tseq = self.twcc_sender.next_seq();
        let res = self
            .probe_controller
            .maybe_schedule_probe(cur_bps, now_us, &mut next_tseq);
        self.twcc_sender.set_next_seq(next_tseq);
        self.sync_probe_stats();
        res
    }

    /// Pop a pending probe packet to be transmitted over the wire.
    pub fn pop_probe_packet(&mut self) -> Option<ScreenProbePacket> {
        let pkt = self.probe_controller.pop_probe_packet()?;
        self.sync_probe_stats();
        Some(pkt)
    }

    /// Synchronize internal probe stats to ScreenStats.
    pub fn sync_probe_stats(&mut self) {
        let st = self.probe_controller.stats();
        self.stats.probe_mode = st.mode.tag();
        self.stats.probe_state = st.state.tag();
        self.stats.probes_triggered = st.probes_triggered;
        self.stats.probes_completed = st.probes_completed;
        self.stats.probes_failed = st.probes_failed;
        self.stats.probe_target_bps = st.last_target_bps;
        self.stats.probe_measured_bps = st.last_measured_bps;
    }

    /// Screen and audio presentation timestamp (PTS) sync mode (v2.10 / ADR-096).
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        self.sync_coordinator.mode()
    }

    /// Set screen and audio presentation timestamp sync mode.
    pub fn set_sync_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.sync_coordinator.set_mode(mode);
        self.sync_sync_stats();
    }

    /// Telemetry statistics for audio-video presentation sync.
    pub fn sync_stats(&self) -> ScreenAudioSyncStats {
        self.sync_coordinator.stats()
    }

    /// Structured sync status report for REST API and CLI.
    pub fn sync_report(&self) -> ScreenAudioSyncReport {
        self.sync_coordinator.report()
    }

    /// Record latest video presentation timestamp in microseconds.
    pub fn record_video_pts(&mut self, pts_us: u64) {
        self.sync_coordinator.record_video_pts(pts_us);
    }

    /// Record latest audio presentation timestamp in microseconds.
    pub fn record_audio_pts(&mut self, pts_us: u64) {
        self.sync_coordinator.record_audio_pts(pts_us);
    }

    /// Evaluates beacon generation interval and produces an outbound sync beacon if due.
    pub fn maybe_generate_sync_beacon(&mut self, now_us: u64) -> Option<ScreenAudioSyncFrame> {
        let frame = self.sync_coordinator.maybe_generate_sync_beacon(now_us);
        if frame.is_some() {
            self.sync_sync_stats();
        }
        frame
    }

    /// Unconditionally trigger a sync beacon (for tests or manual inspection).
    pub fn trigger_sync_beacon(&mut self, now_us: u64) -> ScreenAudioSyncFrame {
        let frame = self.sync_coordinator.trigger_sync_beacon(now_us);
        self.sync_sync_stats();
        frame
    }

    /// Pop a pending sync beacon to be transmitted over the wire.
    pub fn pop_sync_beacon(&mut self) -> Option<ScreenAudioSyncFrame> {
        self.sync_coordinator.pop_sync_beacon()
    }

    /// Synchronize internal sync coordinator stats to ScreenStats.
    pub fn sync_sync_stats(&mut self) {
        let st = self.sync_coordinator.stats();
        self.stats.sync_mode = st.mode.tag();
        self.stats.sync_beacons_sent = st.beacons_sent;
        self.stats.sync_beacons_received = st.beacons_received;
        self.stats.sync_skew_us = st.current_skew_us;
        self.stats.sync_clock_drift_ppm = st.clock_drift_ppm;
        self.stats.sync_adjustments_applied = st.adjustments_applied;
        self.stats.in_sync = st.in_sync;
    }

    pub fn twcc_mode(&self) -> ScreenTwccMode {
        self.twcc_sender.mode()
    }

    pub fn set_twcc_mode(&mut self, mode: ScreenTwccMode) {
        self.twcc_sender.set_mode(mode);
        self.stats.twcc_mode = mode.tag();
    }

    pub fn twcc_stats(&self) -> ScreenTwccStats {
        self.twcc_sender.stats()
    }

    pub fn twcc_report(&self) -> ScreenTwccReport {
        self.twcc_sender.report()
    }

    pub fn twcc_sender(&self) -> &ScreenTwccSenderTracker {
        &self.twcc_sender
    }

    pub fn twcc_sender_mut(&mut self) -> &mut ScreenTwccSenderTracker {
        &mut self.twcc_sender
    }

    pub fn handle_twcc_feedback(
        &mut self,
        feedback: &ScreenTwccFeedback,
        now_us: u64,
    ) -> ScreenTwccReport {
        let rep = self.twcc_sender.on_feedback(feedback, now_us);
        self.stats.twcc_feedbacks_received = rep.feedbacks_received;
        self.stats.twcc_delivery_rate_bps = rep.delivery_rate_bps;
        self.stats.twcc_delay_gradient_us = rep.delay_gradient_us;

        // Evaluate active bandwidth probe cluster with TWCC feedback
        if let Some(_new_bps) =
            self.probe_controller
                .on_twcc_feedback(feedback, &mut self.rate_controller, now_us)
        {
            self.stats.bitrate_bps = self.rate_controller.current_bps();
        }
        self.sync_probe_stats();

        if rep.delivery_rate_bps > 0 {
            let cap_bps = rep.delivery_rate_bps.min(u32::MAX as u64) as u32;
            if cap_bps < self.rate_controller.current_bps() && rep.loss_ratio > 0.05 {
                self.rate_controller.force_step_down();
                self.stats.bitrate_bps = self.rate_controller.current_bps();
            }
        }
        rep
    }

    pub fn harq_mode(&self) -> ScreenHarqMode {
        self.retransmit_buffer.mode()
    }

    pub fn set_harq_mode(&mut self, mode: ScreenHarqMode) {
        self.retransmit_buffer.set_mode(mode);
        self.stats.harq_mode = mode.tag();
    }

    pub fn harq_stats(&self) -> ScreenHarqStats {
        self.retransmit_buffer.stats()
    }

    pub fn harq_report(&self) -> ScreenHarqReport {
        self.retransmit_buffer.report()
    }

    pub fn retransmit_buffer(&self) -> &ScreenRetransmitBuffer {
        &self.retransmit_buffer
    }

    pub fn retransmit_buffer_mut(&mut self) -> &mut ScreenRetransmitBuffer {
        &mut self.retransmit_buffer
    }

    pub fn handle_nack(&mut self, nack: &ScreenNack, now_us: u64) -> Vec<InnerFrame> {
        let frames = self.retransmit_buffer.handle_nack(nack, now_us);
        let st = self.retransmit_buffer.stats();
        self.stats.harq_nacks_received = st.nacks_received;
        self.stats.harq_shards_retransmitted = st.shards_retransmitted;
        if nack.requires_keyframe_fallback {
            self.squelch_controller.force_wake(now_us);
            self.stats.is_squelched = false;
        }
        frames
    }

    pub fn record_outbound_frame(&mut self, frame: &InnerFrame, now_us: u64) {
        self.retransmit_buffer.record_outbound_frame(frame, now_us);
    }

    pub fn trendline_mode(&self) -> ScreenTrendlineMode {
        self.trendline_filter.mode()
    }

    pub fn set_trendline_mode(&mut self, mode: ScreenTrendlineMode) {
        self.trendline_filter.set_mode(mode);
        self.stats.trendline_mode = mode.tag();
        self.stats.trendline_state = self.trendline_filter.state().tag();
    }

    pub fn trendline_state(&self) -> ScreenBandwidthUsageState {
        self.trendline_filter.state()
    }

    pub fn trendline_stats(&self) -> ScreenTrendlineStats {
        self.trendline_filter.stats()
    }

    pub fn trendline_report(&self) -> ScreenTrendlineReport {
        let stats = self.trendline_filter.stats();
        ScreenTrendlineReport {
            ok: true,
            mode: stats.mode,
            state: stats.state,
            samples_count: stats.samples_count,
            current_slope_us_per_frame: stats.current_slope_us_per_frame,
            threshold_us_per_frame: stats.threshold_us_per_frame,
            overuse_count: stats.overuse_count,
            underuse_count: stats.underuse_count,
            accumulated_delay_us: stats.accumulated_delay_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Sets an explicit congestion override for testing and deterministic simulation (v2.37 / ADR-123).
    pub fn set_congested_override(&mut self, forced: Option<bool>) {
        self.congested_override = forced;
    }

    /// Whether the screen link is currently congested (v1.93 / ADR-080).
    /// Congestion is active when in-flight frames reach the queue threshold (>= 8),
    /// or when round-trip latency exceeds the 250ms interactive threshold while frames
    /// are in flight (pacing to 1 frame at a time to prevent bufferbloat without stalling).
    pub fn is_congested(&self) -> bool {
        if let Some(forced) = self.congested_override {
            return forced;
        }
        self.stats.in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || (self.stats.in_flight > 0
                && self
                    .stats
                    .rtt_latest_us
                    .is_some_and(|rtt| rtt >= SCREEN_CONGESTION_RTT_THRESHOLD_US))
    }

    /// Current in-flight unacknowledged frames count.
    pub fn in_flight_count(&self) -> u64 {
        self.stats.in_flight
    }

    /// A trusted peer reached the channel. Idempotent while already past Idle.
    pub fn on_peer_connected(&mut self) {
        if self.state == ScreenState::Idle {
            self.state = ScreenState::Requested;
        }
    }

    /// Local consent. The ONLY transition into `Allowed`. A peer can never
    /// drive this.
    pub fn allow(&mut self) {
        if matches!(self.state, ScreenState::Requested | ScreenState::Allowed) {
            self.state = ScreenState::Allowed;
        }
    }

    pub fn revoke(&mut self) {
        eprintln!("[m6dbg] revoke: state {:?} -> requested-path, next_seq={} (kept)", self.state, self.next_seq);
        self.state = match self.state {
            ScreenState::Idle => ScreenState::Idle,
            _ => ScreenState::Requested,
        };
        self.pending.clear();
        self.in_flight_records.clear();
        self.rtt_tracker.clear();
        self.stats.in_flight = 0;
        self.fps_pacer.reset();
    }

    /// Apply a controller instruction. Returns the reply to send back, if any.
    /// A `Start` before `allow` is counted as refused and does not stream.
    pub fn on_control(&mut self, msg: ControlMsg) -> Result<Option<ControlMsg>, CoreError> {
        self.on_control_with_time(msg, 0)
    }

    /// Apply a controller instruction with a monotonic clock timestamp in microseconds.
    pub fn on_control_with_time(
        &mut self,
        msg: ControlMsg,
        now_us: u64,
    ) -> Result<Option<ControlMsg>, CoreError> {
        match msg {
            ControlMsg::Start => {
                if !matches!(self.state, ScreenState::Allowed | ScreenState::Streaming) {
                    self.stats.refused = self.stats.refused.saturating_add(1);
                    // Reply `Stop` so the controller learns its Start was
                    // refused (consent revoked mid-session): silence would
                    // leave the viewer believing pixels are coming. The
                    // receiver treats an unsolicited Stop reply as a refusal.
                    return Ok(Some(ControlMsg::Stop));
                }
                self.state = ScreenState::Streaming;
                Ok(None)
            }
            ControlMsg::Stop => {
                self.state = match self.state {
                    ScreenState::Streaming => ScreenState::Allowed,
                    other => other,
                };
                self.pending.clear();
                self.in_flight_records.clear();
                self.stats.in_flight = 0;
                self.fps_pacer.reset();
                self.privacy_enabled = false;
                self.privacy_lock_input = false;
                self.stats.privacy_enabled = false;
                self.stats.privacy_input_locked = false;
                self.privacy_last_change_us = now_us;
                Ok(None)
            }
            ControlMsg::Ack { seq } => {
                if seq > self.stats.last_acked_seq {
                    self.stats.last_acked_seq = seq;
                }
                self.stats.acked = self.stats.acked.saturating_add(1);

                // Drain acknowledged frames up to `seq` and calculate RTT if timestamps exist
                let mut matched_rtt = None;
                while let Some(&(f_seq, sent_us)) = self.in_flight_records.front() {
                    if f_seq <= seq {
                        self.in_flight_records.pop_front();
                        if f_seq == seq && now_us >= sent_us && sent_us > 0 {
                            matched_rtt = Some(now_us - sent_us);
                        }
                    } else {
                        break;
                    }
                }
                if let Some(rtt) = matched_rtt {
                    self.rtt_tracker.record(rtt);
                    self.stats.rtt_latest_us = Some(rtt);
                    self.stats.rtt_p95_us = self.rtt_tracker.p95_us();

                    // ADR-099 Screen Streaming Microsecond Latency Breakdown & Multi-Stage Frame Profiler
                    if self.frame_profiler.mode() != ScreenProfilerMode::Disabled {
                        let half_rtt = (rtt / 2) as u32;
                        let pacer_delay = (self.stats.pacer_avg_delay_us as u32).min(20_000);
                        let mut b = ScreenLatencyBreakdown {
                            frame_seq: seq,
                            capture_us: 3200,
                            encode_us: 4100,
                            pacer_us: pacer_delay.max(500),
                            network_us: half_rtt.max(1000),
                            jitter_us: 1200,
                            decode_us: 2400,
                            render_us: 1600,
                            total_e2e_us: 0,
                        };
                        b.calculate_total();
                        self.record_latency_breakdown(b);
                    }

                    // ADR-092 Delay-Based Trendline Bufferbloat Filter
                    let sent_us = now_us.saturating_sub(rtt);
                    let usage =
                        self.trendline_filter
                            .on_ack(sent_us, now_us, self.stats.rtt_p95_us);
                    self.stats.trendline_mode = self.trendline_filter.mode().tag();
                    self.stats.trendline_state = usage.tag();
                    self.stats.trendline_slope_us =
                        self.trendline_filter.stats().current_slope_us_per_frame;
                    self.stats.trendline_overuse_count =
                        self.trendline_filter.stats().overuse_count;

                    if usage == ScreenBandwidthUsageState::Overusing {
                        self.rate_controller.on_congestion(now_us);
                        self.stats.bitrate_bps = self.rate_controller.current_bps();
                        self.matrix_coordinator.on_drop(
                            now_us,
                            self.stats.fps_target,
                            self.stats.scale_pct,
                        );
                        self.stats.network_health = self.matrix_coordinator.health().tag();
                    } else {
                        self.rate_controller.on_ack(
                            rtt,
                            self.in_flight_records.len() as u64,
                            now_us,
                        );
                        self.stats.bitrate_bps = self.rate_controller.current_bps();

                        let outcome = self.matrix_coordinator.on_ack(
                            now_us,
                            rtt,
                            self.in_flight_records.len() as u64,
                            self.stats.fps_target,
                            self.stats.scale_pct,
                            SCREEN_FPS_MAX,
                        );
                        if let Some(new_fps) = outcome.new_fps {
                            let actual_fps = self.fps_pacer.set_fps(new_fps);
                            self.stats.fps_target = actual_fps;
                        }
                        if let Some(new_scale) = outcome.new_scale_pct {
                            let actual_scale = self.scale_controller.set_scale_pct(new_scale);
                            self.stats.scale_pct = actual_scale;
                        }
                        self.stats.network_health = outcome.health.tag();
                    }
                }
                self.stats.in_flight = self.in_flight_records.len() as u64;
                Ok(None)
            }
            ControlMsg::KeyframeRequest => {
                self.stats.keyframes_requested = self.stats.keyframes_requested.saturating_add(1);
                self.squelch_controller.force_wake(now_us);
                self.stats.is_squelched = false;
                Ok(None)
            }
            ControlMsg::SwitchDisplay { display_id } => {
                self.switch_display(display_id, now_us)?;
                Ok(None)
            }
            ControlMsg::SetPrivacyMode {
                enabled,
                lock_input,
            } => {
                self.set_privacy_mode(enabled, lock_input, now_us);
                Ok(None)
            }
            ControlMsg::ResetLatencyProfile => {
                self.reset_latency();
                Ok(None)
            }
            ControlMsg::SetTuningMode { mode } => {
                self.set_tuning_mode(ScreenTunerMode::from_tag(mode));
                Ok(None)
            }
            ControlMsg::ResetTuningProfile => {
                self.reset_tuning();
                Ok(None)
            }
            // Recording disclosure is receiver-side state; the sender only mirrors it.
            ControlMsg::SetRecordingState { .. } => Ok(None),
            // Watermark state is handled at node runtime level.
            ControlMsg::SetWatermarkState { .. } => Ok(None),
            // Laser pointer and annotations are handled at node runtime level.
            ControlMsg::SetLaserPointer { .. } => Ok(None),
            ControlMsg::ClearAnnotations => Ok(None),
            ControlMsg::WhiteboardUndo { .. }
            | ControlMsg::WhiteboardRedo { .. }
            | ControlMsg::SetWhiteboardPolicy { .. } => Ok(None),
            // Region of Interest (ROI) controls are handled at node runtime level.
            ControlMsg::SetScreenRoi { .. }
            | ControlMsg::ResetScreenRoi
            | ControlMsg::SetScreenRoiZoom { .. } => Ok(None),
            // Adaptive color depth controls are handled at node runtime level (ADR-108).
            ControlMsg::SetScreenColorMode { .. }
            | ControlMsg::ResetScreenColorMode => Ok(None),
            // Dynamic differential delta patch controls are handled at node runtime level (ADR-109).
            ControlMsg::SetScreenDeltaMode { .. }
            | ControlMsg::ResetScreenDeltaMode => Ok(None),
            // Objective visual quality metric controls are handled at node runtime level (ADR-110).
            ControlMsg::SetScreenQualityMetricMode { .. }
            | ControlMsg::ResetScreenQualityMetricMode => Ok(None),
            // Autonomous closed-loop QoE adaptation controls are handled at node runtime level (ADR-111).
            ControlMsg::SetScreenQoeMode { .. }
            | ControlMsg::ResetScreenQoeMode => Ok(None),
            // Progressive intra-refresh (PIR) controls are handled at node runtime level (ADR-112).
            ControlMsg::SetScreenPirMode { .. }
            | ControlMsg::ResetScreenPirMode => Ok(None),
            // Long-term reference (LTR) controls are handled at node runtime level (ADR-113).
            ControlMsg::SetScreenLtrMode { .. }
            | ControlMsg::ResetScreenLtrMode
            | ControlMsg::LtrRecoveryRequest { .. }
            | ControlMsg::LtrAnchorSync { .. } => Ok(None),
            // Temporal scalability (SVC-T) controls are handled at node runtime level (ADR-114).
            ControlMsg::SetScreenSvcMode { .. }
            | ControlMsg::ResetScreenSvcMode
            | ControlMsg::SvcT1DropSync { .. } => Ok(None),
            // 2D motion vector estimation and scroll acceleration controls are handled at node runtime level (ADR-115).
            ControlMsg::SetScreenMotionMode { .. }
            | ControlMsg::ResetScreenMotionMode
            | ControlMsg::MotionScrollSync { .. } => Ok(None),
            // Spatial tile-grid decomposition controls are handled at node runtime level (ADR-116).
            ControlMsg::SetScreenTileMode { .. }
            | ControlMsg::ResetScreenTileMode
            | ControlMsg::TileGridSync { .. } => Ok(None),
            // Tile dictionary cache controls are handled at node runtime level (ADR-117).
            ControlMsg::SetScreenTileCacheMode { .. }
            | ControlMsg::ResetScreenTileCacheMode
            | ControlMsg::TileCacheSync { .. } => Ok(None),
            // Foveated focus awareness and region-priority scheduling controls are handled at node runtime level (ADR-118).
            ControlMsg::SetScreenFocusMode { .. }
            | ControlMsg::SetScreenFocusBox { .. }
            | ControlMsg::ResetScreenFocusMode
            | ControlMsg::FocusSync { .. }
            | ControlMsg::FocusPeripheralSync { .. } => Ok(None),
            // Tile-level adaptive palette compression controls are handled at node runtime level (ADR-119).
            ControlMsg::SetScreenTilePaletteMode { .. }
            | ControlMsg::ResetScreenTilePaletteMode
            | ControlMsg::TilePaletteSync { .. } => Ok(None),
            // Tile-level predictive filter compression controls are handled at node runtime level (ADR-120).
            ControlMsg::SetScreenTilePredictMode { .. }
            | ControlMsg::ResetScreenTilePredictMode
            | ControlMsg::TilePredictSync { .. }
            // Tile-level presentation transaction controls are handled at node runtime level (ADR-121).
            | ControlMsg::SetScreenTilePresentMode { .. }
            | ControlMsg::ResetScreenTilePresentMode
            | ControlMsg::TilePresentSync { .. }
            // Tile-level fast retransmission and micro-NACK controls are handled at node runtime level (ADR-122).
            | ControlMsg::TileNackRequest { .. }
            | ControlMsg::ResetScreenTileNackMode
            | ControlMsg::TileNackSync { .. }
            // Tile saliency and semantic importance tiering controls are handled at node runtime level (ADR-123).
            | ControlMsg::SetScreenTileSaliencyMode { .. }
            | ControlMsg::ResetScreenTileSaliencyMode
            | ControlMsg::TileSaliencySync { .. }
            // Tile dynamic LoD downsampling controls are handled at node runtime level (ADR-124).
            | ControlMsg::SetScreenTileLodMode { .. }
            | ControlMsg::ResetScreenTileLodMode
            | ControlMsg::TileLodSync { .. }
            // Tile dynamic batch chunking and pacing controls are handled at node runtime level (ADR-125).
            | ControlMsg::SetScreenTileBatchMode { .. }
            | ControlMsg::ResetScreenTileBatchMode
            | ControlMsg::TileBatchSync { .. }
            // Tile dynamic local motion vector estimation controls are handled at node runtime level (ADR-126).
            | ControlMsg::SetScreenTileMotionMode { .. }
            | ControlMsg::ResetScreenTileMotionMode
            | ControlMsg::TileMotionSync { .. } => Ok(None),
        }
    }

    pub fn active_display_id(&self) -> u8 {
        self.active_display_id
    }

    pub fn displays(&self) -> &[ScreenDisplayInfo] {
        &self.displays
    }

    pub fn set_displays(&mut self, displays: Vec<ScreenDisplayInfo>) {
        if !displays.is_empty() {
            self.stats.display_count = displays.len().min(255) as u8;
            self.displays = displays;
        }
    }

    pub fn switch_display(&mut self, display_id: u8, now_us: u64) -> Result<(), CoreError> {
        if !self.displays.is_empty() && !self.displays.iter().any(|d| d.display_id == display_id) {
            return Err(CoreError::ScreenRefused(format!(
                "Display {display_id} not found"
            )));
        }
        self.active_display_id = display_id;
        self.display_switches_count = self.display_switches_count.saturating_add(1);
        self.last_display_switch_us = now_us;
        self.stats.active_display_id = display_id;
        self.stats.display_switches_count = self.display_switches_count;
        self.stats.keyframes_requested = self.stats.keyframes_requested.saturating_add(1);
        self.squelch_controller.force_wake(now_us);
        self.stats.is_squelched = false;
        Ok(())
    }

    pub fn display_report(&self) -> ScreenDisplayReport {
        ScreenDisplayReport {
            ok: true,
            active_display_id: self.active_display_id,
            displays: self.displays.clone(),
            switches_count: self.display_switches_count,
            last_switch_us: self.last_display_switch_us,
        }
    }

    pub fn with_privacy(enabled: bool, lock_input: bool) -> Self {
        let mut sender = Self::new();
        sender.set_privacy_mode(enabled, lock_input, 0);
        sender
    }

    pub fn privacy_report(&self) -> ScreenPrivacyReport {
        ScreenPrivacyReport {
            ok: true,
            enabled: self.privacy_enabled,
            lock_input: self.privacy_lock_input,
            blanking_method: self.privacy_blanking_method.clone(),
            last_change_us: self.privacy_last_change_us,
        }
    }

    pub fn set_privacy_mode(
        &mut self,
        enabled: bool,
        lock_input: bool,
        now_us: u64,
    ) -> ScreenPrivacyReport {
        self.privacy_enabled = enabled;
        self.privacy_lock_input = lock_input;
        self.privacy_last_change_us = now_us;
        self.stats.privacy_enabled = enabled;
        self.stats.privacy_input_locked = lock_input;
        self.privacy_report()
    }

    /// Queue a frame while streaming. Returns the assigned sequence number.
    pub fn push(&mut self, frame: ScreenFrame) -> Option<u64> {
        self.push_with_time(frame, 0)
    }

    /// Queue a frame while streaming with a monotonic send timestamp in microseconds.
    pub fn push_with_time(&mut self, frame: ScreenFrame, now_us: u64) -> Option<u64> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.stats.sent = self.stats.sent.saturating_add(1);
        self.stats.bytes = self.stats.bytes.saturating_add(frame.bytes.len() as u64);

        if self.in_flight_records.len() >= SCREEN_IN_FLIGHT_MAX {
            self.in_flight_records.pop_front();
        }
        self.in_flight_records.push_back((seq, now_us));
        self.stats.in_flight = self.in_flight_records.len() as u64;

        self.pending.push_back(ScreenFrame { seq, ..frame });
        self.record_video_pts(now_us);
        Some(seq)
    }

    /// Queue a frame while streaming evaluated against dirty region damage and adaptive squelch (v2.03 / ADR-089).
    /// If screen is static and in squelch window, frame encoding/transmission is suppressed (returns None).
    pub fn push_with_damage_time(
        &mut self,
        frame: ScreenFrame,
        is_dirty: bool,
        dirty_ratio: f32,
        now_us: u64,
    ) -> Option<u64> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let decision = self
            .squelch_controller
            .evaluate_decision(is_dirty, dirty_ratio, now_us);
        let squelch_stats = self.squelch_controller.stats();
        self.stats.squelch_mode = squelch_stats.mode.tag();
        self.stats.is_squelched = squelch_stats.is_squelched;
        self.stats.squelched_frames = squelch_stats.frames_squelched;

        match decision {
            ScreenSquelchDecision::SendFrame => self.push_with_time(frame, now_us),
            ScreenSquelchDecision::SquelchSkip => None,
        }
    }

    /// Record a frame dropped on sender side (e.g. due to congestion or backpressure).
    pub fn record_drop(&mut self) {
        self.record_drop_with_time(0);
    }

    /// Record a frame dropped on sender side with timestamp for rate adaptation cooldown.
    pub fn record_drop_with_time(&mut self, now_us: u64) {
        self.stats.dropped = self.stats.dropped.saturating_add(1);
        self.rate_controller.on_congestion(now_us);
        self.stats.bitrate_bps = self.rate_controller.current_bps();

        let outcome =
            self.matrix_coordinator
                .on_drop(now_us, self.stats.fps_target, self.stats.scale_pct);
        if let Some(new_fps) = outcome.new_fps {
            let actual_fps = self.fps_pacer.set_fps(new_fps);
            self.stats.fps_target = actual_fps;
        }
        if let Some(new_scale) = outcome.new_scale_pct {
            let actual_scale = self.scale_controller.set_scale_pct(new_scale);
            self.stats.scale_pct = actual_scale;
        }
        self.stats.network_health = outcome.health.tag();
    }

    /// Explicitly configure target bitrate bounds or manual override (v1.96 / ADR-082).
    pub fn set_bitrate_bps(&mut self, bps: u32) {
        self.rate_controller.set_manual_bps(bps);
        self.stats.bitrate_bps = self.rate_controller.current_bps();
    }

    /// Current target video bitrate in bits per second.
    pub fn bitrate_bps(&self) -> u32 {
        self.stats.bitrate_bps
    }

    /// Target streaming frame rate in frames per second (v1.97 / ADR-083).
    pub fn fps_target(&self) -> u32 {
        self.stats.fps_target
    }

    /// Explicitly configure target streaming frame rate in frames per second.
    pub fn set_fps(&mut self, fps: u32) -> u32 {
        let actual = self.fps_pacer.set_fps(fps);
        self.stats.fps_target = actual;
        actual
    }

    /// Access reference to the internal frame rate pacer.
    pub fn fps_pacer(&self) -> &ScreenFpsPacer {
        &self.fps_pacer
    }

    /// Access mutable reference to the internal frame rate pacer.
    pub fn fps_pacer_mut(&mut self) -> &mut ScreenFpsPacer {
        &mut self.fps_pacer
    }

    /// Target screen resolution scale percentage (v1.98 / ADR-084).
    pub fn scale_pct(&self) -> u32 {
        self.stats.scale_pct
    }

    /// Explicitly configure target screen resolution scale percentage (25..=100).
    pub fn set_scale_pct(&mut self, pct: u32) -> u32 {
        let actual = self.scale_controller.set_scale_pct(pct);
        self.stats.scale_pct = actual;
        actual
    }

    /// Access reference to the internal resolution scale controller.
    pub fn scale_controller(&self) -> &ScreenScaleController {
        &self.scale_controller
    }

    /// Access mutable reference to the internal resolution scale controller.
    pub fn scale_controller_mut(&mut self) -> &mut ScreenScaleController {
        &mut self.scale_controller
    }

    /// Active screen content profile (v1.99 / ADR-085).
    pub fn content_profile(&self) -> ScreenContentProfile {
        self.quality_controller.profile()
    }

    /// Explicitly configure target screen content profile.
    pub fn set_content_profile(&mut self, profile: ScreenContentProfile) -> ScreenContentProfile {
        let actual = self.quality_controller.set_profile(profile);
        self.stats.profile_tag = actual.tag();
        self.stats.qp_min = self.quality_controller.min_qp();
        self.stats.qp_max = self.quality_controller.max_qp();
        actual
    }

    /// Access quantization parameter range (min_qp, max_qp).
    pub fn qp_range(&self) -> (u8, u8) {
        self.quality_controller.qp_range()
    }

    /// Explicitly configure quantization parameter bounds.
    pub fn set_qp_range(&mut self, min_qp: u8, max_qp: u8) -> (u8, u8) {
        let range = self.quality_controller.set_qp_range(min_qp, max_qp);
        self.stats.qp_min = range.0;
        self.stats.qp_max = range.1;
        range
    }

    /// Access reference to the internal quality controller.
    pub fn quality_controller(&self) -> &ScreenQualityController {
        &self.quality_controller
    }

    /// Access mutable reference to the internal quality controller.
    pub fn quality_controller_mut(&mut self) -> &mut ScreenQualityController {
        &mut self.quality_controller
    }

    /// Operation mode of the 4D autonomous adaptation coordinator (v2.00 / ADR-086).
    pub fn matrix_mode(&self) -> ScreenMatrixMode {
        self.matrix_coordinator.mode()
    }

    /// Explicitly configure 4D autonomous adaptation coordinator mode.
    pub fn set_matrix_mode(&mut self, mode: ScreenMatrixMode) -> ScreenMatrixMode {
        self.matrix_coordinator.set_mode(mode);
        self.stats.matrix_mode = mode.tag();
        mode
    }

    /// Assessed real-time network health state.
    pub fn network_health(&self) -> NetworkHealth {
        self.matrix_coordinator.health()
    }

    /// Access reference to the internal 4D matrix coordinator.
    pub fn matrix_coordinator(&self) -> &ScreenMatrixCoordinator {
        &self.matrix_coordinator
    }

    /// Access mutable reference to the internal 4D matrix coordinator.
    pub fn matrix_coordinator_mut(&mut self) -> &mut ScreenMatrixCoordinator {
        &mut self.matrix_coordinator
    }

    /// Check if target frame pacing interval has elapsed.
    pub fn is_pacing_ready(&self, now_us: u64) -> bool {
        self.fps_pacer.is_ready(now_us)
    }

    /// Check and consume a pacing tick if target frame pacing interval has elapsed.
    pub fn consume_pacing(&mut self, now_us: u64) -> bool {
        self.fps_pacer.check_and_consume(now_us)
    }

    /// Hardware cursor decoupling mode (v2.02 / ADR-088).
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        self.cursor_controller.mode()
    }

    /// Explicitly configure hardware cursor decoupling mode.
    pub fn set_cursor_mode(&mut self, mode: ScreenCursorMode) -> ScreenCursorMode {
        self.cursor_controller.set_mode(mode);
        self.stats.cursor_mode = mode.tag();
        mode
    }

    /// Access reference to internal cursor controller.
    pub fn cursor_controller(&self) -> &ScreenCursorController {
        &self.cursor_controller
    }

    /// Access mutable reference to internal cursor controller.
    pub fn cursor_controller_mut(&mut self) -> &mut ScreenCursorController {
        &mut self.cursor_controller
    }

    /// Push high-frequency cursor update. Returns ScreenCursorUpdate if candidate passed dedup and rate-limiting.
    #[allow(clippy::too_many_arguments)]
    pub fn push_cursor_with_time(
        &mut self,
        x: i32,
        y: i32,
        visible: bool,
        shape: CursorShape,
        hotspot: (u16, u16),
        dimensions: (u16, u16),
        custom_rgba: Option<Vec<u8>>,
        now_us: u64,
    ) -> Option<ScreenCursorUpdate> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let update = self.cursor_controller.create_update(
            x,
            y,
            visible,
            shape,
            hotspot,
            dimensions,
            custom_rgba,
            now_us,
        )?;
        self.stats.cursor_x = x;
        self.stats.cursor_y = y;
        self.stats.cursor_visible = visible;
        self.stats.cursor_shape = shape.tag();
        self.stats.cursor_updates_sent = self.cursor_controller.stats().updates_sent;
        Some(update)
    }

    /// Access snapshot of cursor controller statistics.
    pub fn cursor_stats(&self) -> ScreenCursorStats {
        self.cursor_controller.stats()
    }

    /// Push high-frequency cursor update from existing ScreenCursorUpdate.
    pub fn push_cursor_update_with_time(
        &mut self,
        update: ScreenCursorUpdate,
        now_us: u64,
    ) -> Option<ScreenCursorUpdate> {
        self.push_cursor_with_time(
            update.x,
            update.y,
            update.visible,
            update.shape,
            (update.hotspot_x, update.hotspot_y),
            (update.width, update.height),
            update.custom_rgba,
            now_us,
        )
    }

    /// Screen streaming adaptive squelch mode (v2.03 / ADR-089).
    pub fn squelch_mode(&self) -> ScreenSquelchMode {
        self.squelch_controller.mode()
    }

    /// Explicitly configure screen streaming adaptive squelch mode.
    pub fn set_squelch_mode(&mut self, mode: ScreenSquelchMode) -> ScreenSquelchMode {
        let actual = self.squelch_controller.set_mode(mode);
        self.stats.squelch_mode = actual.tag();
        self.stats.is_squelched = self.squelch_controller.is_squelched();
        actual
    }

    /// Whether frame transmission is currently squelched due to static screen.
    pub fn is_squelched(&self) -> bool {
        self.squelch_controller.is_squelched()
    }

    /// Real-time squelch controller telemetry and statistics.
    pub fn squelch_stats(&self) -> ScreenSquelchStats {
        self.squelch_controller.stats()
    }

    /// Access reference to internal squelch controller.
    pub fn squelch_controller(&self) -> &ScreenSquelchController {
        &self.squelch_controller
    }

    /// Access mutable reference to internal squelch controller.
    pub fn squelch_controller_mut(&mut self) -> &mut ScreenSquelchController {
        &mut self.squelch_controller
    }

    /// Wake up squelch controller immediately (e.g. upon user input or client interaction).
    pub fn force_wake_squelch(&mut self, now_us: u64) {
        self.squelch_controller.force_wake(now_us);
        self.stats.is_squelched = false;
    }

    /// Screen forward error correction (FEC) mode (v2.04 / ADR-090).
    pub fn fec_mode(&self) -> ScreenFecMode {
        self.fec_controller.mode()
    }

    /// Explicitly configure screen forward error correction (FEC) mode.
    pub fn set_fec_mode(&mut self, mode: ScreenFecMode) -> ScreenFecMode {
        let actual = self.fec_controller.set_mode(mode);
        self.stats.fec_mode = actual.tag();
        self.stats.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;
        actual
    }

    /// Access reference to internal FEC controller.
    pub fn fec_controller(&self) -> &ScreenFecController {
        &self.fec_controller
    }

    /// Access mutable reference to internal FEC controller.
    pub fn fec_controller_mut(&mut self) -> &mut ScreenFecController {
        &mut self.fec_controller
    }

    /// Real-time FEC controller telemetry and statistics.
    pub fn fec_stats(&self) -> ScreenFecStats {
        self.fec_controller.stats()
    }

    /// Screen packet pacing mode (v2.05 / ADR-091).
    pub fn pacer_mode(&self) -> ScreenPacerMode {
        self.packet_pacer.mode()
    }

    /// Explicitly configure screen packet pacing mode.
    pub fn set_pacer_mode(&mut self, mode: ScreenPacerMode) -> ScreenPacerMode {
        let actual = self.packet_pacer.set_mode(mode);
        self.stats.pacer_mode = actual.tag();
        actual
    }

    /// Access reference to internal packet pacer.
    pub fn packet_pacer(&self) -> &ScreenPacketPacer {
        &self.packet_pacer
    }

    /// Access mutable reference to internal packet pacer.
    pub fn packet_pacer_mut(&mut self) -> &mut ScreenPacketPacer {
        &mut self.packet_pacer
    }

    /// Real-time packet pacer statistics.
    pub fn pacer_stats(&self) -> ScreenPacerStats {
        self.packet_pacer.stats()
    }

    /// Computes packet departure delay offsets (in microseconds) for smoothing bursts.
    pub fn compute_packet_delays(&mut self, packet_count: usize) -> Vec<u64> {
        let delays = self
            .packet_pacer
            .compute_packet_delays(packet_count, self.fps_pacer.fps_target());
        let st = self.packet_pacer.stats();
        self.stats.pacer_packets_paced = st.packets_paced;
        self.stats.pacer_bursts_smoothed = st.bursts_smoothed;
        self.stats.pacer_avg_delay_us = st.avg_delay_us;
        delays
    }

    /// Convert a ScreenFrame into outbound InnerFrames with monotonic send timestamp (microseconds).
    /// Outbound frames are tracked in the sender's sliding window HARQ retransmission buffer.
    pub fn encode_frame_to_inner_frames_with_time(
        &mut self,
        frame: &ScreenFrame,
        packet_loss_ratio: f32,
        now_us: u64,
    ) -> Result<Vec<InnerFrame>, CoreError> {
        let payload = frame.encode_payload()?;
        let m_parity = self
            .fec_controller
            .calculate_parity_shards(1, packet_loss_ratio);
        if m_parity == 0 {
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        // Split payload into shards that comfortably fit within INNER_PAYLOAD_MAX
        let shard_chunk_max = INNER_PAYLOAD_MAX - FEC_HEADER_LEN - 32;
        let k = payload.len().div_ceil(shard_chunk_max).max(1);
        if k > FEC_MAX_DATA_SHARDS {
            // If too large for single FEC block, fallback to standard fragmentation
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        let mut data_buffers = Vec::with_capacity(k);
        for chunk in payload.chunks(shard_chunk_max) {
            data_buffers.push(chunk.to_vec());
        }

        let m = self
            .fec_controller
            .calculate_parity_shards(k, packet_loss_ratio);
        if m == 0 {
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        let mut shards = ScreenFecEncoder::encode_block(frame.seq, 0, &data_buffers, m)?;
        self.fec_controller.record_block_encoded(k, m);
        self.stats.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;

        for shard in &mut shards {
            shard.transport_seq = self
                .twcc_sender
                .allocate_seq(now_us, shard.data.len() + FEC_HEADER_LEN);
        }

        let frames: Vec<InnerFrame> = shards.iter().map(|s| s.to_inner_frame()).collect();
        for f in &frames {
            self.retransmit_buffer.record_outbound_frame(f, now_us);
        }
        Ok(frames)
    }

    /// Convert a ScreenFrame into outbound InnerFrames.
    /// If FEC mode is active (redundancy > 0), encodes payload into Cauchy RS shards (TYPE_SCREEN_FEC).
    /// Otherwise, falls back to standard fragmentation (TYPE_SCREEN_FRAME).
    pub fn encode_frame_to_inner_frames(
        &mut self,
        frame: &ScreenFrame,
        packet_loss_ratio: f32,
    ) -> Result<Vec<InnerFrame>, CoreError> {
        self.encode_frame_to_inner_frames_with_time(frame, packet_loss_ratio, 0)
    }

    pub fn drain(&mut self) -> Vec<ScreenFrame> {
        self.pending.drain(..).collect()
    }

    /// Peer gone: back to Idle, sequence reset so a reconnect starts clean and
    /// cannot replay.
    pub fn on_peer_gone(&mut self) {
        eprintln!("[m6dbg] on_peer_gone: next_seq {} -> 1 (SEQ RESET)", self.next_seq);
        self.state = ScreenState::Idle;
        self.next_seq = 1;
        self.pending.clear();
        self.in_flight_records.clear();
        self.rtt_tracker.clear();
        self.stats.in_flight = 0;
        self.fps_pacer.reset();
        self.cursor_controller.reset();
        self.squelch_controller = ScreenSquelchController::new(self.squelch_controller.mode());
        self.fec_controller = ScreenFecController::new(self.fec_controller.mode());
        self.packet_pacer = ScreenPacketPacer::new(self.packet_pacer.mode());
        self.retransmit_buffer.clear();
        self.twcc_sender.clear();
        self.stats.is_squelched = false;
        self.stats.squelched_frames = 0;
        self.stats.harq_nacks_sent = 0;
        self.stats.harq_nacks_received = 0;
        self.stats.harq_shards_retransmitted = 0;
        self.stats.harq_recovered_frames = 0;
        self.stats.twcc_feedbacks_sent = 0;
        self.stats.twcc_feedbacks_received = 0;
        self.stats.twcc_delivery_rate_bps = 0;
        self.stats.twcc_delay_gradient_us = 0;
        self.stats.probe_state = ScreenProbeState::Idle.tag();
        self.stats.probes_triggered = 0;
        self.stats.probes_completed = 0;
        self.stats.probes_failed = 0;
        self.stats.probe_target_bps = 0;
        self.stats.probe_measured_bps = 0;
        self.probe_controller = ScreenProbeController::new(self.probe_controller.mode());
        self.sync_coordinator.reset();
        self.sync_sync_stats();
        self.stats.active_display_id = 0;
        self.active_display_id = 0;
        self.display_switches_count = 0;
        self.last_display_switch_us = 0;
        self.stats.display_switches_count = 0;
        self.privacy_enabled = false;
        self.privacy_lock_input = false;
        self.stats.privacy_enabled = false;
        self.stats.privacy_input_locked = false;
        self.frame_profiler.reset();
        self.sync_profiler_stats();
    }
}
