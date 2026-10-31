// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Objective PSNR/SSIM metrics and the closed-loop QoE controller.

use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;

/// Default sample interval in frames for screen objective visual quality metric evaluation (v2.24 / ADR-110).
pub const SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL: u8 = 10;

/// Configuration parameters for Screen Objective Visual Quality Metric pipeline (v2.24 / ADR-110).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenQualityMetricConfig {
    pub enabled: bool,
    pub sample_interval: u8,
}

impl Default for ScreenQualityMetricConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_interval: SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL,
        }
    }
}

impl ScreenQualityMetricConfig {
    pub fn new(enabled: bool, sample_interval: Option<u8>) -> Self {
        Self {
            enabled,
            sample_interval: sample_interval
                .unwrap_or(SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL)
                .clamp(1, 120),
        }
    }
}

/// Status and quality telemetry report for the active Screen Quality Metric pipeline (v2.24 / ADR-110).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenQualityMetricReport {
    pub ok: bool,
    pub enabled: bool,
    pub sample_interval: u8,
    pub evaluated_frames: u64,
    pub avg_mse: f64,
    pub avg_psnr_db: f64,
    pub avg_ssim: f64,
    pub last_psnr_db: f64,
    pub last_ssim: f64,
    pub quality_warning: bool,
}

impl Default for ScreenQualityMetricReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            sample_interval: SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL,
            evaluated_frames: 0,
            avg_mse: 0.0,
            avg_psnr_db: 100.0,
            avg_ssim: 1.0,
            last_psnr_db: 100.0,
            last_ssim: 1.0,
            quality_warning: false,
        }
    }
}

/// Default target PSNR in dB for QoE closed-loop adaptation (v2.25 / ADR-111).
pub const SCREEN_QOE_DEFAULT_TARGET_PSNR_DB: u8 = 30;
/// Default target SSIM in percent (85 -> 0.85) for QoE closed-loop adaptation (v2.25 / ADR-111).
pub const SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT: u8 = 85;
/// Default cooldown duration in microseconds between consecutive QoE adaptations (v2.25 / ADR-111).
pub const SCREEN_QOE_COOLDOWN_US: u64 = 500_000;

/// Policy mode for Screen Autonomous Closed-Loop QoE Adaptation (v2.25 / ADR-111).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenQoeMode {
    #[default]
    Off = 0,
    Conservative = 1,
    Balanced = 2,
    Economy = 3,
}

impl ScreenQoeMode {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Conservative,
            2 => Self::Balanced,
            3 => Self::Economy,
            _ => Self::Off,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Conservative => "conservative",
            Self::Balanced => "balanced",
            Self::Economy => "economy",
        }
    }

    pub fn default_targets(&self) -> (u8, u8) {
        match self {
            Self::Off => (0, 0),
            Self::Conservative => (34, 90),
            Self::Balanced => (30, 85),
            Self::Economy => (26, 78),
        }
    }

    pub fn is_active(&self) -> bool {
        *self != Self::Off
    }
}

impl std::fmt::Display for ScreenQoeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenQoeMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "none" => Ok(Self::Off),
            "conservative" | "quality" | "high" => Ok(Self::Conservative),
            "balanced" | "auto" | "default" => Ok(Self::Balanced),
            "economy" | "eco" | "low" => Ok(Self::Economy),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Action decision emitted by Screen Autonomous Closed-Loop QoE Controller (v2.25 / ADR-111).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenQoeAction {
    #[default]
    Hold = 0,
    BoostBitrate = 1,
    UpgradeScale = 2,
    LowerQp = 3,
    ForceKeyframe = 4,
    FallbackThrottle = 5,
}

impl ScreenQoeAction {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::BoostBitrate,
            2 => Self::UpgradeScale,
            3 => Self::LowerQp,
            4 => Self::ForceKeyframe,
            5 => Self::FallbackThrottle,
            _ => Self::Hold,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::BoostBitrate => "boost_bitrate",
            Self::UpgradeScale => "upgrade_scale",
            Self::LowerQp => "lower_qp",
            Self::ForceKeyframe => "force_keyframe",
            Self::FallbackThrottle => "fallback_throttle",
        }
    }
}

impl std::fmt::Display for ScreenQoeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Configuration parameters for Screen Objective QoE Closed-Loop Adaptation (v2.25 / ADR-111).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenQoeConfig {
    pub mode: ScreenQoeMode,
    pub target_psnr_db: u8,
    pub target_ssim_pct: u8,
}

impl Default for ScreenQoeConfig {
    fn default() -> Self {
        Self {
            mode: ScreenQoeMode::Off,
            target_psnr_db: SCREEN_QOE_DEFAULT_TARGET_PSNR_DB,
            target_ssim_pct: SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT,
        }
    }
}

impl ScreenQoeConfig {
    pub fn new(mode: ScreenQoeMode, target_psnr: Option<u8>, target_ssim: Option<u8>) -> Self {
        let (def_psnr, def_ssim) = mode.default_targets();
        Self {
            mode,
            target_psnr_db: target_psnr
                .unwrap_or(if def_psnr > 0 {
                    def_psnr
                } else {
                    SCREEN_QOE_DEFAULT_TARGET_PSNR_DB
                })
                .clamp(10, 60),
            target_ssim_pct: target_ssim
                .unwrap_or(if def_ssim > 0 {
                    def_ssim
                } else {
                    SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT
                })
                .clamp(10, 100),
        }
    }

    pub fn is_active(&self) -> bool {
        self.mode != ScreenQoeMode::Off
    }
}

/// Status and decision telemetry report for the Screen QoE Closed-Loop Pipeline (v2.25 / ADR-111).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenQoeReport {
    pub ok: bool,
    pub mode: String,
    pub target_psnr_db: u8,
    pub target_ssim_pct: u8,
    pub actions_total: u64,
    pub last_action: String,
    pub last_action_reason: String,
    pub in_cooldown: bool,
    pub active_boosts: u32,
}

impl Default for ScreenQoeReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenQoeMode::Off.as_str().to_string(),
            target_psnr_db: SCREEN_QOE_DEFAULT_TARGET_PSNR_DB,
            target_ssim_pct: SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT,
            actions_total: 0,
            last_action: ScreenQoeAction::Hold.as_str().to_string(),
            last_action_reason: "initialized".to_string(),
            in_cooldown: false,
            active_boosts: 0,
        }
    }
}

/// Autonomous Closed-Loop QoE Controller (v2.25 / ADR-111).
#[derive(Debug, Clone)]
pub struct ScreenQoeController {
    pub(super) config: ScreenQoeConfig,
    pub(super) last_action_us: u64,
    pub(super) actions_total: u64,
    pub(super) last_action: ScreenQoeAction,
    pub(super) last_reason: String,
    pub(super) active_boosts: u32,
}

impl Default for ScreenQoeController {
    fn default() -> Self {
        Self::new(ScreenQoeConfig::default())
    }
}

impl ScreenQoeController {
    pub fn new(config: ScreenQoeConfig) -> Self {
        Self {
            config,
            last_action_us: 0,
            actions_total: 0,
            last_action: ScreenQoeAction::Hold,
            last_reason: "idle".to_string(),
            active_boosts: 0,
        }
    }

    pub fn config(&self) -> &ScreenQoeConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: ScreenQoeConfig) {
        self.config = config;
    }

    pub fn actions_total(&self) -> u64 {
        self.actions_total
    }

    pub fn last_action(&self) -> ScreenQoeAction {
        self.last_action
    }

    pub fn is_in_cooldown(&self, now_us: u64) -> bool {
        self.last_action_us != 0
            && now_us.saturating_sub(self.last_action_us) < SCREEN_QOE_COOLDOWN_US
    }

    pub fn evaluate(&mut self, metric: &ScreenQualityMetricReport, now_us: u64) -> ScreenQoeAction {
        if !self.config.is_active() || metric.evaluated_frames == 0 {
            return ScreenQoeAction::Hold;
        }

        // Anti-oscillation cooldown window
        if self.is_in_cooldown(now_us) {
            return ScreenQoeAction::Hold;
        }

        let target_psnr = self.config.target_psnr_db as f64;
        let target_ssim = (self.config.target_ssim_pct as f64) / 100.0;

        let curr_psnr = metric.last_psnr_db;
        let curr_ssim = metric.last_ssim;

        let psnr_deficit = target_psnr - curr_psnr;
        let ssim_deficit = target_ssim - curr_ssim;

        // If quality satisfies configured targets, no action needed
        if psnr_deficit <= 0.0 && ssim_deficit <= 0.0 && !metric.quality_warning {
            if self.active_boosts > 0 && self.last_action != ScreenQoeAction::Hold {
                self.last_action = ScreenQoeAction::Hold;
                self.last_reason = "quality_satisfied".to_string();
            }
            return ScreenQoeAction::Hold;
        }

        // Determine prioritized action based on distortion characteristics
        let (action, reason) = if ssim_deficit > 0.15 {
            // Structural distortion is severe -> Resolution downsampling artifact
            (
                ScreenQoeAction::UpgradeScale,
                format!("ssim_deficit_high: {curr_ssim:.3} < {target_ssim:.3}"),
            )
        } else if psnr_deficit > 8.0 {
            // Large pixel noise/quantization errors -> QP too high
            (
                ScreenQoeAction::LowerQp,
                format!("psnr_deficit_high: {curr_psnr:.1}dB < {target_psnr:.1}dB"),
            )
        } else if ssim_deficit > 0.05 || psnr_deficit > 2.0 || metric.quality_warning {
            // General compression artifacts -> Boost bitrate
            (
                ScreenQoeAction::BoostBitrate,
                format!("general_deficit: psnr={curr_psnr:.1}dB ssim={curr_ssim:.3}"),
            )
        } else if curr_psnr < 20.0 {
            // Severe persistent drift or corruption -> Force Keyframe
            (
                ScreenQoeAction::ForceKeyframe,
                "critical_distortion_keyframe".to_string(),
            )
        } else {
            (ScreenQoeAction::Hold, "acceptable".to_string())
        };

        if action != ScreenQoeAction::Hold {
            self.actions_total = self.actions_total.saturating_add(1);
            self.last_action = action;
            self.last_reason = reason;
            self.last_action_us = now_us;
            self.active_boosts = self.active_boosts.saturating_add(1);
        }

        action
    }

    pub fn report(&self, now_us: u64) -> ScreenQoeReport {
        ScreenQoeReport {
            ok: true,
            mode: self.config.mode.as_str().to_string(),
            target_psnr_db: self.config.target_psnr_db,
            target_ssim_pct: self.config.target_ssim_pct,
            actions_total: self.actions_total,
            last_action: self.last_action.as_str().to_string(),
            last_action_reason: self.last_reason.clone(),
            in_cooldown: self.is_in_cooldown(now_us),
            active_boosts: self.active_boosts,
        }
    }

    pub fn reset(&mut self) {
        self.config = ScreenQoeConfig::default();
        self.last_action_us = 0;
        self.actions_total = 0;
        self.last_action = ScreenQoeAction::Hold;
        self.last_reason = "reset".to_string();
        self.active_boosts = 0;
    }
}
