// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote cursor shape, mode and position controller.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;

/// Standard mouse cursor shape categories (v2.02 / ADR-088).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum CursorShape {
    #[default]
    Arrow = 0,
    IBeam = 1,
    Hand = 2,
    Crosshair = 3,
    SizeAll = 4,
    SizeNS = 5,
    SizeWE = 6,
    SizeNESW = 7,
    SizeNWSE = 8,
    Wait = 9,
    NotAllowed = 10,
    Custom = 11,
}

impl CursorShape {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => CursorShape::IBeam,
            2 => CursorShape::Hand,
            3 => CursorShape::Crosshair,
            4 => CursorShape::SizeAll,
            5 => CursorShape::SizeNS,
            6 => CursorShape::SizeWE,
            7 => CursorShape::SizeNESW,
            8 => CursorShape::SizeNWSE,
            9 => CursorShape::Wait,
            10 => CursorShape::NotAllowed,
            11 => CursorShape::Custom,
            _ => CursorShape::Arrow,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "arrow" | "default" => Some(CursorShape::Arrow),
            "ibeam" | "text" => Some(CursorShape::IBeam),
            "hand" | "pointer" => Some(CursorShape::Hand),
            "crosshair" => Some(CursorShape::Crosshair),
            "sizeall" | "move" => Some(CursorShape::SizeAll),
            "sizens" | "vresize" => Some(CursorShape::SizeNS),
            "sizewe" | "hresize" => Some(CursorShape::SizeWE),
            "sizenesw" => Some(CursorShape::SizeNESW),
            "sizenwse" => Some(CursorShape::SizeNWSE),
            "wait" | "busy" => Some(CursorShape::Wait),
            "notallowed" | "forbidden" => Some(CursorShape::NotAllowed),
            "custom" => Some(CursorShape::Custom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CursorShape::Arrow => "arrow",
            CursorShape::IBeam => "ibeam",
            CursorShape::Hand => "hand",
            CursorShape::Crosshair => "crosshair",
            CursorShape::SizeAll => "size_all",
            CursorShape::SizeNS => "size_ns",
            CursorShape::SizeWE => "size_we",
            CursorShape::SizeNESW => "size_nesw",
            CursorShape::SizeNWSE => "size_nwse",
            CursorShape::Wait => "wait",
            CursorShape::NotAllowed => "not_allowed",
            CursorShape::Custom => "custom",
        }
    }
}

/// Hardware cursor decoupling mode (v2.02 / ADR-088).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ScreenCursorMode {
    /// Decoupled high-frequency cursor stream with zero video encoding latency (v2.02 / ADR-088).
    #[default]
    Decoupled = 0,
    /// Legacy mode: cursor is captured and burned directly into video frames.
    Embedded = 1,
    /// Suppressed mode: cursor is hidden / ignored on receiver presentation.
    Hidden = 2,
}

impl ScreenCursorMode {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenCursorMode::Embedded,
            2 => ScreenCursorMode::Hidden,
            _ => ScreenCursorMode::Decoupled,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "decoupled" | "hardware" | "hw" | "async" => Some(ScreenCursorMode::Decoupled),
            "embedded" | "video" | "burn" => Some(ScreenCursorMode::Embedded),
            "hidden" | "none" | "off" => Some(ScreenCursorMode::Hidden),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenCursorMode::Decoupled => "decoupled",
            ScreenCursorMode::Embedded => "embedded",
            ScreenCursorMode::Hidden => "hidden",
        }
    }
}

/// Decoupled high-frequency mouse cursor update (v2.02 / ADR-088).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCursorUpdate {
    pub seq: u64,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub shape: CursorShape,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub width: u16,
    pub height: u16,
    pub custom_rgba: Option<Vec<u8>>,
}

impl ScreenCursorUpdate {
    /// Fixed header length before optional custom bitmap payload (32 bytes).
    pub const HEADER_LEN: usize = 32;

    pub fn encode_payload(&self) -> Result<Vec<u8>, CoreError> {
        let custom_bytes = self.custom_rgba.as_deref().unwrap_or(&[]);
        if custom_bytes.len() > CURSOR_BITMAP_MAX_BYTES {
            return Err(CoreError::ScreenTooLarge);
        }
        let mut out = Vec::with_capacity(Self::HEADER_LEN + custom_bytes.len());
        out.extend_from_slice(&CURSOR_MAGIC);
        out.push(CURSOR_PROTO);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.x.to_be_bytes());
        out.extend_from_slice(&self.y.to_be_bytes());
        out.push(if self.visible { 1 } else { 0 });
        out.push(self.shape.tag());
        out.extend_from_slice(&self.hotspot_x.to_be_bytes());
        out.extend_from_slice(&self.hotspot_y.to_be_bytes());
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.push(if self.custom_rgba.is_some() { 1 } else { 0 });
        out.extend_from_slice(custom_bytes);
        Ok(out)
    }

    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < Self::HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != CURSOR_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != CURSOR_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let seq = u64::from_be_bytes(payload[5..13].try_into().unwrap());
        let x = i32::from_be_bytes(payload[13..17].try_into().unwrap());
        let y = i32::from_be_bytes(payload[17..21].try_into().unwrap());
        let visible = payload[21] != 0;
        let shape = CursorShape::from_tag(payload[22]);
        let hotspot_x = u16::from_be_bytes(payload[23..25].try_into().unwrap());
        let hotspot_y = u16::from_be_bytes(payload[25..27].try_into().unwrap());
        let width = u16::from_be_bytes(payload[27..29].try_into().unwrap());
        let height = u16::from_be_bytes(payload[29..31].try_into().unwrap());
        let has_custom = payload[31] != 0;
        let custom_rgba = if has_custom {
            let bytes = payload[Self::HEADER_LEN..].to_vec();
            let expected_len = (width as usize) * (height as usize) * 4;
            if bytes.len() != expected_len || bytes.len() > CURSOR_BITMAP_MAX_BYTES {
                return Err(CoreError::ScreenFrame);
            }
            Some(bytes)
        } else {
            None
        };

        Ok(Self {
            seq,
            x,
            y,
            visible,
            shape,
            hotspot_x,
            hotspot_y,
            width,
            height,
            custom_rgba,
        })
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode_payload()?;
        InnerFrame::new(TYPE_SCREEN_CURSOR, payload)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_CURSOR {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }
}

/// Telemetry and counters for screen cursor stream (v2.02 / ADR-088).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenCursorStats {
    pub mode: ScreenCursorMode,
    pub updates_sent: u64,
    pub updates_received: u64,
    pub updates_dropped_dedup: u64,
    pub updates_dropped_rate: u64,
    pub current_x: i32,
    pub current_y: i32,
    pub current_visible: bool,
    pub current_shape: CursorShape,
}

/// Controller governing hardware cursor position deduplication, rate-limiting, and shape caching (v2.02 / ADR-088).
#[derive(Debug, Clone)]
pub struct ScreenCursorController {
    pub(super) mode: ScreenCursorMode,
    pub(super) next_seq: u64,
    pub(super) last_x: i32,
    pub(super) last_y: i32,
    pub(super) last_visible: bool,
    pub(super) last_shape: CursorShape,
    pub(super) last_sent_us: u64,
    pub(super) min_interval_us: u64,
    pub(super) stats: ScreenCursorStats,
}

impl Default for ScreenCursorController {
    fn default() -> Self {
        Self::new(ScreenCursorMode::default())
    }
}

impl ScreenCursorController {
    pub fn new(mode: ScreenCursorMode) -> Self {
        Self {
            mode,
            next_seq: 1,
            last_x: 0,
            last_y: 0,
            last_visible: true,
            last_shape: CursorShape::Arrow,
            last_sent_us: 0,
            min_interval_us: SCREEN_CURSOR_MIN_INTERVAL_US,
            stats: ScreenCursorStats {
                mode,
                current_visible: true,
                current_shape: CursorShape::Arrow,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenCursorMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenCursorMode) -> ScreenCursorMode {
        self.mode = mode;
        self.stats.mode = mode;
        mode
    }

    pub fn min_interval_us(&self) -> u64 {
        self.min_interval_us
    }

    pub fn set_min_interval_us(&mut self, interval_us: u64) {
        self.min_interval_us = interval_us;
    }

    pub fn stats(&self) -> ScreenCursorStats {
        self.stats
    }

    pub fn reset(&mut self) {
        self.next_seq = 1;
        self.last_sent_us = 0;
    }

    /// Evaluates candidate cursor parameters and produces an outbound ScreenCursorUpdate
    /// if decoupled mode is active and parameters pass deduplication and rate-limiting.
    #[allow(clippy::too_many_arguments)]
    pub fn create_update(
        &mut self,
        x: i32,
        y: i32,
        visible: bool,
        shape: CursorShape,
        hotspot: (u16, u16),
        dimensions: (u16, u16),
        custom_rgba: Option<Vec<u8>>,
        now_us: u64,
    ) -> Option<ScreenCursorUpdate> {
        if self.mode != ScreenCursorMode::Decoupled {
            return None;
        }

        // 1. Deduplication: identical position, visibility, and shape without new custom bitmap
        if self.stats.updates_sent > 0
            && x == self.last_x
            && y == self.last_y
            && visible == self.last_visible
            && shape == self.last_shape
            && custom_rgba.is_none()
        {
            self.stats.updates_dropped_dedup = self.stats.updates_dropped_dedup.saturating_add(1);
            return None;
        }

        // 2. High-frequency rate limiting for position moves:
        // Position changes are rate-limited to min_interval_us (e.g. 8ms / 125Hz).
        // Shape or visibility changes or custom bitmaps always bypass rate-limiting.
        let is_semantic_change =
            shape != self.last_shape || visible != self.last_visible || custom_rgba.is_some();
        if !is_semantic_change
            && now_us > 0
            && self.last_sent_us > 0
            && now_us.saturating_sub(self.last_sent_us) < self.min_interval_us
        {
            self.stats.updates_dropped_rate = self.stats.updates_dropped_rate.saturating_add(1);
            return None;
        }

        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);

        self.last_x = x;
        self.last_y = y;
        self.last_visible = visible;
        self.last_shape = shape;
        self.last_sent_us = now_us;

        self.stats.updates_sent = self.stats.updates_sent.saturating_add(1);
        self.stats.current_x = x;
        self.stats.current_y = y;
        self.stats.current_visible = visible;
        self.stats.current_shape = shape;

        Some(ScreenCursorUpdate {
            seq,
            x,
            y,
            visible,
            shape,
            hotspot_x: hotspot.0,
            hotspot_y: hotspot.1,
            width: dimensions.0,
            height: dimensions.1,
            custom_rgba,
        })
    }
}
