// SPDX-License-Identifier: Apache-2.0 OR MIT
//! HARQ mode selection, NACK records and the retransmit buffer.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;

/// Hybrid ARQ (HARQ) selective retransmission mode (v2.07 / ADR-093).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenHarqMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Conservative = 2,
    Disabled = 3,
}

impl ScreenHarqMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenHarqMode::Aggressive,
            2 => ScreenHarqMode::Conservative,
            3 => ScreenHarqMode::Disabled,
            _ => ScreenHarqMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenHarqMode::Auto => "auto",
            ScreenHarqMode::Aggressive => "aggressive",
            ScreenHarqMode::Conservative => "conservative",
            ScreenHarqMode::Disabled => "disabled",
        }
    }
}

impl fmt::Display for ScreenHarqMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenHarqMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenHarqMode::Auto),
            "aggressive" | "agg" => Ok(ScreenHarqMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenHarqMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenHarqMode::Disabled),
            _ => Err(CoreError::ScreenRefused("unknown screen harq mode".into())),
        }
    }
}

pub const SCREEN_NACK_FLAG_FEC: u8 = 0x01;
pub const SCREEN_NACK_FLAG_KEYFRAME_FALLBACK: u8 = 0x02;

/// Fine-grained selective negative acknowledgment (NACK) requesting retransmission
/// of missing fragments or shards before playout deadline (v2.07 / ADR-093).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenNack {
    pub frame_seq: u64,
    pub block_id: u16,
    pub missing_mask: u64,
    pub missing_indices: Vec<u16>,
    pub is_fec: bool,
    pub requires_keyframe_fallback: bool,
    pub rtt_estimate_us: u64,
}

impl ScreenNack {
    pub fn new(frame_seq: u64, block_id: u16, is_fec: bool) -> Self {
        Self {
            frame_seq,
            block_id,
            missing_mask: 0,
            missing_indices: Vec::new(),
            is_fec,
            requires_keyframe_fallback: false,
            rtt_estimate_us: 0,
        }
    }

    pub fn with_missing(mut self, indices: &[u16]) -> Self {
        for &idx in indices {
            if idx < 64 {
                self.missing_mask |= 1u64 << idx;
            }
            if !self.missing_indices.contains(&idx) {
                self.missing_indices.push(idx);
            }
        }
        self.missing_indices.sort_unstable();
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let count = self.missing_indices.len().min(u16::MAX as usize) as u16;
        let mut buf = Vec::with_capacity(SCREEN_NACK_HEADER_LEN + (count as usize) * 2);
        buf.extend_from_slice(&SCREEN_NACK_MAGIC);
        buf.push(SCREEN_NACK_PROTO);
        let mut flags = 0u8;
        if self.is_fec {
            flags |= SCREEN_NACK_FLAG_FEC;
        }
        if self.requires_keyframe_fallback {
            flags |= SCREEN_NACK_FLAG_KEYFRAME_FALLBACK;
        }
        buf.push(flags);
        buf.push(0u8); // reserved
        buf.extend_from_slice(&self.frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.block_id.to_be_bytes());
        buf.extend_from_slice(&self.missing_mask.to_be_bytes());
        buf.extend_from_slice(&count.to_be_bytes());
        for &idx in &self.missing_indices[..count as usize] {
            buf.extend_from_slice(&idx.to_be_bytes());
        }
        buf.extend_from_slice(&self.rtt_estimate_us.to_be_bytes());
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_NACK_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_NACK_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_NACK_PROTO {
            return Err(CoreError::ScreenFrame);
        }
        let flags = payload[6];
        let is_fec = (flags & SCREEN_NACK_FLAG_FEC) != 0;
        let requires_keyframe_fallback = (flags & SCREEN_NACK_FLAG_KEYFRAME_FALLBACK) != 0;
        let frame_seq = u64::from_be_bytes(
            payload[8..16]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let block_id = u16::from_be_bytes(
            payload[16..18]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let missing_mask = u64::from_be_bytes(
            payload[18..26]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let count = u16::from_be_bytes(
            payload[26..28]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        ) as usize;

        let required_len = SCREEN_NACK_HEADER_LEN + count * 2;
        if payload.len() < required_len {
            return Err(CoreError::ScreenFrame);
        }

        let mut offset = 28;
        let mut missing_indices = Vec::with_capacity(count);
        for _ in 0..count {
            let idx = u16::from_be_bytes(
                payload[offset..offset + 2]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            missing_indices.push(idx);
            offset += 2;
        }

        let rtt_estimate_us = u64::from_be_bytes(
            payload[offset..offset + 8]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );

        Ok(Self {
            frame_seq,
            block_id,
            missing_mask,
            missing_indices,
            is_fec,
            requires_keyframe_fallback,
            rtt_estimate_us,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_NACK, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_NACK {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Telemetry statistics for Hybrid ARQ & selective retransmission (v2.07 / ADR-093).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenHarqStats {
    pub mode: ScreenHarqMode,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub shards_retransmitted: u64,
    pub recovered_frames: u64,
    pub retransmit_timeouts: u64,
    pub buffer_occupancy: usize,
    pub buffer_capacity: usize,
}

/// DTO report for REST API and CLI status (v2.07 / ADR-093).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenHarqReport {
    pub ok: bool,
    pub mode: ScreenHarqMode,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub shards_retransmitted: u64,
    pub recovered_frames: u64,
    pub retransmit_timeouts: u64,
    pub buffer_occupancy: usize,
    pub buffer_capacity: usize,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RetransmitEntry {
    pub frame: InnerFrame,
    pub sent_at_us: u64,
    pub retx_count: u8,
    pub frame_seq: u64,
    pub shard_index: u16,
    pub is_fec: bool,
}

/// Sender-side bounded sliding buffer of recently transmitted shards for selective NACK recovery.
#[derive(Debug, Clone)]
pub struct ScreenRetransmitBuffer {
    pub(super) mode: ScreenHarqMode,
    pub(super) capacity: usize,
    pub(super) entries: VecDeque<RetransmitEntry>,
    pub(super) stats: ScreenHarqStats,
}

impl Default for ScreenRetransmitBuffer {
    fn default() -> Self {
        Self::new(ScreenHarqMode::Auto)
    }
}

impl ScreenRetransmitBuffer {
    pub fn new(mode: ScreenHarqMode) -> Self {
        Self {
            mode,
            capacity: SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY,
            entries: VecDeque::with_capacity(SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY),
            stats: ScreenHarqStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenHarqMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenHarqMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenHarqMode::Disabled {
            self.entries.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> ScreenHarqStats {
        let mut s = self.stats;
        s.mode = self.mode;
        s.buffer_occupancy = self.entries.len();
        s.buffer_capacity = self.capacity;
        s
    }

    pub fn record_outbound(
        &mut self,
        frame: &InnerFrame,
        frame_seq: u64,
        shard_index: u16,
        is_fec: bool,
        now_us: u64,
    ) {
        if self.mode == ScreenHarqMode::Disabled {
            return;
        }

        let cutoff = now_us.saturating_sub(SCREEN_HARQ_MAX_AGE_US);
        while let Some(front) = self.entries.front() {
            if front.sent_at_us < cutoff || self.entries.len() >= self.capacity {
                self.entries.pop_front();
            } else {
                break;
            }
        }

        self.entries.push_back(RetransmitEntry {
            frame: frame.clone(),
            sent_at_us: now_us,
            retx_count: 0,
            frame_seq,
            shard_index,
            is_fec,
        });
        self.stats.buffer_occupancy = self.entries.len();
        self.stats.buffer_capacity = self.capacity;
    }

    pub fn handle_nack(&mut self, nack: &ScreenNack, now_us: u64) -> Vec<InnerFrame> {
        if self.mode == ScreenHarqMode::Disabled {
            return Vec::new();
        }
        self.stats.nacks_received = self.stats.nacks_received.saturating_add(1);

        let max_retries = match self.mode {
            ScreenHarqMode::Auto => SCREEN_HARQ_MAX_RETRIES_AUTO,
            ScreenHarqMode::Aggressive => SCREEN_HARQ_MAX_RETRIES_AGGRESSIVE,
            ScreenHarqMode::Conservative => SCREEN_HARQ_MAX_RETRIES_CONSERVATIVE,
            ScreenHarqMode::Disabled => 0,
        };

        let mut to_resend = Vec::new();
        let cutoff = now_us.saturating_sub(SCREEN_HARQ_MAX_AGE_US);

        let mut requested_indices = nack.missing_indices.clone();
        if requested_indices.is_empty() && nack.missing_mask != 0 {
            for i in 0..64 {
                if (nack.missing_mask & (1u64 << i)) != 0 {
                    requested_indices.push(i as u16);
                }
            }
        }

        for idx in requested_indices {
            let mut found = false;
            for entry in self.entries.iter_mut() {
                if entry.frame_seq == nack.frame_seq
                    && entry.is_fec == nack.is_fec
                    && entry.shard_index == idx
                {
                    found = true;
                    if entry.retx_count < max_retries && entry.sent_at_us >= cutoff {
                        entry.retx_count += 1;
                        self.stats.shards_retransmitted =
                            self.stats.shards_retransmitted.saturating_add(1);
                        to_resend.push(entry.frame.clone());
                    } else {
                        self.stats.retransmit_timeouts =
                            self.stats.retransmit_timeouts.saturating_add(1);
                    }
                    break;
                }
            }
            if !found {
                self.stats.retransmit_timeouts = self.stats.retransmit_timeouts.saturating_add(1);
            }
        }

        self.stats.buffer_occupancy = self.entries.len();
        to_resend
    }

    /// Automatically inspect outbound InnerFrame (either TYPE_SCREEN_FEC or TYPE_SCREEN_FRAME)
    /// and record it into the bounded sliding window retransmit buffer.
    pub fn record_outbound_frame(&mut self, frame: &InnerFrame, now_us: u64) {
        if self.mode == ScreenHarqMode::Disabled {
            return;
        }
        if frame.ty == TYPE_SCREEN_FEC {
            if let Ok(shard) = ScreenFecShard::from_inner_frame(frame) {
                let shard_index = if shard.is_parity {
                    shard.k_data_shards as u16 + shard.shard_index as u16
                } else {
                    shard.shard_index as u16
                };
                self.record_outbound(frame, shard.frame_seq, shard_index, true, now_us);
            }
        } else if frame.ty == TYPE_SCREEN_FRAME {
            if frame.payload.starts_with(&SCREEN_MAGIC) {
                if let Ok(sf) = ScreenFrame::from_frame(frame) {
                    self.record_outbound(frame, sf.seq, 0, false, now_us);
                }
            } else if frame.payload.starts_with(&SCREEN_FRAG_MAGIC)
                && frame.payload.len() >= SCREEN_FRAG_HEADER
            {
                let idx = u32::from_be_bytes(frame.payload[8..12].try_into().unwrap()) as u16;
                let seq = u64::from_be_bytes(frame.payload[16..24].try_into().unwrap());
                self.record_outbound(frame, seq, idx, false, now_us);
            }
        }
    }

    pub fn report(&self) -> ScreenHarqReport {
        ScreenHarqReport {
            ok: true,
            mode: self.mode,
            nacks_sent: self.stats.nacks_sent,
            nacks_received: self.stats.nacks_received,
            shards_retransmitted: self.stats.shards_retransmitted,
            recovered_frames: self.stats.recovered_frames,
            retransmit_timeouts: self.stats.retransmit_timeouts,
            buffer_occupancy: self.entries.len(),
            buffer_capacity: self.capacity,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats.buffer_occupancy = 0;
    }
}
