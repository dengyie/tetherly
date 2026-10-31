// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Adaptive color depth and grayscale economy mode.

use serde::Deserialize;
use serde::Serialize;

/// Adaptive color depth and grayscale economy mode (v2.22 / ADR-108).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenColorMode {
    /// Standard 32-bit true color (BGRA8/RGBA8 native capture).
    #[default]
    FullColor32 = 0,
    /// 16-bit color (RGB565 quantization with high-bits replenishment).
    ReducedColor16 = 1,
    /// 8-bit grayscale mode (fast integer luminance Y = (77R + 150G + 29B) >> 8).
    Grayscale8 = 2,
    /// 1-bit monochrome black & white bi-level mode with configurable threshold.
    Monochrome1 = 3,
}

impl ScreenColorMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::ReducedColor16,
            2 => Self::Grayscale8,
            3 => Self::Monochrome1,
            _ => Self::FullColor32,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullColor32 => "full_color32",
            Self::ReducedColor16 => "reduced_color16",
            Self::Grayscale8 => "grayscale8",
            Self::Monochrome1 => "monochrome1",
        }
    }
}

impl std::fmt::Display for ScreenColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenColorMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "full" | "full_color" | "fullcolor" | "full_color32" | "32" => Ok(Self::FullColor32),
            "reduced" | "reduced_color" | "reducedcolor" | "reduced_color16" | "16" | "rgb565" => {
                Ok(Self::ReducedColor16)
            }
            "gray" | "grey" | "grayscale" | "grayscale8" | "8" => Ok(Self::Grayscale8),
            "mono" | "monochrome" | "monochrome1" | "1" | "bw" => Ok(Self::Monochrome1),
            other => Err(format!(
                "unknown color mode '{other}', expected: full, reduced, grayscale, monochrome"
            )),
        }
    }
}

/// Default binarization threshold for Monochrome1 color mode (v2.22 / ADR-108).
pub const SCREEN_COLOR_DEFAULT_THRESHOLD: u8 = 128;

/// Configuration parameters for Screen Adaptive Color Depth pipeline (v2.22 / ADR-108).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenColorConfig {
    pub mode: ScreenColorMode,
    pub threshold: u8,
}

impl Default for ScreenColorConfig {
    fn default() -> Self {
        Self {
            mode: ScreenColorMode::FullColor32,
            threshold: SCREEN_COLOR_DEFAULT_THRESHOLD,
        }
    }
}

impl ScreenColorConfig {
    pub fn new(mode: ScreenColorMode, threshold: Option<u8>) -> Self {
        Self {
            mode,
            threshold: threshold.unwrap_or(128).clamp(1, 254),
        }
    }

    pub fn is_active(&self) -> bool {
        self.mode != ScreenColorMode::FullColor32
    }
}

/// Status report for the active Screen Adaptive Color Mode pipeline (v2.22 / ADR-108).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenColorReport {
    pub ok: bool,
    pub mode: ScreenColorMode,
    pub threshold: u8,
    pub is_active: bool,
    pub update_count: u64,
}

impl Default for ScreenColorReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenColorMode::FullColor32,
            threshold: 128,
            is_active: false,
            update_count: 0,
        }
    }
}
