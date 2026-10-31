// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Dynamic security watermarking and forensic overlay.

use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;

/// Layout pattern for security watermark overlay (v2.18 / ADR-104).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatermarkPattern {
    /// Diagonal repeating grid across the whole screen.
    #[default]
    Grid,
    /// Single watermark placed at a specified alignment.
    Single,
    /// Top banner overlay.
    Banner,
}

impl WatermarkPattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Single => "single",
            Self::Banner => "banner",
        }
    }
}

impl std::str::FromStr for WatermarkPattern {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "grid" => Ok(Self::Grid),
            "single" => Ok(Self::Single),
            "banner" => Ok(Self::Banner),
            _ => Err(CoreError::InvalidConfig(format!(
                "unknown watermark pattern: {s}"
            ))),
        }
    }
}

/// Dynamic security watermark configuration (v2.18 / ADR-104).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenWatermarkConfig {
    pub enabled: bool,
    pub host_enforced: bool,
    pub text_template: String,
    pub opacity: u8,
    pub color_rgb: [u8; 3],
    pub scale: u8,
    pub pattern: WatermarkPattern,
    pub spacing_x: u16,
    pub spacing_y: u16,
}

impl Default for ScreenWatermarkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host_enforced: false,
            text_template: "CONFIDENTIAL {peer} {time}".to_string(),
            opacity: 64, // ~25% alpha
            color_rgb: [220, 220, 220],
            scale: 1,
            pattern: WatermarkPattern::Grid,
            spacing_x: 240,
            spacing_y: 160,
        }
    }
}

impl ScreenWatermarkConfig {
    pub fn report(&self, peer_id: &str, timestamp: &str, frames: u64) -> ScreenWatermarkReport {
        let rendered_text = self
            .text_template
            .replace("{peer}", peer_id)
            .replace("{host}", "host")
            .replace("{time}", timestamp);
        let opacity_pct = ((self.opacity as u32 * 100 + 127) / 255) as u8;
        let color_hex = format!(
            "#{:02X}{:02X}{:02X}",
            self.color_rgb[0], self.color_rgb[1], self.color_rgb[2]
        );
        ScreenWatermarkReport {
            ok: true,
            enabled: self.enabled,
            host_enforced: self.host_enforced,
            text_template: self.text_template.clone(),
            rendered_text,
            opacity: self.opacity,
            opacity_pct,
            color_rgb: self.color_rgb,
            color_hex,
            scale: self.scale,
            pattern: self.pattern.as_str().to_string(),
            spacing_x: self.spacing_x,
            spacing_y: self.spacing_y,
            frames_watermarked: frames,
        }
    }
}

/// Telemetry and state report for dynamic security watermark (v2.18 / ADR-104).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenWatermarkReport {
    pub ok: bool,
    pub enabled: bool,
    pub host_enforced: bool,
    pub text_template: String,
    pub rendered_text: String,
    pub opacity: u8,
    pub opacity_pct: u8,
    pub color_rgb: [u8; 3],
    pub color_hex: String,
    pub scale: u8,
    pub pattern: String,
    pub spacing_x: u16,
    pub spacing_y: u16,
    pub frames_watermarked: u64,
}

impl Default for ScreenWatermarkReport {
    fn default() -> Self {
        ScreenWatermarkConfig::default().report("peer", "0", 0)
    }
}
