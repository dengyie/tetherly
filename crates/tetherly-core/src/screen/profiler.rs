// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Per-frame latency breakdown and multi-stage profiler.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;

/// Primary latency bottleneck stage identified by multi-stage profiling (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenBottleneckStage {
    Capture = 0,
    Encode = 1,
    Pacer = 2,
    Network = 3,
    Jitter = 4,
    Decode = 5,
    Render = 6,
    #[default]
    Balanced = 7,
}

impl ScreenBottleneckStage {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Capture,
            1 => Self::Encode,
            2 => Self::Pacer,
            3 => Self::Network,
            4 => Self::Jitter,
            5 => Self::Decode,
            6 => Self::Render,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Encode => "encode",
            Self::Pacer => "pacer",
            Self::Network => "network",
            Self::Jitter => "jitter",
            Self::Decode => "decode",
            Self::Render => "render",
            Self::Balanced => "balanced",
        }
    }
}

impl std::fmt::Display for ScreenBottleneckStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenBottleneckStage {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "capture" => Ok(Self::Capture),
            "encode" => Ok(Self::Encode),
            "pacer" => Ok(Self::Pacer),
            "network" => Ok(Self::Network),
            "jitter" => Ok(Self::Jitter),
            "decode" => Ok(Self::Decode),
            "render" => Ok(Self::Render),
            "balanced" => Ok(Self::Balanced),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Latency profiling operational mode (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenProfilerMode {
    #[default]
    Auto = 0,
    Detailed = 1,
    Disabled = 2,
}

impl ScreenProfilerMode {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Auto,
            1 => Self::Detailed,
            _ => Self::Disabled,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Detailed => "detailed",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "default" | "normal" => Ok(Self::Auto),
            "detailed" | "high" | "verbose" | "always" => Ok(Self::Detailed),
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

impl std::str::FromStr for ScreenProfilerMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for ScreenProfilerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Granular 7-stage microsecond latency breakdown for a single video frame (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenLatencyBreakdown {
    pub frame_seq: u64,
    pub capture_us: u32,
    pub encode_us: u32,
    pub pacer_us: u32,
    pub network_us: u32,
    pub jitter_us: u32,
    pub decode_us: u32,
    pub render_us: u32,
    pub total_e2e_us: u64,
}

impl ScreenLatencyBreakdown {
    pub fn calculate_total(&mut self) -> u64 {
        self.total_e2e_us = self.capture_us as u64
            + self.encode_us as u64
            + self.pacer_us as u64
            + self.network_us as u64
            + self.jitter_us as u64
            + self.decode_us as u64
            + self.render_us as u64;
        self.total_e2e_us
    }
}

/// Compact wire protocol frame for latency profiling & multi-stage telemetry (56 bytes) (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProfileFrame {
    pub flags: u16,
    pub frame_seq: u64,
    pub capture_us: u32,
    pub encode_us: u32,
    pub pacer_us: u32,
    pub sender_ts_us: u64,
    pub network_us: u32,
    pub jitter_us: u32,
    pub decode_us: u32,
    pub render_us: u32,
    pub total_e2e_us: u32,
}

impl ScreenProfileFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SCREEN_PROFILE_HEADER_LEN);
        buf.extend_from_slice(&SCREEN_PROFILE_MAGIC);
        buf.push(SCREEN_PROFILE_PROTO);
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.capture_us.to_be_bytes());
        buf.extend_from_slice(&self.encode_us.to_be_bytes());
        buf.extend_from_slice(&self.pacer_us.to_be_bytes());
        buf.extend_from_slice(&self.sender_ts_us.to_be_bytes());
        buf.extend_from_slice(&self.network_us.to_be_bytes());
        buf.extend_from_slice(&self.jitter_us.to_be_bytes());
        buf.extend_from_slice(&self.decode_us.to_be_bytes());
        buf.extend_from_slice(&self.render_us.to_be_bytes());
        buf.extend_from_slice(&self.total_e2e_us.to_be_bytes());
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < SCREEN_PROFILE_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if bytes[0..5] != SCREEN_PROFILE_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if bytes[5] != SCREEN_PROFILE_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
        let frame_seq = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        let capture_us = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let encode_us = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        let pacer_us = u32::from_be_bytes(bytes[24..28].try_into().unwrap());
        let sender_ts_us = u64::from_be_bytes(bytes[28..36].try_into().unwrap());
        let network_us = u32::from_be_bytes(bytes[36..40].try_into().unwrap());
        let jitter_us = u32::from_be_bytes(bytes[40..44].try_into().unwrap());
        let decode_us = u32::from_be_bytes(bytes[44..48].try_into().unwrap());
        let render_us = u32::from_be_bytes(bytes[48..52].try_into().unwrap());
        let total_e2e_us = u32::from_be_bytes(bytes[52..56].try_into().unwrap());

        Ok(Self {
            flags,
            frame_seq,
            capture_us,
            encode_us,
            pacer_us,
            sender_ts_us,
            network_us,
            jitter_us,
            decode_us,
            render_us,
            total_e2e_us,
        })
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode();
        Ok(InnerFrame {
            ty: TYPE_SCREEN_PROFILE,
            flags: 0,
            msg_id: 0,
            payload,
        })
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_PROFILE {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }
}

/// DTO report for REST API and CLI formatting (v2.13 / ADR-099).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenLatencyReport {
    pub ok: bool,
    pub mode: String,
    pub frames_profiled: u64,
    pub bottleneck: String,
    pub current: ScreenLatencyBreakdown,
    pub avg_e2e_us: u64,
    pub p50_e2e_us: u64,
    pub p90_e2e_us: u64,
    pub p95_e2e_us: u64,
    pub p99_e2e_us: u64,
    pub min_e2e_us: u64,
    pub max_e2e_us: u64,
    pub avg_capture_us: u32,
    pub avg_encode_us: u32,
    pub avg_pacer_us: u32,
    pub avg_network_us: u32,
    pub avg_jitter_us: u32,
    pub avg_decode_us: u32,
    pub avg_render_us: u32,
}

impl Default for ScreenLatencyReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: "auto".to_string(),
            frames_profiled: 0,
            bottleneck: "balanced".to_string(),
            current: ScreenLatencyBreakdown::default(),
            avg_e2e_us: 0,
            p50_e2e_us: 0,
            p90_e2e_us: 0,
            p95_e2e_us: 0,
            p99_e2e_us: 0,
            min_e2e_us: 0,
            max_e2e_us: 0,
            avg_capture_us: 0,
            avg_encode_us: 0,
            avg_pacer_us: 0,
            avg_network_us: 0,
            avg_jitter_us: 0,
            avg_decode_us: 0,
            avg_render_us: 0,
        }
    }
}

/// Microsecond-precision multi-stage frame profiler & performance telemetry engine (v2.13 / ADR-099).
#[derive(Debug, Clone)]
pub struct ScreenFrameProfiler {
    pub(super) mode: ScreenProfilerMode,
    pub(super) frames_profiled: u64,
    pub(super) current: ScreenLatencyBreakdown,
    pub(super) samples_e2e: VecDeque<u64>,
    pub(super) max_samples: usize,
    pub(super) min_e2e_us: u64,
    pub(super) max_e2e_us: u64,
    pub(super) ewma_capture_us: f64,
    pub(super) ewma_encode_us: f64,
    pub(super) ewma_pacer_us: f64,
    pub(super) ewma_network_us: f64,
    pub(super) ewma_jitter_us: f64,
    pub(super) ewma_decode_us: f64,
    pub(super) ewma_render_us: f64,
    pub(super) ewma_total_e2e_us: f64,
    pub(super) bottleneck: ScreenBottleneckStage,
}

impl Default for ScreenFrameProfiler {
    fn default() -> Self {
        Self::new(ScreenProfilerMode::Auto)
    }
}

impl ScreenFrameProfiler {
    pub fn new(mode: ScreenProfilerMode) -> Self {
        Self {
            mode,
            frames_profiled: 0,
            current: ScreenLatencyBreakdown::default(),
            samples_e2e: VecDeque::with_capacity(SCREEN_PROFILE_RING_CAPACITY),
            max_samples: SCREEN_PROFILE_RING_CAPACITY,
            min_e2e_us: 0,
            max_e2e_us: 0,
            ewma_capture_us: 0.0,
            ewma_encode_us: 0.0,
            ewma_pacer_us: 0.0,
            ewma_network_us: 0.0,
            ewma_jitter_us: 0.0,
            ewma_decode_us: 0.0,
            ewma_render_us: 0.0,
            ewma_total_e2e_us: 0.0,
            bottleneck: ScreenBottleneckStage::Balanced,
        }
    }

    pub fn mode(&self) -> ScreenProfilerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenProfilerMode) {
        self.mode = mode;
        if mode == ScreenProfilerMode::Disabled {
            self.samples_e2e.clear();
        }
    }

    pub fn bottleneck(&self) -> ScreenBottleneckStage {
        self.bottleneck
    }

    pub fn stats(&self) -> ScreenLatencyReport {
        self.report()
    }

    pub fn record_profile_frame(&mut self, f: &ScreenProfileFrame) {
        let b = ScreenLatencyBreakdown {
            frame_seq: f.frame_seq,
            capture_us: f.capture_us,
            encode_us: f.encode_us,
            pacer_us: f.pacer_us,
            network_us: f.network_us,
            jitter_us: f.jitter_us,
            decode_us: f.decode_us,
            render_us: f.render_us,
            total_e2e_us: f.total_e2e_us as u64,
        };
        self.record_frame(b);
    }

    pub fn record_breakdown(&mut self, b: ScreenLatencyBreakdown) {
        self.record_frame(b);
    }

    pub fn record_frame(&mut self, mut b: ScreenLatencyBreakdown) {
        if self.mode == ScreenProfilerMode::Disabled {
            return;
        }
        b.calculate_total();
        self.frames_profiled += 1;
        self.current = b;

        let total = b.total_e2e_us;
        if self.min_e2e_us == 0 || total < self.min_e2e_us {
            self.min_e2e_us = total;
        }
        if total > self.max_e2e_us {
            self.max_e2e_us = total;
        }

        if self.samples_e2e.len() >= self.max_samples {
            self.samples_e2e.pop_front();
        }
        self.samples_e2e.push_back(total);

        let alpha = 0.15;
        if self.frames_profiled == 1 {
            self.ewma_capture_us = b.capture_us as f64;
            self.ewma_encode_us = b.encode_us as f64;
            self.ewma_pacer_us = b.pacer_us as f64;
            self.ewma_network_us = b.network_us as f64;
            self.ewma_jitter_us = b.jitter_us as f64;
            self.ewma_decode_us = b.decode_us as f64;
            self.ewma_render_us = b.render_us as f64;
            self.ewma_total_e2e_us = total as f64;
        } else {
            self.ewma_capture_us =
                (1.0 - alpha) * self.ewma_capture_us + alpha * (b.capture_us as f64);
            self.ewma_encode_us =
                (1.0 - alpha) * self.ewma_encode_us + alpha * (b.encode_us as f64);
            self.ewma_pacer_us = (1.0 - alpha) * self.ewma_pacer_us + alpha * (b.pacer_us as f64);
            self.ewma_network_us =
                (1.0 - alpha) * self.ewma_network_us + alpha * (b.network_us as f64);
            self.ewma_jitter_us =
                (1.0 - alpha) * self.ewma_jitter_us + alpha * (b.jitter_us as f64);
            self.ewma_decode_us =
                (1.0 - alpha) * self.ewma_decode_us + alpha * (b.decode_us as f64);
            self.ewma_render_us =
                (1.0 - alpha) * self.ewma_render_us + alpha * (b.render_us as f64);
            self.ewma_total_e2e_us =
                (1.0 - alpha) * self.ewma_total_e2e_us + alpha * (total as f64);
        }

        // Bottleneck analysis:
        let stages = [
            (ScreenBottleneckStage::Capture, self.ewma_capture_us),
            (ScreenBottleneckStage::Encode, self.ewma_encode_us),
            (ScreenBottleneckStage::Pacer, self.ewma_pacer_us),
            (ScreenBottleneckStage::Network, self.ewma_network_us),
            (ScreenBottleneckStage::Jitter, self.ewma_jitter_us),
            (ScreenBottleneckStage::Decode, self.ewma_decode_us),
            (ScreenBottleneckStage::Render, self.ewma_render_us),
        ];

        let mut max_stage = ScreenBottleneckStage::Balanced;
        let mut max_val = 0.0;
        for (st, val) in stages {
            if val > max_val {
                max_val = val;
                max_stage = st;
            }
        }

        if self.ewma_total_e2e_us > 0.0 && (max_val / self.ewma_total_e2e_us) >= 0.35 {
            self.bottleneck = max_stage;
        } else if total > 0 {
            let mut cur_max_stage = ScreenBottleneckStage::Balanced;
            let mut cur_max_val = 0.0;
            for (st, val) in [
                (ScreenBottleneckStage::Capture, b.capture_us as f64),
                (ScreenBottleneckStage::Encode, b.encode_us as f64),
                (ScreenBottleneckStage::Pacer, b.pacer_us as f64),
                (ScreenBottleneckStage::Network, b.network_us as f64),
                (ScreenBottleneckStage::Jitter, b.jitter_us as f64),
                (ScreenBottleneckStage::Decode, b.decode_us as f64),
                (ScreenBottleneckStage::Render, b.render_us as f64),
            ] {
                if val > cur_max_val {
                    cur_max_val = val;
                    cur_max_stage = st;
                }
            }
            if (cur_max_val / total as f64) >= 0.35 {
                self.bottleneck = cur_max_stage;
            } else {
                self.bottleneck = ScreenBottleneckStage::Balanced;
            }
        } else {
            self.bottleneck = ScreenBottleneckStage::Balanced;
        }
    }

    pub fn percentiles(&self) -> (u64, u64, u64, u64) {
        if self.samples_e2e.is_empty() {
            return (0, 0, 0, 0);
        }
        let mut sorted: Vec<u64> = self.samples_e2e.iter().copied().collect();
        sorted.sort_unstable();
        let len = sorted.len();
        let p50 = sorted[(len * 50 / 100).min(len - 1)];
        let p90 = sorted[(len * 90 / 100).min(len - 1)];
        let p95 = sorted[(len * 95 / 100).min(len - 1)];
        let p99 = sorted[(len * 99 / 100).min(len - 1)];
        (p50, p90, p95, p99)
    }

    pub fn report(&self) -> ScreenLatencyReport {
        let (p50, p90, p95, p99) = self.percentiles();
        ScreenLatencyReport {
            ok: true,
            mode: self.mode.as_str().to_string(),
            frames_profiled: self.frames_profiled,
            bottleneck: self.bottleneck.as_str().to_string(),
            current: self.current,
            avg_e2e_us: self.ewma_total_e2e_us.round() as u64,
            p50_e2e_us: p50,
            p90_e2e_us: p90,
            p95_e2e_us: p95,
            p99_e2e_us: p99,
            min_e2e_us: self.min_e2e_us,
            max_e2e_us: self.max_e2e_us,
            avg_capture_us: self.ewma_capture_us.round() as u32,
            avg_encode_us: self.ewma_encode_us.round() as u32,
            avg_pacer_us: self.ewma_pacer_us.round() as u32,
            avg_network_us: self.ewma_network_us.round() as u32,
            avg_jitter_us: self.ewma_jitter_us.round() as u32,
            avg_decode_us: self.ewma_decode_us.round() as u32,
            avg_render_us: self.ewma_render_us.round() as u32,
        }
    }

    pub fn reset(&mut self) -> ScreenLatencyReport {
        self.frames_profiled = 0;
        self.current = ScreenLatencyBreakdown::default();
        self.samples_e2e.clear();
        self.min_e2e_us = 0;
        self.max_e2e_us = 0;
        self.ewma_capture_us = 0.0;
        self.ewma_encode_us = 0.0;
        self.ewma_pacer_us = 0.0;
        self.ewma_network_us = 0.0;
        self.ewma_jitter_us = 0.0;
        self.ewma_decode_us = 0.0;
        self.ewma_render_us = 0.0;
        self.ewma_total_e2e_us = 0.0;
        self.bottleneck = ScreenBottleneckStage::Balanced;
        self.report()
    }
}
