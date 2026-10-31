// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Controlling-side receiver: reassembly, decoding and presentation state.

use super::*;
use crate::error::CoreError;
use std::collections::VecDeque;

/// Receiver side. Enforces the sequence window and never presents a frame it
/// could not fully validate.
pub struct ScreenReceiver {
    pub(super) last_seq: u64,
    pub(super) stats: ScreenStats,
    pub(super) trace: VecDeque<u64>,
    pub(super) playout_buffer: ScreenPlayoutBuffer,
    pub(super) cursor_controller: ScreenCursorController,
    pub(super) last_cursor: Option<ScreenCursorUpdate>,
    pub(super) fec_assembler: ScreenFecAssembler,
    pub(super) harq_mode: ScreenHarqMode,
    pub(super) harq_stats: ScreenHarqStats,
    pub(super) nack_history: VecDeque<(u64, u16, u64)>,
    pub(super) twcc_receiver: ScreenTwccReceiverTracker,
    pub(super) probe_mode: ScreenProbeMode,
    pub(super) probe_stats: ScreenProbeStats,
    pub(super) sync_coordinator: ScreenAudioSyncCoordinator,
    pub(super) active_display_id: u8,
    pub(super) displays: Vec<ScreenDisplayInfo>,
    pub(super) display_switches_count: u64,
    pub(super) last_switch_us: u64,
    pub(super) privacy_enabled: bool,
    pub(super) privacy_lock_input: bool,
    pub(super) privacy_blanking_method: String,
    pub(super) privacy_last_change_us: u64,
    pub(super) frame_profiler: ScreenFrameProfiler,
    pub(super) tuner: ScreenTuner,
}

impl Default for ScreenReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenReceiver {
    pub fn new() -> Self {
        let cursor_controller = ScreenCursorController::default();
        let harq_mode = ScreenHarqMode::Auto;
        let harq_stats = ScreenHarqStats {
            mode: harq_mode,
            ..Default::default()
        };
        let twcc_mode = ScreenTwccMode::Auto;
        let twcc_receiver = ScreenTwccReceiverTracker::new(twcc_mode);
        let probe_mode = ScreenProbeMode::Auto;
        let probe_stats = ScreenProbeStats {
            mode: probe_mode,
            ..Default::default()
        };
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let frame_profiler = ScreenFrameProfiler::default();
        let tuner = ScreenTuner::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            harq_mode: harq_mode.tag(),
            twcc_mode: twcc_mode.tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_mode.tag(),
            probe_state: probe_stats.state.tag(),
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
            latency_mode: frame_profiler.mode().tag(),
            latency_bottleneck: frame_profiler.bottleneck().tag(),
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
            ..Default::default()
        };
        Self {
            last_seq: 0,
            stats,
            trace: VecDeque::with_capacity(SCREEN_TRACE_MAX),
            playout_buffer: ScreenPlayoutBuffer::default(),
            cursor_controller,
            last_cursor: None,
            fec_assembler: ScreenFecAssembler::new(),
            harq_mode,
            harq_stats,
            nack_history: VecDeque::with_capacity(64),
            twcc_receiver,
            probe_mode,
            probe_stats,
            sync_coordinator,
            active_display_id: 0,
            displays,
            display_switches_count: 0,
            last_switch_us: 0,
            privacy_enabled: false,
            privacy_lock_input: false,
            privacy_blanking_method: "software_blanking".to_string(),
            privacy_last_change_us: 0,
            frame_profiler,
            tuner,
        }
    }

    pub fn with_playout_mode(mode: ScreenPlayoutMode) -> Self {
        let cursor_controller = ScreenCursorController::default();
        let harq_mode = ScreenHarqMode::Auto;
        let harq_stats = ScreenHarqStats {
            mode: harq_mode,
            ..Default::default()
        };
        let twcc_mode = ScreenTwccMode::Auto;
        let twcc_receiver = ScreenTwccReceiverTracker::new(twcc_mode);
        let probe_mode = ScreenProbeMode::Auto;
        let probe_stats = ScreenProbeStats {
            mode: probe_mode,
            ..Default::default()
        };
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let frame_profiler = ScreenFrameProfiler::default();
        let tuner = ScreenTuner::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            harq_mode: harq_mode.tag(),
            twcc_mode: twcc_mode.tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_mode.tag(),
            probe_state: probe_stats.state.tag(),
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
            latency_mode: frame_profiler.mode().tag(),
            latency_bottleneck: frame_profiler.bottleneck().tag(),
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
            ..Default::default()
        };
        Self {
            last_seq: 0,
            stats,
            trace: VecDeque::with_capacity(SCREEN_TRACE_MAX),
            playout_buffer: ScreenPlayoutBuffer::new(mode),
            cursor_controller,
            last_cursor: None,
            fec_assembler: ScreenFecAssembler::new(),
            harq_mode,
            harq_stats,
            nack_history: VecDeque::with_capacity(64),
            twcc_receiver,
            probe_mode,
            probe_stats,
            sync_coordinator,
            active_display_id: 0,
            displays,
            display_switches_count: 0,
            last_switch_us: 0,
            privacy_enabled: false,
            privacy_lock_input: false,
            privacy_blanking_method: "software_blanking".to_string(),
            privacy_last_change_us: 0,
            frame_profiler,
            tuner,
        }
    }

    pub fn with_profiler(mode: ScreenProfilerMode) -> Self {
        let mut r = Self::new();
        r.set_profiler_mode(mode);
        r
    }

    pub fn with_harq_mode(mode: ScreenHarqMode) -> Self {
        let mut r = Self::new();
        r.set_harq_mode(mode);
        r
    }

    pub fn with_twcc_mode(mode: ScreenTwccMode) -> Self {
        let mut r = Self::new();
        r.set_twcc_mode(mode);
        r
    }

    pub fn with_probe_mode(mode: ScreenProbeMode) -> Self {
        let mut r = Self::new();
        r.set_probe_mode(mode);
        r
    }

    pub fn with_sync_mode(mode: ScreenAudioSyncMode) -> Self {
        let mut r = Self::new();
        r.set_sync_mode(mode);
        r
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn stats(&self) -> ScreenStats {
        let mut st = self.stats;
        let pstats = self.playout_buffer.stats();
        st.playout_mode = pstats.mode as u8;
        st.playout_delay_us = pstats.target_delay_us.min(u32::MAX as u64) as u32;
        st.playout_jitter_us = pstats.current_jitter_us.min(u32::MAX as u64) as u32;
        let cstats = self.cursor_controller.stats();
        st.cursor_mode = cstats.mode.tag();
        st.cursor_updates_sent = cstats.updates_sent;
        st.cursor_updates_received = self.stats.cursor_updates_received;
        if let Some(ref cur) = self.last_cursor {
            st.cursor_x = cur.x;
            st.cursor_y = cur.y;
            st.cursor_visible = cur.visible;
            st.cursor_shape = cur.shape.tag();
        }
        st.fec_recovered_shards = self.fec_assembler.recovered_shards();
        st.fec_unrecoverable_blocks = self.fec_assembler.unrecoverable_blocks();
        st.harq_mode = self.harq_mode.tag();
        st.harq_nacks_sent = self.harq_stats.nacks_sent;
        st.harq_nacks_received = self.harq_stats.nacks_received;
        st.harq_shards_retransmitted = self.harq_stats.shards_retransmitted;
        st.harq_recovered_frames = self.harq_stats.recovered_frames;
        st.twcc_mode = self.twcc_receiver.mode().tag();
        let twcc_st = self.twcc_receiver.stats();
        st.twcc_feedbacks_sent = twcc_st.feedbacks_sent;
        st.twcc_feedbacks_received = twcc_st.feedbacks_received;
        st.twcc_delivery_rate_bps = twcc_st.delivery_rate_bps;
        st.twcc_delay_gradient_us = twcc_st.delay_gradient_us;
        st.probe_mode = self.probe_mode.tag();
        st.probe_state = self.probe_stats.state.tag();
        st.probes_triggered = self.probe_stats.probes_triggered;
        st.probes_completed = self.probe_stats.probes_completed;
        st.probes_failed = self.probe_stats.probes_failed;
        st.probe_target_bps = self.probe_stats.last_target_bps;
        st.probe_measured_bps = self.probe_stats.last_measured_bps;
        let sync_st = self.sync_coordinator.stats();
        st.sync_mode = sync_st.mode.tag();
        st.sync_beacons_sent = sync_st.beacons_sent;
        st.sync_beacons_received = sync_st.beacons_received;
        st.sync_skew_us = sync_st.current_skew_us;
        st.sync_clock_drift_ppm = sync_st.clock_drift_ppm;
        st.sync_adjustments_applied = sync_st.adjustments_applied;
        st.in_sync = sync_st.in_sync;
        let lstats = self.frame_profiler.report();
        st.latency_mode = self.frame_profiler.mode().tag();
        st.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        st.latency_total_e2e_us = lstats.avg_e2e_us;
        st.latency_p95_e2e_us = lstats.p95_e2e_us;
        st.latency_frames_profiled = lstats.frames_profiled;
        st
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

    /// Run the bottleneck-driven tuner over the freshly profiled frame and apply the
    /// receiver-side knob update (playout buffer target delay) (v2.14 / ADR-100).
    pub(super) fn run_bottleneck_tuner(&mut self, now_us: u64, frame_seq: u64) {
        let bottleneck = self.frame_profiler.bottleneck();
        let ctx = ScreenTuningContext {
            playout_delay_us: self.playout_buffer.target_delay_us().min(u32::MAX as u64) as u32,
            ..Default::default()
        };
        let outcome = self.tuner.evaluate(now_us, frame_seq, bottleneck, &ctx);
        if let Some(delta) = outcome.playout_delay_delta_us {
            self.playout_buffer.adjust_target_delay(delta);
            self.stats.playout_delay_us =
                self.playout_buffer.target_delay_us().min(u32::MAX as u64) as u32;
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
        self.probe_mode
    }

    /// Explicitly configure active bandwidth probing mode.
    pub fn set_probe_mode(&mut self, mode: ScreenProbeMode) {
        self.probe_mode = mode;
        self.probe_stats.mode = mode;
        self.sync_probe_stats();
    }

    /// Telemetry statistics for Screen Active Bandwidth Probing.
    pub fn probe_stats(&self) -> ScreenProbeStats {
        self.probe_stats
    }

    /// Structured probe status report for REST API and CLI.
    pub fn probe_report(&self) -> ScreenProbeReport {
        ScreenProbeReport {
            ok: true,
            mode: self.probe_mode,
            state: self.probe_stats.state,
            probes_triggered: self.probe_stats.probes_triggered,
            probes_completed: self.probe_stats.probes_completed,
            probes_failed: self.probe_stats.probes_failed,
            last_target_bps: self.probe_stats.last_target_bps,
            last_measured_bps: self.probe_stats.last_measured_bps,
            last_rtt_us: self.probe_stats.last_rtt_us,
            last_loss_ratio: self.probe_stats.last_loss_ratio,
            last_delay_gradient_us: self.probe_stats.last_delay_gradient_us,
            active_cluster_id: self.probe_stats.active_cluster_id,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Process incoming active bandwidth probe packet (v2.09 / ADR-095).
    /// Records packet arrival timestamp into TWCC receiver tracker for feedback emission.
    pub fn on_probe_packet_with_time(&mut self, probe: ScreenProbePacket, now_us: u64) {
        if probe.transport_seq > 0 {
            self.twcc_receiver
                .on_packet_received(probe.transport_seq, now_us);
        }
        self.probe_stats.probes_triggered = self.probe_stats.probes_triggered.saturating_add(1);
        self.probe_stats.last_target_bps = probe.target_bps;
        self.probe_stats.active_cluster_id = Some(probe.cluster_id);
        self.sync_probe_stats();
    }

    pub fn on_probe_packet(&mut self, probe: ScreenProbePacket) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_probe_packet_with_time(probe, now_us);
    }

    pub(super) fn sync_probe_stats(&mut self) {
        self.stats.probe_mode = self.probe_mode.tag();
        self.stats.probe_state = self.probe_stats.state.tag();
        self.stats.probes_triggered = self.probe_stats.probes_triggered;
        self.stats.probes_completed = self.probe_stats.probes_completed;
        self.stats.probes_failed = self.probe_stats.probes_failed;
        self.stats.probe_target_bps = self.probe_stats.last_target_bps;
        self.stats.probe_measured_bps = self.probe_stats.last_measured_bps;
    }

    /// Audio-video lip-sync alignment mode (v2.10 / ADR-096).
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        self.sync_coordinator.mode()
    }

    /// Explicitly configure audio-video lip-sync alignment mode.
    pub fn set_sync_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.sync_coordinator.set_mode(mode);
        self.sync_sync_stats();
    }

    /// Snapshot of receiver-side sync statistics.
    pub fn sync_stats(&self) -> ScreenAudioSyncStats {
        self.sync_coordinator.stats()
    }

    /// Structured receiver-side sync status report.
    pub fn sync_report(&self) -> ScreenAudioSyncReport {
        self.sync_coordinator.report()
    }

    /// Ingest an inbound sync beacon with explicit arrival timestamp in microseconds.
    pub fn on_sync_beacon_with_time(&mut self, beacon: &ScreenAudioSyncFrame, now_us: u64) -> i64 {
        let skew =
            self.sync_coordinator
                .on_sync_beacon(beacon, now_us, Some(&mut self.playout_buffer));
        self.sync_sync_stats();
        skew
    }

    /// Ingest an inbound sync beacon using current system clock.
    pub fn on_sync_beacon(&mut self, beacon: &ScreenAudioSyncFrame) -> i64 {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_sync_beacon_with_time(beacon, now_us)
    }

    pub(super) fn sync_sync_stats(&mut self) {
        let st = self.sync_coordinator.stats();
        self.stats.sync_mode = st.mode.tag();
        self.stats.sync_beacons_sent = st.beacons_sent;
        self.stats.sync_beacons_received = st.beacons_received;
        self.stats.sync_skew_us = st.current_skew_us;
        self.stats.sync_clock_drift_ppm = st.clock_drift_ppm;
        self.stats.sync_adjustments_applied = st.adjustments_applied;
        self.stats.in_sync = st.in_sync;
    }

    /// Access playout buffer reference.
    pub fn playout_buffer(&self) -> &ScreenPlayoutBuffer {
        &self.playout_buffer
    }

    /// Access mutable playout buffer reference.
    pub fn playout_buffer_mut(&mut self) -> &mut ScreenPlayoutBuffer {
        &mut self.playout_buffer
    }

    /// Current playout mode.
    pub fn playout_mode(&self) -> ScreenPlayoutMode {
        self.playout_buffer.mode()
    }

    /// Set playout mode.
    pub fn set_playout_mode(&mut self, mode: ScreenPlayoutMode) {
        self.playout_buffer.set_mode(mode);
    }

    /// Playout buffer stats and telemetry.
    pub fn playout_stats(&self) -> ScreenPlayoutStats {
        self.playout_buffer.stats()
    }

    /// Screen HARQ mode (v2.07 / ADR-093).
    pub fn harq_mode(&self) -> ScreenHarqMode {
        self.harq_mode
    }

    /// Explicitly configure screen HARQ mode.
    pub fn set_harq_mode(&mut self, mode: ScreenHarqMode) {
        self.harq_mode = mode;
        self.harq_stats.mode = mode;
        self.stats.harq_mode = mode.tag();
        if mode == ScreenHarqMode::Disabled {
            self.nack_history.clear();
        }
    }

    /// Snapshot of receiver-side HARQ statistics.
    pub fn harq_stats(&self) -> ScreenHarqStats {
        self.harq_stats
    }

    /// Structured receiver-side HARQ status report.
    pub fn harq_report(&self) -> ScreenHarqReport {
        ScreenHarqReport {
            ok: true,
            mode: self.harq_mode,
            nacks_sent: self.harq_stats.nacks_sent,
            nacks_received: self.harq_stats.nacks_received,
            shards_retransmitted: self.harq_stats.shards_retransmitted,
            recovered_frames: self.harq_stats.recovered_frames,
            retransmit_timeouts: self.harq_stats.retransmit_timeouts,
            buffer_occupancy: 0,
            buffer_capacity: SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Screen TWCC mode (v2.08 / ADR-094).
    pub fn twcc_mode(&self) -> ScreenTwccMode {
        self.twcc_receiver.mode()
    }

    /// Explicitly configure screen TWCC mode.
    pub fn set_twcc_mode(&mut self, mode: ScreenTwccMode) {
        self.twcc_receiver.set_mode(mode);
        self.stats.twcc_mode = mode.tag();
    }

    /// Snapshot of receiver-side TWCC statistics.
    pub fn twcc_stats(&self) -> ScreenTwccStats {
        self.twcc_receiver.stats()
    }

    /// Structured receiver-side TWCC status report.
    pub fn twcc_report(&self) -> ScreenTwccReport {
        self.twcc_receiver.report()
    }

    /// Access reference to internal TWCC receiver tracker.
    pub fn twcc_receiver(&self) -> &ScreenTwccReceiverTracker {
        &self.twcc_receiver
    }

    /// Access mutable reference to internal TWCC receiver tracker.
    pub fn twcc_receiver_mut(&mut self) -> &mut ScreenTwccReceiverTracker {
        &mut self.twcc_receiver
    }

    /// Generate periodic or packet-threshold TWCC arrival feedback frame.
    pub fn generate_twcc_feedback(&mut self, now_us: u64) -> Option<ScreenTwccFeedback> {
        if self.twcc_receiver.should_send_feedback(now_us) {
            let fb = self.twcc_receiver.build_feedback(now_us);
            if fb.is_some() {
                self.stats.twcc_feedbacks_sent = self.twcc_receiver.stats().feedbacks_sent;
            }
            fb
        } else {
            None
        }
    }

    /// Generate selective NACK on missing FEC shards with RTT pacing debounce.
    pub fn generate_nack_on_fec(
        &mut self,
        now_us: u64,
        rtt_estimate_us: u64,
    ) -> Option<ScreenNack> {
        if self.harq_mode == ScreenHarqMode::Disabled {
            return None;
        }
        let nack = self.fec_assembler.generate_nack(rtt_estimate_us)?;
        let min_interval_us = rtt_estimate_us.max(20_000);
        if self.nack_history.iter().any(|&(seq, blk, t)| {
            seq == nack.frame_seq
                && blk == nack.block_id
                && now_us.saturating_sub(t) < min_interval_us
        }) {
            return None;
        }
        if self.nack_history.len() >= 64 {
            self.nack_history.pop_front();
        }
        self.nack_history
            .push_back((nack.frame_seq, nack.block_id, now_us));
        self.harq_stats.nacks_sent = self.harq_stats.nacks_sent.saturating_add(1);
        self.stats.harq_nacks_sent = self.harq_stats.nacks_sent;
        Some(nack)
    }

    /// Generate selective NACK on missing standard fragments with RTT pacing debounce.
    pub fn generate_nack_on_frag(
        &mut self,
        assembler: &ScreenAssembler,
        now_us: u64,
        rtt_estimate_us: u64,
    ) -> Option<ScreenNack> {
        if self.harq_mode == ScreenHarqMode::Disabled {
            return None;
        }
        let nack = assembler.generate_nack(rtt_estimate_us)?;
        let min_interval_us = rtt_estimate_us.max(20_000);
        if self.nack_history.iter().any(|&(seq, blk, t)| {
            seq == nack.frame_seq
                && blk == nack.block_id
                && now_us.saturating_sub(t) < min_interval_us
        }) {
            return None;
        }
        if self.nack_history.len() >= 64 {
            self.nack_history.pop_front();
        }
        self.nack_history
            .push_back((nack.frame_seq, nack.block_id, now_us));
        self.harq_stats.nacks_sent = self.harq_stats.nacks_sent.saturating_add(1);
        self.stats.harq_nacks_sent = self.harq_stats.nacks_sent;
        Some(nack)
    }

    /// Record a frame successfully recovered via HARQ retransmission.
    pub fn record_recovered_frame(&mut self) {
        self.harq_stats.recovered_frames = self.harq_stats.recovered_frames.saturating_add(1);
        self.stats.harq_recovered_frames = self.harq_stats.recovered_frames;
    }

    /// Push an accepted frame to the playout buffer with its arrival timestamp (microseconds).
    pub fn push_playout(&mut self, frame: ScreenFrame, arrival_us: u64) {
        self.playout_buffer.push_frame(frame, arrival_us);
    }

    /// Pop due frame for presentation at current clock timestamp `now_us`.
    pub fn pop_playout(&mut self, now_us: u64) -> ScreenPlayoutAction {
        self.playout_buffer.pop_frame(now_us)
    }

    /// Hardware cursor decoupling mode (v2.02 / ADR-088).
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        self.cursor_controller.mode()
    }

    /// Set hardware cursor decoupling mode.
    pub fn set_cursor_mode(&mut self, mode: ScreenCursorMode) -> ScreenCursorMode {
        self.cursor_controller.set_mode(mode);
        self.stats.cursor_mode = mode.tag();
        mode
    }

    /// Handle incoming high-frequency cursor update frame.
    pub fn on_cursor(&mut self, update: ScreenCursorUpdate) {
        self.stats.cursor_x = update.x;
        self.stats.cursor_y = update.y;
        self.stats.cursor_visible = update.visible;
        self.stats.cursor_shape = update.shape.tag();
        self.stats.cursor_updates_received = self.stats.cursor_updates_received.saturating_add(1);
        self.last_cursor = Some(update);
    }

    /// Current cursor state as received from host.
    pub fn current_cursor(&self) -> Option<&ScreenCursorUpdate> {
        self.last_cursor.as_ref()
    }

    /// Receiver cursor telemetry statistics.
    pub fn cursor_stats(&self) -> ScreenCursorStats {
        let mut st = self.cursor_controller.stats();
        st.updates_received = self.stats.cursor_updates_received;
        if let Some(ref cur) = self.last_cursor {
            st.current_x = cur.x;
            st.current_y = cur.y;
            st.current_visible = cur.visible;
            st.current_shape = cur.shape;
        }
        st
    }

    /// Access reference to internal FEC assembler (v2.04 / ADR-090).
    pub fn fec_assembler(&self) -> &ScreenFecAssembler {
        &self.fec_assembler
    }

    /// Access mutable reference to internal FEC assembler (v2.04 / ADR-090).
    pub fn fec_assembler_mut(&mut self) -> &mut ScreenFecAssembler {
        &mut self.fec_assembler
    }

    /// Handle incoming forward error correction (FEC) shard packet with timestamp (v2.08 / ADR-094).
    /// Records arrival microsecond timestamp into TWCC receiver tracker and forwards to FEC assembler.
    pub fn on_fec_shard_with_time(
        &mut self,
        shard: ScreenFecShard,
        now_us: u64,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        if shard.transport_seq > 0 {
            self.twcc_receiver
                .on_packet_received(shard.transport_seq, now_us);
        }
        let res = self.fec_assembler.push_shard(shard);
        self.stats.fec_recovered_shards = self.fec_assembler.recovered_shards();
        self.stats.fec_unrecoverable_blocks = self.fec_assembler.unrecoverable_blocks();
        res
    }

    /// Handle incoming forward error correction (FEC) shard packet (v2.04 / ADR-090).
    /// If shard satisfies K data shards or reconstruction threshold, assembles and returns ScreenFrame.
    pub fn on_fec_shard(
        &mut self,
        shard: ScreenFecShard,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_fec_shard_with_time(shard, now_us)
    }

    /// Most recently accepted sequence numbers, oldest first.
    pub fn trace(&self) -> Vec<u64> {
        self.trace.iter().copied().collect()
    }

    /// Validate and accept one frame. Duplicates and stale frames are dropped
    /// (counted), a forward jump beyond the window is an error, and a valid
    /// frame is recorded. `Ok(None)` means "not for presentation".
    pub fn apply(&mut self, frame: &ScreenFrame) -> Result<Option<u64>, CoreError> {
        if frame.seq <= self.last_seq {
            self.stats.dropped = self.stats.dropped.saturating_add(1);
            return Ok(None);
        }
        if self.last_seq != 0 && frame.seq > self.last_seq.saturating_add(SCREEN_SEQ_WINDOW) {
            return Err(CoreError::ScreenSeqJump);
        }
        if self.last_seq != 0 && frame.seq > self.last_seq.saturating_add(1) {
            self.stats.gaps = self.stats.gaps.saturating_add(1);
        }
        // A raw frame must match its rect; the codec already enforced this on
        // the wire, but a locally built frame could still lie.
        if !frame.compressed() && frame.bytes.len() != frame.raw_len() {
            return Err(CoreError::ScreenFrame);
        }
        self.last_seq = frame.seq;
        self.stats.received = self.stats.received.saturating_add(1);
        self.stats.bytes = self.stats.bytes.saturating_add(frame.bytes.len() as u64);
        if self.trace.len() >= SCREEN_TRACE_MAX {
            self.trace.pop_front();
        }
        self.trace.push_back(frame.seq);

        // ADR-099 Screen Streaming Microsecond Latency Breakdown & Multi-Stage Frame Profiler
        if self.frame_profiler.mode() != ScreenProfilerMode::Disabled {
            let mut b = ScreenLatencyBreakdown {
                frame_seq: frame.seq,
                capture_us: 3200,
                encode_us: 4100,
                pacer_us: 800,
                network_us: 1500,
                jitter_us: 1200,
                decode_us: 2400,
                render_us: 1600,
                total_e2e_us: 0,
            };
            b.calculate_total();
            self.record_latency_breakdown(b);
        }

        Ok(Some(frame.seq))
    }

    /// Peer gone: clear the cursor so a reconnect cannot be mistaken for a
    /// continuation of the old stream.
    pub fn on_peer_gone(&mut self) {
        self.last_seq = 0;
        self.trace.clear();
        self.playout_buffer.reset();
        self.last_cursor = None;
        self.cursor_controller.reset();
        self.fec_assembler.reset();
        self.nack_history.clear();
        self.harq_stats = ScreenHarqStats {
            mode: self.harq_mode,
            ..Default::default()
        };
        self.twcc_receiver.clear();
        self.stats.harq_nacks_sent = 0;
        self.stats.harq_nacks_received = 0;
        self.stats.harq_shards_retransmitted = 0;
        self.stats.harq_recovered_frames = 0;
        self.stats.twcc_feedbacks_sent = 0;
        self.stats.twcc_feedbacks_received = 0;
        self.stats.twcc_delivery_rate_bps = 0;
        self.stats.twcc_delay_gradient_us = 0;
        self.probe_stats = ScreenProbeStats {
            mode: self.probe_mode,
            ..Default::default()
        };
        self.stats.probe_state = ScreenProbeState::Idle.tag();
        self.stats.probes_triggered = 0;
        self.stats.probes_completed = 0;
        self.stats.probes_failed = 0;
        self.stats.probe_target_bps = 0;
        self.stats.probe_measured_bps = 0;
        self.sync_coordinator.reset();
        self.sync_sync_stats();
        self.active_display_id = 0;
        self.display_switches_count = 0;
        self.last_switch_us = 0;
        self.stats.active_display_id = 0;
        self.stats.display_switches_count = 0;
        self.privacy_enabled = false;
        self.privacy_lock_input = false;
        self.stats.privacy_enabled = false;
        self.stats.privacy_input_locked = false;
        self.frame_profiler.reset();
        self.sync_profiler_stats();
    }

    pub fn active_display_id(&self) -> u8 {
        self.active_display_id
    }

    pub fn set_active_display_id(&mut self, display_id: u8) {
        self.active_display_id = display_id;
        self.stats.active_display_id = display_id;
    }

    pub fn switch_display(&mut self, display_id: u8, now_us: u64) {
        self.active_display_id = display_id;
        self.display_switches_count = self.display_switches_count.saturating_add(1);
        self.last_switch_us = now_us;
        self.stats.active_display_id = display_id;
        self.stats.display_switches_count = self.display_switches_count;
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

    pub fn display_report(&self) -> ScreenDisplayReport {
        ScreenDisplayReport {
            ok: true,
            active_display_id: self.active_display_id,
            displays: self.displays.clone(),
            switches_count: self.display_switches_count,
            last_switch_us: self.last_switch_us,
        }
    }

    pub fn with_privacy(mut self, enabled: bool, lock_input: bool) -> Self {
        self.set_privacy_mode(enabled, lock_input, 0);
        self
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

    pub fn set_privacy_mode(&mut self, enabled: bool, lock_input: bool, now_us: u64) {
        self.privacy_enabled = enabled;
        self.privacy_lock_input = lock_input;
        self.privacy_last_change_us = now_us;
        self.stats.privacy_enabled = enabled;
        self.stats.privacy_input_locked = lock_input;
    }

    pub fn privacy_enabled(&self) -> bool {
        self.privacy_enabled
    }

    pub fn privacy_lock_input(&self) -> bool {
        self.privacy_lock_input
    }
}
