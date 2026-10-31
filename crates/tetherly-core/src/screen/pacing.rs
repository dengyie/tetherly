// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Packet pacer and bandwidth trendline filter.

use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;

pub const SCREEN_PACER_BURST_AUTO: usize = 4;
pub const SCREEN_PACER_BURST_AGGRESSIVE: usize = 2;
pub const SCREEN_PACER_MAX_DELAY_US: u64 = 50_000;

/// Screen streaming packet pacing mode (v2.05 / ADR-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScreenPacerMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Disabled = 2,
}

impl ScreenPacerMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Aggressive,
            2 => Self::Disabled,
            _ => Self::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }
}

impl std::str::FromStr for ScreenPacerMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "aggressive" | "agg" => Ok(Self::Aggressive),
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

impl std::fmt::Display for ScreenPacerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Statistics and counters for screen packet pacing (v2.05 / ADR-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenPacerStats {
    pub mode: ScreenPacerMode,
    pub packets_paced: u64,
    pub bursts_smoothed: u64,
    pub total_delay_us: u64,
    pub avg_delay_us: u64,
    pub last_delay_us: u64,
}

/// JSON report DTO for screen packet pacing (v2.05 / ADR-091).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenPacerReport {
    pub ok: bool,
    pub mode: String,
    pub packets_paced: u64,
    pub bursts_smoothed: u64,
    pub avg_delay_us: u64,
    pub last_delay_us: u64,
    pub timestamp_ms: u64,
}

/// Screen streaming microsecond-level packet pacer & burst shaper (v2.05 / ADR-091).
#[derive(Debug, Clone)]
pub struct ScreenPacketPacer {
    pub(super) mode: ScreenPacerMode,
    pub(super) stats: ScreenPacerStats,
}

impl Default for ScreenPacketPacer {
    fn default() -> Self {
        Self::new(ScreenPacerMode::Auto)
    }
}

impl ScreenPacketPacer {
    pub fn new(mode: ScreenPacerMode) -> Self {
        Self {
            mode,
            stats: ScreenPacerStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenPacerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenPacerMode) -> ScreenPacerMode {
        self.mode = mode;
        self.stats.mode = mode;
        mode
    }

    pub fn stats(&self) -> ScreenPacerStats {
        self.stats
    }

    /// Computes relative departure delay offsets (in microseconds) for a batch of N packets
    /// belonging to a single video frame, smoothing micro-bursts across the frame interval.
    pub fn compute_packet_delays(&mut self, packet_count: usize, fps_target: u32) -> Vec<u64> {
        if packet_count == 0 {
            return Vec::new();
        }
        if self.mode == ScreenPacerMode::Disabled {
            self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
            return vec![0u64; packet_count];
        }

        let burst_limit = match self.mode {
            ScreenPacerMode::Auto => SCREEN_PACER_BURST_AUTO,
            ScreenPacerMode::Aggressive => SCREEN_PACER_BURST_AGGRESSIVE,
            ScreenPacerMode::Disabled => packet_count,
        };

        if packet_count <= burst_limit {
            self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
            return vec![0u64; packet_count];
        }

        // Frame duration in microseconds
        let fps = fps_target.clamp(5, 120) as u64;
        let frame_duration_us = 1_000_000u64 / fps;

        // Pacing budget: 80% of frame duration in Auto, 100% in Aggressive
        let budget_us = match self.mode {
            ScreenPacerMode::Auto => (frame_duration_us * 8) / 10,
            ScreenPacerMode::Aggressive => frame_duration_us,
            ScreenPacerMode::Disabled => 0,
        };

        let paced_shards = packet_count - burst_limit;
        let step_us = (budget_us / (paced_shards as u64 + 1)).min(SCREEN_PACER_MAX_DELAY_US);

        let mut delays = Vec::with_capacity(packet_count);
        let mut burst_delay_sum = 0u64;

        for i in 0..packet_count {
            if i < burst_limit {
                delays.push(0);
            } else {
                let delay = ((i - burst_limit + 1) as u64 * step_us).min(SCREEN_PACER_MAX_DELAY_US);
                delays.push(delay);
                burst_delay_sum = burst_delay_sum.saturating_add(delay);
                self.stats.last_delay_us = delay;
            }
        }

        self.stats.bursts_smoothed = self.stats.bursts_smoothed.saturating_add(1);
        self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
        self.stats.total_delay_us = self.stats.total_delay_us.saturating_add(burst_delay_sum);
        if let Some(avg) = self
            .stats
            .total_delay_us
            .checked_div(self.stats.packets_paced)
        {
            self.stats.avg_delay_us = avg;
        }

        delays
    }
}

/// Maximum sliding window capacity for delay trendline filter (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_WINDOW_MAX: usize = 60;
/// Default sliding window capacity for delay trendline filter (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_WINDOW_DEFAULT: usize = 20;
/// Default trendline slope threshold in microseconds per frame for Auto mode (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME: f64 = 12.5;
/// Aggressive trendline slope threshold in microseconds per frame (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME: f64 = 6.0;
/// Conservative trendline slope threshold in microseconds per frame (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME: f64 = 25.0;
/// Minimum samples required before computing linear regression slope (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_MIN_SAMPLES: usize = 5;

/// Operating modes for the delay-based trendline bufferbloat filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTrendlineMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Conservative = 2,
    Disabled = 3,
}

impl ScreenTrendlineMode {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenTrendlineMode::Aggressive,
            2 => ScreenTrendlineMode::Conservative,
            3 => ScreenTrendlineMode::Disabled,
            _ => ScreenTrendlineMode::Auto,
        }
    }
}

impl std::str::FromStr for ScreenTrendlineMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenTrendlineMode::Auto),
            "aggressive" | "agg" => Ok(ScreenTrendlineMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenTrendlineMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenTrendlineMode::Disabled),
            _ => Err(CoreError::Json(format!(
                "unknown screen trendline mode: {s}"
            ))),
        }
    }
}

impl std::fmt::Display for ScreenTrendlineMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenTrendlineMode::Auto => write!(f, "auto"),
            ScreenTrendlineMode::Aggressive => write!(f, "aggressive"),
            ScreenTrendlineMode::Conservative => write!(f, "conservative"),
            ScreenTrendlineMode::Disabled => write!(f, "disabled"),
        }
    }
}

/// Network bandwidth usage state estimated by the delay trendline filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenBandwidthUsageState {
    #[default]
    Normal = 0,
    Overusing = 1,
    Underusing = 2,
}

impl ScreenBandwidthUsageState {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenBandwidthUsageState::Overusing,
            2 => ScreenBandwidthUsageState::Underusing,
            _ => ScreenBandwidthUsageState::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ScreenBandwidthUsageState::Normal => "normal",
            ScreenBandwidthUsageState::Overusing => "overusing",
            ScreenBandwidthUsageState::Underusing => "underusing",
        }
    }
}

impl std::fmt::Display for ScreenBandwidthUsageState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenBandwidthUsageState::Normal => write!(f, "normal"),
            ScreenBandwidthUsageState::Overusing => write!(f, "overusing"),
            ScreenBandwidthUsageState::Underusing => write!(f, "underusing"),
        }
    }
}

/// Telemetry stats for the delay-based trendline filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenTrendlineStats {
    pub mode: ScreenTrendlineMode,
    pub state: ScreenBandwidthUsageState,
    pub window_size: usize,
    pub samples_count: usize,
    pub current_slope_us_per_frame: f64,
    pub threshold_us_per_frame: f64,
    pub overuse_count: u64,
    pub underuse_count: u64,
    pub accumulated_delay_us: i64,
    pub last_update_ts_ms: u64,
}

impl Default for ScreenTrendlineStats {
    fn default() -> Self {
        Self {
            mode: ScreenTrendlineMode::Auto,
            state: ScreenBandwidthUsageState::Normal,
            window_size: SCREEN_TRENDLINE_WINDOW_DEFAULT,
            samples_count: 0,
            current_slope_us_per_frame: 0.0,
            threshold_us_per_frame: SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            overuse_count: 0,
            underuse_count: 0,
            accumulated_delay_us: 0,
            last_update_ts_ms: 0,
        }
    }
}

/// DTO report for REST API and CLI status formatting (v2.06 / ADR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenTrendlineReport {
    pub ok: bool,
    pub mode: ScreenTrendlineMode,
    pub state: ScreenBandwidthUsageState,
    pub samples_count: usize,
    pub current_slope_us_per_frame: f64,
    pub threshold_us_per_frame: f64,
    pub overuse_count: u64,
    pub underuse_count: u64,
    pub accumulated_delay_us: i64,
    pub timestamp_ms: u64,
}

/// Delay-based trendline bufferbloat estimator utilizing linear regression across
/// a sliding window of inter-departure vs inter-arrival delays (v2.06 / ADR-092).
#[derive(Debug, Clone)]
pub struct ScreenTrendlineFilter {
    pub(super) mode: ScreenTrendlineMode,
    pub(super) state: ScreenBandwidthUsageState,
    pub(super) window_capacity: usize,
    pub(super) samples: VecDeque<(u64, i64)>,
    pub(super) last_send_ts_us: u64,
    pub(super) last_recv_ts_us: u64,
    pub(super) accumulated_delay_us: i64,
    pub(super) consecutive_overuse: usize,
    pub(super) consecutive_underuse: usize,
    pub(super) stats: ScreenTrendlineStats,
}

impl Default for ScreenTrendlineFilter {
    fn default() -> Self {
        Self::new(ScreenTrendlineMode::Auto)
    }
}

impl ScreenTrendlineFilter {
    pub fn new(mode: ScreenTrendlineMode) -> Self {
        let threshold = match mode {
            ScreenTrendlineMode::Auto => SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Disabled => 0.0,
        };
        Self {
            mode,
            state: ScreenBandwidthUsageState::Normal,
            window_capacity: SCREEN_TRENDLINE_WINDOW_DEFAULT,
            samples: VecDeque::with_capacity(SCREEN_TRENDLINE_WINDOW_DEFAULT),
            last_send_ts_us: 0,
            last_recv_ts_us: 0,
            accumulated_delay_us: 0,
            consecutive_overuse: 0,
            consecutive_underuse: 0,
            stats: ScreenTrendlineStats {
                mode,
                threshold_us_per_frame: threshold,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTrendlineMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTrendlineMode) {
        self.mode = mode;
        self.stats.mode = mode;
        self.stats.threshold_us_per_frame = match mode {
            ScreenTrendlineMode::Auto => SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Disabled => 0.0,
        };
        if mode == ScreenTrendlineMode::Disabled {
            self.state = ScreenBandwidthUsageState::Normal;
            self.stats.state = ScreenBandwidthUsageState::Normal;
        }
    }

    pub fn state(&self) -> ScreenBandwidthUsageState {
        self.state
    }

    pub fn stats(&self) -> ScreenTrendlineStats {
        let mut s = self.stats.clone();
        s.mode = self.mode;
        s.state = self.state;
        s.window_size = self.window_capacity;
        s.samples_count = self.samples.len();
        s.accumulated_delay_us = self.accumulated_delay_us;
        s
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.last_send_ts_us = 0;
        self.last_recv_ts_us = 0;
        self.accumulated_delay_us = 0;
        self.consecutive_overuse = 0;
        self.consecutive_underuse = 0;
        self.state = ScreenBandwidthUsageState::Normal;
        self.stats.current_slope_us_per_frame = 0.0;
        self.stats.state = ScreenBandwidthUsageState::Normal;
    }

    /// Process a new frame acknowledgment and update the linear regression trendline.
    pub fn on_ack(
        &mut self,
        sent_ts_us: u64,
        recv_ts_us: u64,
        rtt_p95_us: Option<u64>,
    ) -> ScreenBandwidthUsageState {
        if self.mode == ScreenTrendlineMode::Disabled {
            self.state = ScreenBandwidthUsageState::Normal;
            return self.state;
        }

        if self.last_send_ts_us > 0 && self.last_recv_ts_us > 0 && sent_ts_us > self.last_send_ts_us
        {
            let delta_send = sent_ts_us.saturating_sub(self.last_send_ts_us);
            let delta_recv = recv_ts_us.saturating_sub(self.last_recv_ts_us);
            let delay_variation = delta_recv as i64 - delta_send as i64;
            self.accumulated_delay_us = self.accumulated_delay_us.saturating_add(delay_variation);

            if self.samples.len() >= self.window_capacity {
                self.samples.pop_front();
            }
            self.samples
                .push_back((sent_ts_us, self.accumulated_delay_us));
        } else if self.samples.is_empty() {
            self.samples.push_back((sent_ts_us, 0));
        }

        self.last_send_ts_us = sent_ts_us;
        self.last_recv_ts_us = recv_ts_us;

        // Threshold evaluation
        let threshold = match self.mode {
            ScreenTrendlineMode::Disabled => 0.0,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Auto => {
                if let Some(rtt) = rtt_p95_us {
                    let rtt_factor = (rtt as f64 / 100_000.0).clamp(0.5, 3.0);
                    SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME * rtt_factor
                } else {
                    SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME
                }
            }
        };
        self.stats.threshold_us_per_frame = threshold;

        if self.samples.len() < SCREEN_TRENDLINE_MIN_SAMPLES {
            self.state = ScreenBandwidthUsageState::Normal;
            self.stats.state = self.state;
            return self.state;
        }

        // Ordinary Least Squares (OLS) linear regression
        let n = self.samples.len() as f64;
        let t0 = self.samples[0].0;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (s, d) in &self.samples {
            let x = s.saturating_sub(t0) as f64;
            let y = *d as f64;
            sum_x += x;
            sum_y += y;
        }
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;

        let mut cov_xy = 0.0;
        let mut var_x = 0.0;
        for (s, d) in &self.samples {
            let x = s.saturating_sub(t0) as f64;
            let y = *d as f64;
            cov_xy += (x - mean_x) * (y - mean_y);
            var_x += (x - mean_x) * (x - mean_x);
        }

        let slope_us_per_frame = if var_x > 1.0 {
            let raw_slope = cov_xy / var_x;
            let last_ts = self.samples.back().unwrap().0;
            let total_span_us = last_ts.saturating_sub(t0) as f64;
            let avg_frame_interval_us = total_span_us / (n - 1.0);
            raw_slope * avg_frame_interval_us
        } else {
            0.0
        };

        self.stats.current_slope_us_per_frame = slope_us_per_frame;

        if slope_us_per_frame > threshold {
            self.consecutive_overuse = self.consecutive_overuse.saturating_add(1);
            self.consecutive_underuse = 0;
            if self.consecutive_overuse >= 3 {
                self.state = ScreenBandwidthUsageState::Overusing;
                self.stats.overuse_count = self.stats.overuse_count.saturating_add(1);
            }
        } else if slope_us_per_frame < -threshold {
            self.consecutive_underuse = self.consecutive_underuse.saturating_add(1);
            self.consecutive_overuse = 0;
            if self.consecutive_underuse >= 3 {
                self.state = ScreenBandwidthUsageState::Underusing;
                self.stats.underuse_count = self.stats.underuse_count.saturating_add(1);
            }
        } else {
            self.consecutive_overuse = 0;
            self.consecutive_underuse = 0;
            self.state = ScreenBandwidthUsageState::Normal;
        }

        self.stats.state = self.state;
        self.state
    }
}
