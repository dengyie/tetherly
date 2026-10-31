// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen/audio lip-sync and clock-skew coordination.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;

/// Screen and audio presentation timestamp (PTS) lip-sync alignment and clock synchronization mode (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenAudioSyncMode {
    #[default]
    Auto = 0,
    Strict = 1,
    Loose = 2,
    Disabled = 3,
}

impl ScreenAudioSyncMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenAudioSyncMode::Strict,
            2 => ScreenAudioSyncMode::Loose,
            3 => ScreenAudioSyncMode::Disabled,
            _ => ScreenAudioSyncMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenAudioSyncMode::Auto => "auto",
            ScreenAudioSyncMode::Strict => "strict",
            ScreenAudioSyncMode::Loose => "loose",
            ScreenAudioSyncMode::Disabled => "disabled",
        }
    }

    pub const fn beacon_interval_us(self) -> u64 {
        match self {
            ScreenAudioSyncMode::Auto => SCREEN_SYNC_INTERVAL_US_AUTO,
            ScreenAudioSyncMode::Strict => SCREEN_SYNC_INTERVAL_US_STRICT,
            ScreenAudioSyncMode::Loose => SCREEN_SYNC_INTERVAL_US_LOOSE,
            ScreenAudioSyncMode::Disabled => u64::MAX,
        }
    }

    pub const fn skew_tolerance_us(self) -> i64 {
        match self {
            ScreenAudioSyncMode::Auto => SCREEN_SYNC_SKEW_TOLERANCE_US_AUTO,
            ScreenAudioSyncMode::Strict => SCREEN_SYNC_SKEW_TOLERANCE_US_STRICT,
            ScreenAudioSyncMode::Loose => SCREEN_SYNC_SKEW_TOLERANCE_US_LOOSE,
            ScreenAudioSyncMode::Disabled => i64::MAX,
        }
    }
}

impl fmt::Display for ScreenAudioSyncMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenAudioSyncMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenAudioSyncMode::Auto),
            "strict" | "tight" | "low" => Ok(ScreenAudioSyncMode::Strict),
            "loose" | "relaxed" | "high" => Ok(ScreenAudioSyncMode::Loose),
            "disabled" | "off" | "none" => Ok(ScreenAudioSyncMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Compact wire protocol frame for presentation timestamp (PTS) lip-sync beacon (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenAudioSyncFrame {
    pub flags: u16,
    pub sync_seq: u32,
    pub sender_wallclock_us: u64,
    pub video_pts_us: u64,
    pub audio_pts_us: u64,
}

impl ScreenAudioSyncFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SCREEN_SYNC_HEADER_LEN);
        buf.extend_from_slice(&SCREEN_SYNC_MAGIC);
        buf.push(SCREEN_SYNC_PROTO);
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.sync_seq.to_be_bytes());
        buf.extend_from_slice(&self.sender_wallclock_us.to_be_bytes());
        buf.extend_from_slice(&self.video_pts_us.to_be_bytes());
        buf.extend_from_slice(&self.audio_pts_us.to_be_bytes());
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < SCREEN_SYNC_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if bytes[0..5] != SCREEN_SYNC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if bytes[5] != SCREEN_SYNC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
        let sync_seq = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        let sender_wallclock_us = u64::from_be_bytes(bytes[12..20].try_into().unwrap());
        let video_pts_us = u64::from_be_bytes(bytes[20..28].try_into().unwrap());
        let audio_pts_us = u64::from_be_bytes(bytes[28..36].try_into().unwrap());

        Ok(Self {
            flags,
            sync_seq,
            sender_wallclock_us,
            video_pts_us,
            audio_pts_us,
        })
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        Ok(InnerFrame {
            ty: TYPE_SCREEN_SYNC,
            flags: 0,
            msg_id: 0,
            payload: self.encode(),
        })
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_SYNC {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }
}

/// Statistics and telemetry for audio-video presentation sync (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenAudioSyncStats {
    pub mode: ScreenAudioSyncMode,
    pub beacons_sent: u64,
    pub beacons_received: u64,
    pub current_skew_us: i64,
    pub clock_drift_ppm: f64,
    pub adjustments_applied: u64,
    pub last_sync_timestamp_ms: u64,
    pub in_sync: bool,
}

impl Default for ScreenAudioSyncStats {
    fn default() -> Self {
        Self {
            mode: ScreenAudioSyncMode::Auto,
            beacons_sent: 0,
            beacons_received: 0,
            current_skew_us: 0,
            clock_drift_ppm: 0.0,
            adjustments_applied: 0,
            last_sync_timestamp_ms: 0,
            in_sync: true,
        }
    }
}

/// DTO report for REST API and CLI status formatting (v2.10 / ADR-096).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenAudioSyncReport {
    pub ok: bool,
    pub mode: ScreenAudioSyncMode,
    pub beacons_sent: u64,
    pub beacons_received: u64,
    pub current_skew_us: i64,
    pub clock_drift_ppm: f64,
    pub adjustments_applied: u64,
    pub in_sync: bool,
    pub timestamp_ms: u64,
}

impl Default for ScreenAudioSyncReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenAudioSyncMode::Auto,
            beacons_sent: 0,
            beacons_received: 0,
            current_skew_us: 0,
            clock_drift_ppm: 0.0,
            adjustments_applied: 0,
            in_sync: true,
            timestamp_ms: 0,
        }
    }
}

/// Audio-video lip-sync alignment and clock drift synchronization coordinator (v2.10 / ADR-096).
#[derive(Debug, Clone)]
pub struct ScreenAudioSyncCoordinator {
    pub(super) mode: ScreenAudioSyncMode,
    pub(super) next_sync_seq: u32,
    pub(super) last_beacon_sent_us: u64,
    pub(super) latest_video_pts_us: u64,
    pub(super) latest_audio_pts_us: u64,
    pub(super) recent_samples: VecDeque<(u64, u64)>,
    pub(super) beacon_queue: VecDeque<ScreenAudioSyncFrame>,
    pub(super) stats: ScreenAudioSyncStats,
}

impl Default for ScreenAudioSyncCoordinator {
    fn default() -> Self {
        Self::new(ScreenAudioSyncMode::Auto)
    }
}

impl ScreenAudioSyncCoordinator {
    pub fn new(mode: ScreenAudioSyncMode) -> Self {
        Self {
            mode,
            next_sync_seq: 0,
            last_beacon_sent_us: 0,
            latest_video_pts_us: 0,
            latest_audio_pts_us: 0,
            recent_samples: VecDeque::with_capacity(16),
            beacon_queue: VecDeque::with_capacity(8),
            stats: ScreenAudioSyncStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenAudioSyncMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenAudioSyncMode::Disabled {
            self.beacon_queue.clear();
        }
    }

    pub fn stats(&self) -> ScreenAudioSyncStats {
        self.stats
    }

    pub fn report(&self) -> ScreenAudioSyncReport {
        ScreenAudioSyncReport {
            ok: true,
            mode: self.mode,
            beacons_sent: self.stats.beacons_sent,
            beacons_received: self.stats.beacons_received,
            current_skew_us: self.stats.current_skew_us,
            clock_drift_ppm: self.stats.clock_drift_ppm,
            adjustments_applied: self.stats.adjustments_applied,
            in_sync: self.stats.in_sync,
            timestamp_ms: self.stats.last_sync_timestamp_ms,
        }
    }

    pub fn record_video_pts(&mut self, pts_us: u64) {
        self.latest_video_pts_us = pts_us;
    }

    pub fn record_audio_pts(&mut self, pts_us: u64) {
        self.latest_audio_pts_us = pts_us;
    }

    pub fn latest_video_pts_us(&self) -> u64 {
        self.latest_video_pts_us
    }

    pub fn latest_audio_pts_us(&self) -> u64 {
        self.latest_audio_pts_us
    }

    /// Evaluates beacon generation interval and produces an outbound sync beacon if due.
    pub fn maybe_generate_sync_beacon(&mut self, now_us: u64) -> Option<ScreenAudioSyncFrame> {
        if self.mode == ScreenAudioSyncMode::Disabled {
            return None;
        }
        let interval = self.mode.beacon_interval_us();
        if self.last_beacon_sent_us > 0
            && now_us.saturating_sub(self.last_beacon_sent_us) < interval
        {
            return None;
        }
        Some(self.trigger_sync_beacon(now_us))
    }

    /// Unconditionally trigger a sync beacon (for tests or manual inspection).
    pub fn trigger_sync_beacon(&mut self, now_us: u64) -> ScreenAudioSyncFrame {
        self.last_beacon_sent_us = now_us;
        self.next_sync_seq = self.next_sync_seq.wrapping_add(1);
        let frame = ScreenAudioSyncFrame {
            flags: 0,
            sync_seq: self.next_sync_seq,
            sender_wallclock_us: now_us,
            video_pts_us: self.latest_video_pts_us,
            audio_pts_us: self.latest_audio_pts_us,
        };
        self.stats.beacons_sent = self.stats.beacons_sent.saturating_add(1);
        self.stats.last_sync_timestamp_ms = now_us / 1000;
        self.beacon_queue.push_back(frame);
        frame
    }

    pub fn pop_sync_beacon(&mut self) -> Option<ScreenAudioSyncFrame> {
        self.beacon_queue.pop_front()
    }

    /// Ingest an inbound sync beacon, calculating presentation skew, clock drift, and applying playout compensation.
    pub fn on_sync_beacon(
        &mut self,
        frame: &ScreenAudioSyncFrame,
        arrival_us: u64,
        playout_buffer: Option<&mut ScreenPlayoutBuffer>,
    ) -> i64 {
        let skew_us = (frame.video_pts_us as i64) - (frame.audio_pts_us as i64);
        self.stats.current_skew_us = skew_us;
        self.stats.beacons_received = self.stats.beacons_received.saturating_add(1);
        self.stats.last_sync_timestamp_ms = arrival_us / 1000;

        if self.recent_samples.len() >= 16 {
            self.recent_samples.pop_front();
        }
        self.recent_samples
            .push_back((arrival_us, frame.sender_wallclock_us));

        if self.recent_samples.len() >= 2 {
            let (first_arrival, first_sender) = self.recent_samples.front().copied().unwrap();
            let elapsed_recv = arrival_us.saturating_sub(first_arrival);
            let elapsed_sender = frame.sender_wallclock_us.saturating_sub(first_sender);
            if elapsed_sender >= 100_000 && elapsed_recv > 0 {
                let drift_ppm = ((elapsed_recv as f64 - elapsed_sender as f64)
                    / (elapsed_sender as f64))
                    * 1_000_000.0;
                if self.stats.clock_drift_ppm == 0.0 {
                    self.stats.clock_drift_ppm = drift_ppm;
                } else {
                    self.stats.clock_drift_ppm =
                        (self.stats.clock_drift_ppm * 0.7) + (drift_ppm * 0.3);
                }
            }
        }

        let tol = self.mode.skew_tolerance_us();
        let in_sync = skew_us.abs() <= tol;
        self.stats.in_sync = in_sync;

        if !in_sync && self.mode != ScreenAudioSyncMode::Disabled {
            if let Some(buf) = playout_buffer {
                let delta_us = (skew_us / 2).clamp(-20_000, 20_000);
                if delta_us != 0 {
                    buf.adjust_target_delay(delta_us);
                    self.stats.adjustments_applied =
                        self.stats.adjustments_applied.saturating_add(1);
                }
            }
        }
        skew_us
    }

    pub fn reset(&mut self) {
        self.next_sync_seq = 0;
        self.last_beacon_sent_us = 0;
        self.latest_video_pts_us = 0;
        self.latest_audio_pts_us = 0;
        self.recent_samples.clear();
        self.beacon_queue.clear();
        self.stats = ScreenAudioSyncStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}
