// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Progressive intra refresh, long-term reference frames and temporal scalability.

use super::*;
use crate::error::CoreError;

pub const SCREEN_PIR_DEFAULT_BANDS: u8 = 8;
pub const SCREEN_PIR_MIN_BANDS: u8 = 2;
pub const SCREEN_PIR_MAX_BANDS: u8 = 32;

/// Progressive Intra-Refresh (PIR) Operating Mode (v2.26 / ADR-112).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenPirMode {
    #[default]
    Disabled = 0,
    Continuous = 1,
    OnDemand = 2,
}

impl ScreenPirMode {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenPirMode::Disabled => "disabled",
            ScreenPirMode::Continuous => "continuous",
            ScreenPirMode::OnDemand => "on-demand",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ScreenPirMode::Continuous,
            2 => ScreenPirMode::OnDemand,
            _ => ScreenPirMode::Disabled,
        }
    }

    pub fn is_active(self) -> bool {
        self != ScreenPirMode::Disabled
    }
}

impl std::fmt::Display for ScreenPirMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenPirMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "disabled" | "off" | "0" => Ok(ScreenPirMode::Disabled),
            "continuous" | "rolling" | "1" => Ok(ScreenPirMode::Continuous),
            "on-demand" | "ondemand" | "demand" | "2" => Ok(ScreenPirMode::OnDemand),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Progressive Intra-Refresh (PIR) Configuration (v2.26 / ADR-112).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenPirConfig {
    pub mode: ScreenPirMode,
    pub bands: u8,
}

impl Default for ScreenPirConfig {
    fn default() -> Self {
        Self {
            mode: ScreenPirMode::Disabled,
            bands: SCREEN_PIR_DEFAULT_BANDS,
        }
    }
}

impl ScreenPirConfig {
    pub fn new(mode: ScreenPirMode, bands: Option<u8>) -> Self {
        let b = match bands {
            Some(n) if n > 0 => n.clamp(SCREEN_PIR_MIN_BANDS, SCREEN_PIR_MAX_BANDS),
            _ => SCREEN_PIR_DEFAULT_BANDS,
        };
        Self { mode, bands: b }
    }

    pub fn is_enabled(&self) -> bool {
        self.mode != ScreenPirMode::Disabled
    }
}

/// Progressive Intra-Refresh (PIR) Status and Progress Telemetry Report (v2.26 / ADR-112).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenPirReport {
    pub ok: bool,
    pub mode: String,
    pub bands: u8,
    pub sweeps_completed: u64,
    pub slices_sent: u64,
    pub current_band: u8,
    pub active_sweep: bool,
}

impl Default for ScreenPirReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenPirMode::Disabled.as_str().to_string(),
            bands: SCREEN_PIR_DEFAULT_BANDS,
            sweeps_completed: 0,
            slices_sent: 0,
            current_band: 0,
            active_sweep: false,
        }
    }
}

/// Computes the horizontal band rectangle for a given slice in Progressive Intra-Refresh.
/// Returns `(x, y, w, h)`. The last slice absorbs remainder pixels to strictly cover the full height.
pub fn compute_pir_band_rect(
    width: u32,
    height: u32,
    bands: u8,
    band_index: u8,
) -> (u32, u32, u32, u32) {
    let b = (bands.clamp(SCREEN_PIR_MIN_BANDS, SCREEN_PIR_MAX_BANDS) as u32).min(height.max(1));
    let idx = (band_index as u32) % b;
    let base_h = height / b;
    let y = idx * base_h;
    let h = if idx == b - 1 {
        height.saturating_sub(y)
    } else {
        base_h
    };
    (0, y, width, h)
}

/// Long-Term Reference (LTR) Frame Configuration (v2.27 / ADR-113).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenLtrConfig {
    pub enabled: bool,
    pub auto_interval: u16,
}

impl Default for ScreenLtrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_interval: SCREEN_LTR_DEFAULT_AUTO_INTERVAL,
        }
    }
}

impl ScreenLtrConfig {
    pub fn new(enabled: bool, auto_interval: Option<u16>) -> Self {
        let interval = match auto_interval {
            Some(i) if i > 0 => i.clamp(SCREEN_LTR_MIN_INTERVAL, SCREEN_LTR_MAX_INTERVAL),
            Some(_) => 0,
            None => SCREEN_LTR_DEFAULT_AUTO_INTERVAL,
        };
        Self {
            enabled,
            auto_interval: interval,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Long-Term Reference (LTR) Status and Recovery Telemetry Report (v2.27 / ADR-113).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenLtrReport {
    pub ok: bool,
    pub enabled: bool,
    pub auto_interval: u16,
    pub current_ltr_seq: u64,
    pub anchors_established: u64,
    pub recoveries_requested: u64,
    pub recoveries_completed: u64,
    pub saved_burst_bytes: u64,
}

impl Default for ScreenLtrReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            auto_interval: SCREEN_LTR_DEFAULT_AUTO_INTERVAL,
            current_ltr_seq: 0,
            anchors_established: 0,
            recoveries_requested: 0,
            recoveries_completed: 0,
            saved_burst_bytes: 0,
        }
    }
}

/// Screen 2-Layer Temporal Scalability (SVC-T) Operational Mode (v2.28 / ADR-114).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ScreenSvcMode {
    #[default]
    Disabled = 0,
    Adaptive = 1,
    BaseOnly = 2,
}

impl ScreenSvcMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => ScreenSvcMode::Adaptive,
            2 => ScreenSvcMode::BaseOnly,
            _ => ScreenSvcMode::Disabled,
        }
    }

    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ScreenSvcMode::Disabled => "disabled",
            ScreenSvcMode::Adaptive => "adaptive",
            ScreenSvcMode::BaseOnly => "base_only",
        }
    }

    pub fn parse_mode(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "adaptive" | "adapt" | "1" => ScreenSvcMode::Adaptive,
            "base_only" | "baseonly" | "base" | "2" => ScreenSvcMode::BaseOnly,
            _ => ScreenSvcMode::Disabled,
        }
    }
}

/// Screen 2-Layer Temporal Scalability (SVC-T) Configuration (v2.28 / ADR-114).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ScreenSvcConfig {
    pub mode: ScreenSvcMode,
    pub drop_t1: bool,
}

impl ScreenSvcConfig {
    pub fn new(mode: ScreenSvcMode, drop_t1: bool) -> Self {
        Self { mode, drop_t1 }
    }

    pub fn is_enabled(&self) -> bool {
        self.mode != ScreenSvcMode::Disabled
    }

    pub fn should_drop_t1(&self) -> bool {
        self.mode == ScreenSvcMode::BaseOnly || self.drop_t1
    }
}

/// Screen 2-Layer Temporal Scalability (SVC-T) Telemetry Report (v2.28 / ADR-114).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenSvcReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: String,
    pub drop_t1: bool,
    pub t0_frames_sent: u64,
    pub t1_frames_sent: u64,
    pub t1_frames_dropped: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenSvcReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            mode: "disabled".into(),
            drop_t1: false,
            t0_frames_sent: 0,
            t1_frames_sent: 0,
            t1_frames_dropped: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}
