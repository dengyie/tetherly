// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Session state, rate/fps/scale/quality adaptation and the adaptation matrix.

use super::*;
use crate::error::CoreError;
use std::collections::VecDeque;

/// Where a screen session currently sits. `Allowed` is only ever reached by a
/// local, explicit action (`allow`), never by anything a peer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// No peer on the channel, or the channel is closed.
    Idle,
    /// A trusted peer opened the channel but this machine has not consented.
    Requested,
    /// Consent given, waiting for the controller's `Start`.
    Allowed,
    /// Actively sending frames.
    Streaming,
}

impl ScreenState {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenState::Idle => "idle",
            ScreenState::Requested => "requested",
            ScreenState::Allowed => "allowed",
            ScreenState::Streaming => "streaming",
        }
    }

    pub fn is_streaming(self) -> bool {
        matches!(self, ScreenState::Streaming)
    }
}

/// Running counters for the local UI. Cheap to read; never carries pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenStats {
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
    pub bytes: u64,
    /// Control messages refused because consent had not been given.
    pub refused: u64,
    /// Number of frames acknowledged by the receiver (v1.93 / ADR-080).
    pub acked: u64,
    /// Highest sequence number acknowledged so far.
    pub last_acked_seq: u64,
    /// Current number of frames in flight (sent but not yet acked).
    pub in_flight: u64,
    /// Latest measured round-trip time in microseconds.
    pub rtt_latest_us: Option<u64>,
    /// 95th-percentile round-trip time in microseconds over recent window.
    pub rtt_p95_us: Option<u64>,
    /// Number of keyframe requests received/serviced (v1.95 / ADR-081).
    pub keyframes_requested: u64,
    /// Number of sequence gaps detected by receiver (v1.95 / ADR-081).
    pub gaps: u64,
    /// Target video bitrate in bits per second (v1.96 / ADR-082).
    pub bitrate_bps: u32,
    /// Target screen streaming frame rate in frames per second (v1.97 / ADR-083).
    pub fps_target: u32,
    /// Target screen resolution scale percentage (25..=100) (v1.98 / ADR-084).
    pub scale_pct: u32,
    /// Active screen content profile tag (0=text, 1=motion, 2=auto) (v1.99 / ADR-085).
    pub profile_tag: u8,
    /// Minimum quantization parameter boundary (0..=51) (v1.99 / ADR-085).
    pub qp_min: u8,
    /// Maximum quantization parameter boundary (0..=51) (v1.99 / ADR-085).
    pub qp_max: u8,
    /// 4D Autonomous Adaptation Coordinator mode (0=auto, 1=conservative, 2=manual) (v2.00 / ADR-086).
    pub matrix_mode: u8,
    /// Real-time assessed network health state (0=healthy, 1=warning, 2=congested, 3=critical) (v2.00 / ADR-086).
    pub network_health: u8,
    /// Receiver playout dejitter mode (0=adaptive, 1=zerolatency, 2=smooth) (v2.01 / ADR-087).
    pub playout_mode: u8,
    /// Target playout buffer delay in microseconds (v2.01 / ADR-087).
    pub playout_delay_us: u32,
    /// Measured interarrival jitter in microseconds (v2.01 / ADR-087).
    pub playout_jitter_us: u32,
    /// Hardware cursor decoupling mode (0=decoupled, 1=embedded, 2=hidden) (v2.02 / ADR-088).
    pub cursor_mode: u8,
    /// Last observed cursor X position in screen pixels (v2.02 / ADR-088).
    pub cursor_x: i32,
    /// Last observed cursor Y position in screen pixels (v2.02 / ADR-088).
    pub cursor_y: i32,
    /// Last observed cursor visibility (v2.02 / ADR-088).
    pub cursor_visible: bool,
    /// Last observed cursor shape tag (v2.02 / ADR-088).
    pub cursor_shape: u8,
    /// Total cursor updates sent by host (v2.02 / ADR-088).
    pub cursor_updates_sent: u64,
    /// Total cursor updates received by client (v2.02 / ADR-088).
    pub cursor_updates_received: u64,
    /// Squelch controller mode (0=auto, 1=aggressive, 2=disabled) (v2.03 / ADR-089).
    pub squelch_mode: u8,
    /// Indicates whether zero-motion squelch is actively suppressing video frames (v2.03 / ADR-089).
    pub is_squelched: bool,
    /// Cumulative count of video frames squelched/suppressed (v2.03 / ADR-089).
    pub squelched_frames: u64,
    /// Forward error correction (FEC) mode tag (0=auto, 1=conservative, 2=aggressive, 3=disabled) (v2.04 / ADR-090).
    pub fec_mode: u8,
    /// Active FEC redundancy percentage (0..=35%) (v2.04 / ADR-090).
    pub fec_redundancy_pct: u8,
    /// Total FEC lost shards successfully reconstructed by receiver (v2.04 / ADR-090).
    pub fec_recovered_shards: u64,
    /// Total FEC blocks that experienced unrecoverable loss exceeding parity budget (v2.04 / ADR-090).
    pub fec_unrecoverable_blocks: u64,
    /// Microsecond packet pacer mode tag (0=auto, 1=aggressive, 2=disabled) (v2.05 / ADR-091).
    pub pacer_mode: u8,
    /// Total packets passed through the microsecond pacer (v2.05 / ADR-091).
    pub pacer_packets_paced: u64,
    /// Total micro-bursts smoothed across the frame interval (v2.05 / ADR-091).
    pub pacer_bursts_smoothed: u64,
    /// Average packet pacing delay in microseconds (v2.05 / ADR-091).
    pub pacer_avg_delay_us: u64,
    /// Delay-based trendline filter mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.06 / ADR-092).
    pub trendline_mode: u8,
    /// Delay-based bandwidth usage state tag (0=normal, 1=overusing, 2=underusing) (v2.06 / ADR-092).
    pub trendline_state: u8,
    /// Estimated queue delay gradient slope in microseconds per frame (v2.06 / ADR-092).
    pub trendline_slope_us: f64,
    /// Total bufferbloat overuse events detected (v2.06 / ADR-092).
    pub trendline_overuse_count: u64,
    /// Hybrid ARQ (HARQ) selective retransmission mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.07 / ADR-093).
    pub harq_mode: u8,
    /// Cumulative count of NACK requests sent by receiver (v2.07 / ADR-093).
    pub harq_nacks_sent: u64,
    /// Cumulative count of NACK requests received by sender (v2.07 / ADR-093).
    pub harq_nacks_received: u64,
    /// Total video shards/fragments retransmitted by sender (v2.07 / ADR-093).
    pub harq_shards_retransmitted: u64,
    /// Total frames successfully recovered via selective retransmission (v2.07 / ADR-093).
    pub harq_recovered_frames: u64,
    /// Transport-Wide Congestion Control (TWCC) mode tag (0=auto, 1=conservative, 2=aggressive, 3=disabled) (v2.08 / ADR-094).
    pub twcc_mode: u8,
    /// Cumulative count of TWCC feedback packets sent by receiver (v2.08 / ADR-094).
    pub twcc_feedbacks_sent: u64,
    /// Cumulative count of TWCC feedback packets received by sender (v2.08 / ADR-094).
    pub twcc_feedbacks_received: u64,
    /// Estimated network delivery rate in bits per second derived from TWCC arrival deltas (v2.08 / ADR-094).
    pub twcc_delivery_rate_bps: u64,
    /// Inter-packet queuing delay gradient in microseconds (v2.08 / ADR-094).
    pub twcc_delay_gradient_us: i64,
    /// Active bandwidth probing mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.09 / ADR-095).
    pub probe_mode: u8,
    /// Active bandwidth probing state tag (0=idle, 1=probing, 2=awaiting_feedback, 3=completed, 4=failed) (v2.09 / ADR-095).
    pub probe_state: u8,
    /// Cumulative count of bandwidth probe clusters triggered (v2.09 / ADR-095).
    pub probes_triggered: u64,
    /// Cumulative count of bandwidth probe clusters successfully completed (v2.09 / ADR-095).
    pub probes_completed: u64,
    /// Cumulative count of bandwidth probe clusters failed (v2.09 / ADR-095).
    pub probes_failed: u64,
    /// Last target probing bitrate in bits per second (v2.09 / ADR-095).
    pub probe_target_bps: u32,
    /// Last measured delivery rate during probe in bits per second (v2.09 / ADR-095).
    pub probe_measured_bps: u64,
    /// Audio-video lip-sync and clock synchronization mode tag (0=auto, 1=strict, 2=loose, 3=disabled) (v2.10 / ADR-096).
    pub sync_mode: u8,
    /// Cumulative count of sync beacons sent (v2.10 / ADR-096).
    pub sync_beacons_sent: u64,
    /// Cumulative count of sync beacons received (v2.10 / ADR-096).
    pub sync_beacons_received: u64,
    /// Latest measured presentation timestamp (PTS) skew in microseconds (video PTS - audio PTS) (v2.10 / ADR-096).
    pub sync_skew_us: i64,
    /// Estimated sender vs receiver hardware clock drift in parts per million (PPM) (v2.10 / ADR-096).
    pub sync_clock_drift_ppm: f64,
    /// Total playout target delay adjustments applied for lip-sync alignment (v2.10 / ADR-096).
    pub sync_adjustments_applied: u64,
    /// Indicates whether audio and video presentation timestamps are currently within tolerance (v2.10 / ADR-096).
    pub in_sync: bool,
    /// Active display ID currently selected for streaming capture (v2.11 / ADR-097).
    pub active_display_id: u8,
    /// Total count of available displays discovered on the host (v2.11 / ADR-097).
    pub display_count: u8,
    /// Total number of dynamic display switches executed (v2.11 / ADR-097).
    pub display_switches_count: u64,
    /// Whether screen privacy mode (curtain blanking) is active on the host (v2.12 / ADR-098).
    pub privacy_enabled: bool,
    /// Whether local physical input lockout is active on the host (v2.12 / ADR-098).
    pub privacy_input_locked: bool,
    /// Latency profiler mode tag (0=auto, 1=detailed, 2=disabled) (v2.13 / ADR-099).
    pub latency_mode: u8,
    /// Primary bottleneck stage tag (0=capture, 1=encode, 2=pacer, 3=network, 4=jitter, 5=decode, 6=render, 7=balanced) (v2.13 / ADR-099).
    pub latency_bottleneck: u8,
    /// Latest / EWMA total end-to-end latency in microseconds (v2.13 / ADR-099).
    pub latency_total_e2e_us: u64,
    /// 95th-percentile total end-to-end latency in microseconds (v2.13 / ADR-099).
    pub latency_p95_e2e_us: u64,
    /// Total frames profiled by the multi-stage latency profiler (v2.13 / ADR-099).
    pub latency_frames_profiled: u64,
    /// Bottleneck-driven autonomous tuner mode tag (0=disabled, 1=standard, 2=aggressive) (v2.14 / ADR-100).
    pub tuning_mode: u8,
    /// Cumulative count of autonomous tuning actions applied (v2.14 / ADR-100).
    pub tuning_actions_total: u64,
    /// Cumulative count of autonomous tuning recovery steps applied (v2.14 / ADR-100).
    pub tuning_recoveries: u64,
    /// Bottleneck stage tag of the most recent tuning action (v2.14 / ADR-100).
    pub tuning_last_stage: u8,
    /// Action tag of the most recent tuning action (v2.14 / ADR-100).
    pub tuning_last_action: u8,
}

impl Default for ScreenStats {
    fn default() -> Self {
        Self {
            sent: 0,
            received: 0,
            dropped: 0,
            bytes: 0,
            refused: 0,
            acked: 0,
            last_acked_seq: 0,
            in_flight: 0,
            rtt_latest_us: None,
            rtt_p95_us: None,
            keyframes_requested: 0,
            gaps: 0,
            bitrate_bps: 0,
            fps_target: 0,
            scale_pct: 0,
            profile_tag: 0,
            qp_min: 0,
            qp_max: 0,
            matrix_mode: 0,
            network_health: 0,
            playout_mode: 0,
            playout_delay_us: 0,
            playout_jitter_us: 0,
            cursor_mode: 0,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
            cursor_shape: 0,
            cursor_updates_sent: 0,
            cursor_updates_received: 0,
            squelch_mode: 0,
            is_squelched: false,
            squelched_frames: 0,
            fec_mode: 0,
            fec_redundancy_pct: 0,
            fec_recovered_shards: 0,
            fec_unrecoverable_blocks: 0,
            pacer_mode: 0,
            pacer_packets_paced: 0,
            pacer_bursts_smoothed: 0,
            pacer_avg_delay_us: 0,
            trendline_mode: 0,
            trendline_state: 0,
            trendline_slope_us: 0.0,
            trendline_overuse_count: 0,
            harq_mode: 0,
            harq_nacks_sent: 0,
            harq_nacks_received: 0,
            harq_shards_retransmitted: 0,
            harq_recovered_frames: 0,
            twcc_mode: 0,
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: 0,
            probe_state: 0,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: 0,
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            privacy_enabled: false,
            privacy_input_locked: false,
            latency_mode: 0,
            latency_bottleneck: 7,
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
            tuning_mode: 1,
            tuning_actions_total: 0,
            tuning_recoveries: 0,
            tuning_last_stage: 7,
            tuning_last_action: 0,
        }
    }
}

impl ScreenStats {
    /// Latency profiler mode.
    pub fn latency_mode(&self) -> ScreenProfilerMode {
        ScreenProfilerMode::from_tag(self.latency_mode)
    }

    /// Latency bottleneck stage.
    pub fn latency_bottleneck(&self) -> ScreenBottleneckStage {
        ScreenBottleneckStage::from_tag(self.latency_bottleneck)
    }
    /// Audio-video lip-sync and clock synchronization mode.
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        ScreenAudioSyncMode::from_tag(self.sync_mode)
    }

    /// Active bandwidth probing mode.
    pub fn probe_mode(&self) -> ScreenProbeMode {
        ScreenProbeMode::from_tag(self.probe_mode)
    }

    /// Active bandwidth probing state.
    pub fn probe_state(&self) -> ScreenProbeState {
        ScreenProbeState::from_tag(self.probe_state)
    }

    /// Transport-Wide Congestion Control (TWCC) mode.
    pub fn twcc_mode(&self) -> ScreenTwccMode {
        ScreenTwccMode::from_tag(self.twcc_mode)
    }

    /// Hybrid ARQ (HARQ) selective retransmission mode.
    pub fn harq_mode(&self) -> ScreenHarqMode {
        ScreenHarqMode::from_tag(self.harq_mode)
    }

    /// Delay-based trendline filter mode.
    pub fn trendline_mode(&self) -> ScreenTrendlineMode {
        ScreenTrendlineMode::from_tag(self.trendline_mode)
    }

    /// Delay-based bandwidth usage state.
    pub fn trendline_state(&self) -> ScreenBandwidthUsageState {
        ScreenBandwidthUsageState::from_tag(self.trendline_state)
    }

    /// Microsecond packet pacer mode.
    pub fn pacer_mode(&self) -> ScreenPacerMode {
        ScreenPacerMode::from_tag(self.pacer_mode)
    }

    /// Forward error correction (FEC) mode.
    pub fn fec_mode(&self) -> ScreenFecMode {
        ScreenFecMode::from_tag(self.fec_mode)
    }

    /// Squelch controller mode.
    pub fn squelch_mode(&self) -> ScreenSquelchMode {
        ScreenSquelchMode::from_tag(self.squelch_mode)
    }

    /// Active screen content profile resolved from profile tag.
    pub fn content_profile(&self) -> ScreenContentProfile {
        ScreenContentProfile::from_tag(self.profile_tag)
    }

    /// Autonomous 4D matrix coordinator mode.
    pub fn matrix_mode(&self) -> ScreenMatrixMode {
        ScreenMatrixMode::from_tag(self.matrix_mode)
    }

    /// Real-time assessed network health state.
    pub fn network_health(&self) -> NetworkHealth {
        NetworkHealth::from_tag(self.network_health)
    }

    /// Receiver playout dejitter mode.
    pub fn playout_mode(&self) -> ScreenPlayoutMode {
        ScreenPlayoutMode::from_tag(self.playout_mode)
    }

    /// Hardware cursor decoupling mode.
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        ScreenCursorMode::from_tag(self.cursor_mode)
    }

    /// Cursor shape.
    pub fn cursor_shape(&self) -> CursorShape {
        CursorShape::from_tag(self.cursor_shape)
    }
}

/// Rolling window tracker for screen frame round-trip times (v1.93 / ADR-080).
#[derive(Debug, Clone)]
pub struct ScreenRttTracker {
    pub(super) samples: VecDeque<u64>,
    pub(super) capacity: usize,
}

impl ScreenRttTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity.min(256)),
            capacity: capacity.max(8),
        }
    }

    pub fn record(&mut self, sample_us: u64) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample_us);
    }

    pub fn latest(&self) -> Option<u64> {
        self.samples.back().copied()
    }

    pub fn p95_us(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        Some(sorted[idx.min(sorted.len() - 1)])
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Dynamic video rate controller using AIMD (Additive Increase, Multiplicative Decrease)
/// adaptation based on live RTT, frame loss, and in-flight queue telemetry (v1.96 / ADR-082).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenRateController {
    pub(super) current_bps: u32,
    pub(super) min_bps: u32,
    pub(super) max_bps: u32,
    pub(super) last_decrease_us: u64,
    pub(super) consecutive_good_frames: u32,
}

impl Default for ScreenRateController {
    fn default() -> Self {
        Self::new(SCREEN_BITRATE_DEFAULT_BPS)
    }
}

impl ScreenRateController {
    pub fn new(initial_bps: u32) -> Self {
        let clamped = initial_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);
        Self {
            current_bps: clamped,
            min_bps: SCREEN_BITRATE_MIN_BPS,
            max_bps: SCREEN_BITRATE_MAX_BPS,
            last_decrease_us: 0,
            consecutive_good_frames: 0,
        }
    }

    pub fn with_bounds(initial_bps: u32, min_bps: u32, max_bps: u32) -> Self {
        let min = min_bps.max(SCREEN_BITRATE_MIN_BPS);
        let max = max_bps.clamp(min, SCREEN_BITRATE_MAX_BPS);
        let clamped = initial_bps.clamp(min, max);
        Self {
            current_bps: clamped,
            min_bps: min,
            max_bps: max,
            last_decrease_us: 0,
            consecutive_good_frames: 0,
        }
    }

    pub fn current_bps(&self) -> u32 {
        self.current_bps
    }

    pub fn set_manual_bps(&mut self, bps: u32) {
        self.current_bps = bps.clamp(self.min_bps, self.max_bps);
        self.consecutive_good_frames = 0;
    }

    /// Trigger multiplicative decrease on congestion signals (RTT spike, packet drop, or gap).
    /// Returns true if bitrate was decreased.
    pub fn on_congestion(&mut self, now_us: u64) -> bool {
        self.consecutive_good_frames = 0;
        if self.last_decrease_us != 0
            && now_us.saturating_sub(self.last_decrease_us) < SCREEN_BITRATE_COOLDOWN_US
        {
            return false;
        }
        self.last_decrease_us = now_us;
        let old = self.current_bps;
        // Multiplicative decrease (MD): scale down to 80%
        let decreased = ((self.current_bps as u64 * 80) / 100) as u32;
        self.current_bps = decreased.max(self.min_bps);
        self.current_bps != old
    }

    /// Force an immediate multiplicative step down regardless of cooldown (used by TWCC on acute congestion).
    pub fn force_step_down(&mut self) -> bool {
        self.consecutive_good_frames = 0;
        let old = self.current_bps;
        let decreased = ((self.current_bps as u64 * 80) / 100) as u32;
        self.current_bps = decreased.max(self.min_bps);
        self.current_bps != old
    }

    /// Fast ramp-up to a verified probe bitrate (v2.09 / ADR-095).
    /// Clamps target to [min_bps, max_bps] and steps up immediately without waiting for additive increase steps.
    pub fn fast_ramp_up(&mut self, target_bps: u32) -> bool {
        let clamped = target_bps.clamp(self.min_bps, self.max_bps);
        if clamped > self.current_bps {
            self.current_bps = clamped;
            self.consecutive_good_frames = 0;
            true
        } else {
            false
        }
    }

    /// Process a frame acknowledgment with measured RTT and in-flight count.
    /// Returns true if bitrate changed.
    pub fn on_ack(&mut self, rtt_us: u64, in_flight: u64, now_us: u64) -> bool {
        if in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || rtt_us >= SCREEN_CONGESTION_RTT_THRESHOLD_US
        {
            return self.on_congestion(now_us);
        }

        // Low latency and clear network: accumulate consecutive good frames
        if in_flight == 0 && rtt_us < 120_000 {
            self.consecutive_good_frames = self.consecutive_good_frames.saturating_add(1);
            if self.consecutive_good_frames >= 5 {
                self.consecutive_good_frames = 0;
                let old = self.current_bps;
                self.current_bps = self
                    .current_bps
                    .saturating_add(SCREEN_BITRATE_STEP_INCREASE_BPS)
                    .min(self.max_bps);
                return self.current_bps != old;
            }
        } else {
            self.consecutive_good_frames = 0;
        }
        false
    }
}

/// Dynamic screen frame pacing controller managing target frame rates and inter-frame pacing intervals (v1.97 / ADR-083).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFpsPacer {
    pub(super) fps_target: u32,
    pub(super) min_fps: u32,
    pub(super) max_fps: u32,
    pub(super) interval_us: u64,
    pub(super) last_frame_us: u64,
}

impl Default for ScreenFpsPacer {
    fn default() -> Self {
        Self::new(SCREEN_FPS_DEFAULT)
    }
}

impl ScreenFpsPacer {
    pub fn new(initial_fps: u32) -> Self {
        let clamped = initial_fps.clamp(SCREEN_FPS_MIN, SCREEN_FPS_MAX);
        let interval = 1_000_000u64 / (clamped as u64);
        Self {
            fps_target: clamped,
            min_fps: SCREEN_FPS_MIN,
            max_fps: SCREEN_FPS_MAX,
            interval_us: interval,
            last_frame_us: 0,
        }
    }

    pub fn with_bounds(initial_fps: u32, min_fps: u32, max_fps: u32) -> Self {
        let min = min_fps.max(SCREEN_FPS_MIN);
        let max = max_fps.clamp(min, SCREEN_FPS_MAX);
        let clamped = initial_fps.clamp(min, max);
        let interval = 1_000_000u64 / (clamped as u64);
        Self {
            fps_target: clamped,
            min_fps: min,
            max_fps: max,
            interval_us: interval,
            last_frame_us: 0,
        }
    }

    pub fn fps_target(&self) -> u32 {
        self.fps_target
    }

    pub fn interval_us(&self) -> u64 {
        self.interval_us
    }

    pub fn min_fps(&self) -> u32 {
        self.min_fps
    }

    pub fn max_fps(&self) -> u32 {
        self.max_fps
    }

    pub fn set_fps(&mut self, fps: u32) -> u32 {
        let clamped = fps.clamp(self.min_fps, self.max_fps);
        self.fps_target = clamped;
        self.interval_us = 1_000_000u64 / (clamped as u64);
        clamped
    }

    /// Check if enough time has elapsed to send the next frame under the target pacing.
    /// If ready, updates last_frame_us and returns true. If not, returns false.
    pub fn check_and_consume(&mut self, now_us: u64) -> bool {
        if self.last_frame_us == 0 || now_us.saturating_sub(self.last_frame_us) >= self.interval_us
        {
            self.last_frame_us = now_us;
            true
        } else {
            false
        }
    }

    /// Peek if enough time has elapsed without consuming the tick.
    pub fn is_ready(&self, now_us: u64) -> bool {
        self.last_frame_us == 0 || now_us.saturating_sub(self.last_frame_us) >= self.interval_us
    }

    pub fn reset(&mut self) {
        self.last_frame_us = 0;
    }
}

/// Dynamic screen resolution downscaling controller managing target scale percentage (v1.98 / ADR-084).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenScaleController {
    pub(super) scale_pct: u32,
    pub(super) min_scale_pct: u32,
    pub(super) max_scale_pct: u32,
}

impl Default for ScreenScaleController {
    fn default() -> Self {
        Self::new(SCREEN_SCALE_DEFAULT_PCT)
    }
}

impl ScreenScaleController {
    pub fn new(initial_pct: u32) -> Self {
        let clamped = initial_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
        Self {
            scale_pct: clamped,
            min_scale_pct: SCREEN_SCALE_MIN_PCT,
            max_scale_pct: SCREEN_SCALE_MAX_PCT,
        }
    }

    pub fn with_bounds(initial_pct: u32, min_pct: u32, max_pct: u32) -> Self {
        let min = min_pct.max(SCREEN_SCALE_MIN_PCT);
        let max = max_pct.clamp(min, SCREEN_SCALE_MAX_PCT);
        let clamped = initial_pct.clamp(min, max);
        Self {
            scale_pct: clamped,
            min_scale_pct: min,
            max_scale_pct: max,
        }
    }

    pub fn scale_pct(&self) -> u32 {
        self.scale_pct
    }

    pub fn min_scale_pct(&self) -> u32 {
        self.min_scale_pct
    }

    pub fn max_scale_pct(&self) -> u32 {
        self.max_scale_pct
    }

    pub fn set_scale_pct(&mut self, pct: u32) -> u32 {
        let clamped = pct.clamp(self.min_scale_pct, self.max_scale_pct);
        self.scale_pct = clamped;
        clamped
    }

    pub fn compute_scaled_dims(&self, orig_w: u16, orig_h: u16) -> (u16, u16) {
        Self::scale_dims(orig_w, orig_h, self.scale_pct)
    }

    /// Calculate scaled width and height ensuring even dimensions (minimum 2x2) for YUV/H.264 codecs.
    pub fn scale_dims(orig_w: u16, orig_h: u16, scale_pct: u32) -> (u16, u16) {
        let clamped = scale_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
        if clamped >= 100 {
            return (orig_w, orig_h);
        }
        let w = (orig_w as u32 * clamped / 100) as u16;
        let h = (orig_h as u32 * clamped / 100) as u16;
        let even_w = (w & !1).max(2);
        let even_h = (h & !1).max(2);
        (even_w, even_h)
    }
}

/// Dynamic screen quality & content profile controller (v1.99 / ADR-085).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenQualityController {
    pub(super) profile: ScreenContentProfile,
    pub(super) min_qp: u8,
    pub(super) max_qp: u8,
    pub(super) custom_qp: bool,
}

impl Default for ScreenQualityController {
    fn default() -> Self {
        Self::new(ScreenContentProfile::default())
    }
}

impl ScreenQualityController {
    pub fn new(profile: ScreenContentProfile) -> Self {
        let (min_qp, max_qp) = profile.default_qp_range();
        Self {
            profile,
            min_qp,
            max_qp,
            custom_qp: false,
        }
    }

    pub fn with_qp(profile: ScreenContentProfile, min_qp: u8, max_qp: u8) -> Self {
        let min = min_qp.clamp(SCREEN_QP_ABSOLUTE_MIN, SCREEN_QP_ABSOLUTE_MAX);
        let max = max_qp.clamp(min, SCREEN_QP_ABSOLUTE_MAX);
        Self {
            profile,
            min_qp: min,
            max_qp: max,
            custom_qp: true,
        }
    }

    pub fn profile(&self) -> ScreenContentProfile {
        self.profile
    }

    pub fn set_profile(&mut self, profile: ScreenContentProfile) -> ScreenContentProfile {
        self.profile = profile;
        if !self.custom_qp {
            let (min, max) = profile.default_qp_range();
            self.min_qp = min;
            self.max_qp = max;
        }
        self.profile
    }

    pub fn min_qp(&self) -> u8 {
        self.min_qp
    }

    pub fn max_qp(&self) -> u8 {
        self.max_qp
    }

    pub fn qp_range(&self) -> (u8, u8) {
        (self.min_qp, self.max_qp)
    }

    pub fn set_qp_range(&mut self, min_qp: u8, max_qp: u8) -> (u8, u8) {
        let min = min_qp.clamp(SCREEN_QP_ABSOLUTE_MIN, SCREEN_QP_ABSOLUTE_MAX);
        let max = max_qp.clamp(min, SCREEN_QP_ABSOLUTE_MAX);
        self.min_qp = min;
        self.max_qp = max;
        self.custom_qp = true;
        (min, max)
    }

    pub fn reset_default_qp(&mut self) -> (u8, u8) {
        let (min, max) = self.profile.default_qp_range();
        self.min_qp = min;
        self.max_qp = max;
        self.custom_qp = false;
        (min, max)
    }

    pub fn custom_qp(&self) -> bool {
        self.custom_qp
    }

    pub fn is_custom(&self) -> bool {
        self.custom_qp
    }

    /// Resolve effective content profile for encoder configuration (translating Auto to Text for desktop sharing).
    pub fn effective_profile(&self) -> ScreenContentProfile {
        match self.profile {
            ScreenContentProfile::Auto => ScreenContentProfile::Text,
            other => other,
        }
    }
}

/// Real-time assessed network health state for autonomous adaptation (v2.00 / ADR-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum NetworkHealth {
    #[default]
    Healthy = 0,
    Warning = 1,
    Congested = 2,
    Critical = 3,
}

impl NetworkHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkHealth::Healthy => "healthy",
            NetworkHealth::Warning => "warning",
            NetworkHealth::Congested => "congested",
            NetworkHealth::Critical => "critical",
        }
    }

    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => NetworkHealth::Healthy,
            1 => NetworkHealth::Warning,
            2 => NetworkHealth::Congested,
            3 => NetworkHealth::Critical,
            _ => NetworkHealth::Healthy,
        }
    }
}

/// Operation mode of the 4-dimensional autonomous adaptation coordinator (v2.00 / ADR-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenMatrixMode {
    /// Autonomous 4D cross-layer coordination across bitrate, FPS, scale, and QP.
    #[default]
    Auto = 0,
    /// Conservative mode: adapts bitrate and FPS, preserves native spatial scale.
    Conservative = 1,
    /// Manual mode: coordinator passive; parameters modulated only by explicit user/API calls.
    Manual = 2,
}

impl ScreenMatrixMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenMatrixMode::Auto => "auto",
            ScreenMatrixMode::Conservative => "conservative",
            ScreenMatrixMode::Manual => "manual",
        }
    }

    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenMatrixMode::Auto,
            1 => ScreenMatrixMode::Conservative,
            2 => ScreenMatrixMode::Manual,
            _ => ScreenMatrixMode::Auto,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        let lower = s.trim().to_ascii_lowercase();
        match lower.as_str() {
            "auto" | "dynamic" => Some(ScreenMatrixMode::Auto),
            "conservative" | "preserve_scale" | "scale_locked" => {
                Some(ScreenMatrixMode::Conservative)
            }
            "manual" | "disabled" | "off" => Some(ScreenMatrixMode::Manual),
            _ => None,
        }
    }
}

/// Minimum microsecond duration between automatic tier demotions (cooldown) (400ms).
pub const SCREEN_MATRIX_DEMOTE_COOLDOWN_US: u64 = 400_000;
/// Minimum microsecond duration of sustained healthy telemetry before promoting a tier (hysteresis) (3.0s).
pub const SCREEN_MATRIX_PROMOTE_COOLDOWN_US: u64 = 3_000_000;
/// Minimum consecutive healthy acknowledgements before considering an up-tier promotion.
pub const SCREEN_MATRIX_HEALTHY_ACKS_REQUIRED: u32 = 15;
/// Maximum consecutive frame drops before escalating to Critical health tier.
pub const SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL: u32 = 2;

/// Helper calculating next lower FPS tier during congestion.
pub fn screen_matrix_next_lower_fps(current_fps: u32) -> u32 {
    if current_fps > 30 {
        30
    } else if current_fps > 24 {
        24
    } else if current_fps > 15 {
        15
    } else {
        SCREEN_FPS_MIN
    }
}

/// Helper calculating next higher FPS tier during sustained network recovery.
pub fn screen_matrix_next_higher_fps(current_fps: u32, max_fps: u32) -> u32 {
    let next = if current_fps < 15 {
        15
    } else if current_fps < 24 {
        24
    } else if current_fps < 30 {
        30
    } else {
        60
    };
    next.min(max_fps)
}

/// Helper calculating next lower resolution scale tier during critical congestion.
pub fn screen_matrix_next_lower_scale(current_scale: u32) -> u32 {
    if current_scale > 75 {
        75
    } else if current_scale > 50 {
        50
    } else {
        25
    }
}

/// Helper calculating next higher resolution scale tier during sustained network recovery.
pub fn screen_matrix_next_higher_scale(current_scale: u32) -> u32 {
    if current_scale < 50 {
        50
    } else if current_scale < 75 {
        75
    } else {
        100
    }
}

/// Recommendation outcome from evaluating the 4D matrix coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenMatrixOutcome {
    pub health: NetworkHealth,
    pub new_fps: Option<u32>,
    pub new_scale_pct: Option<u32>,
    pub force_idr: bool,
}

/// 4-Dimensional closed-loop autonomous adaptation coordinator (v2.00 / ADR-086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenMatrixCoordinator {
    pub(super) mode: ScreenMatrixMode,
    pub(super) health: NetworkHealth,
    pub(super) auto_fps: bool,
    pub(super) auto_scale: bool,
    pub(super) consecutive_healthy_acks: u32,
    pub(super) consecutive_drops: u32,
    pub(super) last_tier_change_us: u64,
}

impl Default for ScreenMatrixCoordinator {
    fn default() -> Self {
        Self::new(ScreenMatrixMode::default())
    }
}

impl ScreenMatrixCoordinator {
    pub fn new(mode: ScreenMatrixMode) -> Self {
        Self {
            mode,
            health: NetworkHealth::Healthy,
            auto_fps: true,
            auto_scale: true,
            consecutive_healthy_acks: 0,
            consecutive_drops: 0,
            last_tier_change_us: 0,
        }
    }

    pub fn mode(&self) -> ScreenMatrixMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenMatrixMode) {
        self.mode = mode;
    }

    pub fn health(&self) -> NetworkHealth {
        self.health
    }

    pub fn auto_fps(&self) -> bool {
        self.auto_fps
    }

    pub fn set_auto_fps(&mut self, auto: bool) {
        self.auto_fps = auto;
    }

    pub fn auto_scale(&self) -> bool {
        self.auto_scale
    }

    pub fn set_auto_scale(&mut self, auto: bool) {
        self.auto_scale = auto;
    }

    pub fn consecutive_healthy_acks(&self) -> u32 {
        self.consecutive_healthy_acks
    }

    pub fn consecutive_drops(&self) -> u32 {
        self.consecutive_drops
    }

    /// Handle frame drop event with microsecond timestamp.
    pub fn on_drop(
        &mut self,
        now_us: u64,
        current_fps: u32,
        current_scale: u32,
    ) -> ScreenMatrixOutcome {
        self.consecutive_drops = self.consecutive_drops.saturating_add(1);
        self.consecutive_healthy_acks = 0;

        if self.consecutive_drops >= SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL {
            self.health = NetworkHealth::Critical;
        } else {
            self.health = NetworkHealth::Congested;
        }

        if self.mode == ScreenMatrixMode::Manual {
            return ScreenMatrixOutcome {
                health: self.health,
                new_fps: None,
                new_scale_pct: None,
                force_idr: false,
            };
        }

        let mut outcome = ScreenMatrixOutcome {
            health: self.health,
            new_fps: None,
            new_scale_pct: None,
            force_idr: false,
        };

        if now_us.saturating_sub(self.last_tier_change_us) >= SCREEN_MATRIX_DEMOTE_COOLDOWN_US {
            if self.health == NetworkHealth::Critical {
                // Demote both scale (if Auto mode) and FPS.
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale > 25 {
                    let next_scale = screen_matrix_next_lower_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                }
                if self.auto_fps && current_fps > SCREEN_FPS_MIN {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                }
                if outcome.new_scale_pct.is_some() || outcome.new_fps.is_some() {
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                }
            } else if self.health == NetworkHealth::Congested {
                // Demote FPS first to cut socket pressure.
                if self.auto_fps && current_fps > 15 {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                    self.last_tier_change_us = now_us;
                }
            }
        }

        outcome
    }

    /// Handle incoming frame acknowledgment with microsecond RTT and in-flight count.
    pub fn on_ack(
        &mut self,
        now_us: u64,
        rtt_us: u64,
        in_flight: u64,
        current_fps: u32,
        current_scale: u32,
        target_max_fps: u32,
    ) -> ScreenMatrixOutcome {
        // Assess current network health.
        if in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || rtt_us >= 500_000
            || self.consecutive_drops >= SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL
        {
            self.health = NetworkHealth::Critical;
            self.consecutive_healthy_acks = 0;
        } else if in_flight >= (SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD / 2)
            || rtt_us >= SCREEN_CONGESTION_RTT_THRESHOLD_US
            || self.consecutive_drops > 0
        {
            self.health = NetworkHealth::Congested;
            self.consecutive_healthy_acks = 0;
        } else if in_flight > 0 || rtt_us >= 120_000 {
            self.health = NetworkHealth::Warning;
            self.consecutive_healthy_acks = 0;
        } else {
            self.health = NetworkHealth::Healthy;
            self.consecutive_drops = 0;
            self.consecutive_healthy_acks = self.consecutive_healthy_acks.saturating_add(1);
        }

        if self.mode == ScreenMatrixMode::Manual {
            return ScreenMatrixOutcome {
                health: self.health,
                new_fps: None,
                new_scale_pct: None,
                force_idr: false,
            };
        }

        let mut outcome = ScreenMatrixOutcome {
            health: self.health,
            new_fps: None,
            new_scale_pct: None,
            force_idr: false,
        };

        if self.health == NetworkHealth::Healthy {
            // Check if hysteresis threshold is met for promotion.
            if self.consecutive_healthy_acks >= SCREEN_MATRIX_HEALTHY_ACKS_REQUIRED
                && now_us.saturating_sub(self.last_tier_change_us)
                    >= SCREEN_MATRIX_PROMOTE_COOLDOWN_US
            {
                // Recovery Ladder:
                // 1. Promote resolution scale first (if below 100%).
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale < 100 {
                    let next_scale = screen_matrix_next_higher_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                    self.consecutive_healthy_acks = 0;
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                } else if self.auto_fps && current_fps < target_max_fps {
                    // 2. Once scale is at 100%, promote FPS.
                    let next_fps = screen_matrix_next_higher_fps(current_fps, target_max_fps);
                    outcome.new_fps = Some(next_fps);
                    self.consecutive_healthy_acks = 0;
                    self.last_tier_change_us = now_us;
                }
            }
        } else if (self.health == NetworkHealth::Critical
            || self.health == NetworkHealth::Congested)
            && now_us.saturating_sub(self.last_tier_change_us) >= SCREEN_MATRIX_DEMOTE_COOLDOWN_US
        {
            // Under sustained high RTT / in-flight, demote if appropriate.
            if self.health == NetworkHealth::Critical {
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale > 25 {
                    let next_scale = screen_matrix_next_lower_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                }
                if self.auto_fps && current_fps > SCREEN_FPS_MIN {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                }
                if outcome.new_scale_pct.is_some() || outcome.new_fps.is_some() {
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                }
            } else if self.health == NetworkHealth::Congested && self.auto_fps && current_fps > 15 {
                let next_fps = screen_matrix_next_lower_fps(current_fps);
                outcome.new_fps = Some(next_fps);
                self.last_tier_change_us = now_us;
            }
        }

        outcome
    }
}

/// Bilinear downscaling for 32-bit BGRA/RGBA buffers with 16.16 fixed-point arithmetic (v1.98 / ADR-084).
pub fn downscale_bgra8(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    if src_w == dst_w && src_h == dst_h {
        return src.to_vec();
    }
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Vec::new();
    }
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    let x_ratio = ((src_w << 16) / dst_w) as u32;
    let y_ratio = ((src_h << 16) / dst_h) as u32;

    for dy in 0..dst_h {
        let sy_fp = (dy as u32) * y_ratio;
        let sy0 = (sy_fp >> 16) as usize;
        let sy1 = (sy0 + 1).min(src_h - 1);
        let y_weight = (sy_fp & 0xffff) >> 8;
        let y_weight_inv = 256 - y_weight;

        let dst_row_offset = dy * dst_w * 4;
        let src_row0_offset = sy0 * src_w * 4;
        let src_row1_offset = sy1 * src_w * 4;

        for dx in 0..dst_w {
            let sx_fp = (dx as u32) * x_ratio;
            let sx0 = (sx_fp >> 16) as usize;
            let sx1 = (sx0 + 1).min(src_w - 1);
            let x_weight = (sx_fp & 0xffff) >> 8;
            let x_weight_inv = 256 - x_weight;

            let p00_idx = src_row0_offset + sx0 * 4;
            let p10_idx = src_row0_offset + sx1 * 4;
            let p01_idx = src_row1_offset + sx0 * 4;
            let p11_idx = src_row1_offset + sx1 * 4;

            let dst_idx = dst_row_offset + dx * 4;

            for c in 0..4 {
                let v00 = src[p00_idx + c] as u32;
                let v10 = src[p10_idx + c] as u32;
                let v01 = src[p01_idx + c] as u32;
                let v11 = src[p11_idx + c] as u32;

                let top = (v00 * x_weight_inv + v10 * x_weight) >> 8;
                let bottom = (v01 * x_weight_inv + v11 * x_weight) >> 8;
                let val = (top * y_weight_inv + bottom * y_weight) >> 8;

                dst[dst_idx + c] = val.min(255) as u8;
            }
        }
    }
    dst
}

/// Downscale an uncompressed raw ScreenFrame according to target scale percentage (v1.98 / ADR-084).
pub fn downscale_frame(frame: &ScreenFrame, scale_pct: u32) -> Result<ScreenFrame, CoreError> {
    let clamped_pct = scale_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
    if clamped_pct >= 100 {
        return Ok(frame.clone());
    }
    if frame.compressed() {
        return Err(CoreError::ScreenFrame);
    }
    let (dst_w, dst_h) = ScreenScaleController::scale_dims(frame.rect.w, frame.rect.h, clamped_pct);
    if dst_w == frame.rect.w && dst_h == frame.rect.h {
        return Ok(frame.clone());
    }
    let expected_src_len = frame.rect.pixel_bytes(frame.format);
    if frame.bytes.len() != expected_src_len {
        return Err(CoreError::ScreenFrame);
    }
    let scaled_bytes = downscale_bgra8(
        &frame.bytes,
        frame.rect.w as usize,
        frame.rect.h as usize,
        dst_w as usize,
        dst_h as usize,
    );
    let new_rect = ScreenRect::new(frame.rect.x, frame.rect.y, dst_w, dst_h)?;
    Ok(ScreenFrame {
        seq: frame.seq,
        rect: new_rect,
        format: frame.format,
        codec_tag: frame.codec_tag,
        stride: dst_w as u32 * frame.format.bpp() as u32,
        bytes: scaled_bytes,
    })
}
