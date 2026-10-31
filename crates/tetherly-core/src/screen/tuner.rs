// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Bottleneck-driven autonomous tuning closed loop.

use super::*;
use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;

/// Bottleneck-driven autonomous tuning operational mode (v2.14 / ADR-100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenTunerMode {
    Disabled = 0,
    #[default]
    Standard = 1,
    Aggressive = 2,
}

impl ScreenTunerMode {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Disabled,
            2 => Self::Aggressive,
            _ => Self::Standard,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Standard => "standard",
            Self::Aggressive => "aggressive",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            "standard" | "auto" | "default" | "normal" => Ok(Self::Standard),
            "aggressive" | "fast" | "boost" => Ok(Self::Aggressive),
            _ => Err(CoreError::ScreenFrame),
        }
    }

    /// Consecutive same-bottleneck frames required before acting.
    pub fn trigger_frames(self) -> u32 {
        match self {
            Self::Aggressive => SCREEN_TUNER_TRIGGER_FRAMES_AGGRESSIVE,
            _ => SCREEN_TUNER_TRIGGER_FRAMES_STANDARD,
        }
    }

    /// Consecutive balanced frames required before recovering one step.
    pub fn recovery_frames(self) -> u32 {
        match self {
            Self::Aggressive => SCREEN_TUNER_RECOVERY_FRAMES_AGGRESSIVE,
            _ => SCREEN_TUNER_RECOVERY_FRAMES_STANDARD,
        }
    }

    /// Minimum spacing between two actual tuner actions, in microseconds.
    pub fn cooldown_us(self) -> u64 {
        match self {
            Self::Aggressive => SCREEN_TUNER_COOLDOWN_US_AGGRESSIVE,
            _ => SCREEN_TUNER_COOLDOWN_US_STANDARD,
        }
    }
}

impl std::str::FromStr for ScreenTunerMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for ScreenTunerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Discrete actions the bottleneck-driven autonomous tuner can apply (v2.14 / ADR-100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenTuningAction {
    #[default]
    Hold = 0,
    ReduceFps = 1,
    ReduceScale = 2,
    ReduceBitrate = 3,
    RaiseQp = 4,
    RaisePlayoutDelay = 5,
    RecoverBitrate = 6,
    RecoverFps = 7,
    RecoverScale = 8,
    RecoverQp = 9,
    LowerPlayoutDelay = 10,
}

impl ScreenTuningAction {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::ReduceFps,
            2 => Self::ReduceScale,
            3 => Self::ReduceBitrate,
            4 => Self::RaiseQp,
            5 => Self::RaisePlayoutDelay,
            6 => Self::RecoverBitrate,
            7 => Self::RecoverFps,
            8 => Self::RecoverScale,
            9 => Self::RecoverQp,
            10 => Self::LowerPlayoutDelay,
            _ => Self::Hold,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::ReduceFps => "reduce_fps",
            Self::ReduceScale => "reduce_scale",
            Self::ReduceBitrate => "reduce_bitrate",
            Self::RaiseQp => "raise_qp",
            Self::RaisePlayoutDelay => "raise_playout_delay",
            Self::RecoverBitrate => "recover_bitrate",
            Self::RecoverFps => "recover_fps",
            Self::RecoverScale => "recover_scale",
            Self::RecoverQp => "recover_qp",
            Self::LowerPlayoutDelay => "lower_playout_delay",
        }
    }

    pub fn is_recovery(self) -> bool {
        matches!(
            self,
            Self::RecoverBitrate
                | Self::RecoverFps
                | Self::RecoverScale
                | Self::RecoverQp
                | Self::LowerPlayoutDelay
        )
    }
}

impl std::fmt::Display for ScreenTuningAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenTuningAction {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hold" => Ok(Self::Hold),
            "reduce_fps" => Ok(Self::ReduceFps),
            "reduce_scale" => Ok(Self::ReduceScale),
            "reduce_bitrate" => Ok(Self::ReduceBitrate),
            "raise_qp" => Ok(Self::RaiseQp),
            "raise_playout_delay" => Ok(Self::RaisePlayoutDelay),
            "recover_bitrate" => Ok(Self::RecoverBitrate),
            "recover_fps" => Ok(Self::RecoverFps),
            "recover_scale" => Ok(Self::RecoverScale),
            "recover_qp" => Ok(Self::RecoverQp),
            "lower_playout_delay" => Ok(Self::LowerPlayoutDelay),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Snapshot of the current pipeline knobs passed into a tuner evaluation (v2.14 / ADR-100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenTuningContext {
    pub bitrate_bps: u32,
    pub bitrate_min_bps: u32,
    pub bitrate_max_bps: u32,
    pub fps: u32,
    pub scale_pct: u32,
    pub qp_min: u8,
    pub qp_max: u8,
    pub playout_delay_us: u32,
}

/// Decision emitted by one tuner evaluation; unchanged dimensions stay `None` (v2.14 / ADR-100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenTuningDecision {
    pub frame_seq: u64,
    pub stage: String,
    pub action: String,
    pub new_bitrate_bps: Option<u32>,
    pub new_fps: Option<u32>,
    pub new_scale_pct: Option<u32>,
    pub new_qp_max: Option<u8>,
    pub new_playout_delay_us: Option<u32>,
    pub recovered: bool,
    pub at_us: u64,
}

impl Default for ScreenTuningDecision {
    fn default() -> Self {
        Self {
            frame_seq: 0,
            stage: ScreenBottleneckStage::Balanced.as_str().to_string(),
            action: ScreenTuningAction::Hold.as_str().to_string(),
            new_bitrate_bps: None,
            new_fps: None,
            new_scale_pct: None,
            new_qp_max: None,
            new_playout_delay_us: None,
            recovered: false,
            at_us: 0,
        }
    }
}

/// Result of one tuner evaluation: decision plus the concrete knob updates to apply (v2.14 / ADR-100).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScreenTuningOutcome {
    pub decision: ScreenTuningDecision,
    pub bitrate_bps: Option<u32>,
    pub fps: Option<u32>,
    pub scale_pct: Option<u32>,
    pub qp_range: Option<(u8, u8)>,
    pub playout_delay_delta_us: Option<i64>,
}

impl ScreenTuningOutcome {
    pub(super) fn hold(frame_seq: u64, stage: ScreenBottleneckStage, now_us: u64) -> Self {
        Self {
            decision: ScreenTuningDecision {
                frame_seq,
                stage: stage.as_str().to_string(),
                action: ScreenTuningAction::Hold.as_str().to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
        .with_stamp(now_us)
    }

    pub(super) fn with_stamp(mut self, now_us: u64) -> Self {
        self.decision.at_us = now_us;
        self
    }
}

/// DTO report for REST API and CLI formatting (v2.14 / ADR-100).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenTuningReport {
    pub ok: bool,
    pub mode: String,
    pub actions_applied: u64,
    pub recoveries_applied: u64,
    pub last_stage: String,
    pub last_action: String,
    pub bottleneck_streak: u32,
    pub balanced_streak: u32,
    pub last_decision: Option<ScreenTuningDecision>,
}

impl Default for ScreenTuningReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenTunerMode::default().as_str().to_string(),
            actions_applied: 0,
            recoveries_applied: 0,
            last_stage: ScreenBottleneckStage::Balanced.as_str().to_string(),
            last_action: ScreenTuningAction::Hold.as_str().to_string(),
            bottleneck_streak: 0,
            balanced_streak: 0,
            last_decision: None,
        }
    }
}

/// Bottleneck-driven autonomous tuner: turns the ADR-099 dominant bottleneck verdict into
/// targeted parameter actions with hysteresis, cooldown and anti-oscillation recovery (v2.14 / ADR-100).
#[derive(Debug, Clone)]
pub struct ScreenTuner {
    pub(super) mode: ScreenTunerMode,
    pub(super) bottleneck_streak: u32,
    pub(super) last_bottleneck: ScreenBottleneckStage,
    pub(super) balanced_streak: u32,
    pub(super) last_action_us: u64,
    pub(super) actions_applied: u64,
    pub(super) recoveries_applied: u64,
    pub(super) degraded: VecDeque<ScreenTuningAction>,
    pub(super) last_decision: Option<ScreenTuningDecision>,
    pub(super) last_action: ScreenTuningAction,
    pub(super) last_stage: ScreenBottleneckStage,
}

impl Default for ScreenTuner {
    fn default() -> Self {
        Self::new(ScreenTunerMode::default())
    }
}

impl ScreenTuner {
    pub fn new(mode: ScreenTunerMode) -> Self {
        Self {
            mode,
            bottleneck_streak: 0,
            last_bottleneck: ScreenBottleneckStage::Balanced,
            balanced_streak: 0,
            last_action_us: 0,
            actions_applied: 0,
            recoveries_applied: 0,
            degraded: VecDeque::new(),
            last_decision: None,
            last_action: ScreenTuningAction::Hold,
            last_stage: ScreenBottleneckStage::Balanced,
        }
    }

    pub fn mode(&self) -> ScreenTunerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTunerMode) {
        self.mode = mode;
        self.bottleneck_streak = 0;
        self.balanced_streak = 0;
    }

    pub fn actions_applied(&self) -> u64 {
        self.actions_applied
    }

    pub fn recoveries_applied(&self) -> u64 {
        self.recoveries_applied
    }

    pub fn bottleneck_streak(&self) -> u32 {
        self.bottleneck_streak
    }

    pub fn balanced_streak(&self) -> u32 {
        self.balanced_streak
    }

    pub fn last_decision(&self) -> Option<&ScreenTuningDecision> {
        self.last_decision.as_ref()
    }

    /// Tag of the bottleneck stage of the most recent applied action.
    pub fn last_stage_tag(&self) -> u8 {
        self.last_stage.tag()
    }

    /// Tag of the most recent applied action.
    pub fn last_action_tag(&self) -> u8 {
        self.last_action.tag()
    }

    pub fn report(&self) -> ScreenTuningReport {
        ScreenTuningReport {
            ok: true,
            mode: self.mode.as_str().to_string(),
            actions_applied: self.actions_applied,
            recoveries_applied: self.recoveries_applied,
            last_stage: self.last_stage.as_str().to_string(),
            last_action: self.last_action.as_str().to_string(),
            bottleneck_streak: self.bottleneck_streak,
            balanced_streak: self.balanced_streak,
            last_decision: self.last_decision.clone(),
        }
    }

    /// Clear counters, hysteresis windows and the recovery stack; keep the mode.
    pub fn reset(&mut self) {
        self.bottleneck_streak = 0;
        self.balanced_streak = 0;
        self.last_action_us = 0;
        self.actions_applied = 0;
        self.recoveries_applied = 0;
        self.degraded.clear();
        self.last_decision = None;
        self.last_action = ScreenTuningAction::Hold;
        self.last_stage = ScreenBottleneckStage::Balanced;
    }

    /// Evaluate one profiled frame: update hysteresis windows and, when the
    /// trigger thresholds and cooldown allow, emit a targeted action.
    pub fn evaluate(
        &mut self,
        now_us: u64,
        frame_seq: u64,
        bottleneck: ScreenBottleneckStage,
        ctx: &ScreenTuningContext,
    ) -> ScreenTuningOutcome {
        if self.mode == ScreenTunerMode::Disabled {
            return ScreenTuningOutcome::hold(frame_seq, bottleneck, now_us);
        }

        if bottleneck == ScreenBottleneckStage::Balanced {
            self.balanced_streak = self.balanced_streak.saturating_add(1);
            self.bottleneck_streak = 0;
        } else if bottleneck == self.last_bottleneck {
            self.bottleneck_streak = self.bottleneck_streak.saturating_add(1);
            self.balanced_streak = 0;
        } else {
            self.bottleneck_streak = 1;
            self.balanced_streak = 0;
        }
        self.last_bottleneck = bottleneck;

        // `last_action_us == 0` means no action has been applied yet, so the
        // cooldown window only applies after the first real action.
        let in_cooldown = self.last_action_us != 0
            && now_us.saturating_sub(self.last_action_us) < self.mode.cooldown_us();

        // Recovery path: sustained balance lets one degraded knob step back up.
        if bottleneck == ScreenBottleneckStage::Balanced
            && self.balanced_streak >= self.mode.recovery_frames()
            && !self.degraded.is_empty()
            && !in_cooldown
        {
            if let Some(action) = self.degraded.pop_back() {
                let outcome = self.apply_recovery(now_us, frame_seq, action, ctx);
                self.last_decision = Some(outcome.decision.clone());
                return outcome;
            }
        }

        // Degradation path: sustained same-stage bottleneck triggers a targeted relief action.
        if bottleneck != ScreenBottleneckStage::Balanced
            && self.bottleneck_streak >= self.mode.trigger_frames()
            && !in_cooldown
        {
            let outcome = self.apply_relief(now_us, frame_seq, bottleneck, ctx);
            self.last_decision = Some(outcome.decision.clone());
            return outcome;
        }

        ScreenTuningOutcome::hold(frame_seq, bottleneck, now_us)
    }

    pub(super) fn apply_relief(
        &mut self,
        now_us: u64,
        frame_seq: u64,
        stage: ScreenBottleneckStage,
        ctx: &ScreenTuningContext,
    ) -> ScreenTuningOutcome {
        let mut outcome = ScreenTuningOutcome::hold(frame_seq, stage, now_us);
        let action = match stage {
            ScreenBottleneckStage::Capture => {
                if ctx.fps > SCREEN_FPS_MIN {
                    outcome.fps = Some(screen_matrix_next_lower_fps(ctx.fps));
                    ScreenTuningAction::ReduceFps
                } else {
                    ScreenTuningAction::Hold
                }
            }
            ScreenBottleneckStage::Encode => {
                if ctx.qp_max <= SCREEN_QP_ABSOLUTE_MAX.saturating_sub(SCREEN_TUNER_QP_RELIEF_STEP)
                {
                    outcome.qp_range = Some((
                        ctx.qp_min,
                        (ctx.qp_max + SCREEN_TUNER_QP_RELIEF_STEP).min(SCREEN_QP_ABSOLUTE_MAX),
                    ));
                    ScreenTuningAction::RaiseQp
                } else if ctx.scale_pct > SCREEN_SCALE_MIN_PCT {
                    outcome.scale_pct = Some(screen_matrix_next_lower_scale(ctx.scale_pct));
                    ScreenTuningAction::ReduceScale
                } else {
                    ScreenTuningAction::Hold
                }
            }
            ScreenBottleneckStage::Pacer | ScreenBottleneckStage::Network => {
                if ctx.bitrate_bps > ctx.bitrate_min_bps {
                    let dropped = drop_bitrate(ctx.bitrate_bps, ctx.bitrate_min_bps);
                    outcome.bitrate_bps = Some(dropped);
                    ScreenTuningAction::ReduceBitrate
                } else {
                    ScreenTuningAction::Hold
                }
            }
            ScreenBottleneckStage::Jitter => {
                outcome.playout_delay_delta_us = Some(SCREEN_TUNER_PLAYOUT_RELIEF_DELTA_US);
                ScreenTuningAction::RaisePlayoutDelay
            }
            ScreenBottleneckStage::Decode => {
                if ctx.scale_pct > SCREEN_SCALE_MIN_PCT {
                    outcome.scale_pct = Some(screen_matrix_next_lower_scale(ctx.scale_pct));
                    ScreenTuningAction::ReduceScale
                } else if ctx.fps > SCREEN_FPS_MIN {
                    outcome.fps = Some(screen_matrix_next_lower_fps(ctx.fps));
                    ScreenTuningAction::ReduceFps
                } else {
                    ScreenTuningAction::Hold
                }
            }
            ScreenBottleneckStage::Render => {
                if ctx.fps > SCREEN_FPS_MIN {
                    outcome.fps = Some(screen_matrix_next_lower_fps(ctx.fps));
                    ScreenTuningAction::ReduceFps
                } else if ctx.scale_pct > SCREEN_SCALE_MIN_PCT {
                    outcome.scale_pct = Some(screen_matrix_next_lower_scale(ctx.scale_pct));
                    ScreenTuningAction::ReduceScale
                } else {
                    ScreenTuningAction::Hold
                }
            }
            ScreenBottleneckStage::Balanced => ScreenTuningAction::Hold,
        };

        if action == ScreenTuningAction::Hold {
            return outcome;
        }

        self.actions_applied = self.actions_applied.saturating_add(1);
        self.last_action_us = now_us;
        self.degraded.push_back(action);
        self.last_action = action;
        self.last_stage = stage;
        outcome.decision.action = action.as_str().to_string();
        outcome.decision.recovered = false;
        outcome.decision.new_bitrate_bps = outcome.bitrate_bps;
        outcome.decision.new_fps = outcome.fps;
        outcome.decision.new_scale_pct = outcome.scale_pct;
        outcome.decision.new_qp_max = outcome.qp_range.map(|(_, max)| max);
        outcome.decision.new_playout_delay_us = outcome
            .playout_delay_delta_us
            .map(|delta| (ctx.playout_delay_us as i64 + delta).max(0) as u32);
        outcome
    }

    pub(super) fn apply_recovery(
        &mut self,
        now_us: u64,
        frame_seq: u64,
        action: ScreenTuningAction,
        ctx: &ScreenTuningContext,
    ) -> ScreenTuningOutcome {
        let mut outcome =
            ScreenTuningOutcome::hold(frame_seq, ScreenBottleneckStage::Balanced, now_us);
        match action {
            ScreenTuningAction::ReduceBitrate => {
                let recovered = ctx
                    .bitrate_bps
                    .saturating_mul(100 + SCREEN_TUNER_BITRATE_RECOVER_PCT)
                    / 100;
                outcome.bitrate_bps = Some(recovered.min(ctx.bitrate_max_bps));
                outcome.decision.action = ScreenTuningAction::RecoverBitrate.as_str().to_string();
            }
            ScreenTuningAction::ReduceFps => {
                outcome.fps = Some(screen_matrix_next_higher_fps(ctx.fps, SCREEN_FPS_MAX));
                outcome.decision.action = ScreenTuningAction::RecoverFps.as_str().to_string();
            }
            ScreenTuningAction::ReduceScale => {
                outcome.scale_pct = Some(screen_matrix_next_higher_scale(ctx.scale_pct));
                outcome.decision.action = ScreenTuningAction::RecoverScale.as_str().to_string();
            }
            ScreenTuningAction::RaiseQp => {
                outcome.qp_range = Some((
                    ctx.qp_min,
                    ctx.qp_max
                        .saturating_sub(SCREEN_TUNER_QP_RECOVER_STEP)
                        .max(ctx.qp_min),
                ));
                outcome.decision.action = ScreenTuningAction::RecoverQp.as_str().to_string();
            }
            ScreenTuningAction::RaisePlayoutDelay => {
                outcome.playout_delay_delta_us = Some(-SCREEN_TUNER_PLAYOUT_RELIEF_DELTA_US);
                outcome.decision.action =
                    ScreenTuningAction::LowerPlayoutDelay.as_str().to_string();
            }
            _ => {
                outcome.decision.action = ScreenTuningAction::Hold.as_str().to_string();
            }
        }

        if outcome.decision.action == ScreenTuningAction::Hold.as_str() {
            return outcome;
        }

        self.recoveries_applied = self.recoveries_applied.saturating_add(1);
        self.last_action_us = now_us;
        self.last_action = outcome
            .decision
            .action
            .parse::<ScreenTuningAction>()
            .ok()
            .unwrap_or(ScreenTuningAction::Hold);
        self.last_stage = ScreenBottleneckStage::Balanced;
        outcome.decision.recovered = true;
        outcome.decision.new_bitrate_bps = outcome.bitrate_bps;
        outcome.decision.new_fps = outcome.fps;
        outcome.decision.new_scale_pct = outcome.scale_pct;
        outcome.decision.new_qp_max = outcome.qp_range.map(|(_, max)| max);
        outcome.decision.new_playout_delay_us = outcome
            .playout_delay_delta_us
            .map(|delta| (ctx.playout_delay_us as i64 + delta).max(0) as u32);
        outcome
    }
}

pub(super) fn drop_bitrate(current_bps: u32, min_bps: u32) -> u32 {
    let dropped =
        current_bps.saturating_sub(current_bps.saturating_mul(SCREEN_TUNER_BITRATE_DROP_PCT) / 100);
    dropped.max(min_bps)
}
