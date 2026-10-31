// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Idle squelch controller and static-screen suppression decisions.

use super::*;
use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;

/// Screen streaming adaptive squelch mode (v2.03 / ADR-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ScreenSquelchMode {
    /// Adaptive intelligent squelch: halts full-frame video push after 500ms of static screen,
    /// waking instantly on damage and sending a refresh keepalive every 2.0s.
    #[default]
    Auto = 0,
    /// Aggressive power/bandwidth saving squelch: squelches after 150ms with 5.0s keepalive.
    Aggressive = 1,
    /// Disabled / passthrough: continuously pushes video frames regardless of damage state.
    Disabled = 2,
}

impl ScreenSquelchMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }

    pub const fn tag(&self) -> u8 {
        *self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Aggressive,
            2 => Self::Disabled,
            _ => Self::Auto,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "aggressive" | "agg" | "aggro" => Some(Self::Aggressive),
            "disabled" | "off" | "none" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl fmt::Display for ScreenSquelchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenSquelchMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or(CoreError::ScreenFrame)
    }
}

/// Squelch dispatch decision returned on each frame candidate evaluation (v2.03 / ADR-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSquelchDecision {
    /// Proceed with encoding and transmitting the video frame.
    SendFrame,
    /// Suppress frame encoding and transmission because screen is static.
    SquelchSkip,
}

/// Real-time telemetry and running counters for the squelch controller (v2.03 / ADR-089).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenSquelchStats {
    pub mode: ScreenSquelchMode,
    pub is_squelched: bool,
    pub frames_evaluated: u64,
    pub frames_passed: u64,
    pub frames_squelched: u64,
    pub keepalive_frames: u64,
    pub last_dirty_ratio: f32,
    pub consecutive_static_frames: u64,
}

/// Zero-motion adaptive squelch controller (v2.03 / ADR-089).
#[derive(Debug, Clone)]
pub struct ScreenSquelchController {
    pub(super) mode: ScreenSquelchMode,
    pub(super) stats: ScreenSquelchStats,
    pub(super) last_change_us: u64,
    pub(super) last_sent_us: u64,
}

impl Default for ScreenSquelchController {
    fn default() -> Self {
        Self::new(ScreenSquelchMode::default())
    }
}

impl ScreenSquelchController {
    pub fn new(mode: ScreenSquelchMode) -> Self {
        Self {
            mode,
            stats: ScreenSquelchStats {
                mode,
                ..Default::default()
            },
            last_change_us: 0,
            last_sent_us: 0,
        }
    }

    pub fn mode(&self) -> ScreenSquelchMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenSquelchMode) -> ScreenSquelchMode {
        self.mode = mode;
        self.stats.mode = mode;
        mode
    }

    pub fn is_squelched(&self) -> bool {
        self.stats.is_squelched
    }

    pub fn stats(&self) -> ScreenSquelchStats {
        self.stats.clone()
    }

    /// Wake up immediately upon explicit user input or keyframe request.
    pub fn force_wake(&mut self, now_us: u64) {
        self.last_change_us = now_us;
        self.stats.is_squelched = false;
        self.stats.consecutive_static_frames = 0;
    }

    /// Evaluate candidate frame against damage state and rate limits.
    pub fn evaluate_decision(
        &mut self,
        is_dirty: bool,
        dirty_ratio: f32,
        now_us: u64,
    ) -> ScreenSquelchDecision {
        if self.mode == ScreenSquelchMode::Disabled {
            self.stats.frames_evaluated = self.stats.frames_evaluated.saturating_add(1);
            self.stats.frames_passed = self.stats.frames_passed.saturating_add(1);
            self.stats.is_squelched = false;
            self.stats.last_dirty_ratio = dirty_ratio;
            self.last_sent_us = now_us;
            return ScreenSquelchDecision::SendFrame;
        }

        self.stats.frames_evaluated = self.stats.frames_evaluated.saturating_add(1);
        self.stats.last_dirty_ratio = dirty_ratio;

        if is_dirty {
            self.last_change_us = now_us;
            self.last_sent_us = now_us;
            self.stats.is_squelched = false;
            self.stats.consecutive_static_frames = 0;
            self.stats.frames_passed = self.stats.frames_passed.saturating_add(1);
            ScreenSquelchDecision::SendFrame
        } else {
            self.stats.consecutive_static_frames =
                self.stats.consecutive_static_frames.saturating_add(1);
            let (delay_us, keepalive_us) = match self.mode {
                ScreenSquelchMode::Auto => (
                    SCREEN_SQUELCH_AUTO_DELAY_US,
                    SCREEN_SQUELCH_AUTO_KEEPALIVE_US,
                ),
                ScreenSquelchMode::Aggressive => (
                    SCREEN_SQUELCH_AGGRESSIVE_DELAY_US,
                    SCREEN_SQUELCH_AGGRESSIVE_KEEPALIVE_US,
                ),
                ScreenSquelchMode::Disabled => (0, 0),
            };

            // Check if we entered squelch state
            if self.last_change_us > 0 && now_us.saturating_sub(self.last_change_us) >= delay_us {
                self.stats.is_squelched = true;
                // Periodic keepalive check
                if self.last_sent_us == 0
                    || now_us.saturating_sub(self.last_sent_us) >= keepalive_us
                {
                    self.last_sent_us = now_us;
                    self.stats.frames_passed = self.stats.frames_passed.saturating_add(1);
                    self.stats.keepalive_frames = self.stats.keepalive_frames.saturating_add(1);
                    ScreenSquelchDecision::SendFrame
                } else {
                    self.stats.frames_squelched = self.stats.frames_squelched.saturating_add(1);
                    ScreenSquelchDecision::SquelchSkip
                }
            } else {
                // Grace transition window
                self.stats.is_squelched = false;
                self.last_sent_us = now_us;
                self.stats.frames_passed = self.stats.frames_passed.saturating_add(1);
                ScreenSquelchDecision::SendFrame
            }
        }
    }
}
