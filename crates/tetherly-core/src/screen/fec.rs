// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Reed-Solomon style GF(256) FEC encoder, decoder and shard assembler.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use crate::frame::FLAG_MUST_UNDERSTAND;
use serde::Deserialize;
use serde::Serialize;

/// Multiplication in GF(2^8) with irreducible polynomial 0x11D (x^8 + x^4 + x^3 + x^2 + 1) (v2.04 / ADR-090).
pub fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1d;
        }
        b >>= 1;
    }
    p
}

/// Multiplicative inverse in GF(2^8) using Fermat's Little Theorem (a^254 = a^-1) (v2.04 / ADR-090).
pub fn gf_inv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let mut res = 1u8;
    let mut base = a;
    let mut exp = 254;
    while exp > 0 {
        if (exp & 1) != 0 {
            res = gf_mul(res, base);
        }
        base = gf_mul(base, base);
        exp >>= 1;
    }
    res
}

/// Division in GF(2^8): a / b = a * inv(b) (v2.04 / ADR-090).
pub fn gf_div(a: u8, b: u8) -> u8 {
    if b == 0 {
        0
    } else {
        gf_mul(a, gf_inv(b))
    }
}

/// Cauchy matrix coefficient at row (parity index `r`) and col (data index `c`) (v2.04 / ADR-090).
/// X_c = c, Y_r = k + r. Since X_c != Y_r in 0..255, X_c ^ Y_r is non-zero.
pub fn cauchy_coeff(r: usize, c: usize, k: usize) -> u8 {
    let x = c as u8;
    let y = (k + r) as u8;
    gf_inv(x ^ y)
}

/// Forward error correction (FEC) operating mode (v2.04 / ADR-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ScreenFecMode {
    /// Dynamically scales redundancy percentage (0%~30%) based on measured loss & network health.
    #[default]
    Auto = 0,
    /// Fixed baseline 10% redundancy protection for predictable packet loss.
    Conservative = 1,
    /// High-protection 25% redundancy for lossy wireless or mobile cellular links.
    Aggressive = 2,
    /// Passthrough mode with zero parity shards generated or transmitted.
    Disabled = 3,
}

impl ScreenFecMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Conservative => "conservative",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }

    pub const fn tag(&self) -> u8 {
        *self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Conservative,
            2 => Self::Aggressive,
            3 => Self::Disabled,
            _ => Self::Auto,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "conservative" | "cons" | "low" => Some(Self::Conservative),
            "aggressive" | "agg" | "aggro" | "high" => Some(Self::Aggressive),
            "disabled" | "off" | "none" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenFecMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or(CoreError::ScreenFrame)
    }
}

/// Telemetry metrics for forward error correction (v2.04 / ADR-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenFecStats {
    pub mode: ScreenFecMode,
    pub redundancy_pct: u8,
    pub blocks_encoded: u64,
    pub data_shards_sent: u64,
    pub parity_shards_sent: u64,
    pub blocks_decoded: u64,
    pub recovered_shards: u64,
    pub unrecoverable_blocks: u64,
}

/// JSON report DTO for forward error correction inspection (v2.04 / ADR-090).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenFecReport {
    pub ok: bool,
    pub mode: String,
    pub redundancy_pct: u8,
    pub blocks_encoded: u64,
    pub data_shards_sent: u64,
    pub parity_shards_sent: u64,
    pub blocks_decoded: u64,
    pub recovered_shards: u64,
    pub unrecoverable_blocks: u64,
    pub timestamp_ms: u64,
}

/// Individual forward error correction shard (data or parity) (v2.04 / ADR-090).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFecShard {
    pub frame_seq: u64,
    pub block_id: u16,
    pub k_data_shards: u8,
    pub m_parity_shards: u8,
    pub shard_index: u8,
    pub is_parity: bool,
    pub transport_seq: u16,
    pub data: Vec<u8>,
}

impl ScreenFecShard {
    pub fn encode_header(&self) -> [u8; FEC_HEADER_LEN] {
        let mut hdr = [0u8; FEC_HEADER_LEN];
        hdr[0..4].copy_from_slice(&FEC_MAGIC);
        hdr[4] = FEC_PROTO;
        hdr[5] = 0; // flags
        hdr[6..8].copy_from_slice(&self.transport_seq.to_be_bytes());
        hdr[8..16].copy_from_slice(&self.frame_seq.to_be_bytes());
        hdr[16..18].copy_from_slice(&self.block_id.to_be_bytes());
        hdr[18] = self.k_data_shards;
        hdr[19] = self.m_parity_shards;
        hdr[20] = self.shard_index;
        hdr[21] = if self.is_parity { 1 } else { 0 };
        hdr[22..24].copy_from_slice(&(self.data.len() as u16).to_be_bytes());
        hdr
    }

    pub fn to_inner_frame(&self) -> InnerFrame {
        let hdr = self.encode_header();
        let mut payload = Vec::with_capacity(FEC_HEADER_LEN + self.data.len());
        payload.extend_from_slice(&hdr);
        payload.extend_from_slice(&self.data);
        InnerFrame {
            ty: TYPE_SCREEN_FEC,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload,
        }
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_FEC {
            return Err(CoreError::ScreenFrame);
        }
        if frame.payload.len() < FEC_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if frame.payload[0..4] != FEC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if frame.payload[4] != FEC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let transport_seq = u16::from_be_bytes(frame.payload[6..8].try_into().unwrap());
        let frame_seq = u64::from_be_bytes(frame.payload[8..16].try_into().unwrap());
        let block_id = u16::from_be_bytes(frame.payload[16..18].try_into().unwrap());
        let k_data_shards = frame.payload[18];
        let m_parity_shards = frame.payload[19];
        let shard_index = frame.payload[20];
        let is_parity = frame.payload[21] != 0;
        let shard_len = u16::from_be_bytes(frame.payload[22..24].try_into().unwrap()) as usize;
        if frame.payload.len() < FEC_HEADER_LEN + shard_len {
            return Err(CoreError::TruncatedFrame);
        }
        let data = frame.payload[FEC_HEADER_LEN..FEC_HEADER_LEN + shard_len].to_vec();
        Ok(Self {
            frame_seq,
            block_id,
            k_data_shards,
            m_parity_shards,
            shard_index,
            is_parity,
            transport_seq,
            data,
        })
    }
}

/// Cauchy Reed-Solomon encoder over GF(2^8) (v2.04 / ADR-090).
pub struct ScreenFecEncoder;

impl ScreenFecEncoder {
    /// Encodes a list of data buffers into K data shards + M parity shards.
    /// Each data shard payload is formatted as: 2-byte orig_len + raw bytes,
    /// then padded to maximum shard size S for Galois field matrix encoding.
    #[allow(clippy::needless_range_loop)]
    pub fn encode_block(
        frame_seq: u64,
        block_id: u16,
        data_buffers: &[Vec<u8>],
        m_parity: usize,
    ) -> Result<Vec<ScreenFecShard>, CoreError> {
        let k = data_buffers.len();
        if k == 0 || k > FEC_MAX_DATA_SHARDS || m_parity > FEC_MAX_PARITY_SHARDS {
            return Err(CoreError::ScreenFrame);
        }

        // Format each data shard: 2-byte orig_len (big-endian) + data
        let mut formatted_data: Vec<Vec<u8>> = Vec::with_capacity(k);
        let mut max_shard_len = 0;
        for buf in data_buffers {
            let orig_len = buf.len();
            if orig_len > u16::MAX as usize {
                return Err(CoreError::ScreenTooLarge);
            }
            let mut shard_bytes = Vec::with_capacity(2 + orig_len);
            shard_bytes.extend_from_slice(&(orig_len as u16).to_be_bytes());
            shard_bytes.extend_from_slice(buf);
            max_shard_len = max_shard_len.max(shard_bytes.len());
            formatted_data.push(shard_bytes);
        }

        // Pad all formatted shards to max_shard_len
        for shard in &mut formatted_data {
            if shard.len() < max_shard_len {
                shard.resize(max_shard_len, 0);
            }
        }

        let mut shards = Vec::with_capacity(k + m_parity);

        // Add K data shards
        for (idx, shard_data) in formatted_data.iter().enumerate() {
            shards.push(ScreenFecShard {
                frame_seq,
                block_id,
                k_data_shards: k as u8,
                m_parity_shards: m_parity as u8,
                shard_index: idx as u8,
                is_parity: false,
                transport_seq: 0,
                data: shard_data.clone(),
            });
        }

        // Generate M Cauchy parity shards
        for r in 0..m_parity {
            let mut parity_buf = vec![0u8; max_shard_len];
            for c in 0..k {
                let coeff = cauchy_coeff(r, c, k);
                let data_shard = &formatted_data[c];
                for b in 0..max_shard_len {
                    parity_buf[b] ^= gf_mul(coeff, data_shard[b]);
                }
            }
            shards.push(ScreenFecShard {
                frame_seq,
                block_id,
                k_data_shards: k as u8,
                m_parity_shards: m_parity as u8,
                shard_index: r as u8,
                is_parity: true,
                transport_seq: 0,
                data: parity_buf,
            });
        }

        Ok(shards)
    }
}

/// Cauchy Reed-Solomon decoder over GF(2^8) (v2.04 / ADR-090).
pub struct ScreenFecDecoder;

impl ScreenFecDecoder {
    /// Attempts to decode/reconstruct the original K data buffers from received shards.
    /// Returns:
    /// - Ok(Some(recovered_data_buffers)) if all K shards are present or successfully recovered.
    /// - Ok(None) if fewer than K distinct shards have arrived so far (unrecoverable without more shards).
    /// - Err(CoreError) if shards are corrupted or have invalid parameters.
    #[allow(clippy::needless_range_loop)]
    pub fn decode_block(shards: &[ScreenFecShard]) -> Result<Option<Vec<Vec<u8>>>, CoreError> {
        if shards.is_empty() {
            return Ok(None);
        }
        let k = shards[0].k_data_shards as usize;
        let m = shards[0].m_parity_shards as usize;
        if k == 0 || k > FEC_MAX_DATA_SHARDS || m > FEC_MAX_PARITY_SHARDS {
            return Err(CoreError::ScreenFrame);
        }

        let mut data_shards_map: std::collections::BTreeMap<u8, Vec<u8>> =
            std::collections::BTreeMap::new();
        let mut parity_shards_map: std::collections::BTreeMap<u8, Vec<u8>> =
            std::collections::BTreeMap::new();

        for shard in shards {
            if shard.is_parity {
                if (shard.shard_index as usize) < m {
                    parity_shards_map.insert(shard.shard_index, shard.data.clone());
                }
            } else if (shard.shard_index as usize) < k {
                data_shards_map.insert(shard.shard_index, shard.data.clone());
            }
        }

        // Fast path: all K data shards arrived intact!
        if data_shards_map.len() == k {
            let mut out = Vec::with_capacity(k);
            for idx in 0..k {
                let shard_buf = data_shards_map.get(&(idx as u8)).unwrap();
                if shard_buf.len() < 2 {
                    return Err(CoreError::TruncatedFrame);
                }
                let orig_len = u16::from_be_bytes(shard_buf[0..2].try_into().unwrap()) as usize;
                if shard_buf.len() < 2 + orig_len {
                    return Err(CoreError::TruncatedFrame);
                }
                out.push(shard_buf[2..2 + orig_len].to_vec());
            }
            return Ok(Some(out));
        }

        // Missing data shards
        let total_received = data_shards_map.len() + parity_shards_map.len();
        if total_received < k {
            // Cannot recover yet: fewer than K total shards
            return Ok(None);
        }

        let mut missing_indices = Vec::new();
        for idx in 0..k {
            if !data_shards_map.contains_key(&(idx as u8)) {
                missing_indices.push(idx);
            }
        }
        let num_missing = missing_indices.len();
        if parity_shards_map.len() < num_missing {
            // Too many lost shards to recover with available parity
            return Ok(None);
        }

        // Select num_missing parity shards
        let selected_parity: Vec<(usize, Vec<u8>)> = parity_shards_map
            .into_iter()
            .take(num_missing)
            .map(|(idx, data)| (idx as usize, data))
            .collect();

        let shard_len = shards[0].data.len();

        // Construct linear system: A * X = V
        // A is num_missing x num_missing matrix in GF(2^8)
        let mut a: Vec<Vec<u8>> = Vec::with_capacity(num_missing);
        let mut v: Vec<Vec<u8>> = Vec::with_capacity(num_missing);

        for (p_idx, p_data) in selected_parity.iter() {
            let mut a_row = Vec::with_capacity(num_missing);
            for &m_idx in &missing_indices {
                a_row.push(cauchy_coeff(*p_idx, m_idx, k));
            }
            a.push(a_row);

            let mut v_row = p_data.clone();
            if v_row.len() < shard_len {
                v_row.resize(shard_len, 0);
            }

            // Subtract (XOR) the contributions from already available data shards
            for (&c_idx, c_data) in &data_shards_map {
                let coeff = cauchy_coeff(*p_idx, c_idx as usize, k);
                for b in 0..shard_len {
                    let byte_val = if b < c_data.len() { c_data[b] } else { 0 };
                    v_row[b] ^= gf_mul(coeff, byte_val);
                }
            }
            v.push(v_row);
        }

        // Solve A * X = V via Gaussian Elimination over GF(2^8)
        for i in 0..num_missing {
            // Find pivot
            let mut pivot = i;
            while pivot < num_missing && a[pivot][i] == 0 {
                pivot += 1;
            }
            if pivot == num_missing {
                return Err(CoreError::ScreenFrame); // Singular matrix
            }
            if pivot != i {
                a.swap(i, pivot);
                v.swap(i, pivot);
            }

            // Scale row i so A[i][i] = 1
            let inv_diag = gf_inv(a[i][i]);
            for col in i..num_missing {
                a[i][col] = gf_mul(a[i][col], inv_diag);
            }
            for b in 0..shard_len {
                v[i][b] = gf_mul(v[i][b], inv_diag);
            }

            // Eliminate other rows
            for r in 0..num_missing {
                if r != i && a[r][i] != 0 {
                    let factor = a[r][i];
                    for col in i..num_missing {
                        a[r][col] ^= gf_mul(factor, a[i][col]);
                    }
                    for b in 0..shard_len {
                        let scaled_v = gf_mul(factor, v[i][b]);
                        v[r][b] ^= scaled_v;
                    }
                }
            }
        }

        // v[i] now contains the reconstructed missing data shard for missing_indices[i]
        for (i, &m_idx) in missing_indices.iter().enumerate() {
            data_shards_map.insert(m_idx as u8, v[i].clone());
        }

        // Unpack original data payloads
        let mut out = Vec::with_capacity(k);
        for idx in 0..k {
            let shard_buf = data_shards_map.get(&(idx as u8)).unwrap();
            if shard_buf.len() < 2 {
                return Err(CoreError::TruncatedFrame);
            }
            let orig_len = u16::from_be_bytes(shard_buf[0..2].try_into().unwrap()) as usize;
            if shard_buf.len() < 2 + orig_len {
                return Err(CoreError::TruncatedFrame);
            }
            out.push(shard_buf[2..2 + orig_len].to_vec());
        }

        Ok(Some(out))
    }
}

/// Adaptive controller for forward error correction redundancy (v2.04 / ADR-090).
#[derive(Debug, Clone, Default)]
pub struct ScreenFecController {
    pub(super) mode: ScreenFecMode,
    pub(super) stats: ScreenFecStats,
}

impl ScreenFecController {
    pub fn new(mode: ScreenFecMode) -> Self {
        let redundancy_pct = match mode {
            ScreenFecMode::Auto => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Disabled => 0,
        };
        Self {
            mode,
            stats: ScreenFecStats {
                mode,
                redundancy_pct,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenFecMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenFecMode) -> ScreenFecMode {
        self.mode = mode;
        self.stats.mode = mode;
        self.stats.redundancy_pct = match mode {
            ScreenFecMode::Auto => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Disabled => 0,
        };
        mode
    }

    pub fn stats(&self) -> ScreenFecStats {
        self.stats
    }

    /// Calculate required parity shards M for a block of K data shards based on current mode and packet loss rate.
    pub fn calculate_parity_shards(&mut self, k: usize, packet_loss_ratio: f32) -> usize {
        if k == 0 || self.mode == ScreenFecMode::Disabled {
            self.stats.redundancy_pct = 0;
            return 0;
        }

        let redundancy_pct = match self.mode {
            ScreenFecMode::Disabled => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Auto => {
                if packet_loss_ratio < 0.005 {
                    0
                } else if packet_loss_ratio < 0.03 {
                    10
                } else if packet_loss_ratio < 0.08 {
                    20
                } else {
                    30
                }
            }
        };

        self.stats.redundancy_pct = redundancy_pct;

        if redundancy_pct == 0 {
            0
        } else {
            let m = ((k as f32) * (redundancy_pct as f32) / 100.0).ceil() as usize;
            m.clamp(1, FEC_MAX_PARITY_SHARDS)
        }
    }

    pub fn record_block_encoded(&mut self, k: usize, m: usize) {
        self.stats.blocks_encoded = self.stats.blocks_encoded.saturating_add(1);
        self.stats.data_shards_sent = self.stats.data_shards_sent.saturating_add(k as u64);
        self.stats.parity_shards_sent = self.stats.parity_shards_sent.saturating_add(m as u64);
    }

    pub fn record_block_decoded(&mut self, recovered_count: usize) {
        self.stats.blocks_decoded = self.stats.blocks_decoded.saturating_add(1);
        self.stats.recovered_shards = self
            .stats
            .recovered_shards
            .saturating_add(recovered_count as u64);
    }

    pub fn record_unrecoverable(&mut self) {
        self.stats.unrecoverable_blocks = self.stats.unrecoverable_blocks.saturating_add(1);
    }
}

/// Tracks and reconstructs incoming ScreenFrame from TYPE_SCREEN_FEC inner frames (v2.04 / ADR-090).
#[derive(Debug, Default)]
pub struct ScreenFecAssembler {
    pub(super) current_seq: u64,
    pub(super) current_block_id: u16,
    pub(super) received_shards: Vec<ScreenFecShard>,
    pub(super) recovered_shards_total: u64,
    pub(super) unrecoverable_blocks_total: u64,
}

impl ScreenFecAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_shard(&mut self, shard: ScreenFecShard) -> Result<Option<ScreenFrame>, CoreError> {
        // Stream rollover or new frame sequence
        if shard.frame_seq != self.current_seq || shard.block_id != self.current_block_id {
            if self.current_seq != 0 && !self.received_shards.is_empty() {
                // If previous block never finished, count as unrecoverable
                self.unrecoverable_blocks_total = self.unrecoverable_blocks_total.saturating_add(1);
            }
            self.current_seq = shard.frame_seq;
            self.current_block_id = shard.block_id;
            self.received_shards.clear();
        }

        // Dedup shard
        if !self
            .received_shards
            .iter()
            .any(|s| s.is_parity == shard.is_parity && s.shard_index == shard.shard_index)
        {
            self.received_shards.push(shard);
        }

        match ScreenFecDecoder::decode_block(&self.received_shards)? {
            Some(data_buffers) => {
                let k = self.received_shards[0].k_data_shards as usize;
                let data_arrived = self.received_shards.iter().filter(|s| !s.is_parity).count();
                if data_arrived < k {
                    let recovered = k - data_arrived;
                    self.recovered_shards_total =
                        self.recovered_shards_total.saturating_add(recovered as u64);
                }
                self.received_shards.clear();

                // Concatenate data buffers
                let mut full_payload = Vec::new();
                for buf in data_buffers {
                    full_payload.extend_from_slice(&buf);
                }
                let frame = ScreenFrame::decode_payload(&full_payload)?;
                Ok(Some(frame))
            }
            None => Ok(None),
        }
    }

    pub fn recovered_shards(&self) -> u64 {
        self.recovered_shards_total
    }

    pub fn unrecoverable_blocks(&self) -> u64 {
        self.unrecoverable_blocks_total
    }

    /// Generate selective NACK for any missing shards in the current FEC block (v2.07 / ADR-093).
    pub fn generate_nack(&self, rtt_estimate_us: u64) -> Option<ScreenNack> {
        if self.received_shards.is_empty() {
            return None;
        }
        let first = &self.received_shards[0];
        let k = first.k_data_shards as usize;
        let m = first.m_parity_shards as usize;
        let total = k + m;
        let mut missing = Vec::new();
        let mut mask = 0u64;

        for i in 0..total {
            let is_parity = i >= k;
            let shard_idx = if is_parity { (i - k) as u8 } else { i as u8 };
            let present = self
                .received_shards
                .iter()
                .any(|s| s.is_parity == is_parity && s.shard_index == shard_idx);
            if !present {
                if i < 64 {
                    mask |= 1u64 << i;
                }
                missing.push(i as u16);
            }
        }

        if missing.is_empty() {
            return None;
        }

        let missing_data = missing.iter().filter(|&&idx| (idx as usize) < k).count();
        let fallback = missing_data > m;

        Some(ScreenNack {
            frame_seq: self.current_seq,
            block_id: self.current_block_id,
            missing_mask: mask,
            missing_indices: missing,
            is_fec: true,
            requires_keyframe_fallback: fallback,
            rtt_estimate_us,
        })
    }

    pub fn reset(&mut self) {
        self.current_seq = 0;
        self.current_block_id = 0;
        self.received_shards.clear();
    }
}
