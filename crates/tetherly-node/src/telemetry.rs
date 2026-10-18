// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Unified network traffic and protocol telemetry counters.
//!
//! Tracks bandwidth usage (TX/RX bytes) across all four TCP channels:
//! 1. Control channel (45717)
//! 2. File streaming channel (45718)
//! 3. Remote input channel (45719)
//! 4. Remote screen channel (45720)
//!
//! Also records message-level counters for core protocol frames (pings, pongs,
//! notifications, clipboards, file offers) and session connection lifecycle.
//!
//! Security Red Line: ZERO sensitive payloads or user text. Pure numeric counters.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tetherly_core::frame::{
    TYPE_CLIP_SET, TYPE_FILE_OFFER, TYPE_NOTIFY_PUSH, TYPE_PING, TYPE_PONG,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelTrafficDto {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_frames: u64,
    pub rx_frames: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolMessagesDto {
    pub pings_sent: u64,
    pub pings_recv: u64,
    pub pongs_sent: u64,
    pub pongs_recv: u64,
    pub notifies_sent: u64,
    pub notifies_recv: u64,
    pub clips_sent: u64,
    pub clips_recv: u64,
    pub file_offers_sent: u64,
    pub file_offers_recv: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrafficReport {
    pub ok: bool,
    pub uptime_secs: u64,
    pub total_tx_bytes: u64,
    pub total_rx_bytes: u64,
    pub total_tx_frames: u64,
    pub total_rx_frames: u64,
    pub control: ChannelTrafficDto,
    pub file: ChannelTrafficDto,
    pub input: ChannelTrafficDto,
    pub screen: ChannelTrafficDto,
    pub messages: ProtocolMessagesDto,
    pub sessions_established: u64,
    pub sessions_closed: u64,
    pub handshakes_failed: u64,
}

#[derive(Debug, Default)]
pub struct TrafficTelemetry {
    // Channel 45717 (Control)
    control_tx_bytes: AtomicU64,
    control_rx_bytes: AtomicU64,
    control_tx_frames: AtomicU64,
    control_rx_frames: AtomicU64,

    // Channel 45718 (File)
    file_tx_bytes: AtomicU64,
    file_rx_bytes: AtomicU64,

    // Channel 45719 (Input)
    input_tx_bytes: AtomicU64,
    input_rx_bytes: AtomicU64,
    input_tx_frames: AtomicU64,
    input_rx_frames: AtomicU64,

    // Channel 45720 (Screen)
    screen_tx_bytes: AtomicU64,
    screen_rx_bytes: AtomicU64,
    screen_tx_frames: AtomicU64,
    screen_rx_frames: AtomicU64,

    // Protocol Messages
    pings_sent: AtomicU64,
    pings_recv: AtomicU64,
    pongs_sent: AtomicU64,
    pongs_recv: AtomicU64,
    notifies_sent: AtomicU64,
    notifies_recv: AtomicU64,
    clips_sent: AtomicU64,
    clips_recv: AtomicU64,
    file_offers_sent: AtomicU64,
    file_offers_recv: AtomicU64,

    // Session Lifecycle
    sessions_established: AtomicU64,
    sessions_closed: AtomicU64,
    handshakes_failed: AtomicU64,
}

impl TrafficTelemetry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_control_tx(&self, bytes: u64, ty: u16) {
        self.control_tx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.control_tx_frames.fetch_add(1, Ordering::Relaxed);
        match ty {
            TYPE_PING => {
                self.pings_sent.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_PONG => {
                self.pongs_sent.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_NOTIFY_PUSH => {
                self.notifies_sent.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_CLIP_SET => {
                self.clips_sent.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_FILE_OFFER => {
                self.file_offers_sent.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn record_control_rx(&self, bytes: u64, ty: u16) {
        self.control_rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.control_rx_frames.fetch_add(1, Ordering::Relaxed);
        match ty {
            TYPE_PING => {
                self.pings_recv.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_PONG => {
                self.pongs_recv.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_NOTIFY_PUSH => {
                self.notifies_recv.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_CLIP_SET => {
                self.clips_recv.fetch_add(1, Ordering::Relaxed);
            }
            TYPE_FILE_OFFER => {
                self.file_offers_recv.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn record_file_tx(&self, bytes: u64) {
        self.file_tx_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_file_rx(&self, bytes: u64) {
        self.file_rx_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_input_tx(&self, bytes: u64) {
        self.input_tx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.input_tx_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_input_rx(&self, bytes: u64) {
        self.input_rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.input_rx_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_screen_tx(&self, bytes: u64) {
        self.screen_tx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.screen_tx_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_screen_rx(&self, bytes: u64) {
        self.screen_rx_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.screen_rx_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_session_established(&self) {
        self.sessions_established.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_session_closed(&self) {
        self.sessions_closed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_handshake_failed(&self) {
        self.handshakes_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self, uptime_secs: u64) -> TrafficReport {
        let c_tx_b = self.control_tx_bytes.load(Ordering::Relaxed);
        let c_rx_b = self.control_rx_bytes.load(Ordering::Relaxed);
        let c_tx_f = self.control_tx_frames.load(Ordering::Relaxed);
        let c_rx_f = self.control_rx_frames.load(Ordering::Relaxed);

        let f_tx_b = self.file_tx_bytes.load(Ordering::Relaxed);
        let f_rx_b = self.file_rx_bytes.load(Ordering::Relaxed);

        let i_tx_b = self.input_tx_bytes.load(Ordering::Relaxed);
        let i_rx_b = self.input_rx_bytes.load(Ordering::Relaxed);
        let i_tx_f = self.input_tx_frames.load(Ordering::Relaxed);
        let i_rx_f = self.input_rx_frames.load(Ordering::Relaxed);

        let s_tx_b = self.screen_tx_bytes.load(Ordering::Relaxed);
        let s_rx_b = self.screen_rx_bytes.load(Ordering::Relaxed);
        let s_tx_f = self.screen_tx_frames.load(Ordering::Relaxed);
        let s_rx_f = self.screen_rx_frames.load(Ordering::Relaxed);

        let total_tx_bytes = c_tx_b
            .saturating_add(f_tx_b)
            .saturating_add(i_tx_b)
            .saturating_add(s_tx_b);
        let total_rx_bytes = c_rx_b
            .saturating_add(f_rx_b)
            .saturating_add(i_rx_b)
            .saturating_add(s_rx_b);
        let total_tx_frames = c_tx_f.saturating_add(i_tx_f).saturating_add(s_tx_f);
        let total_rx_frames = c_rx_f.saturating_add(i_rx_f).saturating_add(s_rx_f);

        TrafficReport {
            ok: true,
            uptime_secs,
            total_tx_bytes,
            total_rx_bytes,
            total_tx_frames,
            total_rx_frames,
            control: ChannelTrafficDto {
                tx_bytes: c_tx_b,
                rx_bytes: c_rx_b,
                tx_frames: c_tx_f,
                rx_frames: c_rx_f,
            },
            file: ChannelTrafficDto {
                tx_bytes: f_tx_b,
                rx_bytes: f_rx_b,
                tx_frames: 0,
                rx_frames: 0,
            },
            input: ChannelTrafficDto {
                tx_bytes: i_tx_b,
                rx_bytes: i_rx_b,
                tx_frames: i_tx_f,
                rx_frames: i_rx_f,
            },
            screen: ChannelTrafficDto {
                tx_bytes: s_tx_b,
                rx_bytes: s_rx_b,
                tx_frames: s_tx_f,
                rx_frames: s_rx_f,
            },
            messages: ProtocolMessagesDto {
                pings_sent: self.pings_sent.load(Ordering::Relaxed),
                pings_recv: self.pings_recv.load(Ordering::Relaxed),
                pongs_sent: self.pongs_sent.load(Ordering::Relaxed),
                pongs_recv: self.pongs_recv.load(Ordering::Relaxed),
                notifies_sent: self.notifies_sent.load(Ordering::Relaxed),
                notifies_recv: self.notifies_recv.load(Ordering::Relaxed),
                clips_sent: self.clips_sent.load(Ordering::Relaxed),
                clips_recv: self.clips_recv.load(Ordering::Relaxed),
                file_offers_sent: self.file_offers_sent.load(Ordering::Relaxed),
                file_offers_recv: self.file_offers_recv.load(Ordering::Relaxed),
            },
            sessions_established: self.sessions_established.load(Ordering::Relaxed),
            sessions_closed: self.sessions_closed.load(Ordering::Relaxed),
            handshakes_failed: self.handshakes_failed.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.control_tx_bytes.store(0, Ordering::Relaxed);
        self.control_rx_bytes.store(0, Ordering::Relaxed);
        self.control_tx_frames.store(0, Ordering::Relaxed);
        self.control_rx_frames.store(0, Ordering::Relaxed);

        self.file_tx_bytes.store(0, Ordering::Relaxed);
        self.file_rx_bytes.store(0, Ordering::Relaxed);

        self.input_tx_bytes.store(0, Ordering::Relaxed);
        self.input_rx_bytes.store(0, Ordering::Relaxed);
        self.input_tx_frames.store(0, Ordering::Relaxed);
        self.input_rx_frames.store(0, Ordering::Relaxed);

        self.screen_tx_bytes.store(0, Ordering::Relaxed);
        self.screen_rx_bytes.store(0, Ordering::Relaxed);
        self.screen_tx_frames.store(0, Ordering::Relaxed);
        self.screen_rx_frames.store(0, Ordering::Relaxed);

        self.pings_sent.store(0, Ordering::Relaxed);
        self.pings_recv.store(0, Ordering::Relaxed);
        self.pongs_sent.store(0, Ordering::Relaxed);
        self.pongs_recv.store(0, Ordering::Relaxed);
        self.notifies_sent.store(0, Ordering::Relaxed);
        self.notifies_recv.store(0, Ordering::Relaxed);
        self.clips_sent.store(0, Ordering::Relaxed);
        self.clips_recv.store(0, Ordering::Relaxed);
        self.file_offers_sent.store(0, Ordering::Relaxed);
        self.file_offers_recv.store(0, Ordering::Relaxed);

        self.sessions_established.store(0, Ordering::Relaxed);
        self.sessions_closed.store(0, Ordering::Relaxed);
        self.handshakes_failed.store(0, Ordering::Relaxed);
    }
}
