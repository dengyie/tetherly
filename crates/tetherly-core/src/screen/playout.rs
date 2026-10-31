// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Jitter-absorbing playout buffer and adaptive delay controller.

use super::*;
use std::collections::VecDeque;

/// Minimum playout target delay in microseconds (5ms) for ScreenPlayoutBuffer (v2.01 / ADR-087).
pub const SCREEN_PLAYOUT_MIN_DELAY_US: u64 = 5_000;
/// Maximum playout target delay in microseconds (300ms) for ScreenPlayoutBuffer (v2.01 / ADR-087).
pub const SCREEN_PLAYOUT_MAX_DELAY_US: u64 = 300_000;
/// Default base target delay for adaptive mode in microseconds (15ms).
pub const SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US: u64 = 15_000;
/// Default base target delay for smooth mode in microseconds (35ms).
pub const SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US: u64 = 35_000;
/// Maximum queue capacity for receiver playout buffer (frames).
pub const SCREEN_PLAYOUT_MAX_CAPACITY: usize = 12;
/// Late frame drop threshold in microseconds (80ms): frames scheduled further in the past are dropped to eliminate latency drift.
pub const SCREEN_PLAYOUT_LATE_THRESHOLD_US: u64 = 80_000;

/// Receiver playout dejittering and presentation mode (v2.01 / ADR-087).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenPlayoutMode {
    /// Adaptive dejittering: automatically adjusts target playout delay based on interarrival jitter.
    #[default]
    Adaptive = 0,
    /// Zero-latency: minimal buffering (0-1 frame) and aggressive late-frame dropping for ultra-responsive interaction.
    ZeroLatency = 1,
    /// Smooth: conservative buffering (+35ms base cushion) prioritizing steady cadence for video/scrolling.
    Smooth = 2,
}

impl ScreenPlayoutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenPlayoutMode::Adaptive => "adaptive",
            ScreenPlayoutMode::ZeroLatency => "zerolatency",
            ScreenPlayoutMode::Smooth => "smooth",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "adaptive" | "auto" => Some(ScreenPlayoutMode::Adaptive),
            "zerolatency" | "zero" | "zero-latency" | "low-latency" => {
                Some(ScreenPlayoutMode::ZeroLatency)
            }
            "smooth" | "buffered" => Some(ScreenPlayoutMode::Smooth),
            _ => None,
        }
    }

    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenPlayoutMode::ZeroLatency,
            2 => ScreenPlayoutMode::Smooth,
            _ => ScreenPlayoutMode::Adaptive,
        }
    }
}

/// Action returned by ScreenPlayoutBuffer when checking for frames ready to be presented.
#[derive(Debug)]
pub enum ScreenPlayoutAction {
    /// Frame is ready for presentation.
    Play(ScreenFrame),
    /// No frame is due yet; sleep or wait until this microsecond timestamp.
    WaitUntil(u64),
    /// Playout buffer is empty.
    Empty,
}

/// Statistics and telemetry for receiver playout buffer (v2.01 / ADR-087).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenPlayoutStats {
    pub mode: ScreenPlayoutMode,
    pub target_delay_us: u64,
    pub current_jitter_us: u64,
    pub buffered_frames: usize,
    pub played_frames: u64,
    pub dropped_late_frames: u64,
    pub dropped_overflow_frames: u64,
    pub custom_delay: bool,
}

#[derive(Debug, Clone)]
pub(super) struct PlayoutEntry {
    pub(super) frame: ScreenFrame,
    pub(super) arrival_us: u64,
    pub(super) scheduled_playout_us: u64,
}

/// Receiver playout buffer and adaptive jitter dejitterizer (v2.01 / ADR-087).
///
/// Absorbs network packet delay variation (jitter), calculates interarrival jitter using
/// RFC 3550 exponential moving average, schedules presentation timestamps to eliminate
/// micro-stutter, and drops late or overflowing frames to prevent latency drift.
#[derive(Debug, Clone)]
pub struct ScreenPlayoutBuffer {
    pub(super) mode: ScreenPlayoutMode,
    pub(super) custom_target_delay_us: Option<u64>,
    pub(super) queue: VecDeque<PlayoutEntry>,
    pub(super) last_arrival_us: u64,
    pub(super) last_scheduled_playout_us: u64,
    pub(super) jitter_us: u64,
    pub(super) target_delay_us: u64,
    pub(super) max_capacity: usize,
    pub(super) late_threshold_us: u64,
    pub(super) played_count: u64,
    pub(super) dropped_late_count: u64,
    pub(super) dropped_overflow_count: u64,
}

impl Default for ScreenPlayoutBuffer {
    fn default() -> Self {
        Self::new(ScreenPlayoutMode::default())
    }
}

impl ScreenPlayoutBuffer {
    pub fn new(mode: ScreenPlayoutMode) -> Self {
        let initial_delay = match mode {
            ScreenPlayoutMode::Adaptive => SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US,
            ScreenPlayoutMode::ZeroLatency => SCREEN_PLAYOUT_MIN_DELAY_US,
            ScreenPlayoutMode::Smooth => SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US,
        };
        Self {
            mode,
            custom_target_delay_us: None,
            queue: VecDeque::with_capacity(SCREEN_PLAYOUT_MAX_CAPACITY),
            last_arrival_us: 0,
            last_scheduled_playout_us: 0,
            jitter_us: 0,
            target_delay_us: initial_delay,
            max_capacity: SCREEN_PLAYOUT_MAX_CAPACITY,
            late_threshold_us: SCREEN_PLAYOUT_LATE_THRESHOLD_US,
            played_count: 0,
            dropped_late_count: 0,
            dropped_overflow_count: 0,
        }
    }

    pub fn mode(&self) -> ScreenPlayoutMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenPlayoutMode) {
        self.mode = mode;
        self.recompute_target_delay();
    }

    pub fn set_custom_target_delay_us(&mut self, delay_us: Option<u64>) {
        self.custom_target_delay_us =
            delay_us.map(|d| d.clamp(SCREEN_PLAYOUT_MIN_DELAY_US, SCREEN_PLAYOUT_MAX_DELAY_US));
        self.recompute_target_delay();
    }

    pub fn adjust_target_delay(&mut self, delta_us: i64) {
        let current = self.target_delay_us as i64;
        let new_target = (current + delta_us).clamp(
            SCREEN_PLAYOUT_MIN_DELAY_US as i64,
            SCREEN_PLAYOUT_MAX_DELAY_US as i64,
        ) as u64;
        self.custom_target_delay_us = Some(new_target);
        self.recompute_target_delay();
    }

    pub fn custom_target_delay_us(&self) -> Option<u64> {
        self.custom_target_delay_us
    }

    pub fn target_delay_us(&self) -> u64 {
        self.target_delay_us
    }

    pub fn current_jitter_us(&self) -> u64 {
        self.jitter_us
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Earliest arrival timestamp among currently buffered frames.
    pub fn oldest_arrival_us(&self) -> Option<u64> {
        self.queue.front().map(|e| e.arrival_us)
    }

    /// Next scheduled playout timestamp among currently buffered frames.
    pub fn next_scheduled_playout_us(&self) -> Option<u64> {
        self.queue.front().map(|e| e.scheduled_playout_us)
    }

    pub(super) fn recompute_target_delay(&mut self) {
        if let Some(custom) = self.custom_target_delay_us {
            self.target_delay_us = custom;
            return;
        }
        let calculated = match self.mode {
            ScreenPlayoutMode::Adaptive => SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US
                .saturating_add(self.jitter_us.saturating_mul(2)),
            ScreenPlayoutMode::ZeroLatency => {
                SCREEN_PLAYOUT_MIN_DELAY_US.saturating_add(self.jitter_us / 4)
            }
            ScreenPlayoutMode::Smooth => {
                SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US.saturating_add(self.jitter_us.saturating_mul(3))
            }
        };
        self.target_delay_us =
            calculated.clamp(SCREEN_PLAYOUT_MIN_DELAY_US, SCREEN_PLAYOUT_MAX_DELAY_US);
    }

    /// Push an incoming reassembled and decoded frame with its arrival timestamp (microseconds).
    pub fn push_frame(&mut self, frame: ScreenFrame, arrival_us: u64) {
        // 1. Calculate interarrival jitter (RFC 3550 style).
        if self.last_arrival_us > 0 && arrival_us >= self.last_arrival_us {
            let arrival_interval = arrival_us - self.last_arrival_us;
            let nominal_interval = 33_333u64;
            let diff = arrival_interval.abs_diff(nominal_interval);
            let current = self.jitter_us as i64;
            let sample = diff as i64;
            let next = current + (sample - current) / 16;
            self.jitter_us = next.max(0) as u64;
        }
        self.last_arrival_us = arrival_us;
        self.recompute_target_delay();

        // 2. Schedule playout time.
        let min_playout = arrival_us.saturating_add(self.target_delay_us);
        let scheduled_playout_us =
            if self.last_scheduled_playout_us > 0 && self.last_scheduled_playout_us >= arrival_us {
                let spaced = self.last_scheduled_playout_us.saturating_add(10_000);
                let max_allowed = arrival_us.saturating_add(self.target_delay_us.saturating_mul(2));
                spaced.clamp(min_playout, max_allowed)
            } else {
                min_playout
            };
        self.last_scheduled_playout_us = scheduled_playout_us;

        // 3. Overflow protection: if queue exceeds capacity, drop the oldest frame.
        if self.queue.len() >= self.max_capacity {
            self.queue.pop_front();
            self.dropped_overflow_count = self.dropped_overflow_count.saturating_add(1);
        }

        self.queue.push_back(PlayoutEntry {
            frame,
            arrival_us,
            scheduled_playout_us,
        });
    }

    /// Check the playout queue at the given `now_us` clock time.
    /// Returns `Play(frame)` if due, `WaitUntil(ts)` if pending, or `Empty`.
    pub fn pop_frame(&mut self, now_us: u64) -> ScreenPlayoutAction {
        // Late frame dropping: if front frame is older than now_us - late_threshold and newer frames exist, drop it.
        while let Some(front) = self.queue.front() {
            if self.queue.len() > 1
                && now_us
                    > front
                        .scheduled_playout_us
                        .saturating_add(self.late_threshold_us)
            {
                self.queue.pop_front();
                self.dropped_late_count = self.dropped_late_count.saturating_add(1);
            } else {
                break;
            }
        }

        match self.queue.front() {
            Some(entry) => {
                if now_us >= entry.scheduled_playout_us {
                    let entry = self.queue.pop_front().expect("front exists");
                    self.played_count = self.played_count.saturating_add(1);
                    ScreenPlayoutAction::Play(entry.frame)
                } else {
                    ScreenPlayoutAction::WaitUntil(entry.scheduled_playout_us)
                }
            }
            None => ScreenPlayoutAction::Empty,
        }
    }

    /// Reset playout queue and pacing state on disconnect or peer gone.
    pub fn reset(&mut self) {
        self.queue.clear();
        self.last_arrival_us = 0;
        self.last_scheduled_playout_us = 0;
    }

    /// Return playout buffer stats and telemetry.
    pub fn stats(&self) -> ScreenPlayoutStats {
        ScreenPlayoutStats {
            mode: self.mode,
            target_delay_us: self.target_delay_us,
            current_jitter_us: self.jitter_us,
            buffered_frames: self.queue.len(),
            played_frames: self.played_count,
            dropped_late_frames: self.dropped_late_count,
            dropped_overflow_frames: self.dropped_overflow_count,
            custom_delay: self.custom_target_delay_us.is_some(),
        }
    }
}
