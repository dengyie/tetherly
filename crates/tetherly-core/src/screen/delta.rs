// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Differential delta-patch configuration and reporting.

use serde::Deserialize;
use serde::Serialize;

/// Default keyframe interval for dynamic differential delta patch pipeline (v2.23 / ADR-109).
pub const SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL: u16 = 60;

/// Configuration parameters for Screen Dynamic Differential Delta Patch pipeline (v2.23 / ADR-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenDeltaConfig {
    pub enabled: bool,
    pub keyframe_interval: u16,
}

impl Default for ScreenDeltaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keyframe_interval: SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL,
        }
    }
}

impl ScreenDeltaConfig {
    pub fn new(enabled: bool, keyframe_interval: Option<u16>) -> Self {
        Self {
            enabled,
            keyframe_interval: keyframe_interval
                .unwrap_or(SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL)
                .clamp(1, 600),
        }
    }
}

/// Status report for the active Screen Differential Delta Patch pipeline (v2.23 / ADR-109).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenDeltaReport {
    pub ok: bool,
    pub enabled: bool,
    pub keyframe_interval: u16,
    pub delta_frames_sent: u64,
    pub keyframes_sent: u64,
    pub delta_frames_received: u64,
    pub keyframes_received: u64,
    pub saved_bytes: u64,
}

impl Default for ScreenDeltaReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            keyframe_interval: SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL,
            delta_frames_sent: 0,
            keyframes_sent: 0,
            delta_frames_received: 0,
            keyframes_received: 0,
            saved_bytes: 0,
        }
    }
}
