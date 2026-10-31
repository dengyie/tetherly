// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Transport-wide congestion control feedback trackers.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;

/// Receiver-driven transport-wide congestion control (TWCC) feedback mode (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTwccMode {
    #[default]
    Auto = 0,
    Conservative = 1,
    Aggressive = 2,
    Disabled = 3,
}

impl ScreenTwccMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenTwccMode::Conservative,
            2 => ScreenTwccMode::Aggressive,
            3 => ScreenTwccMode::Disabled,
            _ => ScreenTwccMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenTwccMode::Auto => "auto",
            ScreenTwccMode::Conservative => "conservative",
            ScreenTwccMode::Aggressive => "aggressive",
            ScreenTwccMode::Disabled => "disabled",
        }
    }

    pub const fn interval_us(self) -> u64 {
        match self {
            ScreenTwccMode::Auto => SCREEN_TWCC_AUTO_INTERVAL_US,
            ScreenTwccMode::Conservative => SCREEN_TWCC_CONSERVATIVE_INTERVAL_US,
            ScreenTwccMode::Aggressive => SCREEN_TWCC_AGGRESSIVE_INTERVAL_US,
            ScreenTwccMode::Disabled => u64::MAX,
        }
    }

    pub const fn packet_threshold(self) -> usize {
        match self {
            ScreenTwccMode::Auto => SCREEN_TWCC_AUTO_PACKET_THRESHOLD,
            ScreenTwccMode::Conservative => SCREEN_TWCC_CONSERVATIVE_PACKET_THRESHOLD,
            ScreenTwccMode::Aggressive => SCREEN_TWCC_AGGRESSIVE_PACKET_THRESHOLD,
            ScreenTwccMode::Disabled => usize::MAX,
        }
    }
}

impl fmt::Display for ScreenTwccMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenTwccMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenTwccMode::Auto),
            "conservative" | "cons" => Ok(ScreenTwccMode::Conservative),
            "aggressive" | "agg" => Ok(ScreenTwccMode::Aggressive),
            "disabled" | "off" | "none" => Ok(ScreenTwccMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Individual packet arrival record in TWCC feedback (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenTwccPacketRecord {
    pub transport_seq: u16,
    pub recv_delta_us: i16,
}

/// Receiver-to-sender TWCC feedback message (v2.08 / ADR-094).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenTwccFeedback {
    pub feedback_seq: u8,
    pub base_seq: u16,
    pub packet_count: u16,
    pub reference_time_us: u64,
    pub records: Vec<ScreenTwccPacketRecord>,
}

impl ScreenTwccFeedback {
    pub fn encode(&self) -> Vec<u8> {
        let count = self.records.len().min(u16::MAX as usize) as u16;
        let mut buf =
            Vec::with_capacity(SCREEN_TWCC_HEADER_LEN + (count as usize) * SCREEN_TWCC_RECORD_LEN);
        buf.extend_from_slice(&SCREEN_TWCC_MAGIC);
        buf.push(SCREEN_TWCC_PROTO);
        buf.push(0); // flags
        buf.push(self.feedback_seq);
        buf.extend_from_slice(&self.base_seq.to_be_bytes());
        buf.extend_from_slice(&count.to_be_bytes());
        buf.extend_from_slice(&self.reference_time_us.to_be_bytes());
        for rec in &self.records[..count as usize] {
            buf.extend_from_slice(&rec.transport_seq.to_be_bytes());
            buf.extend_from_slice(&rec.recv_delta_us.to_be_bytes());
        }
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_TWCC_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_TWCC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_TWCC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let feedback_seq = payload[7];
        let base_seq = u16::from_be_bytes(
            payload[8..10]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let packet_count = u16::from_be_bytes(
            payload[10..12]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let reference_time_us = u64::from_be_bytes(
            payload[12..20]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );

        let required_len =
            SCREEN_TWCC_HEADER_LEN + (packet_count as usize) * SCREEN_TWCC_RECORD_LEN;
        if payload.len() < required_len {
            return Err(CoreError::TruncatedFrame);
        }

        let mut records = Vec::with_capacity(packet_count as usize);
        let mut offset = SCREEN_TWCC_HEADER_LEN;
        for _ in 0..packet_count {
            let transport_seq = u16::from_be_bytes(
                payload[offset..offset + 2]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            let recv_delta_us = i16::from_be_bytes(
                payload[offset + 2..offset + 4]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            records.push(ScreenTwccPacketRecord {
                transport_seq,
                recv_delta_us,
            });
            offset += SCREEN_TWCC_RECORD_LEN;
        }

        Ok(Self {
            feedback_seq,
            base_seq,
            packet_count,
            reference_time_us,
            records,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_TWCC, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_TWCC {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Telemetry statistics for Transport-Wide Congestion Control (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenTwccStats {
    pub mode: ScreenTwccMode,
    pub feedbacks_sent: u64,
    pub feedbacks_received: u64,
    pub packets_tracked: u64,
    pub packets_acknowledged: u64,
    pub packets_lost: u64,
    pub loss_ratio: f32,
    pub delivery_rate_bps: u64,
    pub delay_gradient_us: i64,
    pub smoothed_rtt_us: u64,
}

impl Default for ScreenTwccStats {
    fn default() -> Self {
        Self {
            mode: ScreenTwccMode::Auto,
            feedbacks_sent: 0,
            feedbacks_received: 0,
            packets_tracked: 0,
            packets_acknowledged: 0,
            packets_lost: 0,
            loss_ratio: 0.0,
            delivery_rate_bps: 0,
            delay_gradient_us: 0,
            smoothed_rtt_us: 0,
        }
    }
}

/// DTO report for REST API and CLI status (v2.08 / ADR-094).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenTwccReport {
    pub ok: bool,
    pub mode: ScreenTwccMode,
    pub feedbacks_sent: u64,
    pub feedbacks_received: u64,
    pub packets_tracked: u64,
    pub packets_acknowledged: u64,
    pub packets_lost: u64,
    pub loss_ratio: f32,
    pub delivery_rate_bps: u64,
    pub delay_gradient_us: i64,
    pub smoothed_rtt_us: u64,
    pub timestamp_ms: u64,
}

/// Receiver-side arrival tracker that records packet arrival times and emits TWCC feedback (v2.08 / ADR-094).
#[derive(Debug, Clone)]
pub struct ScreenTwccReceiverTracker {
    pub(super) mode: ScreenTwccMode,
    pub(super) feedback_seq: u8,
    pub(super) last_feedback_us: u64,
    pub(super) arrivals: VecDeque<(u16, u64)>,
    pub(super) stats: ScreenTwccStats,
}

impl Default for ScreenTwccReceiverTracker {
    fn default() -> Self {
        Self::new(ScreenTwccMode::Auto)
    }
}

impl ScreenTwccReceiverTracker {
    pub fn new(mode: ScreenTwccMode) -> Self {
        Self {
            mode,
            feedback_seq: 0,
            last_feedback_us: 0,
            arrivals: VecDeque::with_capacity(512),
            stats: ScreenTwccStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTwccMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTwccMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenTwccMode::Disabled {
            self.arrivals.clear();
        }
    }

    pub fn stats(&self) -> ScreenTwccStats {
        self.stats
    }

    pub fn report(&self) -> ScreenTwccReport {
        ScreenTwccReport {
            ok: true,
            mode: self.mode,
            feedbacks_sent: self.stats.feedbacks_sent,
            feedbacks_received: self.stats.feedbacks_received,
            packets_tracked: self.stats.packets_tracked,
            packets_acknowledged: self.stats.packets_acknowledged,
            packets_lost: self.stats.packets_lost,
            loss_ratio: self.stats.loss_ratio,
            delivery_rate_bps: self.stats.delivery_rate_bps,
            delay_gradient_us: self.stats.delay_gradient_us,
            smoothed_rtt_us: self.stats.smoothed_rtt_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn on_packet_received(&mut self, transport_seq: u16, recv_us: u64) {
        if self.mode == ScreenTwccMode::Disabled || transport_seq == 0 {
            return;
        }
        if self.arrivals.iter().any(|&(s, _)| s == transport_seq) {
            return;
        }
        if self.last_feedback_us == 0 {
            self.last_feedback_us = recv_us;
        }
        while self.arrivals.len() >= 512 {
            self.arrivals.pop_front();
        }
        self.arrivals.push_back((transport_seq, recv_us));
        self.stats.packets_tracked = self.stats.packets_tracked.saturating_add(1);
    }

    pub fn should_send_feedback(&self, now_us: u64) -> bool {
        if self.mode == ScreenTwccMode::Disabled || self.arrivals.is_empty() {
            return false;
        }
        if self.arrivals.len() >= self.mode.packet_threshold() {
            return true;
        }
        now_us.saturating_sub(self.last_feedback_us) >= self.mode.interval_us()
    }

    pub fn build_feedback(&mut self, now_us: u64) -> Option<ScreenTwccFeedback> {
        if self.mode == ScreenTwccMode::Disabled || self.arrivals.is_empty() {
            return None;
        }
        let base_seq = self.arrivals.front().unwrap().0;
        let max_seq = self.arrivals.back().unwrap().0;
        let span = (max_seq.wrapping_sub(base_seq) as usize) + 1;
        let count = span.clamp(1, 256);

        let reference_time_us = self.arrivals.front().unwrap().1;
        let mut records = Vec::with_capacity(count);

        for i in 0..count {
            let seq = base_seq.wrapping_add(i as u16);
            if let Some(&(_, arrival_us)) = self.arrivals.iter().find(|&&(s, _)| s == seq) {
                let delta = (arrival_us as i64) - (reference_time_us as i64);
                let clamped_delta =
                    delta.clamp(SCREEN_TWCC_MISSING_DELTA as i64 + 1, i16::MAX as i64) as i16;
                records.push(ScreenTwccPacketRecord {
                    transport_seq: seq,
                    recv_delta_us: clamped_delta,
                });
            } else {
                records.push(ScreenTwccPacketRecord {
                    transport_seq: seq,
                    recv_delta_us: SCREEN_TWCC_MISSING_DELTA,
                });
            }
        }

        self.arrivals.retain(|&(seq, _)| {
            let diff = seq.wrapping_sub(base_seq);
            diff >= count as u16
        });

        let fb_seq = self.feedback_seq;
        self.feedback_seq = self.feedback_seq.wrapping_add(1);
        self.last_feedback_us = now_us;
        self.stats.feedbacks_sent = self.stats.feedbacks_sent.saturating_add(1);

        Some(ScreenTwccFeedback {
            feedback_seq: fb_seq,
            base_seq,
            packet_count: records.len() as u16,
            reference_time_us,
            records,
        })
    }

    pub fn clear(&mut self) {
        self.arrivals.clear();
        self.stats = ScreenTwccStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}

#[derive(Debug, Clone)]
pub struct TwccSentPacket {
    pub seq: u16,
    pub send_us: u64,
    pub bytes: usize,
}

/// Sender-side sequence allocator and feedback processor that derives delivery rate and delay gradient (v2.08 / ADR-094).
#[derive(Debug, Clone)]
pub struct ScreenTwccSenderTracker {
    pub(super) mode: ScreenTwccMode,
    pub(super) next_seq: u16,
    pub(super) in_flight: VecDeque<TwccSentPacket>,
    pub(super) stats: ScreenTwccStats,
}

impl Default for ScreenTwccSenderTracker {
    fn default() -> Self {
        Self::new(ScreenTwccMode::Auto)
    }
}

impl ScreenTwccSenderTracker {
    pub fn new(mode: ScreenTwccMode) -> Self {
        Self {
            mode,
            next_seq: 1,
            in_flight: VecDeque::with_capacity(1024),
            stats: ScreenTwccStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTwccMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTwccMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenTwccMode::Disabled {
            self.in_flight.clear();
        }
    }

    pub fn stats(&self) -> ScreenTwccStats {
        self.stats
    }

    pub fn report(&self) -> ScreenTwccReport {
        ScreenTwccReport {
            ok: true,
            mode: self.mode,
            feedbacks_sent: self.stats.feedbacks_sent,
            feedbacks_received: self.stats.feedbacks_received,
            packets_tracked: self.stats.packets_tracked,
            packets_acknowledged: self.stats.packets_acknowledged,
            packets_lost: self.stats.packets_lost,
            loss_ratio: self.stats.loss_ratio,
            delivery_rate_bps: self.stats.delivery_rate_bps,
            delay_gradient_us: self.stats.delay_gradient_us,
            smoothed_rtt_us: self.stats.smoothed_rtt_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn next_seq(&self) -> u16 {
        self.next_seq
    }

    pub fn set_next_seq(&mut self, next: u16) {
        self.next_seq = next;
    }

    pub fn allocate_seq(&mut self, send_us: u64, bytes: usize) -> u16 {
        if self.mode == ScreenTwccMode::Disabled {
            return 0;
        }
        let seq = self.next_seq;
        self.next_seq = if self.next_seq == u16::MAX {
            1
        } else {
            self.next_seq + 1
        };
        while self.in_flight.len() >= 1024 {
            self.in_flight.pop_front();
        }
        self.in_flight.push_back(TwccSentPacket {
            seq,
            send_us,
            bytes,
        });
        self.stats.packets_tracked = self.stats.packets_tracked.saturating_add(1);
        seq
    }

    pub fn on_feedback(&mut self, feedback: &ScreenTwccFeedback, now_us: u64) -> ScreenTwccReport {
        if self.mode == ScreenTwccMode::Disabled {
            return self.report();
        }
        self.stats.feedbacks_received = self.stats.feedbacks_received.saturating_add(1);

        let mut acked_in_block = 0u64;
        let mut lost_in_block = 0u64;
        let mut delivered_bytes = 0usize;
        let mut min_arrival_us = u64::MAX;
        let mut max_arrival_us = 0u64;

        let mut prev_send_us: Option<u64> = None;
        let mut prev_recv_us: Option<u64> = None;
        let mut delay_gradients = Vec::new();

        for rec in &feedback.records {
            if rec.recv_delta_us == SCREEN_TWCC_MISSING_DELTA {
                lost_in_block += 1;
                self.stats.packets_lost = self.stats.packets_lost.saturating_add(1);
            } else {
                acked_in_block += 1;
                self.stats.packets_acknowledged = self.stats.packets_acknowledged.saturating_add(1);
                let recv_us =
                    (feedback.reference_time_us as i64 + rec.recv_delta_us as i64).max(0) as u64;
                min_arrival_us = min_arrival_us.min(recv_us);
                max_arrival_us = max_arrival_us.max(recv_us);

                if let Some(sent) = self.in_flight.iter().find(|p| p.seq == rec.transport_seq) {
                    delivered_bytes += sent.bytes;
                    let sample_rtt = now_us.saturating_sub(sent.send_us);
                    if self.stats.smoothed_rtt_us == 0 {
                        self.stats.smoothed_rtt_us = sample_rtt;
                    } else {
                        self.stats.smoothed_rtt_us =
                            (self.stats.smoothed_rtt_us * 7 + sample_rtt) / 8;
                    }

                    if let (Some(p_send), Some(p_recv)) = (prev_send_us, prev_recv_us) {
                        let send_delta = sent.send_us as i64 - p_send as i64;
                        let recv_delta = recv_us as i64 - p_recv as i64;
                        delay_gradients.push(recv_delta - send_delta);
                    }
                    prev_send_us = Some(sent.send_us);
                    prev_recv_us = Some(recv_us);
                }
            }
        }

        let total_block = acked_in_block + lost_in_block;
        if total_block > 0 {
            let block_loss = lost_in_block as f32 / total_block as f32;
            if self.stats.feedbacks_received <= 1 {
                self.stats.loss_ratio = block_loss;
            } else {
                self.stats.loss_ratio = self.stats.loss_ratio * 0.7 + block_loss * 0.3;
            }
        }

        if delivered_bytes > 0 && max_arrival_us > min_arrival_us {
            let dt_us = max_arrival_us - min_arrival_us;
            if dt_us >= 100 {
                let inst_rate_bps = ((delivered_bytes as u64) * 8 * 1_000_000) / dt_us;
                if self.stats.delivery_rate_bps == 0 || self.stats.feedbacks_received <= 1 {
                    self.stats.delivery_rate_bps = inst_rate_bps;
                } else {
                    self.stats.delivery_rate_bps =
                        (self.stats.delivery_rate_bps * 7 + inst_rate_bps) / 8;
                }
            }
        }

        if !delay_gradients.is_empty() {
            let avg_grad = delay_gradients.iter().sum::<i64>() / (delay_gradients.len() as i64);
            if self.stats.feedbacks_received <= 1 {
                self.stats.delay_gradient_us = avg_grad;
            } else {
                self.stats.delay_gradient_us = (self.stats.delay_gradient_us * 3 + avg_grad) / 4;
            }
        }

        self.in_flight.retain(|p| {
            let diff = p.seq.wrapping_sub(feedback.base_seq);
            diff >= feedback.packet_count
        });

        self.report()
    }

    pub fn clear(&mut self) {
        self.in_flight.clear();
        self.next_seq = 1;
        self.stats = ScreenTwccStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}
