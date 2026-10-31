// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Active bandwidth probing clusters and ramp-up controller.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;

/// Screen active bandwidth probing operational modes (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenProbeMode {
    #[default]
    Auto,
    Aggressive,
    Conservative,
    Disabled,
}

impl ScreenProbeMode {
    pub const fn tag(self) -> u8 {
        match self {
            ScreenProbeMode::Auto => 0,
            ScreenProbeMode::Aggressive => 1,
            ScreenProbeMode::Conservative => 2,
            ScreenProbeMode::Disabled => 3,
        }
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenProbeMode::Auto,
            1 => ScreenProbeMode::Aggressive,
            2 => ScreenProbeMode::Conservative,
            _ => ScreenProbeMode::Disabled,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenProbeMode::Auto => "auto",
            ScreenProbeMode::Aggressive => "aggressive",
            ScreenProbeMode::Conservative => "conservative",
            ScreenProbeMode::Disabled => "disabled",
        }
    }

    pub const fn cluster_size(self) -> u8 {
        match self {
            ScreenProbeMode::Auto => SCREEN_PROBE_CLUSTER_SIZE_AUTO,
            ScreenProbeMode::Aggressive => SCREEN_PROBE_CLUSTER_SIZE_AGGRESSIVE,
            ScreenProbeMode::Conservative => SCREEN_PROBE_CLUSTER_SIZE_CONSERVATIVE,
            ScreenProbeMode::Disabled => 0,
        }
    }

    pub const fn min_interval_us(self) -> u64 {
        match self {
            ScreenProbeMode::Auto => SCREEN_PROBE_MIN_INTERVAL_US_AUTO,
            ScreenProbeMode::Aggressive => SCREEN_PROBE_MIN_INTERVAL_US_AGGRESSIVE,
            ScreenProbeMode::Conservative => SCREEN_PROBE_MIN_INTERVAL_US_CONSERVATIVE,
            ScreenProbeMode::Disabled => u64::MAX,
        }
    }
}

impl fmt::Display for ScreenProbeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenProbeMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenProbeMode::Auto),
            "aggressive" | "agg" => Ok(ScreenProbeMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenProbeMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenProbeMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Screen active bandwidth probing state machine (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenProbeState {
    #[default]
    Idle,
    Probing,
    AwaitingFeedback,
    Completed,
    Failed,
}

impl ScreenProbeState {
    pub const fn tag(self) -> u8 {
        match self {
            ScreenProbeState::Idle => 0,
            ScreenProbeState::Probing => 1,
            ScreenProbeState::AwaitingFeedback => 2,
            ScreenProbeState::Completed => 3,
            ScreenProbeState::Failed => 4,
        }
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenProbeState::Idle,
            1 => ScreenProbeState::Probing,
            2 => ScreenProbeState::AwaitingFeedback,
            3 => ScreenProbeState::Completed,
            _ => ScreenProbeState::Failed,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenProbeState::Idle => "idle",
            ScreenProbeState::Probing => "probing",
            ScreenProbeState::AwaitingFeedback => "awaiting_feedback",
            ScreenProbeState::Completed => "completed",
            ScreenProbeState::Failed => "failed",
        }
    }
}

impl fmt::Display for ScreenProbeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Screen active bandwidth probe packet (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProbePacket {
    pub flags: u8,
    pub cluster_id: u8,
    pub cluster_index: u8,
    pub cluster_size: u8,
    pub transport_seq: u16,
    pub target_bps: u32,
    pub send_time_us: u64,
    pub padding: Vec<u8>,
}

impl ScreenProbePacket {
    pub fn encode(&self) -> Vec<u8> {
        let padding_len = self.padding.len().min(u16::MAX as usize) as u16;
        let mut buf = Vec::with_capacity(SCREEN_PROBE_HEADER_LEN + padding_len as usize);
        buf.extend_from_slice(&SCREEN_PROBE_MAGIC);
        buf.push(SCREEN_PROBE_PROTO);
        buf.push(self.flags);
        buf.push(self.cluster_id);
        buf.push(self.cluster_index);
        buf.push(self.cluster_size);
        buf.extend_from_slice(&self.transport_seq.to_be_bytes());
        buf.extend_from_slice(&self.target_bps.to_be_bytes());
        buf.extend_from_slice(&self.send_time_us.to_be_bytes());
        buf.extend_from_slice(&padding_len.to_be_bytes());
        buf.extend_from_slice(&self.padding[..padding_len as usize]);
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_PROBE_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_PROBE_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_PROBE_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = payload[6];
        let cluster_id = payload[7];
        let cluster_index = payload[8];
        let cluster_size = payload[9];
        let transport_seq = u16::from_be_bytes(
            payload[10..12]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let target_bps = u32::from_be_bytes(
            payload[12..16]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let send_time_us = u64::from_be_bytes(
            payload[16..24]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let padding_len = u16::from_be_bytes(
            payload[24..26]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        ) as usize;

        if payload.len() < SCREEN_PROBE_HEADER_LEN + padding_len {
            return Err(CoreError::TruncatedFrame);
        }

        let padding =
            payload[SCREEN_PROBE_HEADER_LEN..SCREEN_PROBE_HEADER_LEN + padding_len].to_vec();

        Ok(Self {
            flags,
            cluster_id,
            cluster_index,
            cluster_size,
            transport_seq,
            target_bps,
            send_time_us,
            padding,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_PROBE, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_PROBE {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Active bandwidth probe cluster tracking metadata (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProbeCluster {
    pub cluster_id: u8,
    pub target_bps: u32,
    pub cluster_size: u8,
    pub packets_sent: u8,
    pub first_send_time_us: u64,
    pub last_send_time_us: u64,
    pub total_bytes: u64,
    pub start_seq: u16,
    pub end_seq: u16,
}

/// Telemetry statistics for Screen Active Bandwidth Probing (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenProbeStats {
    pub mode: ScreenProbeMode,
    pub state: ScreenProbeState,
    pub probes_triggered: u64,
    pub probes_completed: u64,
    pub probes_failed: u64,
    pub last_target_bps: u32,
    pub last_measured_bps: u64,
    pub last_rtt_us: u64,
    pub last_loss_ratio: f32,
    pub last_delay_gradient_us: i64,
    pub active_cluster_id: Option<u8>,
}

impl Default for ScreenProbeStats {
    fn default() -> Self {
        Self {
            mode: ScreenProbeMode::Auto,
            state: ScreenProbeState::Idle,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            last_target_bps: 0,
            last_measured_bps: 0,
            last_rtt_us: 0,
            last_loss_ratio: 0.0,
            last_delay_gradient_us: 0,
            active_cluster_id: None,
        }
    }
}

/// DTO report for REST API and CLI status (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenProbeReport {
    pub ok: bool,
    pub mode: ScreenProbeMode,
    pub state: ScreenProbeState,
    pub probes_triggered: u64,
    pub probes_completed: u64,
    pub probes_failed: u64,
    pub last_target_bps: u32,
    pub last_measured_bps: u64,
    pub last_rtt_us: u64,
    pub last_loss_ratio: f32,
    pub last_delay_gradient_us: i64,
    pub active_cluster_id: Option<u8>,
    pub timestamp_ms: u64,
}

impl Default for ScreenProbeReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenProbeMode::Auto,
            state: ScreenProbeState::Idle,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            last_target_bps: 0,
            last_measured_bps: 0,
            last_rtt_us: 0,
            last_loss_ratio: 0.0,
            last_delay_gradient_us: 0,
            active_cluster_id: None,
            timestamp_ms: 0,
        }
    }
}

/// Controller coordinating active bandwidth probing bursts and fast ramp-up (v2.09 / ADR-095).
#[derive(Debug, Clone)]
pub struct ScreenProbeController {
    pub(super) mode: ScreenProbeMode,
    pub(super) state: ScreenProbeState,
    pub(super) next_cluster_id: u8,
    pub(super) active_cluster: Option<ScreenProbeCluster>,
    pub(super) last_probe_us: u64,
    pub(super) packet_queue: VecDeque<ScreenProbePacket>,
    pub(super) stats: ScreenProbeStats,
}

impl Default for ScreenProbeController {
    fn default() -> Self {
        Self::new(ScreenProbeMode::Auto)
    }
}

impl ScreenProbeController {
    pub fn new(mode: ScreenProbeMode) -> Self {
        Self {
            mode,
            state: ScreenProbeState::Idle,
            next_cluster_id: 0,
            active_cluster: None,
            last_probe_us: 0,
            packet_queue: VecDeque::with_capacity(32),
            stats: ScreenProbeStats {
                mode,
                state: ScreenProbeState::Idle,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenProbeMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenProbeMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenProbeMode::Disabled {
            self.packet_queue.clear();
            self.active_cluster = None;
            self.state = ScreenProbeState::Idle;
            self.stats.state = ScreenProbeState::Idle;
            self.stats.active_cluster_id = None;
        }
    }

    pub fn state(&self) -> ScreenProbeState {
        self.state
    }

    pub fn stats(&self) -> ScreenProbeStats {
        self.stats
    }

    pub fn report(&self) -> ScreenProbeReport {
        ScreenProbeReport {
            ok: true,
            mode: self.mode,
            state: self.state,
            probes_triggered: self.stats.probes_triggered,
            probes_completed: self.stats.probes_completed,
            probes_failed: self.stats.probes_failed,
            last_target_bps: self.stats.last_target_bps,
            last_measured_bps: self.stats.last_measured_bps,
            last_rtt_us: self.stats.last_rtt_us,
            last_loss_ratio: self.stats.last_loss_ratio,
            last_delay_gradient_us: self.stats.last_delay_gradient_us,
            active_cluster_id: self.stats.active_cluster_id,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn has_pending_packets(&self) -> bool {
        !self.packet_queue.is_empty()
    }

    pub fn pop_probe_packet(&mut self) -> Option<ScreenProbePacket> {
        let pkt = self.packet_queue.pop_front()?;
        if let Some(ref mut cl) = self.active_cluster {
            cl.packets_sent = cl.packets_sent.saturating_add(1);
        }
        if self.packet_queue.is_empty() && self.state == ScreenProbeState::Probing {
            self.state = ScreenProbeState::AwaitingFeedback;
            self.stats.state = ScreenProbeState::AwaitingFeedback;
        }
        Some(pkt)
    }

    /// Trigger an active bandwidth probe burst to probe for target_bps.
    pub fn trigger_probe(
        &mut self,
        target_bps: u32,
        now_us: u64,
        next_transport_seq: &mut u16,
    ) -> bool {
        if self.mode == ScreenProbeMode::Disabled {
            return false;
        }

        // If an active cluster is still waiting, check timeout (1.5s)
        if let Some(ref cl) = self.active_cluster {
            if now_us.saturating_sub(cl.last_send_time_us) > 1_500_000 {
                self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
                self.active_cluster = None;
            } else {
                return false;
            }
        }

        self.next_cluster_id = (self.next_cluster_id % 254) + 1;
        let cluster_id = self.next_cluster_id;
        let cluster_size = self.mode.cluster_size().max(4);
        let target_bps = target_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);

        self.packet_queue.clear();
        let mut start_seq = 0u16;
        let mut end_seq = 0u16;

        let padding_len = SCREEN_PROBE_DEFAULT_PACKET_LEN - SCREEN_PROBE_HEADER_LEN;
        let dummy_padding = vec![0x5a; padding_len];

        for i in 0..cluster_size {
            *next_transport_seq = next_transport_seq.wrapping_add(1);
            if *next_transport_seq == 0 {
                *next_transport_seq = 1;
            }
            let t_seq = *next_transport_seq;
            if i == 0 {
                start_seq = t_seq;
            }
            if i == cluster_size - 1 {
                end_seq = t_seq;
            }
            let pkt = ScreenProbePacket {
                flags: 0,
                cluster_id,
                cluster_index: i,
                cluster_size,
                transport_seq: t_seq,
                target_bps,
                send_time_us: now_us,
                padding: dummy_padding.clone(),
            };
            self.packet_queue.push_back(pkt);
        }

        self.active_cluster = Some(ScreenProbeCluster {
            cluster_id,
            target_bps,
            cluster_size,
            packets_sent: 0,
            first_send_time_us: now_us,
            last_send_time_us: now_us,
            total_bytes: (cluster_size as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64),
            start_seq,
            end_seq,
        });

        self.state = ScreenProbeState::Probing;
        self.stats.state = ScreenProbeState::Probing;
        self.stats.probes_triggered = self.stats.probes_triggered.saturating_add(1);
        self.stats.last_target_bps = target_bps;
        self.stats.active_cluster_id = Some(cluster_id);
        self.last_probe_us = now_us;
        true
    }

    /// Trigger an active bandwidth probe burst using pre-allocated transport sequence numbers.
    pub fn trigger_probe_with_seqs(&mut self, target_bps: u32, now_us: u64, seqs: &[u16]) -> bool {
        if self.mode == ScreenProbeMode::Disabled || seqs.is_empty() {
            return false;
        }

        if let Some(ref cl) = self.active_cluster {
            if now_us.saturating_sub(cl.last_send_time_us) > 1_500_000 {
                self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
                self.active_cluster = None;
            } else {
                return false;
            }
        }

        self.next_cluster_id = (self.next_cluster_id % 254) + 1;
        let cluster_id = self.next_cluster_id;
        let cluster_size = seqs.len() as u8;
        let target_bps = target_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);

        self.packet_queue.clear();
        let start_seq = seqs[0];
        let end_seq = seqs[seqs.len() - 1];

        let padding_len = SCREEN_PROBE_DEFAULT_PACKET_LEN - SCREEN_PROBE_HEADER_LEN;
        let dummy_padding = vec![0x5a; padding_len];

        for (i, &t_seq) in seqs.iter().enumerate() {
            let pkt = ScreenProbePacket {
                flags: 0,
                cluster_id,
                cluster_index: i as u8,
                cluster_size,
                transport_seq: t_seq,
                target_bps,
                send_time_us: now_us,
                padding: dummy_padding.clone(),
            };
            self.packet_queue.push_back(pkt);
        }

        self.active_cluster = Some(ScreenProbeCluster {
            cluster_id,
            target_bps,
            cluster_size,
            packets_sent: 0,
            first_send_time_us: now_us,
            last_send_time_us: now_us,
            total_bytes: (cluster_size as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64),
            start_seq,
            end_seq,
        });

        self.state = ScreenProbeState::Probing;
        self.stats.state = ScreenProbeState::Probing;
        self.stats.probes_triggered = self.stats.probes_triggered.saturating_add(1);
        self.stats.last_target_bps = target_bps;
        self.stats.active_cluster_id = Some(cluster_id);
        self.last_probe_us = now_us;
        true
    }

    /// Automatically schedule a bandwidth probe if network conditions and intervals allow.
    pub fn maybe_schedule_probe(
        &mut self,
        current_bps: u32,
        now_us: u64,
        next_transport_seq: &mut u16,
    ) -> bool {
        if self.mode == ScreenProbeMode::Disabled || self.active_cluster.is_some() {
            return false;
        }
        if self.last_probe_us != 0
            && now_us.saturating_sub(self.last_probe_us) < self.mode.min_interval_us()
        {
            return false;
        }
        if current_bps >= SCREEN_BITRATE_MAX_BPS {
            return false;
        }

        let target_bps = match self.mode {
            ScreenProbeMode::Aggressive => {
                let step = (current_bps as u64 * 16) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Auto => {
                let step = (current_bps as u64 * 14) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Conservative => {
                let step = (current_bps as u64 * 12) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Disabled => return false,
        };

        if target_bps > current_bps {
            self.trigger_probe(target_bps, now_us, next_transport_seq)
        } else {
            false
        }
    }

    /// Process incoming TWCC feedback to evaluate the delivery rate and delay gradient of the active probe cluster.
    /// If probe passed, executes fast ramp-up on rate_controller and returns Some(new_bps).
    pub fn on_twcc_feedback(
        &mut self,
        feedback: &ScreenTwccFeedback,
        rate_controller: &mut ScreenRateController,
        _now_us: u64,
    ) -> Option<u32> {
        let cluster = self.active_cluster.as_ref()?.clone();

        // Check if any packet in this cluster is reported in feedback.records
        let cluster_records: Vec<_> = feedback
            .records
            .iter()
            .filter(|r| {
                let diff = r.transport_seq.wrapping_sub(cluster.start_seq);
                diff < cluster.cluster_size as u16
            })
            .collect();

        if cluster_records.is_empty() {
            return None;
        }

        let mut arrived_deltas = Vec::new();
        let mut missing_count = 0;

        for rec in &cluster_records {
            if rec.recv_delta_us == SCREEN_TWCC_MISSING_DELTA {
                missing_count += 1;
            } else {
                arrived_deltas.push(rec.recv_delta_us as i64);
            }
        }

        let loss_ratio = (missing_count as f32) / (cluster.cluster_size as f32);
        self.stats.last_loss_ratio = loss_ratio;

        let delivered_rate_bps = if arrived_deltas.len() >= 2 {
            let min_d = *arrived_deltas.iter().min().unwrap();
            let max_d = *arrived_deltas.iter().max().unwrap();
            let delta_us = (max_d - min_d).max(1) as u64;
            let delivered_bytes =
                (arrived_deltas.len() as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64);
            (delivered_bytes * 8 * 1_000_000) / delta_us
        } else {
            0
        };

        self.stats.last_measured_bps = delivered_rate_bps;

        // Estimate delay gradient over probe cluster: recv span vs send span
        let delay_gradient_us = if arrived_deltas.len() >= 2 {
            let recv_span =
                (*arrived_deltas.iter().max().unwrap()) - (*arrived_deltas.iter().min().unwrap());
            recv_span
        } else {
            0
        };
        self.stats.last_delay_gradient_us = delay_gradient_us;

        // Validation gate:
        // 1. Loss ratio <= 15%
        // 2. Measured rate >= 70% of target_bps
        // 3. Delay gradient <= 20,000 us (20ms queue growth during burst)
        let passed = loss_ratio <= 0.15
            && delivered_rate_bps >= ((cluster.target_bps as u64 * 70) / 100)
            && delay_gradient_us <= 20_000;

        if passed {
            self.stats.probes_completed = self.stats.probes_completed.saturating_add(1);
            self.state = ScreenProbeState::Completed;
            self.stats.state = ScreenProbeState::Completed;
            self.active_cluster = None;
            self.stats.active_cluster_id = None;
            rate_controller.fast_ramp_up(cluster.target_bps);
            Some(cluster.target_bps)
        } else if cluster_records.len() >= (cluster.cluster_size as usize * 3 / 4) {
            // Cluster largely evaluated but failed validation gate
            self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
            self.state = ScreenProbeState::Failed;
            self.stats.state = ScreenProbeState::Failed;
            self.active_cluster = None;
            self.stats.active_cluster_id = None;
            None
        } else {
            None
        }
    }
}
