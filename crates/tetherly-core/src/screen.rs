// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote-screen protocol. Binary frames only — never JSON pixels.
//! OS capture and presentation live in tetherly-node; this module is OS-free.
//!
//! One direction: the controlled side grabs frames and pushes them; the
//! controlling side presents them. Consent is explicit and local — a peer that
//! reaches the channel may NOT start a stream until this machine has allowed
//! it. Frames carry no executable instruction, so a peer URL can never ride in
//! on a screen payload (spec §10 / §9.7).

use crate::error::CoreError;
use crate::frame::{InnerFrame, FLAG_MUST_UNDERSTAND, INNER_PAYLOAD_MAX};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;

/// Screen channel inner types. Not listed in the JSON control-plane table.
pub const TYPE_SCREEN_FRAME: u16 = 0x0501;
pub const TYPE_SCREEN_CONTROL: u16 = 0x0502;
/// Screen cursor channel inner type (v2.02 / ADR-088).
pub const TYPE_SCREEN_CURSOR: u16 = 0x0503;
/// Screen forward error correction (FEC) channel inner type (v2.04 / ADR-090).
pub const TYPE_SCREEN_FEC: u16 = 0x0504;

pub const SCREEN_MAGIC: [u8; 4] = *b"TMV1";
pub const SCREEN_PROTO: u8 = 1;

pub const CURSOR_MAGIC: [u8; 4] = *b"TMCU";
pub const CURSOR_PROTO: u8 = 1;

pub const FEC_MAGIC: [u8; 4] = *b"TFEC";
pub const FEC_PROTO: u8 = 1;
pub const FEC_HEADER_LEN: usize = 24;
pub const FEC_MAX_DATA_SHARDS: usize = 32;
pub const FEC_MAX_PARITY_SHARDS: usize = 16;

/// Maximum payload bytes for a custom cursor bitmap (64x64 RGBA8 = 16,384 bytes).
pub const CURSOR_BITMAP_MAX_BYTES: usize = 64 * 64 * 4;

/// Minimum interval between outbound cursor position packets in microseconds (~125 Hz).
pub const SCREEN_CURSOR_MIN_INTERVAL_US: u64 = 8_000;

/// Static delay threshold in microseconds before triggering zero-motion frame squelch in Auto mode (500 ms) (v2.03 / ADR-089).
pub const SCREEN_SQUELCH_AUTO_DELAY_US: u64 = 500_000;

/// Static delay threshold in microseconds before triggering zero-motion frame squelch in Aggressive mode (150 ms) (v2.03 / ADR-089).
pub const SCREEN_SQUELCH_AGGRESSIVE_DELAY_US: u64 = 150_000;

/// Periodic refresh heartbeat / keepalive frame interval in microseconds under squelched Auto mode (2.0 s) (v2.03 / ADR-089).
pub const SCREEN_SQUELCH_AUTO_KEEPALIVE_US: u64 = 2_000_000;

/// Periodic refresh heartbeat / keepalive frame interval in microseconds under squelched Aggressive mode (5.0 s) (v2.03 / ADR-089).
pub const SCREEN_SQUELCH_AGGRESSIVE_KEEPALIVE_US: u64 = 5_000_000;

/// Grid block dimension in pixels for tile-based screen damage / dirty rect evaluation (32x32 pixels) (v2.03 / ADR-089).
pub const SCREEN_DAMAGE_BLOCK_SIZE: usize = 32;

/// Drop-and-ignore anything older, and refuse a forward jump this large.
/// Same shape as the input window: the wire is a live stream, not a log.
pub const SCREEN_SEQ_WINDOW: u64 = 65_536;

/// Payload bytes accepted for one screen frame. The Noise inner frame caps at
/// `INNER_PAYLOAD_MAX` per fragment, so a full-resolution frame travels as
/// several inner frames (see `SCREEN_FRAG_MAGIC` and `ScreenFrame::to_frames`);
/// this is the hard ceiling the codec enforces on the concatenated pixel body.
/// 8 MiB fits one 1920x1080 BGRA frame worst case even uncompressed; any
/// realistic zstd-compressed frame sits far below it.
pub const SCREEN_PAYLOAD_MAX: usize = 8 * 1024 * 1024;

/// Maximum in-flight frame tracking entries kept by ScreenSender.
pub const SCREEN_IN_FLIGHT_MAX: usize = 64;

/// In-flight frame count threshold indicating link congestion (v1.93 / ADR-080).
/// At 30fps pacing, 8 unacked frames equals ~267ms of queued video latency.
pub const SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD: u64 = 8;

/// RTT threshold in microseconds indicating link congestion (250ms).
pub const SCREEN_CONGESTION_RTT_THRESHOLD_US: u64 = 250_000;

/// Default video bitrate in bits per second (4 Mbps) (v1.96 / ADR-082).
pub const SCREEN_BITRATE_DEFAULT_BPS: u32 = 4_000_000;

/// Minimum video bitrate floor in bits per second (200 kbps).
pub const SCREEN_BITRATE_MIN_BPS: u32 = 200_000;

/// Maximum video bitrate ceiling in bits per second (20 Mbps).
pub const SCREEN_BITRATE_MAX_BPS: u32 = 20_000_000;

/// Additive Increase step for rate controller (150 kbps).
pub const SCREEN_BITRATE_STEP_INCREASE_BPS: u32 = 150_000;

/// Minimum interval between multiplicative rate decreases in microseconds (400ms).
pub const SCREEN_BITRATE_COOLDOWN_US: u64 = 400_000;

/// Default screen streaming frame rate in frames per second (30 FPS) (v1.97 / ADR-083).
pub const SCREEN_FPS_DEFAULT: u32 = 30;

/// Minimum screen streaming frame rate floor in frames per second (5 FPS).
pub const SCREEN_FPS_MIN: u32 = 5;

/// Maximum screen streaming frame rate ceiling in frames per second (120 FPS).
pub const SCREEN_FPS_MAX: u32 = 120;

/// Default screen streaming resolution scale percentage (100% native) (v1.98 / ADR-084).
pub const SCREEN_SCALE_DEFAULT_PCT: u32 = 100;

/// Minimum screen streaming resolution scale percentage floor (25%).
pub const SCREEN_SCALE_MIN_PCT: u32 = 25;

/// Maximum screen streaming resolution scale percentage ceiling (100%).
pub const SCREEN_SCALE_MAX_PCT: u32 = 100;

/// Default quantization parameter minimum bound (v1.99 / ADR-085).
pub const SCREEN_QP_MIN_DEFAULT: u8 = 12;

/// Default quantization parameter maximum bound (v1.99 / ADR-085).
pub const SCREEN_QP_MAX_DEFAULT: u8 = 36;

/// Absolute quantization parameter minimum floor in H.264 (0 = lossless).
pub const SCREEN_QP_ABSOLUTE_MIN: u8 = 0;

/// Absolute quantization parameter maximum ceiling in H.264 (51 = maximum compression).
pub const SCREEN_QP_ABSOLUTE_MAX: u8 = 51;

/// Default quantization parameter range for text/productivity profile: (min_qp=12, max_qp=35).
pub const SCREEN_QP_TEXT_DEFAULT: (u8, u8) = (12, 35);

/// Default quantization parameter range for motion/video profile: (min_qp=16, max_qp=46).
pub const SCREEN_QP_MOTION_DEFAULT: (u8, u8) = (16, 46);

/// Content optimization profile for screen streaming (v1.99 / ADR-085).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ScreenContentProfile {
    /// Text / Code / Terminal / Desktop productivity optimized.
    /// Preserves sharp character glyphs, high contrast edges, and prevents text blur.
    /// Maps to OpenH264 ScreenContentRealTime.
    #[default]
    Text = 0,
    /// Fluid motion / Video / Animation / 3D graphics optimized.
    /// Tolerates higher spatial quantization under high entropy to maintain smooth framerate.
    /// Maps to OpenH264 CameraVideoRealTime.
    Motion = 1,
    /// Automatic detection / adaptive profile selection based on network and screen activity.
    Auto = 2,
}

impl ScreenContentProfile {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Motion => "motion",
            Self::Auto => "auto",
        }
    }

    pub const fn tag(&self) -> u8 {
        *self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Motion,
            2 => Self::Auto,
            _ => Self::Text,
        }
    }

    pub fn default_qp_range(&self) -> (u8, u8) {
        match self {
            Self::Text | Self::Auto => SCREEN_QP_TEXT_DEFAULT,
            Self::Motion => SCREEN_QP_MOTION_DEFAULT,
        }
    }
}

impl std::fmt::Display for ScreenContentProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenContentProfile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "text" | "desktop" | "code" => Ok(Self::Text),
            "motion" | "video" | "media" => Ok(Self::Motion),
            "auto" | "default" => Ok(Self::Auto),
            other => Err(format!(
                "unknown screen content profile: {other} (expected: text, motion, auto)"
            )),
        }
    }
}

/// Fragment transport for screen frames too large for one inner frame.
/// A fragment stream is a `TMF1`-enveloped run of inner frames; the receiver
/// reassembles before `decode_payload`. A `TMV1`-prefixed payload is a whole
/// frame and takes the fast path, so small frames stay byte-identical to the
/// single-frame wire format.
pub const SCREEN_FRAG_MAGIC: [u8; 4] = *b"TMF1";

/// Fragment envelope: magic(4) | total_len(4) | frag_index(4) | frag_count(4)
/// | seq(8) = 24 bytes, followed by the chunk.
pub const SCREEN_FRAG_HEADER: usize = 24;

/// Per-fragment chunk budget so every fragment inner frame stays inside the
/// transport budget: `SCREEN_FRAG_HEADER + chunk <= INNER_PAYLOAD_MAX`, i.e. the
/// fragment payload plus the 12-byte inner header plus the 16-byte AEAD tag all
/// fit one Noise message. `INNER_PAYLOAD_MAX` already accounts for those, so
/// `to_frames` never hands the transport a plaintext it cannot encrypt.
pub const SCREEN_FRAG_CHUNK_MAX: usize = INNER_PAYLOAD_MAX - SCREEN_FRAG_HEADER;

/// Most recent presented frames the memory sink retains, so a long-lived
/// receiver cannot grow its trace without bound.
pub const SCREEN_TRACE_MAX: usize = 64;

/// Compression tag for the pixel body. `None` is raw; `Zstd` is the default
/// lossless codec. A decoder that does not know the tag refuses the frame
/// rather than guessing.
pub const SCREEN_COMPRESS_NONE: u8 = 0;
pub const SCREEN_COMPRESS_ZSTD: u8 = 1;
/// H.264 (OpenH264, annex-B bitstream, IDR-first). See ADR-079.
pub const SCREEN_COMPRESS_H264: u8 = 3;

pub fn is_screen_type(ty: u16) -> bool {
    matches!(
        ty,
        TYPE_SCREEN_FRAME | TYPE_SCREEN_CONTROL | TYPE_SCREEN_CURSOR | TYPE_SCREEN_FEC
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Windows / DXGI order. Alpha is ignored on present.
    Bgra8,
    Rgba8,
}

impl PixelFormat {
    fn tag(self) -> u8 {
        match self {
            PixelFormat::Bgra8 => 0,
            PixelFormat::Rgba8 => 1,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, CoreError> {
        match tag {
            0 => Ok(PixelFormat::Bgra8),
            1 => Ok(PixelFormat::Rgba8),
            _ => Err(CoreError::ScreenFrame),
        }
    }

    /// Bytes per pixel. Both v1 formats are 4 bytes.
    pub fn bpp(self) -> usize {
        4
    }

    /// Swap channels in place. Used when a source and a sink disagree.
    pub fn converted(self, to: PixelFormat, pixels: &mut [u8]) {
        if self == to {
            return;
        }
        // Bgra8 <-> Rgba8 are the only pair, so one swap covers both ways.
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
    }
}

/// A screen region in device pixels. Width and height are never zero once
/// validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

impl ScreenRect {
    pub fn new(x: i32, y: i32, w: u16, h: u16) -> Result<Self, CoreError> {
        if w == 0 || h == 0 {
            return Err(CoreError::ScreenFrame);
        }
        Ok(Self { x, y, w, h })
    }

    pub fn pixel_bytes(&self, format: PixelFormat) -> usize {
        // Widened to u64 then saturated to usize so a peer-declared 65535x65535
        // rect cannot overflow on a 32-bit target (Android armeabi-v7a) before
        // the caller's cap check runs.
        let n = (self.w as u64) * (self.h as u64) * (format.bpp() as u64);
        usize::try_from(n).unwrap_or(usize::MAX)
    }
}

/// One screen frame as it travels. `bytes` is the pixel body only; the header
/// is rebuilt on the wire and never trusted from the peer without a length
/// check against `rect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFrame {
    pub seq: u64,
    pub rect: ScreenRect,
    pub format: PixelFormat,
    /// Wire body codec tag (`SCREEN_COMPRESS_*`); `0` means the body is raw
    /// pixels. Kept as a tag (not a bool) since ADR-079 added H.264 alongside
    /// zstd — the receiver dispatches its decoder on this value.
    pub codec_tag: u8,
    /// Row stride in bytes as produced by the source. `0` means tightly packed.
    pub stride: u32,
    pub bytes: Vec<u8>,
}

impl ScreenFrame {
    /// Whether the body still needs a decode pass before presenting.
    pub fn compressed(&self) -> bool {
        self.codec_tag != SCREEN_COMPRESS_NONE
    }
}

impl ScreenFrame {
    /// A tightly packed raw frame. Validates size against the rect.
    pub fn raw(
        seq: u64,
        rect: ScreenRect,
        format: PixelFormat,
        bytes: Vec<u8>,
    ) -> Result<Self, CoreError> {
        let expect = rect.pixel_bytes(format);
        if bytes.len() != expect {
            return Err(CoreError::ScreenFrame);
        }
        Ok(Self {
            seq,
            rect,
            format,
            codec_tag: SCREEN_COMPRESS_NONE,
            stride: 0,
            bytes,
        })
    }

    /// Row stride in bytes, defaulting a packed frame to `w * bpp`.
    pub fn effective_stride(&self) -> u32 {
        if self.stride == 0 {
            self.rect.w as u32 * self.format.bpp() as u32
        } else {
            self.stride
        }
    }

    /// Uncompressed size the pixels decode to.
    pub fn raw_len(&self) -> usize {
        self.rect.pixel_bytes(self.format)
    }

    fn encode_header(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        out.extend_from_slice(&SCREEN_MAGIC);
        out.push(SCREEN_PROTO);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.rect.x.to_be_bytes());
        out.extend_from_slice(&self.rect.y.to_be_bytes());
        out.extend_from_slice(&self.rect.w.to_be_bytes());
        out.extend_from_slice(&self.rect.h.to_be_bytes());
        out.push(self.format.tag());
        out.push(self.codec_tag);
        out.extend_from_slice(&self.stride.to_be_bytes());
        out
    }

    pub fn encode_payload(&self) -> Result<Vec<u8>, CoreError> {
        if self.bytes.len() > SCREEN_PAYLOAD_MAX {
            return Err(CoreError::ScreenTooLarge);
        }
        let header = self.encode_header();
        let mut out = Vec::with_capacity(header.len() + self.bytes.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&self.bytes);
        Ok(out)
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode_payload()?;
        if payload.len() > INNER_PAYLOAD_MAX {
            return Err(CoreError::ScreenTooLarge);
        }
        Ok(InnerFrame {
            ty: TYPE_SCREEN_FRAME,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload,
        })
    }

    /// Envelope a whole (small) frame into one inner frame, or split a large
    /// frame into `TMF1` fragment inner frames each under the 64 KiB inner
    /// budget. Each fragment is independently `TYPE_SCREEN_FRAME`; a receiver
    /// reassembles with `ScreenAssembler` before `decode_payload`. This is the
    /// only way a full-screen BGRA frame crosses the 45720 wire.
    pub fn to_frames(&self) -> Result<Vec<InnerFrame>, CoreError> {
        let payload = self.encode_payload()?;
        if payload.len() <= INNER_PAYLOAD_MAX {
            return Ok(vec![InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload,
            }]);
        }
        let total = payload.len();
        let count = total.div_ceil(SCREEN_FRAG_CHUNK_MAX);
        if count > u32::MAX as usize {
            return Err(CoreError::ScreenTooLarge);
        }
        let mut frames = Vec::with_capacity(count);
        for idx in 0..count {
            let start = idx * SCREEN_FRAG_CHUNK_MAX;
            let end = (start + SCREEN_FRAG_CHUNK_MAX).min(total);
            let mut frag = vec![0u8; SCREEN_FRAG_HEADER + (end - start)];
            frag[..4].copy_from_slice(&SCREEN_FRAG_MAGIC);
            frag[4..8].copy_from_slice(&(total as u32).to_be_bytes());
            frag[8..12].copy_from_slice(&(idx as u32).to_be_bytes());
            frag[12..16].copy_from_slice(&(count as u32).to_be_bytes());
            frag[16..24].copy_from_slice(&self.seq.to_be_bytes());
            frag[SCREEN_FRAG_HEADER..].copy_from_slice(&payload[start..end]);
            frames.push(InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload: frag,
            });
        }
        Ok(frames)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_FRAME {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }

    /// Header is 24 bytes: magic(4) proto(1) seq(8) x(4) y(4) w(2) h(2)
    /// fmt(1) compress(1) stride(4).
    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        // magic(4) + proto(1) + seq(8) + x(4) + y(4) + w(2) + h(2) + fmt(1)
        // + compress(1) + stride(4) = 31. A shorter body is a truncated frame,
        // never a slice panic.
        const HEADER: usize = 31;
        if payload.len() < HEADER {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != SCREEN_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != SCREEN_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let seq = u64::from_be_bytes(payload[5..13].try_into().unwrap());
        let x = i32::from_be_bytes(payload[13..17].try_into().unwrap());
        let y = i32::from_be_bytes(payload[17..21].try_into().unwrap());
        let w = u16::from_be_bytes(payload[21..23].try_into().unwrap());
        let h = u16::from_be_bytes(payload[23..25].try_into().unwrap());
        let format = PixelFormat::from_tag(payload[25])?;
        let codec_tag = match payload[26] {
            tag @ (SCREEN_COMPRESS_NONE | SCREEN_COMPRESS_ZSTD | SCREEN_COMPRESS_H264) => tag,
            _ => return Err(CoreError::ScreenFrame),
        };
        let compressed = codec_tag != SCREEN_COMPRESS_NONE;
        let stride = u32::from_be_bytes(payload[27..31].try_into().unwrap());
        let rect = ScreenRect::new(x, y, w, h)?;
        // The decoded body is `w*h*bpp`; `w,h` are peer-controlled u16, so this
        // can reach ~17 GB. Cap it here (the whole-frame payload cap does not
        // cover the *decoded* size) so a small compressed frame cannot drive an
        // unbounded allocation in the decoder that follows.
        if rect.pixel_bytes(format) > SCREEN_PAYLOAD_MAX {
            return Err(CoreError::ScreenTooLarge);
        }
        let bytes = payload[HEADER..].to_vec();
        if bytes.is_empty() {
            return Err(CoreError::ScreenFrame);
        }
        if bytes.len() > SCREEN_PAYLOAD_MAX {
            return Err(CoreError::ScreenTooLarge);
        }
        // A packed raw frame must be exact. A compressed frame may shrink, but
        // the caller must never claim a raw body larger than the rect allows.
        if !compressed && bytes.len() != rect.pixel_bytes(format) {
            return Err(CoreError::ScreenFrame);
        }
        if compressed && bytes.len() > rect.pixel_bytes(format) {
            return Err(CoreError::ScreenFrame);
        }
        Ok(Self {
            seq,
            rect,
            format,
            codec_tag,
            stride,
            bytes,
        })
    }
}

/// The only legal instructions a controller may send. There is deliberately no
/// "run" or "open" here: the screen channel carries pixels and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMsg {
    /// Controller asks to begin receiving. Refused unless allowed locally.
    Start,
    Stop,
    /// Controller confirms it presented `seq`.
    Ack {
        seq: u64,
    },
    /// Viewer asks sender to emit an independent keyframe (IDR for H.264, v1.95 / ADR-081).
    KeyframeRequest,
}

/// Control payload layout: magic(4) | proto(1) | tag(1) | seq(8).
const CONTROL_LEN: usize = 14;
const CONTROL_TAG_AT: usize = 5;
const CONTROL_SEQ_AT: usize = 6;

impl ControlMsg {
    fn tag(self) -> u8 {
        match self {
            ControlMsg::Start => 0,
            ControlMsg::Stop => 1,
            ControlMsg::Ack { .. } => 2,
            ControlMsg::KeyframeRequest => 3,
        }
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CONTROL_LEN);
        out.extend_from_slice(&SCREEN_MAGIC);
        out.push(SCREEN_PROTO);
        out.push(self.tag());
        match self {
            ControlMsg::Ack { seq } => out.extend_from_slice(&seq.to_be_bytes()),
            _ => out.extend_from_slice(&0u64.to_be_bytes()),
        }
        out
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        Ok(InnerFrame {
            ty: TYPE_SCREEN_CONTROL,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: self.encode_payload(),
        })
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_CONTROL {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }

    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() != CONTROL_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != SCREEN_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != SCREEN_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let seq = u64::from_be_bytes(payload[CONTROL_SEQ_AT..CONTROL_LEN].try_into().unwrap());
        match payload[CONTROL_TAG_AT] {
            0 => Ok(ControlMsg::Start),
            1 => Ok(ControlMsg::Stop),
            2 => Ok(ControlMsg::Ack { seq }),
            3 => Ok(ControlMsg::KeyframeRequest),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

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

/// Where a screen session currently sits. `Allowed` is only ever reached by a
/// local, explicit action (`allow`), never by anything a peer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenState {
    /// No peer on the channel, or the channel is closed.
    Idle,
    /// A trusted peer opened the channel but this machine has not consented.
    Requested,
    /// Consent given, waiting for the controller's `Start`.
    Allowed,
    /// Actively sending frames.
    Streaming,
}

impl ScreenState {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenState::Idle => "idle",
            ScreenState::Requested => "requested",
            ScreenState::Allowed => "allowed",
            ScreenState::Streaming => "streaming",
        }
    }

    pub fn is_streaming(self) -> bool {
        matches!(self, ScreenState::Streaming)
    }
}

/// Running counters for the local UI. Cheap to read; never carries pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScreenStats {
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
    pub bytes: u64,
    /// Control messages refused because consent had not been given.
    pub refused: u64,
    /// Number of frames acknowledged by the receiver (v1.93 / ADR-080).
    pub acked: u64,
    /// Highest sequence number acknowledged so far.
    pub last_acked_seq: u64,
    /// Current number of frames in flight (sent but not yet acked).
    pub in_flight: u64,
    /// Latest measured round-trip time in microseconds.
    pub rtt_latest_us: Option<u64>,
    /// 95th-percentile round-trip time in microseconds over recent window.
    pub rtt_p95_us: Option<u64>,
    /// Number of keyframe requests received/serviced (v1.95 / ADR-081).
    pub keyframes_requested: u64,
    /// Number of sequence gaps detected by receiver (v1.95 / ADR-081).
    pub gaps: u64,
    /// Target video bitrate in bits per second (v1.96 / ADR-082).
    pub bitrate_bps: u32,
    /// Target screen streaming frame rate in frames per second (v1.97 / ADR-083).
    pub fps_target: u32,
    /// Target screen resolution scale percentage (25..=100) (v1.98 / ADR-084).
    pub scale_pct: u32,
    /// Active screen content profile tag (0=text, 1=motion, 2=auto) (v1.99 / ADR-085).
    pub profile_tag: u8,
    /// Minimum quantization parameter boundary (0..=51) (v1.99 / ADR-085).
    pub qp_min: u8,
    /// Maximum quantization parameter boundary (0..=51) (v1.99 / ADR-085).
    pub qp_max: u8,
    /// 4D Autonomous Adaptation Coordinator mode (0=auto, 1=conservative, 2=manual) (v2.00 / ADR-086).
    pub matrix_mode: u8,
    /// Real-time assessed network health state (0=healthy, 1=warning, 2=congested, 3=critical) (v2.00 / ADR-086).
    pub network_health: u8,
    /// Receiver playout dejitter mode (0=adaptive, 1=zerolatency, 2=smooth) (v2.01 / ADR-087).
    pub playout_mode: u8,
    /// Target playout buffer delay in microseconds (v2.01 / ADR-087).
    pub playout_delay_us: u32,
    /// Measured interarrival jitter in microseconds (v2.01 / ADR-087).
    pub playout_jitter_us: u32,
    /// Hardware cursor decoupling mode (0=decoupled, 1=embedded, 2=hidden) (v2.02 / ADR-088).
    pub cursor_mode: u8,
    /// Last observed cursor X position in screen pixels (v2.02 / ADR-088).
    pub cursor_x: i32,
    /// Last observed cursor Y position in screen pixels (v2.02 / ADR-088).
    pub cursor_y: i32,
    /// Last observed cursor visibility (v2.02 / ADR-088).
    pub cursor_visible: bool,
    /// Last observed cursor shape tag (v2.02 / ADR-088).
    pub cursor_shape: u8,
    /// Total cursor updates sent by host (v2.02 / ADR-088).
    pub cursor_updates_sent: u64,
    /// Total cursor updates received by client (v2.02 / ADR-088).
    pub cursor_updates_received: u64,
    /// Squelch controller mode (0=auto, 1=aggressive, 2=disabled) (v2.03 / ADR-089).
    pub squelch_mode: u8,
    /// Indicates whether zero-motion squelch is actively suppressing video frames (v2.03 / ADR-089).
    pub is_squelched: bool,
    /// Cumulative count of video frames squelched/suppressed (v2.03 / ADR-089).
    pub squelched_frames: u64,
    /// Forward error correction (FEC) mode tag (0=auto, 1=conservative, 2=aggressive, 3=disabled) (v2.04 / ADR-090).
    pub fec_mode: u8,
    /// Active FEC redundancy percentage (0..=35%) (v2.04 / ADR-090).
    pub fec_redundancy_pct: u8,
    /// Total FEC lost shards successfully reconstructed by receiver (v2.04 / ADR-090).
    pub fec_recovered_shards: u64,
    /// Total FEC blocks that experienced unrecoverable loss exceeding parity budget (v2.04 / ADR-090).
    pub fec_unrecoverable_blocks: u64,
    /// Microsecond packet pacer mode tag (0=auto, 1=aggressive, 2=disabled) (v2.05 / ADR-091).
    pub pacer_mode: u8,
    /// Total packets passed through the microsecond pacer (v2.05 / ADR-091).
    pub pacer_packets_paced: u64,
    /// Total micro-bursts smoothed across the frame interval (v2.05 / ADR-091).
    pub pacer_bursts_smoothed: u64,
    /// Average packet pacing delay in microseconds (v2.05 / ADR-091).
    pub pacer_avg_delay_us: u64,
    /// Delay-based trendline filter mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.06 / ADR-092).
    pub trendline_mode: u8,
    /// Delay-based bandwidth usage state tag (0=normal, 1=overusing, 2=underusing) (v2.06 / ADR-092).
    pub trendline_state: u8,
    /// Estimated queue delay gradient slope in microseconds per frame (v2.06 / ADR-092).
    pub trendline_slope_us: f64,
    /// Total bufferbloat overuse events detected (v2.06 / ADR-092).
    pub trendline_overuse_count: u64,
}

impl ScreenStats {
    /// Delay-based trendline filter mode.
    pub fn trendline_mode(&self) -> ScreenTrendlineMode {
        ScreenTrendlineMode::from_tag(self.trendline_mode)
    }

    /// Delay-based bandwidth usage state.
    pub fn trendline_state(&self) -> ScreenBandwidthUsageState {
        ScreenBandwidthUsageState::from_tag(self.trendline_state)
    }

    /// Microsecond packet pacer mode.
    pub fn pacer_mode(&self) -> ScreenPacerMode {
        ScreenPacerMode::from_tag(self.pacer_mode)
    }

    /// Forward error correction (FEC) mode.
    pub fn fec_mode(&self) -> ScreenFecMode {
        ScreenFecMode::from_tag(self.fec_mode)
    }

    /// Squelch controller mode.
    pub fn squelch_mode(&self) -> ScreenSquelchMode {
        ScreenSquelchMode::from_tag(self.squelch_mode)
    }

    /// Active screen content profile resolved from profile tag.
    pub fn content_profile(&self) -> ScreenContentProfile {
        ScreenContentProfile::from_tag(self.profile_tag)
    }

    /// Autonomous 4D matrix coordinator mode.
    pub fn matrix_mode(&self) -> ScreenMatrixMode {
        ScreenMatrixMode::from_tag(self.matrix_mode)
    }

    /// Real-time assessed network health state.
    pub fn network_health(&self) -> NetworkHealth {
        NetworkHealth::from_tag(self.network_health)
    }

    /// Receiver playout dejitter mode.
    pub fn playout_mode(&self) -> ScreenPlayoutMode {
        ScreenPlayoutMode::from_tag(self.playout_mode)
    }

    /// Hardware cursor decoupling mode.
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        ScreenCursorMode::from_tag(self.cursor_mode)
    }

    /// Cursor shape.
    pub fn cursor_shape(&self) -> CursorShape {
        CursorShape::from_tag(self.cursor_shape)
    }
}

/// Rolling window tracker for screen frame round-trip times (v1.93 / ADR-080).
#[derive(Debug, Clone)]
pub struct ScreenRttTracker {
    samples: VecDeque<u64>,
    capacity: usize,
}

impl ScreenRttTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity.min(256)),
            capacity: capacity.max(8),
        }
    }

    pub fn record(&mut self, sample_us: u64) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample_us);
    }

    pub fn latest(&self) -> Option<u64> {
        self.samples.back().copied()
    }

    pub fn p95_us(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        Some(sorted[idx.min(sorted.len() - 1)])
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Dynamic video rate controller using AIMD (Additive Increase, Multiplicative Decrease)
/// adaptation based on live RTT, frame loss, and in-flight queue telemetry (v1.96 / ADR-082).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenRateController {
    current_bps: u32,
    min_bps: u32,
    max_bps: u32,
    last_decrease_us: u64,
    consecutive_good_frames: u32,
}

impl Default for ScreenRateController {
    fn default() -> Self {
        Self::new(SCREEN_BITRATE_DEFAULT_BPS)
    }
}

impl ScreenRateController {
    pub fn new(initial_bps: u32) -> Self {
        let clamped = initial_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);
        Self {
            current_bps: clamped,
            min_bps: SCREEN_BITRATE_MIN_BPS,
            max_bps: SCREEN_BITRATE_MAX_BPS,
            last_decrease_us: 0,
            consecutive_good_frames: 0,
        }
    }

    pub fn with_bounds(initial_bps: u32, min_bps: u32, max_bps: u32) -> Self {
        let min = min_bps.max(SCREEN_BITRATE_MIN_BPS);
        let max = max_bps.clamp(min, SCREEN_BITRATE_MAX_BPS);
        let clamped = initial_bps.clamp(min, max);
        Self {
            current_bps: clamped,
            min_bps: min,
            max_bps: max,
            last_decrease_us: 0,
            consecutive_good_frames: 0,
        }
    }

    pub fn current_bps(&self) -> u32 {
        self.current_bps
    }

    pub fn set_manual_bps(&mut self, bps: u32) {
        self.current_bps = bps.clamp(self.min_bps, self.max_bps);
        self.consecutive_good_frames = 0;
    }

    /// Trigger multiplicative decrease on congestion signals (RTT spike, packet drop, or gap).
    /// Returns true if bitrate was decreased.
    pub fn on_congestion(&mut self, now_us: u64) -> bool {
        self.consecutive_good_frames = 0;
        if self.last_decrease_us != 0
            && now_us.saturating_sub(self.last_decrease_us) < SCREEN_BITRATE_COOLDOWN_US
        {
            return false;
        }
        self.last_decrease_us = now_us;
        let old = self.current_bps;
        // Multiplicative decrease (MD): scale down to 80%
        let decreased = ((self.current_bps as u64 * 80) / 100) as u32;
        self.current_bps = decreased.max(self.min_bps);
        self.current_bps != old
    }

    /// Process a frame acknowledgment with measured RTT and in-flight count.
    /// Returns true if bitrate changed.
    pub fn on_ack(&mut self, rtt_us: u64, in_flight: u64, now_us: u64) -> bool {
        if in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || rtt_us >= SCREEN_CONGESTION_RTT_THRESHOLD_US
        {
            return self.on_congestion(now_us);
        }

        // Low latency and clear network: accumulate consecutive good frames
        if in_flight == 0 && rtt_us < 120_000 {
            self.consecutive_good_frames = self.consecutive_good_frames.saturating_add(1);
            if self.consecutive_good_frames >= 5 {
                self.consecutive_good_frames = 0;
                let old = self.current_bps;
                self.current_bps = self
                    .current_bps
                    .saturating_add(SCREEN_BITRATE_STEP_INCREASE_BPS)
                    .min(self.max_bps);
                return self.current_bps != old;
            }
        } else {
            self.consecutive_good_frames = 0;
        }
        false
    }
}

/// Dynamic screen frame pacing controller managing target frame rates and inter-frame pacing intervals (v1.97 / ADR-083).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFpsPacer {
    fps_target: u32,
    min_fps: u32,
    max_fps: u32,
    interval_us: u64,
    last_frame_us: u64,
}

impl Default for ScreenFpsPacer {
    fn default() -> Self {
        Self::new(SCREEN_FPS_DEFAULT)
    }
}

impl ScreenFpsPacer {
    pub fn new(initial_fps: u32) -> Self {
        let clamped = initial_fps.clamp(SCREEN_FPS_MIN, SCREEN_FPS_MAX);
        let interval = 1_000_000u64 / (clamped as u64);
        Self {
            fps_target: clamped,
            min_fps: SCREEN_FPS_MIN,
            max_fps: SCREEN_FPS_MAX,
            interval_us: interval,
            last_frame_us: 0,
        }
    }

    pub fn with_bounds(initial_fps: u32, min_fps: u32, max_fps: u32) -> Self {
        let min = min_fps.max(SCREEN_FPS_MIN);
        let max = max_fps.clamp(min, SCREEN_FPS_MAX);
        let clamped = initial_fps.clamp(min, max);
        let interval = 1_000_000u64 / (clamped as u64);
        Self {
            fps_target: clamped,
            min_fps: min,
            max_fps: max,
            interval_us: interval,
            last_frame_us: 0,
        }
    }

    pub fn fps_target(&self) -> u32 {
        self.fps_target
    }

    pub fn interval_us(&self) -> u64 {
        self.interval_us
    }

    pub fn min_fps(&self) -> u32 {
        self.min_fps
    }

    pub fn max_fps(&self) -> u32 {
        self.max_fps
    }

    pub fn set_fps(&mut self, fps: u32) -> u32 {
        let clamped = fps.clamp(self.min_fps, self.max_fps);
        self.fps_target = clamped;
        self.interval_us = 1_000_000u64 / (clamped as u64);
        clamped
    }

    /// Check if enough time has elapsed to send the next frame under the target pacing.
    /// If ready, updates last_frame_us and returns true. If not, returns false.
    pub fn check_and_consume(&mut self, now_us: u64) -> bool {
        if self.last_frame_us == 0 || now_us.saturating_sub(self.last_frame_us) >= self.interval_us
        {
            self.last_frame_us = now_us;
            true
        } else {
            false
        }
    }

    /// Peek if enough time has elapsed without consuming the tick.
    pub fn is_ready(&self, now_us: u64) -> bool {
        self.last_frame_us == 0 || now_us.saturating_sub(self.last_frame_us) >= self.interval_us
    }

    pub fn reset(&mut self) {
        self.last_frame_us = 0;
    }
}

/// Dynamic screen resolution downscaling controller managing target scale percentage (v1.98 / ADR-084).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenScaleController {
    scale_pct: u32,
    min_scale_pct: u32,
    max_scale_pct: u32,
}

impl Default for ScreenScaleController {
    fn default() -> Self {
        Self::new(SCREEN_SCALE_DEFAULT_PCT)
    }
}

impl ScreenScaleController {
    pub fn new(initial_pct: u32) -> Self {
        let clamped = initial_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
        Self {
            scale_pct: clamped,
            min_scale_pct: SCREEN_SCALE_MIN_PCT,
            max_scale_pct: SCREEN_SCALE_MAX_PCT,
        }
    }

    pub fn with_bounds(initial_pct: u32, min_pct: u32, max_pct: u32) -> Self {
        let min = min_pct.max(SCREEN_SCALE_MIN_PCT);
        let max = max_pct.clamp(min, SCREEN_SCALE_MAX_PCT);
        let clamped = initial_pct.clamp(min, max);
        Self {
            scale_pct: clamped,
            min_scale_pct: min,
            max_scale_pct: max,
        }
    }

    pub fn scale_pct(&self) -> u32 {
        self.scale_pct
    }

    pub fn min_scale_pct(&self) -> u32 {
        self.min_scale_pct
    }

    pub fn max_scale_pct(&self) -> u32 {
        self.max_scale_pct
    }

    pub fn set_scale_pct(&mut self, pct: u32) -> u32 {
        let clamped = pct.clamp(self.min_scale_pct, self.max_scale_pct);
        self.scale_pct = clamped;
        clamped
    }

    pub fn compute_scaled_dims(&self, orig_w: u16, orig_h: u16) -> (u16, u16) {
        Self::scale_dims(orig_w, orig_h, self.scale_pct)
    }

    /// Calculate scaled width and height ensuring even dimensions (minimum 2x2) for YUV/H.264 codecs.
    pub fn scale_dims(orig_w: u16, orig_h: u16, scale_pct: u32) -> (u16, u16) {
        let clamped = scale_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
        if clamped >= 100 {
            return (orig_w, orig_h);
        }
        let w = (orig_w as u32 * clamped / 100) as u16;
        let h = (orig_h as u32 * clamped / 100) as u16;
        let even_w = (w & !1).max(2);
        let even_h = (h & !1).max(2);
        (even_w, even_h)
    }
}

/// Dynamic screen quality & content profile controller (v1.99 / ADR-085).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenQualityController {
    profile: ScreenContentProfile,
    min_qp: u8,
    max_qp: u8,
    custom_qp: bool,
}

impl Default for ScreenQualityController {
    fn default() -> Self {
        Self::new(ScreenContentProfile::default())
    }
}

impl ScreenQualityController {
    pub fn new(profile: ScreenContentProfile) -> Self {
        let (min_qp, max_qp) = profile.default_qp_range();
        Self {
            profile,
            min_qp,
            max_qp,
            custom_qp: false,
        }
    }

    pub fn with_qp(profile: ScreenContentProfile, min_qp: u8, max_qp: u8) -> Self {
        let min = min_qp.clamp(SCREEN_QP_ABSOLUTE_MIN, SCREEN_QP_ABSOLUTE_MAX);
        let max = max_qp.clamp(min, SCREEN_QP_ABSOLUTE_MAX);
        Self {
            profile,
            min_qp: min,
            max_qp: max,
            custom_qp: true,
        }
    }

    pub fn profile(&self) -> ScreenContentProfile {
        self.profile
    }

    pub fn set_profile(&mut self, profile: ScreenContentProfile) -> ScreenContentProfile {
        self.profile = profile;
        if !self.custom_qp {
            let (min, max) = profile.default_qp_range();
            self.min_qp = min;
            self.max_qp = max;
        }
        self.profile
    }

    pub fn min_qp(&self) -> u8 {
        self.min_qp
    }

    pub fn max_qp(&self) -> u8 {
        self.max_qp
    }

    pub fn qp_range(&self) -> (u8, u8) {
        (self.min_qp, self.max_qp)
    }

    pub fn set_qp_range(&mut self, min_qp: u8, max_qp: u8) -> (u8, u8) {
        let min = min_qp.clamp(SCREEN_QP_ABSOLUTE_MIN, SCREEN_QP_ABSOLUTE_MAX);
        let max = max_qp.clamp(min, SCREEN_QP_ABSOLUTE_MAX);
        self.min_qp = min;
        self.max_qp = max;
        self.custom_qp = true;
        (min, max)
    }

    pub fn reset_default_qp(&mut self) -> (u8, u8) {
        let (min, max) = self.profile.default_qp_range();
        self.min_qp = min;
        self.max_qp = max;
        self.custom_qp = false;
        (min, max)
    }

    pub fn custom_qp(&self) -> bool {
        self.custom_qp
    }

    pub fn is_custom(&self) -> bool {
        self.custom_qp
    }

    /// Resolve effective content profile for encoder configuration (translating Auto to Text for desktop sharing).
    pub fn effective_profile(&self) -> ScreenContentProfile {
        match self.profile {
            ScreenContentProfile::Auto => ScreenContentProfile::Text,
            other => other,
        }
    }
}

/// Real-time assessed network health state for autonomous adaptation (v2.00 / ADR-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum NetworkHealth {
    #[default]
    Healthy = 0,
    Warning = 1,
    Congested = 2,
    Critical = 3,
}

impl NetworkHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkHealth::Healthy => "healthy",
            NetworkHealth::Warning => "warning",
            NetworkHealth::Congested => "congested",
            NetworkHealth::Critical => "critical",
        }
    }

    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => NetworkHealth::Healthy,
            1 => NetworkHealth::Warning,
            2 => NetworkHealth::Congested,
            3 => NetworkHealth::Critical,
            _ => NetworkHealth::Healthy,
        }
    }
}

/// Operation mode of the 4-dimensional autonomous adaptation coordinator (v2.00 / ADR-086).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenMatrixMode {
    /// Autonomous 4D cross-layer coordination across bitrate, FPS, scale, and QP.
    #[default]
    Auto = 0,
    /// Conservative mode: adapts bitrate and FPS, preserves native spatial scale.
    Conservative = 1,
    /// Manual mode: coordinator passive; parameters modulated only by explicit user/API calls.
    Manual = 2,
}

impl ScreenMatrixMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenMatrixMode::Auto => "auto",
            ScreenMatrixMode::Conservative => "conservative",
            ScreenMatrixMode::Manual => "manual",
        }
    }

    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenMatrixMode::Auto,
            1 => ScreenMatrixMode::Conservative,
            2 => ScreenMatrixMode::Manual,
            _ => ScreenMatrixMode::Auto,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        let lower = s.trim().to_ascii_lowercase();
        match lower.as_str() {
            "auto" | "dynamic" => Some(ScreenMatrixMode::Auto),
            "conservative" | "preserve_scale" | "scale_locked" => {
                Some(ScreenMatrixMode::Conservative)
            }
            "manual" | "disabled" | "off" => Some(ScreenMatrixMode::Manual),
            _ => None,
        }
    }
}

/// Minimum microsecond duration between automatic tier demotions (cooldown) (400ms).
pub const SCREEN_MATRIX_DEMOTE_COOLDOWN_US: u64 = 400_000;
/// Minimum microsecond duration of sustained healthy telemetry before promoting a tier (hysteresis) (3.0s).
pub const SCREEN_MATRIX_PROMOTE_COOLDOWN_US: u64 = 3_000_000;
/// Minimum consecutive healthy acknowledgements before considering an up-tier promotion.
pub const SCREEN_MATRIX_HEALTHY_ACKS_REQUIRED: u32 = 15;
/// Maximum consecutive frame drops before escalating to Critical health tier.
pub const SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL: u32 = 2;

/// Helper calculating next lower FPS tier during congestion.
pub fn screen_matrix_next_lower_fps(current_fps: u32) -> u32 {
    if current_fps > 30 {
        30
    } else if current_fps > 24 {
        24
    } else if current_fps > 15 {
        15
    } else {
        SCREEN_FPS_MIN
    }
}

/// Helper calculating next higher FPS tier during sustained network recovery.
pub fn screen_matrix_next_higher_fps(current_fps: u32, max_fps: u32) -> u32 {
    let next = if current_fps < 15 {
        15
    } else if current_fps < 24 {
        24
    } else if current_fps < 30 {
        30
    } else {
        60
    };
    next.min(max_fps)
}

/// Helper calculating next lower resolution scale tier during critical congestion.
pub fn screen_matrix_next_lower_scale(current_scale: u32) -> u32 {
    if current_scale > 75 {
        75
    } else if current_scale > 50 {
        50
    } else {
        25
    }
}

/// Helper calculating next higher resolution scale tier during sustained network recovery.
pub fn screen_matrix_next_higher_scale(current_scale: u32) -> u32 {
    if current_scale < 50 {
        50
    } else if current_scale < 75 {
        75
    } else {
        100
    }
}

/// Recommendation outcome from evaluating the 4D matrix coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScreenMatrixOutcome {
    pub health: NetworkHealth,
    pub new_fps: Option<u32>,
    pub new_scale_pct: Option<u32>,
    pub force_idr: bool,
}

/// 4-Dimensional closed-loop autonomous adaptation coordinator (v2.00 / ADR-086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenMatrixCoordinator {
    mode: ScreenMatrixMode,
    health: NetworkHealth,
    auto_fps: bool,
    auto_scale: bool,
    consecutive_healthy_acks: u32,
    consecutive_drops: u32,
    last_tier_change_us: u64,
}

impl Default for ScreenMatrixCoordinator {
    fn default() -> Self {
        Self::new(ScreenMatrixMode::default())
    }
}

impl ScreenMatrixCoordinator {
    pub fn new(mode: ScreenMatrixMode) -> Self {
        Self {
            mode,
            health: NetworkHealth::Healthy,
            auto_fps: true,
            auto_scale: true,
            consecutive_healthy_acks: 0,
            consecutive_drops: 0,
            last_tier_change_us: 0,
        }
    }

    pub fn mode(&self) -> ScreenMatrixMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenMatrixMode) {
        self.mode = mode;
    }

    pub fn health(&self) -> NetworkHealth {
        self.health
    }

    pub fn auto_fps(&self) -> bool {
        self.auto_fps
    }

    pub fn set_auto_fps(&mut self, auto: bool) {
        self.auto_fps = auto;
    }

    pub fn auto_scale(&self) -> bool {
        self.auto_scale
    }

    pub fn set_auto_scale(&mut self, auto: bool) {
        self.auto_scale = auto;
    }

    pub fn consecutive_healthy_acks(&self) -> u32 {
        self.consecutive_healthy_acks
    }

    pub fn consecutive_drops(&self) -> u32 {
        self.consecutive_drops
    }

    /// Handle frame drop event with microsecond timestamp.
    pub fn on_drop(
        &mut self,
        now_us: u64,
        current_fps: u32,
        current_scale: u32,
    ) -> ScreenMatrixOutcome {
        self.consecutive_drops = self.consecutive_drops.saturating_add(1);
        self.consecutive_healthy_acks = 0;

        if self.consecutive_drops >= SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL {
            self.health = NetworkHealth::Critical;
        } else {
            self.health = NetworkHealth::Congested;
        }

        if self.mode == ScreenMatrixMode::Manual {
            return ScreenMatrixOutcome {
                health: self.health,
                new_fps: None,
                new_scale_pct: None,
                force_idr: false,
            };
        }

        let mut outcome = ScreenMatrixOutcome {
            health: self.health,
            new_fps: None,
            new_scale_pct: None,
            force_idr: false,
        };

        if now_us.saturating_sub(self.last_tier_change_us) >= SCREEN_MATRIX_DEMOTE_COOLDOWN_US {
            if self.health == NetworkHealth::Critical {
                // Demote both scale (if Auto mode) and FPS.
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale > 25 {
                    let next_scale = screen_matrix_next_lower_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                }
                if self.auto_fps && current_fps > SCREEN_FPS_MIN {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                }
                if outcome.new_scale_pct.is_some() || outcome.new_fps.is_some() {
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                }
            } else if self.health == NetworkHealth::Congested {
                // Demote FPS first to cut socket pressure.
                if self.auto_fps && current_fps > 15 {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                    self.last_tier_change_us = now_us;
                }
            }
        }

        outcome
    }

    /// Handle incoming frame acknowledgment with microsecond RTT and in-flight count.
    pub fn on_ack(
        &mut self,
        now_us: u64,
        rtt_us: u64,
        in_flight: u64,
        current_fps: u32,
        current_scale: u32,
        target_max_fps: u32,
    ) -> ScreenMatrixOutcome {
        // Assess current network health.
        if in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || rtt_us >= 500_000
            || self.consecutive_drops >= SCREEN_MATRIX_CONSECUTIVE_DROPS_CRITICAL
        {
            self.health = NetworkHealth::Critical;
            self.consecutive_healthy_acks = 0;
        } else if in_flight >= (SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD / 2)
            || rtt_us >= SCREEN_CONGESTION_RTT_THRESHOLD_US
            || self.consecutive_drops > 0
        {
            self.health = NetworkHealth::Congested;
            self.consecutive_healthy_acks = 0;
        } else if in_flight > 0 || rtt_us >= 120_000 {
            self.health = NetworkHealth::Warning;
            self.consecutive_healthy_acks = 0;
        } else {
            self.health = NetworkHealth::Healthy;
            self.consecutive_drops = 0;
            self.consecutive_healthy_acks = self.consecutive_healthy_acks.saturating_add(1);
        }

        if self.mode == ScreenMatrixMode::Manual {
            return ScreenMatrixOutcome {
                health: self.health,
                new_fps: None,
                new_scale_pct: None,
                force_idr: false,
            };
        }

        let mut outcome = ScreenMatrixOutcome {
            health: self.health,
            new_fps: None,
            new_scale_pct: None,
            force_idr: false,
        };

        if self.health == NetworkHealth::Healthy {
            // Check if hysteresis threshold is met for promotion.
            if self.consecutive_healthy_acks >= SCREEN_MATRIX_HEALTHY_ACKS_REQUIRED
                && now_us.saturating_sub(self.last_tier_change_us)
                    >= SCREEN_MATRIX_PROMOTE_COOLDOWN_US
            {
                // Recovery Ladder:
                // 1. Promote resolution scale first (if below 100%).
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale < 100 {
                    let next_scale = screen_matrix_next_higher_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                    self.consecutive_healthy_acks = 0;
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                } else if self.auto_fps && current_fps < target_max_fps {
                    // 2. Once scale is at 100%, promote FPS.
                    let next_fps = screen_matrix_next_higher_fps(current_fps, target_max_fps);
                    outcome.new_fps = Some(next_fps);
                    self.consecutive_healthy_acks = 0;
                    self.last_tier_change_us = now_us;
                }
            }
        } else if (self.health == NetworkHealth::Critical
            || self.health == NetworkHealth::Congested)
            && now_us.saturating_sub(self.last_tier_change_us) >= SCREEN_MATRIX_DEMOTE_COOLDOWN_US
        {
            // Under sustained high RTT / in-flight, demote if appropriate.
            if self.health == NetworkHealth::Critical {
                if self.mode == ScreenMatrixMode::Auto && self.auto_scale && current_scale > 25 {
                    let next_scale = screen_matrix_next_lower_scale(current_scale);
                    outcome.new_scale_pct = Some(next_scale);
                }
                if self.auto_fps && current_fps > SCREEN_FPS_MIN {
                    let next_fps = screen_matrix_next_lower_fps(current_fps);
                    outcome.new_fps = Some(next_fps);
                }
                if outcome.new_scale_pct.is_some() || outcome.new_fps.is_some() {
                    self.last_tier_change_us = now_us;
                    outcome.force_idr = true;
                }
            } else if self.health == NetworkHealth::Congested && self.auto_fps && current_fps > 15 {
                let next_fps = screen_matrix_next_lower_fps(current_fps);
                outcome.new_fps = Some(next_fps);
                self.last_tier_change_us = now_us;
            }
        }

        outcome
    }
}

/// Bilinear downscaling for 32-bit BGRA/RGBA buffers with 16.16 fixed-point arithmetic (v1.98 / ADR-084).
pub fn downscale_bgra8(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    if src_w == dst_w && src_h == dst_h {
        return src.to_vec();
    }
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Vec::new();
    }
    let mut dst = vec![0u8; dst_w * dst_h * 4];
    let x_ratio = ((src_w << 16) / dst_w) as u32;
    let y_ratio = ((src_h << 16) / dst_h) as u32;

    for dy in 0..dst_h {
        let sy_fp = (dy as u32) * y_ratio;
        let sy0 = (sy_fp >> 16) as usize;
        let sy1 = (sy0 + 1).min(src_h - 1);
        let y_weight = (sy_fp & 0xffff) >> 8;
        let y_weight_inv = 256 - y_weight;

        let dst_row_offset = dy * dst_w * 4;
        let src_row0_offset = sy0 * src_w * 4;
        let src_row1_offset = sy1 * src_w * 4;

        for dx in 0..dst_w {
            let sx_fp = (dx as u32) * x_ratio;
            let sx0 = (sx_fp >> 16) as usize;
            let sx1 = (sx0 + 1).min(src_w - 1);
            let x_weight = (sx_fp & 0xffff) >> 8;
            let x_weight_inv = 256 - x_weight;

            let p00_idx = src_row0_offset + sx0 * 4;
            let p10_idx = src_row0_offset + sx1 * 4;
            let p01_idx = src_row1_offset + sx0 * 4;
            let p11_idx = src_row1_offset + sx1 * 4;

            let dst_idx = dst_row_offset + dx * 4;

            for c in 0..4 {
                let v00 = src[p00_idx + c] as u32;
                let v10 = src[p10_idx + c] as u32;
                let v01 = src[p01_idx + c] as u32;
                let v11 = src[p11_idx + c] as u32;

                let top = (v00 * x_weight_inv + v10 * x_weight) >> 8;
                let bottom = (v01 * x_weight_inv + v11 * x_weight) >> 8;
                let val = (top * y_weight_inv + bottom * y_weight) >> 8;

                dst[dst_idx + c] = val.min(255) as u8;
            }
        }
    }
    dst
}

/// Downscale an uncompressed raw ScreenFrame according to target scale percentage (v1.98 / ADR-084).
pub fn downscale_frame(frame: &ScreenFrame, scale_pct: u32) -> Result<ScreenFrame, CoreError> {
    let clamped_pct = scale_pct.clamp(SCREEN_SCALE_MIN_PCT, SCREEN_SCALE_MAX_PCT);
    if clamped_pct >= 100 {
        return Ok(frame.clone());
    }
    if frame.compressed() {
        return Err(CoreError::ScreenFrame);
    }
    let (dst_w, dst_h) = ScreenScaleController::scale_dims(frame.rect.w, frame.rect.h, clamped_pct);
    if dst_w == frame.rect.w && dst_h == frame.rect.h {
        return Ok(frame.clone());
    }
    let expected_src_len = frame.rect.pixel_bytes(frame.format);
    if frame.bytes.len() != expected_src_len {
        return Err(CoreError::ScreenFrame);
    }
    let scaled_bytes = downscale_bgra8(
        &frame.bytes,
        frame.rect.w as usize,
        frame.rect.h as usize,
        dst_w as usize,
        dst_h as usize,
    );
    let new_rect = ScreenRect::new(frame.rect.x, frame.rect.y, dst_w, dst_h)?;
    Ok(ScreenFrame {
        seq: frame.seq,
        rect: new_rect,
        format: frame.format,
        codec_tag: frame.codec_tag,
        stride: dst_w as u32 * frame.format.bpp() as u32,
        bytes: scaled_bytes,
    })
}

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
struct PlayoutEntry {
    frame: ScreenFrame,
    arrival_us: u64,
    scheduled_playout_us: u64,
}

/// Receiver playout buffer and adaptive jitter dejitterizer (v2.01 / ADR-087).
///
/// Absorbs network packet delay variation (jitter), calculates interarrival jitter using
/// RFC 3550 exponential moving average, schedules presentation timestamps to eliminate
/// micro-stutter, and drops late or overflowing frames to prevent latency drift.
#[derive(Debug, Clone)]
pub struct ScreenPlayoutBuffer {
    mode: ScreenPlayoutMode,
    custom_target_delay_us: Option<u64>,
    queue: VecDeque<PlayoutEntry>,
    last_arrival_us: u64,
    last_scheduled_playout_us: u64,
    jitter_us: u64,
    target_delay_us: u64,
    max_capacity: usize,
    late_threshold_us: u64,
    played_count: u64,
    dropped_late_count: u64,
    dropped_overflow_count: u64,
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

    fn recompute_target_delay(&mut self) {
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
    mode: ScreenCursorMode,
    next_seq: u64,
    last_x: i32,
    last_y: i32,
    last_visible: bool,
    last_shape: CursorShape,
    last_sent_us: u64,
    min_interval_us: u64,
    stats: ScreenCursorStats,
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

/// Bounding rectangle for detected dirty regions on screen (v2.03 / ADR-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenDamageRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl ScreenDamageRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Summary report of dirty region evaluation across consecutive screen frames (v2.03 / ADR-089).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenDamageReport {
    pub dirty_pixels: u64,
    pub total_pixels: u64,
    pub dirty_ratio: f32,
    pub num_dirty_blocks: usize,
    pub total_blocks: usize,
    pub is_static: bool,
    pub bounding_rect: Option<ScreenDamageRect>,
}

impl ScreenDamageReport {
    pub fn is_dirty(&self) -> bool {
        !self.is_static
    }

    pub fn rect(&self) -> Option<ScreenDamageRect> {
        self.bounding_rect
    }
}

/// Grid-based dirty block damage detector for screen frames (v2.03 / ADR-089).
#[derive(Debug, Clone, Default)]
pub struct ScreenDamageTracker {
    has_prior: bool,
}

impl ScreenDamageTracker {
    pub fn new() -> Self {
        Self { has_prior: false }
    }

    /// Evaluates damage between two RGBA8 frames of given width and height.
    /// Operates on 32x32 pixel macroblocks for fast 64-bit comparison.
    pub fn evaluate_rgba(
        &mut self,
        prev_rgba: &[u8],
        curr_rgba: &[u8],
        width: u32,
        height: u32,
    ) -> ScreenDamageReport {
        let total_pixels = (width as u64) * (height as u64);
        if total_pixels == 0
            || prev_rgba.len() != curr_rgba.len()
            || prev_rgba.len() < (total_pixels as usize) * 4
        {
            return ScreenDamageReport {
                total_pixels,
                dirty_pixels: total_pixels,
                dirty_ratio: 1.0,
                num_dirty_blocks: 1,
                total_blocks: 1,
                is_static: false,
                bounding_rect: Some(ScreenDamageRect {
                    x: 0,
                    y: 0,
                    width: width as u16,
                    height: height as u16,
                }),
            };
        }

        let block_w = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let block_h = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let grid_cols = width.div_ceil(block_w);
        let grid_rows = height.div_ceil(block_h);
        let total_blocks = (grid_cols * grid_rows) as usize;

        let mut num_dirty_blocks = 0usize;
        let mut dirty_pixels = 0u64;
        let mut min_bx = u32::MAX;
        let mut max_bx = 0u32;
        let mut min_by = u32::MAX;
        let mut max_by = 0u32;

        let stride = (width * 4) as usize;

        for by in 0..grid_rows {
            let y_start = by * block_h;
            let y_end = (y_start + block_h).min(height);
            let bh = y_end - y_start;

            for bx in 0..grid_cols {
                let x_start = bx * block_w;
                let x_end = (x_start + block_w).min(width);
                let bw = x_end - x_start;
                let row_bytes = (bw * 4) as usize;

                let mut block_dirty = false;
                for y in y_start..y_end {
                    let offset = (y as usize) * stride + (x_start as usize) * 4;
                    if prev_rgba[offset..offset + row_bytes]
                        != curr_rgba[offset..offset + row_bytes]
                    {
                        block_dirty = true;
                        break;
                    }
                }

                if block_dirty {
                    num_dirty_blocks += 1;
                    dirty_pixels += (bw as u64) * (bh as u64);
                    min_bx = min_bx.min(x_start);
                    max_bx = max_bx.max(x_end);
                    min_by = min_by.min(y_start);
                    max_by = max_by.max(y_end);
                }
            }
        }

        let is_static = num_dirty_blocks == 0;
        let dirty_ratio = if total_pixels > 0 {
            (dirty_pixels as f32) / (total_pixels as f32)
        } else {
            0.0
        };

        let bounding_rect = if !is_static && min_bx < max_bx && min_by < max_by {
            Some(ScreenDamageRect {
                x: min_bx as u16,
                y: min_by as u16,
                width: (max_bx - min_bx) as u16,
                height: (max_by - min_by) as u16,
            })
        } else {
            None
        };

        self.has_prior = true;

        ScreenDamageReport {
            dirty_pixels,
            total_pixels,
            dirty_ratio,
            num_dirty_blocks,
            total_blocks,
            is_static,
            bounding_rect,
        }
    }

    /// Evaluates damage based on an explicit boolean change hint (e.g. from capture driver or mock).
    pub fn evaluate_hint(&mut self, is_dirty: bool, width: u32, height: u32) -> ScreenDamageReport {
        let total_pixels = (width as u64) * (height as u64);
        let block_w = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let block_h = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let grid_cols = width.div_ceil(block_w);
        let grid_rows = height.div_ceil(block_h);
        let total_blocks = (grid_cols * grid_rows) as usize;

        if is_dirty {
            ScreenDamageReport {
                dirty_pixels: total_pixels,
                total_pixels,
                dirty_ratio: 1.0,
                num_dirty_blocks: total_blocks,
                total_blocks,
                is_static: false,
                bounding_rect: Some(ScreenDamageRect {
                    x: 0,
                    y: 0,
                    width: width as u16,
                    height: height as u16,
                }),
            }
        } else {
            ScreenDamageReport {
                dirty_pixels: 0,
                total_pixels,
                dirty_ratio: 0.0,
                num_dirty_blocks: 0,
                total_blocks,
                is_static: true,
                bounding_rect: None,
            }
        }
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
    mode: ScreenSquelchMode,
    stats: ScreenSquelchStats,
    last_change_us: u64,
    last_sent_us: u64,
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

/// Multiplication in GF(2^8) with irreducible polynomial 0x11D (x^8 + x^4 + x^3 + x^2 + 1) (v2.04 / ADR-090).
pub fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        if (b & 1) != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1d;
        }
        b >>= 1;
    }
    p
}

/// Multiplicative inverse in GF(2^8) using Fermat's Little Theorem (a^254 = a^-1) (v2.04 / ADR-090).
pub fn gf_inv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let mut res = 1u8;
    let mut base = a;
    let mut exp = 254;
    while exp > 0 {
        if (exp & 1) != 0 {
            res = gf_mul(res, base);
        }
        base = gf_mul(base, base);
        exp >>= 1;
    }
    res
}

/// Division in GF(2^8): a / b = a * inv(b) (v2.04 / ADR-090).
pub fn gf_div(a: u8, b: u8) -> u8 {
    if b == 0 {
        0
    } else {
        gf_mul(a, gf_inv(b))
    }
}

/// Cauchy matrix coefficient at row (parity index `r`) and col (data index `c`) (v2.04 / ADR-090).
/// X_c = c, Y_r = k + r. Since X_c != Y_r in 0..255, X_c ^ Y_r is non-zero.
pub fn cauchy_coeff(r: usize, c: usize, k: usize) -> u8 {
    let x = c as u8;
    let y = (k + r) as u8;
    gf_inv(x ^ y)
}

/// Forward error correction (FEC) operating mode (v2.04 / ADR-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ScreenFecMode {
    /// Dynamically scales redundancy percentage (0%~30%) based on measured loss & network health.
    #[default]
    Auto = 0,
    /// Fixed baseline 10% redundancy protection for predictable packet loss.
    Conservative = 1,
    /// High-protection 25% redundancy for lossy wireless or mobile cellular links.
    Aggressive = 2,
    /// Passthrough mode with zero parity shards generated or transmitted.
    Disabled = 3,
}

impl ScreenFecMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Conservative => "conservative",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }

    pub const fn tag(&self) -> u8 {
        *self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Conservative,
            2 => Self::Aggressive,
            3 => Self::Disabled,
            _ => Self::Auto,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "conservative" | "cons" | "low" => Some(Self::Conservative),
            "aggressive" | "agg" | "aggro" | "high" => Some(Self::Aggressive),
            "disabled" | "off" | "none" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenFecMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or(CoreError::ScreenFrame)
    }
}

/// Telemetry metrics for forward error correction (v2.04 / ADR-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenFecStats {
    pub mode: ScreenFecMode,
    pub redundancy_pct: u8,
    pub blocks_encoded: u64,
    pub data_shards_sent: u64,
    pub parity_shards_sent: u64,
    pub blocks_decoded: u64,
    pub recovered_shards: u64,
    pub unrecoverable_blocks: u64,
}

/// JSON report DTO for forward error correction inspection (v2.04 / ADR-090).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenFecReport {
    pub ok: bool,
    pub mode: String,
    pub redundancy_pct: u8,
    pub blocks_encoded: u64,
    pub data_shards_sent: u64,
    pub parity_shards_sent: u64,
    pub blocks_decoded: u64,
    pub recovered_shards: u64,
    pub unrecoverable_blocks: u64,
    pub timestamp_ms: u64,
}

/// Individual forward error correction shard (data or parity) (v2.04 / ADR-090).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenFecShard {
    pub frame_seq: u64,
    pub block_id: u16,
    pub k_data_shards: u8,
    pub m_parity_shards: u8,
    pub shard_index: u8,
    pub is_parity: bool,
    pub data: Vec<u8>,
}

impl ScreenFecShard {
    pub fn encode_header(&self) -> [u8; FEC_HEADER_LEN] {
        let mut hdr = [0u8; FEC_HEADER_LEN];
        hdr[0..4].copy_from_slice(&FEC_MAGIC);
        hdr[4] = FEC_PROTO;
        hdr[5] = 0; // flags
        hdr[6..8].copy_from_slice(&[0, 0]); // reserved
        hdr[8..16].copy_from_slice(&self.frame_seq.to_be_bytes());
        hdr[16..18].copy_from_slice(&self.block_id.to_be_bytes());
        hdr[18] = self.k_data_shards;
        hdr[19] = self.m_parity_shards;
        hdr[20] = self.shard_index;
        hdr[21] = if self.is_parity { 1 } else { 0 };
        hdr[22..24].copy_from_slice(&(self.data.len() as u16).to_be_bytes());
        hdr
    }

    pub fn to_inner_frame(&self) -> InnerFrame {
        let hdr = self.encode_header();
        let mut payload = Vec::with_capacity(FEC_HEADER_LEN + self.data.len());
        payload.extend_from_slice(&hdr);
        payload.extend_from_slice(&self.data);
        InnerFrame {
            ty: TYPE_SCREEN_FEC,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload,
        }
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_FEC {
            return Err(CoreError::ScreenFrame);
        }
        if frame.payload.len() < FEC_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if frame.payload[0..4] != FEC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if frame.payload[4] != FEC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let frame_seq = u64::from_be_bytes(frame.payload[8..16].try_into().unwrap());
        let block_id = u16::from_be_bytes(frame.payload[16..18].try_into().unwrap());
        let k_data_shards = frame.payload[18];
        let m_parity_shards = frame.payload[19];
        let shard_index = frame.payload[20];
        let is_parity = frame.payload[21] != 0;
        let shard_len = u16::from_be_bytes(frame.payload[22..24].try_into().unwrap()) as usize;
        if frame.payload.len() < FEC_HEADER_LEN + shard_len {
            return Err(CoreError::TruncatedFrame);
        }
        let data = frame.payload[FEC_HEADER_LEN..FEC_HEADER_LEN + shard_len].to_vec();
        Ok(Self {
            frame_seq,
            block_id,
            k_data_shards,
            m_parity_shards,
            shard_index,
            is_parity,
            data,
        })
    }
}

/// Cauchy Reed-Solomon encoder over GF(2^8) (v2.04 / ADR-090).
pub struct ScreenFecEncoder;

impl ScreenFecEncoder {
    /// Encodes a list of data buffers into K data shards + M parity shards.
    /// Each data shard payload is formatted as: 2-byte orig_len + raw bytes,
    /// then padded to maximum shard size S for Galois field matrix encoding.
    #[allow(clippy::needless_range_loop)]
    pub fn encode_block(
        frame_seq: u64,
        block_id: u16,
        data_buffers: &[Vec<u8>],
        m_parity: usize,
    ) -> Result<Vec<ScreenFecShard>, CoreError> {
        let k = data_buffers.len();
        if k == 0 || k > FEC_MAX_DATA_SHARDS || m_parity > FEC_MAX_PARITY_SHARDS {
            return Err(CoreError::ScreenFrame);
        }

        // Format each data shard: 2-byte orig_len (big-endian) + data
        let mut formatted_data: Vec<Vec<u8>> = Vec::with_capacity(k);
        let mut max_shard_len = 0;
        for buf in data_buffers {
            let orig_len = buf.len();
            if orig_len > u16::MAX as usize {
                return Err(CoreError::ScreenTooLarge);
            }
            let mut shard_bytes = Vec::with_capacity(2 + orig_len);
            shard_bytes.extend_from_slice(&(orig_len as u16).to_be_bytes());
            shard_bytes.extend_from_slice(buf);
            max_shard_len = max_shard_len.max(shard_bytes.len());
            formatted_data.push(shard_bytes);
        }

        // Pad all formatted shards to max_shard_len
        for shard in &mut formatted_data {
            if shard.len() < max_shard_len {
                shard.resize(max_shard_len, 0);
            }
        }

        let mut shards = Vec::with_capacity(k + m_parity);

        // Add K data shards
        for (idx, shard_data) in formatted_data.iter().enumerate() {
            shards.push(ScreenFecShard {
                frame_seq,
                block_id,
                k_data_shards: k as u8,
                m_parity_shards: m_parity as u8,
                shard_index: idx as u8,
                is_parity: false,
                data: shard_data.clone(),
            });
        }

        // Generate M Cauchy parity shards
        for r in 0..m_parity {
            let mut parity_buf = vec![0u8; max_shard_len];
            for c in 0..k {
                let coeff = cauchy_coeff(r, c, k);
                let data_shard = &formatted_data[c];
                for b in 0..max_shard_len {
                    parity_buf[b] ^= gf_mul(coeff, data_shard[b]);
                }
            }
            shards.push(ScreenFecShard {
                frame_seq,
                block_id,
                k_data_shards: k as u8,
                m_parity_shards: m_parity as u8,
                shard_index: r as u8,
                is_parity: true,
                data: parity_buf,
            });
        }

        Ok(shards)
    }
}

/// Cauchy Reed-Solomon decoder over GF(2^8) (v2.04 / ADR-090).
pub struct ScreenFecDecoder;

impl ScreenFecDecoder {
    /// Attempts to decode/reconstruct the original K data buffers from received shards.
    /// Returns:
    /// - Ok(Some(recovered_data_buffers)) if all K shards are present or successfully recovered.
    /// - Ok(None) if fewer than K distinct shards have arrived so far (unrecoverable without more shards).
    /// - Err(CoreError) if shards are corrupted or have invalid parameters.
    #[allow(clippy::needless_range_loop)]
    pub fn decode_block(shards: &[ScreenFecShard]) -> Result<Option<Vec<Vec<u8>>>, CoreError> {
        if shards.is_empty() {
            return Ok(None);
        }
        let k = shards[0].k_data_shards as usize;
        let m = shards[0].m_parity_shards as usize;
        if k == 0 || k > FEC_MAX_DATA_SHARDS || m > FEC_MAX_PARITY_SHARDS {
            return Err(CoreError::ScreenFrame);
        }

        let mut data_shards_map: std::collections::BTreeMap<u8, Vec<u8>> =
            std::collections::BTreeMap::new();
        let mut parity_shards_map: std::collections::BTreeMap<u8, Vec<u8>> =
            std::collections::BTreeMap::new();

        for shard in shards {
            if shard.is_parity {
                if (shard.shard_index as usize) < m {
                    parity_shards_map.insert(shard.shard_index, shard.data.clone());
                }
            } else if (shard.shard_index as usize) < k {
                data_shards_map.insert(shard.shard_index, shard.data.clone());
            }
        }

        // Fast path: all K data shards arrived intact!
        if data_shards_map.len() == k {
            let mut out = Vec::with_capacity(k);
            for idx in 0..k {
                let shard_buf = data_shards_map.get(&(idx as u8)).unwrap();
                if shard_buf.len() < 2 {
                    return Err(CoreError::TruncatedFrame);
                }
                let orig_len = u16::from_be_bytes(shard_buf[0..2].try_into().unwrap()) as usize;
                if shard_buf.len() < 2 + orig_len {
                    return Err(CoreError::TruncatedFrame);
                }
                out.push(shard_buf[2..2 + orig_len].to_vec());
            }
            return Ok(Some(out));
        }

        // Missing data shards
        let total_received = data_shards_map.len() + parity_shards_map.len();
        if total_received < k {
            // Cannot recover yet: fewer than K total shards
            return Ok(None);
        }

        let mut missing_indices = Vec::new();
        for idx in 0..k {
            if !data_shards_map.contains_key(&(idx as u8)) {
                missing_indices.push(idx);
            }
        }
        let num_missing = missing_indices.len();
        if parity_shards_map.len() < num_missing {
            // Too many lost shards to recover with available parity
            return Ok(None);
        }

        // Select num_missing parity shards
        let selected_parity: Vec<(usize, Vec<u8>)> = parity_shards_map
            .into_iter()
            .take(num_missing)
            .map(|(idx, data)| (idx as usize, data))
            .collect();

        let shard_len = shards[0].data.len();

        // Construct linear system: A * X = V
        // A is num_missing x num_missing matrix in GF(2^8)
        let mut a: Vec<Vec<u8>> = Vec::with_capacity(num_missing);
        let mut v: Vec<Vec<u8>> = Vec::with_capacity(num_missing);

        for (p_idx, p_data) in selected_parity.iter() {
            let mut a_row = Vec::with_capacity(num_missing);
            for &m_idx in &missing_indices {
                a_row.push(cauchy_coeff(*p_idx, m_idx, k));
            }
            a.push(a_row);

            let mut v_row = p_data.clone();
            if v_row.len() < shard_len {
                v_row.resize(shard_len, 0);
            }

            // Subtract (XOR) the contributions from already available data shards
            for (&c_idx, c_data) in &data_shards_map {
                let coeff = cauchy_coeff(*p_idx, c_idx as usize, k);
                for b in 0..shard_len {
                    let byte_val = if b < c_data.len() { c_data[b] } else { 0 };
                    v_row[b] ^= gf_mul(coeff, byte_val);
                }
            }
            v.push(v_row);
        }

        // Solve A * X = V via Gaussian Elimination over GF(2^8)
        for i in 0..num_missing {
            // Find pivot
            let mut pivot = i;
            while pivot < num_missing && a[pivot][i] == 0 {
                pivot += 1;
            }
            if pivot == num_missing {
                return Err(CoreError::ScreenFrame); // Singular matrix
            }
            if pivot != i {
                a.swap(i, pivot);
                v.swap(i, pivot);
            }

            // Scale row i so A[i][i] = 1
            let inv_diag = gf_inv(a[i][i]);
            for col in i..num_missing {
                a[i][col] = gf_mul(a[i][col], inv_diag);
            }
            for b in 0..shard_len {
                v[i][b] = gf_mul(v[i][b], inv_diag);
            }

            // Eliminate other rows
            for r in 0..num_missing {
                if r != i && a[r][i] != 0 {
                    let factor = a[r][i];
                    for col in i..num_missing {
                        a[r][col] ^= gf_mul(factor, a[i][col]);
                    }
                    for b in 0..shard_len {
                        let scaled_v = gf_mul(factor, v[i][b]);
                        v[r][b] ^= scaled_v;
                    }
                }
            }
        }

        // v[i] now contains the reconstructed missing data shard for missing_indices[i]
        for (i, &m_idx) in missing_indices.iter().enumerate() {
            data_shards_map.insert(m_idx as u8, v[i].clone());
        }

        // Unpack original data payloads
        let mut out = Vec::with_capacity(k);
        for idx in 0..k {
            let shard_buf = data_shards_map.get(&(idx as u8)).unwrap();
            if shard_buf.len() < 2 {
                return Err(CoreError::TruncatedFrame);
            }
            let orig_len = u16::from_be_bytes(shard_buf[0..2].try_into().unwrap()) as usize;
            if shard_buf.len() < 2 + orig_len {
                return Err(CoreError::TruncatedFrame);
            }
            out.push(shard_buf[2..2 + orig_len].to_vec());
        }

        Ok(Some(out))
    }
}

/// Adaptive controller for forward error correction redundancy (v2.04 / ADR-090).
#[derive(Debug, Clone, Default)]
pub struct ScreenFecController {
    mode: ScreenFecMode,
    stats: ScreenFecStats,
}

impl ScreenFecController {
    pub fn new(mode: ScreenFecMode) -> Self {
        let redundancy_pct = match mode {
            ScreenFecMode::Auto => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Disabled => 0,
        };
        Self {
            mode,
            stats: ScreenFecStats {
                mode,
                redundancy_pct,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenFecMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenFecMode) -> ScreenFecMode {
        self.mode = mode;
        self.stats.mode = mode;
        self.stats.redundancy_pct = match mode {
            ScreenFecMode::Auto => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Disabled => 0,
        };
        mode
    }

    pub fn stats(&self) -> ScreenFecStats {
        self.stats
    }

    /// Calculate required parity shards M for a block of K data shards based on current mode and packet loss rate.
    pub fn calculate_parity_shards(&mut self, k: usize, packet_loss_ratio: f32) -> usize {
        if k == 0 || self.mode == ScreenFecMode::Disabled {
            self.stats.redundancy_pct = 0;
            return 0;
        }

        let redundancy_pct = match self.mode {
            ScreenFecMode::Disabled => 0,
            ScreenFecMode::Conservative => 10,
            ScreenFecMode::Aggressive => 25,
            ScreenFecMode::Auto => {
                if packet_loss_ratio < 0.005 {
                    0
                } else if packet_loss_ratio < 0.03 {
                    10
                } else if packet_loss_ratio < 0.08 {
                    20
                } else {
                    30
                }
            }
        };

        self.stats.redundancy_pct = redundancy_pct;

        if redundancy_pct == 0 {
            0
        } else {
            let m = ((k as f32) * (redundancy_pct as f32) / 100.0).ceil() as usize;
            m.clamp(1, FEC_MAX_PARITY_SHARDS)
        }
    }

    pub fn record_block_encoded(&mut self, k: usize, m: usize) {
        self.stats.blocks_encoded = self.stats.blocks_encoded.saturating_add(1);
        self.stats.data_shards_sent = self.stats.data_shards_sent.saturating_add(k as u64);
        self.stats.parity_shards_sent = self.stats.parity_shards_sent.saturating_add(m as u64);
    }

    pub fn record_block_decoded(&mut self, recovered_count: usize) {
        self.stats.blocks_decoded = self.stats.blocks_decoded.saturating_add(1);
        self.stats.recovered_shards = self
            .stats
            .recovered_shards
            .saturating_add(recovered_count as u64);
    }

    pub fn record_unrecoverable(&mut self) {
        self.stats.unrecoverable_blocks = self.stats.unrecoverable_blocks.saturating_add(1);
    }
}

/// Tracks and reconstructs incoming ScreenFrame from TYPE_SCREEN_FEC inner frames (v2.04 / ADR-090).
#[derive(Debug, Default)]
pub struct ScreenFecAssembler {
    current_seq: u64,
    current_block_id: u16,
    received_shards: Vec<ScreenFecShard>,
    recovered_shards_total: u64,
    unrecoverable_blocks_total: u64,
}

impl ScreenFecAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_shard(&mut self, shard: ScreenFecShard) -> Result<Option<ScreenFrame>, CoreError> {
        // Stream rollover or new frame sequence
        if shard.frame_seq != self.current_seq || shard.block_id != self.current_block_id {
            if self.current_seq != 0 && !self.received_shards.is_empty() {
                // If previous block never finished, count as unrecoverable
                self.unrecoverable_blocks_total = self.unrecoverable_blocks_total.saturating_add(1);
            }
            self.current_seq = shard.frame_seq;
            self.current_block_id = shard.block_id;
            self.received_shards.clear();
        }

        // Dedup shard
        if !self
            .received_shards
            .iter()
            .any(|s| s.is_parity == shard.is_parity && s.shard_index == shard.shard_index)
        {
            self.received_shards.push(shard);
        }

        match ScreenFecDecoder::decode_block(&self.received_shards)? {
            Some(data_buffers) => {
                let k = self.received_shards[0].k_data_shards as usize;
                let data_arrived = self.received_shards.iter().filter(|s| !s.is_parity).count();
                if data_arrived < k {
                    let recovered = k - data_arrived;
                    self.recovered_shards_total =
                        self.recovered_shards_total.saturating_add(recovered as u64);
                }
                self.received_shards.clear();

                // Concatenate data buffers
                let mut full_payload = Vec::new();
                for buf in data_buffers {
                    full_payload.extend_from_slice(&buf);
                }
                let frame = ScreenFrame::decode_payload(&full_payload)?;
                Ok(Some(frame))
            }
            None => Ok(None),
        }
    }

    pub fn recovered_shards(&self) -> u64 {
        self.recovered_shards_total
    }

    pub fn unrecoverable_blocks(&self) -> u64 {
        self.unrecoverable_blocks_total
    }

    pub fn reset(&mut self) {
        self.current_seq = 0;
        self.current_block_id = 0;
        self.received_shards.clear();
    }
}

pub const SCREEN_PACER_BURST_AUTO: usize = 4;
pub const SCREEN_PACER_BURST_AGGRESSIVE: usize = 2;
pub const SCREEN_PACER_MAX_DELAY_US: u64 = 50_000;

/// Screen streaming packet pacing mode (v2.05 / ADR-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScreenPacerMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Disabled = 2,
}

impl ScreenPacerMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Aggressive,
            2 => Self::Disabled,
            _ => Self::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Aggressive => "aggressive",
            Self::Disabled => "disabled",
        }
    }
}

impl std::str::FromStr for ScreenPacerMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "aggressive" | "agg" => Ok(Self::Aggressive),
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

impl std::fmt::Display for ScreenPacerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Statistics and counters for screen packet pacing (v2.05 / ADR-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenPacerStats {
    pub mode: ScreenPacerMode,
    pub packets_paced: u64,
    pub bursts_smoothed: u64,
    pub total_delay_us: u64,
    pub avg_delay_us: u64,
    pub last_delay_us: u64,
}

/// JSON report DTO for screen packet pacing (v2.05 / ADR-091).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenPacerReport {
    pub ok: bool,
    pub mode: String,
    pub packets_paced: u64,
    pub bursts_smoothed: u64,
    pub avg_delay_us: u64,
    pub last_delay_us: u64,
    pub timestamp_ms: u64,
}

/// Screen streaming microsecond-level packet pacer & burst shaper (v2.05 / ADR-091).
#[derive(Debug, Clone)]
pub struct ScreenPacketPacer {
    mode: ScreenPacerMode,
    stats: ScreenPacerStats,
}

impl Default for ScreenPacketPacer {
    fn default() -> Self {
        Self::new(ScreenPacerMode::Auto)
    }
}

impl ScreenPacketPacer {
    pub fn new(mode: ScreenPacerMode) -> Self {
        Self {
            mode,
            stats: ScreenPacerStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenPacerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenPacerMode) -> ScreenPacerMode {
        self.mode = mode;
        self.stats.mode = mode;
        mode
    }

    pub fn stats(&self) -> ScreenPacerStats {
        self.stats
    }

    /// Computes relative departure delay offsets (in microseconds) for a batch of N packets
    /// belonging to a single video frame, smoothing micro-bursts across the frame interval.
    pub fn compute_packet_delays(&mut self, packet_count: usize, fps_target: u32) -> Vec<u64> {
        if packet_count == 0 {
            return Vec::new();
        }
        if self.mode == ScreenPacerMode::Disabled {
            self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
            return vec![0u64; packet_count];
        }

        let burst_limit = match self.mode {
            ScreenPacerMode::Auto => SCREEN_PACER_BURST_AUTO,
            ScreenPacerMode::Aggressive => SCREEN_PACER_BURST_AGGRESSIVE,
            ScreenPacerMode::Disabled => packet_count,
        };

        if packet_count <= burst_limit {
            self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
            return vec![0u64; packet_count];
        }

        // Frame duration in microseconds
        let fps = fps_target.clamp(5, 120) as u64;
        let frame_duration_us = 1_000_000u64 / fps;

        // Pacing budget: 80% of frame duration in Auto, 100% in Aggressive
        let budget_us = match self.mode {
            ScreenPacerMode::Auto => (frame_duration_us * 8) / 10,
            ScreenPacerMode::Aggressive => frame_duration_us,
            ScreenPacerMode::Disabled => 0,
        };

        let paced_shards = packet_count - burst_limit;
        let step_us = (budget_us / (paced_shards as u64 + 1)).min(SCREEN_PACER_MAX_DELAY_US);

        let mut delays = Vec::with_capacity(packet_count);
        let mut burst_delay_sum = 0u64;

        for i in 0..packet_count {
            if i < burst_limit {
                delays.push(0);
            } else {
                let delay = ((i - burst_limit + 1) as u64 * step_us).min(SCREEN_PACER_MAX_DELAY_US);
                delays.push(delay);
                burst_delay_sum = burst_delay_sum.saturating_add(delay);
                self.stats.last_delay_us = delay;
            }
        }

        self.stats.bursts_smoothed = self.stats.bursts_smoothed.saturating_add(1);
        self.stats.packets_paced = self.stats.packets_paced.saturating_add(packet_count as u64);
        self.stats.total_delay_us = self.stats.total_delay_us.saturating_add(burst_delay_sum);
        if let Some(avg) = self
            .stats
            .total_delay_us
            .checked_div(self.stats.packets_paced)
        {
            self.stats.avg_delay_us = avg;
        }

        delays
    }
}

/// Maximum sliding window capacity for delay trendline filter (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_WINDOW_MAX: usize = 60;
/// Default sliding window capacity for delay trendline filter (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_WINDOW_DEFAULT: usize = 20;
/// Default trendline slope threshold in microseconds per frame for Auto mode (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME: f64 = 12.5;
/// Aggressive trendline slope threshold in microseconds per frame (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME: f64 = 6.0;
/// Conservative trendline slope threshold in microseconds per frame (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME: f64 = 25.0;
/// Minimum samples required before computing linear regression slope (v2.06 / ADR-092).
pub const SCREEN_TRENDLINE_MIN_SAMPLES: usize = 5;

/// Operating modes for the delay-based trendline bufferbloat filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTrendlineMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Conservative = 2,
    Disabled = 3,
}

impl ScreenTrendlineMode {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenTrendlineMode::Aggressive,
            2 => ScreenTrendlineMode::Conservative,
            3 => ScreenTrendlineMode::Disabled,
            _ => ScreenTrendlineMode::Auto,
        }
    }
}

impl std::str::FromStr for ScreenTrendlineMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenTrendlineMode::Auto),
            "aggressive" | "agg" => Ok(ScreenTrendlineMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenTrendlineMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenTrendlineMode::Disabled),
            _ => Err(CoreError::Json(format!(
                "unknown screen trendline mode: {s}"
            ))),
        }
    }
}

impl std::fmt::Display for ScreenTrendlineMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenTrendlineMode::Auto => write!(f, "auto"),
            ScreenTrendlineMode::Aggressive => write!(f, "aggressive"),
            ScreenTrendlineMode::Conservative => write!(f, "conservative"),
            ScreenTrendlineMode::Disabled => write!(f, "disabled"),
        }
    }
}

/// Network bandwidth usage state estimated by the delay trendline filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenBandwidthUsageState {
    #[default]
    Normal = 0,
    Overusing = 1,
    Underusing = 2,
}

impl ScreenBandwidthUsageState {
    pub fn tag(&self) -> u8 {
        *self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenBandwidthUsageState::Overusing,
            2 => ScreenBandwidthUsageState::Underusing,
            _ => ScreenBandwidthUsageState::Normal,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ScreenBandwidthUsageState::Normal => "normal",
            ScreenBandwidthUsageState::Overusing => "overusing",
            ScreenBandwidthUsageState::Underusing => "underusing",
        }
    }
}

impl std::fmt::Display for ScreenBandwidthUsageState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScreenBandwidthUsageState::Normal => write!(f, "normal"),
            ScreenBandwidthUsageState::Overusing => write!(f, "overusing"),
            ScreenBandwidthUsageState::Underusing => write!(f, "underusing"),
        }
    }
}

/// Telemetry stats for the delay-based trendline filter (v2.06 / ADR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenTrendlineStats {
    pub mode: ScreenTrendlineMode,
    pub state: ScreenBandwidthUsageState,
    pub window_size: usize,
    pub samples_count: usize,
    pub current_slope_us_per_frame: f64,
    pub threshold_us_per_frame: f64,
    pub overuse_count: u64,
    pub underuse_count: u64,
    pub accumulated_delay_us: i64,
    pub last_update_ts_ms: u64,
}

impl Default for ScreenTrendlineStats {
    fn default() -> Self {
        Self {
            mode: ScreenTrendlineMode::Auto,
            state: ScreenBandwidthUsageState::Normal,
            window_size: SCREEN_TRENDLINE_WINDOW_DEFAULT,
            samples_count: 0,
            current_slope_us_per_frame: 0.0,
            threshold_us_per_frame: SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            overuse_count: 0,
            underuse_count: 0,
            accumulated_delay_us: 0,
            last_update_ts_ms: 0,
        }
    }
}

/// DTO report for REST API and CLI status formatting (v2.06 / ADR-092).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScreenTrendlineReport {
    pub ok: bool,
    pub mode: ScreenTrendlineMode,
    pub state: ScreenBandwidthUsageState,
    pub samples_count: usize,
    pub current_slope_us_per_frame: f64,
    pub threshold_us_per_frame: f64,
    pub overuse_count: u64,
    pub underuse_count: u64,
    pub accumulated_delay_us: i64,
    pub timestamp_ms: u64,
}

/// Delay-based trendline bufferbloat estimator utilizing linear regression across
/// a sliding window of inter-departure vs inter-arrival delays (v2.06 / ADR-092).
#[derive(Debug, Clone)]
pub struct ScreenTrendlineFilter {
    mode: ScreenTrendlineMode,
    state: ScreenBandwidthUsageState,
    window_capacity: usize,
    samples: VecDeque<(u64, i64)>,
    last_send_ts_us: u64,
    last_recv_ts_us: u64,
    accumulated_delay_us: i64,
    consecutive_overuse: usize,
    consecutive_underuse: usize,
    stats: ScreenTrendlineStats,
}

impl Default for ScreenTrendlineFilter {
    fn default() -> Self {
        Self::new(ScreenTrendlineMode::Auto)
    }
}

impl ScreenTrendlineFilter {
    pub fn new(mode: ScreenTrendlineMode) -> Self {
        let threshold = match mode {
            ScreenTrendlineMode::Auto => SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Disabled => 0.0,
        };
        Self {
            mode,
            state: ScreenBandwidthUsageState::Normal,
            window_capacity: SCREEN_TRENDLINE_WINDOW_DEFAULT,
            samples: VecDeque::with_capacity(SCREEN_TRENDLINE_WINDOW_DEFAULT),
            last_send_ts_us: 0,
            last_recv_ts_us: 0,
            accumulated_delay_us: 0,
            consecutive_overuse: 0,
            consecutive_underuse: 0,
            stats: ScreenTrendlineStats {
                mode,
                threshold_us_per_frame: threshold,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTrendlineMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTrendlineMode) {
        self.mode = mode;
        self.stats.mode = mode;
        self.stats.threshold_us_per_frame = match mode {
            ScreenTrendlineMode::Auto => SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Disabled => 0.0,
        };
        if mode == ScreenTrendlineMode::Disabled {
            self.state = ScreenBandwidthUsageState::Normal;
            self.stats.state = ScreenBandwidthUsageState::Normal;
        }
    }

    pub fn state(&self) -> ScreenBandwidthUsageState {
        self.state
    }

    pub fn stats(&self) -> ScreenTrendlineStats {
        let mut s = self.stats.clone();
        s.mode = self.mode;
        s.state = self.state;
        s.window_size = self.window_capacity;
        s.samples_count = self.samples.len();
        s.accumulated_delay_us = self.accumulated_delay_us;
        s
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.last_send_ts_us = 0;
        self.last_recv_ts_us = 0;
        self.accumulated_delay_us = 0;
        self.consecutive_overuse = 0;
        self.consecutive_underuse = 0;
        self.state = ScreenBandwidthUsageState::Normal;
        self.stats.current_slope_us_per_frame = 0.0;
        self.stats.state = ScreenBandwidthUsageState::Normal;
    }

    /// Process a new frame acknowledgment and update the linear regression trendline.
    pub fn on_ack(
        &mut self,
        sent_ts_us: u64,
        recv_ts_us: u64,
        rtt_p95_us: Option<u64>,
    ) -> ScreenBandwidthUsageState {
        if self.mode == ScreenTrendlineMode::Disabled {
            self.state = ScreenBandwidthUsageState::Normal;
            return self.state;
        }

        if self.last_send_ts_us > 0 && self.last_recv_ts_us > 0 && sent_ts_us > self.last_send_ts_us
        {
            let delta_send = sent_ts_us.saturating_sub(self.last_send_ts_us);
            let delta_recv = recv_ts_us.saturating_sub(self.last_recv_ts_us);
            let delay_variation = delta_recv as i64 - delta_send as i64;
            self.accumulated_delay_us = self.accumulated_delay_us.saturating_add(delay_variation);

            if self.samples.len() >= self.window_capacity {
                self.samples.pop_front();
            }
            self.samples
                .push_back((sent_ts_us, self.accumulated_delay_us));
        } else if self.samples.is_empty() {
            self.samples.push_back((sent_ts_us, 0));
        }

        self.last_send_ts_us = sent_ts_us;
        self.last_recv_ts_us = recv_ts_us;

        // Threshold evaluation
        let threshold = match self.mode {
            ScreenTrendlineMode::Disabled => 0.0,
            ScreenTrendlineMode::Aggressive => SCREEN_TRENDLINE_AGGRESSIVE_THRESHOLD_US_PER_FRAME,
            ScreenTrendlineMode::Conservative => {
                SCREEN_TRENDLINE_CONSERVATIVE_THRESHOLD_US_PER_FRAME
            }
            ScreenTrendlineMode::Auto => {
                if let Some(rtt) = rtt_p95_us {
                    let rtt_factor = (rtt as f64 / 100_000.0).clamp(0.5, 3.0);
                    SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME * rtt_factor
                } else {
                    SCREEN_TRENDLINE_DEFAULT_THRESHOLD_US_PER_FRAME
                }
            }
        };
        self.stats.threshold_us_per_frame = threshold;

        if self.samples.len() < SCREEN_TRENDLINE_MIN_SAMPLES {
            self.state = ScreenBandwidthUsageState::Normal;
            self.stats.state = self.state;
            return self.state;
        }

        // Ordinary Least Squares (OLS) linear regression
        let n = self.samples.len() as f64;
        let t0 = self.samples[0].0;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        for (s, d) in &self.samples {
            let x = s.saturating_sub(t0) as f64;
            let y = *d as f64;
            sum_x += x;
            sum_y += y;
        }
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;

        let mut cov_xy = 0.0;
        let mut var_x = 0.0;
        for (s, d) in &self.samples {
            let x = s.saturating_sub(t0) as f64;
            let y = *d as f64;
            cov_xy += (x - mean_x) * (y - mean_y);
            var_x += (x - mean_x) * (x - mean_x);
        }

        let slope_us_per_frame = if var_x > 1.0 {
            let raw_slope = cov_xy / var_x;
            let last_ts = self.samples.back().unwrap().0;
            let total_span_us = last_ts.saturating_sub(t0) as f64;
            let avg_frame_interval_us = total_span_us / (n - 1.0);
            raw_slope * avg_frame_interval_us
        } else {
            0.0
        };

        self.stats.current_slope_us_per_frame = slope_us_per_frame;

        if slope_us_per_frame > threshold {
            self.consecutive_overuse = self.consecutive_overuse.saturating_add(1);
            self.consecutive_underuse = 0;
            if self.consecutive_overuse >= 3 {
                self.state = ScreenBandwidthUsageState::Overusing;
                self.stats.overuse_count = self.stats.overuse_count.saturating_add(1);
            }
        } else if slope_us_per_frame < -threshold {
            self.consecutive_underuse = self.consecutive_underuse.saturating_add(1);
            self.consecutive_overuse = 0;
            if self.consecutive_underuse >= 3 {
                self.state = ScreenBandwidthUsageState::Underusing;
                self.stats.underuse_count = self.stats.underuse_count.saturating_add(1);
            }
        } else {
            self.consecutive_overuse = 0;
            self.consecutive_underuse = 0;
            self.state = ScreenBandwidthUsageState::Normal;
        }

        self.stats.state = self.state;
        self.state
    }
}

/// Sender side. Owns the monotonic sequence so a reconnect cannot replay old
/// frames, and so the controller can detect a gap.
pub struct ScreenSender {
    next_seq: u64,
    state: ScreenState,
    stats: ScreenStats,
    pending: VecDeque<ScreenFrame>,
    /// Track (seq, sent_time_us) of frames in-flight (max SCREEN_IN_FLIGHT_MAX).
    in_flight_records: VecDeque<(u64, u64)>,
    rtt_tracker: ScreenRttTracker,
    rate_controller: ScreenRateController,
    fps_pacer: ScreenFpsPacer,
    scale_controller: ScreenScaleController,
    quality_controller: ScreenQualityController,
    matrix_coordinator: ScreenMatrixCoordinator,
    cursor_controller: ScreenCursorController,
    squelch_controller: ScreenSquelchController,
    fec_controller: ScreenFecController,
    packet_pacer: ScreenPacketPacer,
    trendline_filter: ScreenTrendlineFilter,
}

impl Default for ScreenSender {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenSender {
    pub fn new() -> Self {
        let rate_controller = ScreenRateController::default();
        let fps_pacer = ScreenFpsPacer::default();
        let scale_controller = ScreenScaleController::default();
        let quality_controller = ScreenQualityController::default();
        let matrix_coordinator = ScreenMatrixCoordinator::default();
        let cursor_controller = ScreenCursorController::default();
        let squelch_controller = ScreenSquelchController::default();
        let fec_controller = ScreenFecController::default();
        let packet_pacer = ScreenPacketPacer::default();
        let trendline_filter = ScreenTrendlineFilter::default();
        let stats = ScreenStats {
            bitrate_bps: rate_controller.current_bps(),
            fps_target: fps_pacer.fps_target(),
            scale_pct: scale_controller.scale_pct(),
            profile_tag: quality_controller.profile().tag(),
            qp_min: quality_controller.min_qp(),
            qp_max: quality_controller.max_qp(),
            matrix_mode: matrix_coordinator.mode().tag(),
            network_health: matrix_coordinator.health().tag(),
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            squelch_mode: squelch_controller.mode().tag(),
            is_squelched: squelch_controller.is_squelched(),
            squelched_frames: 0,
            fec_mode: fec_controller.mode().tag(),
            fec_redundancy_pct: fec_controller.stats().redundancy_pct,
            fec_recovered_shards: 0,
            fec_unrecoverable_blocks: 0,
            pacer_mode: packet_pacer.mode().tag(),
            pacer_packets_paced: 0,
            pacer_bursts_smoothed: 0,
            pacer_avg_delay_us: 0,
            trendline_mode: trendline_filter.mode().tag(),
            trendline_state: trendline_filter.state().tag(),
            trendline_slope_us: 0.0,
            trendline_overuse_count: 0,
            ..Default::default()
        };
        Self {
            next_seq: 1,
            state: ScreenState::Idle,
            stats,
            pending: VecDeque::new(),
            in_flight_records: VecDeque::new(),
            rtt_tracker: ScreenRttTracker::new(SCREEN_IN_FLIGHT_MAX),
            rate_controller,
            fps_pacer,
            scale_controller,
            quality_controller,
            matrix_coordinator,
            cursor_controller,
            squelch_controller,
            fec_controller,
            packet_pacer,
            trendline_filter,
        }
    }

    pub fn with_trendline(trendline: ScreenTrendlineMode) -> Self {
        let mut sender = Self::new();
        sender.set_trendline_mode(trendline);
        sender
    }

    pub fn with_pacer(pacer: ScreenPacerMode) -> Self {
        let mut sender = Self::new();
        sender.set_pacer_mode(pacer);
        sender
    }

    pub fn with_fec(fec: ScreenFecMode) -> Self {
        let mut sender = Self::new();
        sender.set_fec_mode(fec);
        sender
    }

    pub fn with_fps(fps: u32) -> Self {
        let mut sender = Self::new();
        sender.set_fps(fps);
        sender
    }

    pub fn with_scale(scale_pct: u32) -> Self {
        let mut sender = Self::new();
        sender.set_scale_pct(scale_pct);
        sender
    }

    pub fn with_profile(profile: ScreenContentProfile) -> Self {
        let mut sender = Self::new();
        sender.set_content_profile(profile);
        sender
    }

    pub fn state(&self) -> ScreenState {
        self.state
    }

    pub fn stats(&self) -> ScreenStats {
        let mut s = self.stats;
        s.fec_mode = self.fec_controller.mode().tag();
        s.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;
        s.trendline_mode = self.trendline_filter.mode().tag();
        s.trendline_state = self.trendline_filter.state().tag();
        s.trendline_slope_us = self.trendline_filter.stats().current_slope_us_per_frame;
        s.trendline_overuse_count = self.trendline_filter.stats().overuse_count;
        s
    }

    pub fn trendline_mode(&self) -> ScreenTrendlineMode {
        self.trendline_filter.mode()
    }

    pub fn set_trendline_mode(&mut self, mode: ScreenTrendlineMode) {
        self.trendline_filter.set_mode(mode);
        self.stats.trendline_mode = mode.tag();
        self.stats.trendline_state = self.trendline_filter.state().tag();
    }

    pub fn trendline_state(&self) -> ScreenBandwidthUsageState {
        self.trendline_filter.state()
    }

    pub fn trendline_stats(&self) -> ScreenTrendlineStats {
        self.trendline_filter.stats()
    }

    pub fn trendline_report(&self) -> ScreenTrendlineReport {
        let stats = self.trendline_filter.stats();
        ScreenTrendlineReport {
            ok: true,
            mode: stats.mode,
            state: stats.state,
            samples_count: stats.samples_count,
            current_slope_us_per_frame: stats.current_slope_us_per_frame,
            threshold_us_per_frame: stats.threshold_us_per_frame,
            overuse_count: stats.overuse_count,
            underuse_count: stats.underuse_count,
            accumulated_delay_us: stats.accumulated_delay_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Whether the screen link is currently congested (v1.93 / ADR-080).
    /// Congestion is active when in-flight frames reach the queue threshold (>= 8),
    /// or when round-trip latency exceeds the 250ms interactive threshold while frames
    /// are in flight (pacing to 1 frame at a time to prevent bufferbloat without stalling).
    pub fn is_congested(&self) -> bool {
        self.stats.in_flight >= SCREEN_CONGESTION_IN_FLIGHT_THRESHOLD
            || (self.stats.in_flight > 0
                && self
                    .stats
                    .rtt_latest_us
                    .is_some_and(|rtt| rtt >= SCREEN_CONGESTION_RTT_THRESHOLD_US))
    }

    /// Current in-flight unacknowledged frames count.
    pub fn in_flight_count(&self) -> u64 {
        self.stats.in_flight
    }

    /// A trusted peer reached the channel. Idempotent while already past Idle.
    pub fn on_peer_connected(&mut self) {
        if self.state == ScreenState::Idle {
            self.state = ScreenState::Requested;
        }
    }

    /// Local consent. The ONLY transition into `Allowed`. A peer can never
    /// drive this.
    pub fn allow(&mut self) {
        if matches!(self.state, ScreenState::Requested | ScreenState::Allowed) {
            self.state = ScreenState::Allowed;
        }
    }

    pub fn revoke(&mut self) {
        self.state = match self.state {
            ScreenState::Idle => ScreenState::Idle,
            _ => ScreenState::Requested,
        };
        self.pending.clear();
        self.in_flight_records.clear();
        self.rtt_tracker.clear();
        self.stats.in_flight = 0;
        self.fps_pacer.reset();
    }

    /// Apply a controller instruction. Returns the reply to send back, if any.
    /// A `Start` before `allow` is counted as refused and does not stream.
    pub fn on_control(&mut self, msg: ControlMsg) -> Result<Option<ControlMsg>, CoreError> {
        self.on_control_with_time(msg, 0)
    }

    /// Apply a controller instruction with a monotonic clock timestamp in microseconds.
    pub fn on_control_with_time(
        &mut self,
        msg: ControlMsg,
        now_us: u64,
    ) -> Result<Option<ControlMsg>, CoreError> {
        match msg {
            ControlMsg::Start => {
                if !matches!(self.state, ScreenState::Allowed | ScreenState::Streaming) {
                    self.stats.refused = self.stats.refused.saturating_add(1);
                    return Ok(None);
                }
                self.state = ScreenState::Streaming;
                Ok(None)
            }
            ControlMsg::Stop => {
                self.state = match self.state {
                    ScreenState::Streaming => ScreenState::Allowed,
                    other => other,
                };
                self.pending.clear();
                self.in_flight_records.clear();
                self.stats.in_flight = 0;
                self.fps_pacer.reset();
                Ok(None)
            }
            ControlMsg::Ack { seq } => {
                if seq > self.stats.last_acked_seq {
                    self.stats.last_acked_seq = seq;
                }
                self.stats.acked = self.stats.acked.saturating_add(1);

                // Drain acknowledged frames up to `seq` and calculate RTT if timestamps exist
                let mut matched_rtt = None;
                while let Some(&(f_seq, sent_us)) = self.in_flight_records.front() {
                    if f_seq <= seq {
                        self.in_flight_records.pop_front();
                        if f_seq == seq && now_us >= sent_us && sent_us > 0 {
                            matched_rtt = Some(now_us - sent_us);
                        }
                    } else {
                        break;
                    }
                }
                if let Some(rtt) = matched_rtt {
                    self.rtt_tracker.record(rtt);
                    self.stats.rtt_latest_us = Some(rtt);
                    self.stats.rtt_p95_us = self.rtt_tracker.p95_us();

                    // ADR-092 Delay-Based Trendline Bufferbloat Filter
                    let sent_us = now_us.saturating_sub(rtt);
                    let usage =
                        self.trendline_filter
                            .on_ack(sent_us, now_us, self.stats.rtt_p95_us);
                    self.stats.trendline_mode = self.trendline_filter.mode().tag();
                    self.stats.trendline_state = usage.tag();
                    self.stats.trendline_slope_us =
                        self.trendline_filter.stats().current_slope_us_per_frame;
                    self.stats.trendline_overuse_count =
                        self.trendline_filter.stats().overuse_count;

                    if usage == ScreenBandwidthUsageState::Overusing {
                        self.rate_controller.on_congestion(now_us);
                        self.stats.bitrate_bps = self.rate_controller.current_bps();
                        self.matrix_coordinator.on_drop(
                            now_us,
                            self.stats.fps_target,
                            self.stats.scale_pct,
                        );
                        self.stats.network_health = self.matrix_coordinator.health().tag();
                    } else {
                        self.rate_controller.on_ack(
                            rtt,
                            self.in_flight_records.len() as u64,
                            now_us,
                        );
                        self.stats.bitrate_bps = self.rate_controller.current_bps();

                        let outcome = self.matrix_coordinator.on_ack(
                            now_us,
                            rtt,
                            self.in_flight_records.len() as u64,
                            self.stats.fps_target,
                            self.stats.scale_pct,
                            SCREEN_FPS_MAX,
                        );
                        if let Some(new_fps) = outcome.new_fps {
                            let actual_fps = self.fps_pacer.set_fps(new_fps);
                            self.stats.fps_target = actual_fps;
                        }
                        if let Some(new_scale) = outcome.new_scale_pct {
                            let actual_scale = self.scale_controller.set_scale_pct(new_scale);
                            self.stats.scale_pct = actual_scale;
                        }
                        self.stats.network_health = outcome.health.tag();
                    }
                }
                self.stats.in_flight = self.in_flight_records.len() as u64;
                Ok(None)
            }
            ControlMsg::KeyframeRequest => {
                self.stats.keyframes_requested = self.stats.keyframes_requested.saturating_add(1);
                self.squelch_controller.force_wake(now_us);
                self.stats.is_squelched = false;
                Ok(None)
            }
        }
    }

    /// Queue a frame while streaming. Returns the assigned sequence number.
    pub fn push(&mut self, frame: ScreenFrame) -> Option<u64> {
        self.push_with_time(frame, 0)
    }

    /// Queue a frame while streaming with a monotonic send timestamp in microseconds.
    pub fn push_with_time(&mut self, frame: ScreenFrame, now_us: u64) -> Option<u64> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.stats.sent = self.stats.sent.saturating_add(1);
        self.stats.bytes = self.stats.bytes.saturating_add(frame.bytes.len() as u64);

        if self.in_flight_records.len() >= SCREEN_IN_FLIGHT_MAX {
            self.in_flight_records.pop_front();
        }
        self.in_flight_records.push_back((seq, now_us));
        self.stats.in_flight = self.in_flight_records.len() as u64;

        self.pending.push_back(ScreenFrame { seq, ..frame });
        Some(seq)
    }

    /// Queue a frame while streaming evaluated against dirty region damage and adaptive squelch (v2.03 / ADR-089).
    /// If screen is static and in squelch window, frame encoding/transmission is suppressed (returns None).
    pub fn push_with_damage_time(
        &mut self,
        frame: ScreenFrame,
        is_dirty: bool,
        dirty_ratio: f32,
        now_us: u64,
    ) -> Option<u64> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let decision = self
            .squelch_controller
            .evaluate_decision(is_dirty, dirty_ratio, now_us);
        let squelch_stats = self.squelch_controller.stats();
        self.stats.squelch_mode = squelch_stats.mode.tag();
        self.stats.is_squelched = squelch_stats.is_squelched;
        self.stats.squelched_frames = squelch_stats.frames_squelched;

        match decision {
            ScreenSquelchDecision::SendFrame => self.push_with_time(frame, now_us),
            ScreenSquelchDecision::SquelchSkip => None,
        }
    }

    /// Record a frame dropped on sender side (e.g. due to congestion or backpressure).
    pub fn record_drop(&mut self) {
        self.record_drop_with_time(0);
    }

    /// Record a frame dropped on sender side with timestamp for rate adaptation cooldown.
    pub fn record_drop_with_time(&mut self, now_us: u64) {
        self.stats.dropped = self.stats.dropped.saturating_add(1);
        self.rate_controller.on_congestion(now_us);
        self.stats.bitrate_bps = self.rate_controller.current_bps();

        let outcome =
            self.matrix_coordinator
                .on_drop(now_us, self.stats.fps_target, self.stats.scale_pct);
        if let Some(new_fps) = outcome.new_fps {
            let actual_fps = self.fps_pacer.set_fps(new_fps);
            self.stats.fps_target = actual_fps;
        }
        if let Some(new_scale) = outcome.new_scale_pct {
            let actual_scale = self.scale_controller.set_scale_pct(new_scale);
            self.stats.scale_pct = actual_scale;
        }
        self.stats.network_health = outcome.health.tag();
    }

    /// Explicitly configure target bitrate bounds or manual override (v1.96 / ADR-082).
    pub fn set_bitrate_bps(&mut self, bps: u32) {
        self.rate_controller.set_manual_bps(bps);
        self.stats.bitrate_bps = self.rate_controller.current_bps();
    }

    /// Current target video bitrate in bits per second.
    pub fn bitrate_bps(&self) -> u32 {
        self.stats.bitrate_bps
    }

    /// Target streaming frame rate in frames per second (v1.97 / ADR-083).
    pub fn fps_target(&self) -> u32 {
        self.stats.fps_target
    }

    /// Explicitly configure target streaming frame rate in frames per second.
    pub fn set_fps(&mut self, fps: u32) -> u32 {
        let actual = self.fps_pacer.set_fps(fps);
        self.stats.fps_target = actual;
        actual
    }

    /// Access reference to the internal frame rate pacer.
    pub fn fps_pacer(&self) -> &ScreenFpsPacer {
        &self.fps_pacer
    }

    /// Access mutable reference to the internal frame rate pacer.
    pub fn fps_pacer_mut(&mut self) -> &mut ScreenFpsPacer {
        &mut self.fps_pacer
    }

    /// Target screen resolution scale percentage (v1.98 / ADR-084).
    pub fn scale_pct(&self) -> u32 {
        self.stats.scale_pct
    }

    /// Explicitly configure target screen resolution scale percentage (25..=100).
    pub fn set_scale_pct(&mut self, pct: u32) -> u32 {
        let actual = self.scale_controller.set_scale_pct(pct);
        self.stats.scale_pct = actual;
        actual
    }

    /// Access reference to the internal resolution scale controller.
    pub fn scale_controller(&self) -> &ScreenScaleController {
        &self.scale_controller
    }

    /// Access mutable reference to the internal resolution scale controller.
    pub fn scale_controller_mut(&mut self) -> &mut ScreenScaleController {
        &mut self.scale_controller
    }

    /// Active screen content profile (v1.99 / ADR-085).
    pub fn content_profile(&self) -> ScreenContentProfile {
        self.quality_controller.profile()
    }

    /// Explicitly configure target screen content profile.
    pub fn set_content_profile(&mut self, profile: ScreenContentProfile) -> ScreenContentProfile {
        let actual = self.quality_controller.set_profile(profile);
        self.stats.profile_tag = actual.tag();
        self.stats.qp_min = self.quality_controller.min_qp();
        self.stats.qp_max = self.quality_controller.max_qp();
        actual
    }

    /// Access quantization parameter range (min_qp, max_qp).
    pub fn qp_range(&self) -> (u8, u8) {
        self.quality_controller.qp_range()
    }

    /// Explicitly configure quantization parameter bounds.
    pub fn set_qp_range(&mut self, min_qp: u8, max_qp: u8) -> (u8, u8) {
        let range = self.quality_controller.set_qp_range(min_qp, max_qp);
        self.stats.qp_min = range.0;
        self.stats.qp_max = range.1;
        range
    }

    /// Access reference to the internal quality controller.
    pub fn quality_controller(&self) -> &ScreenQualityController {
        &self.quality_controller
    }

    /// Access mutable reference to the internal quality controller.
    pub fn quality_controller_mut(&mut self) -> &mut ScreenQualityController {
        &mut self.quality_controller
    }

    /// Operation mode of the 4D autonomous adaptation coordinator (v2.00 / ADR-086).
    pub fn matrix_mode(&self) -> ScreenMatrixMode {
        self.matrix_coordinator.mode()
    }

    /// Explicitly configure 4D autonomous adaptation coordinator mode.
    pub fn set_matrix_mode(&mut self, mode: ScreenMatrixMode) -> ScreenMatrixMode {
        self.matrix_coordinator.set_mode(mode);
        self.stats.matrix_mode = mode.tag();
        mode
    }

    /// Assessed real-time network health state.
    pub fn network_health(&self) -> NetworkHealth {
        self.matrix_coordinator.health()
    }

    /// Access reference to the internal 4D matrix coordinator.
    pub fn matrix_coordinator(&self) -> &ScreenMatrixCoordinator {
        &self.matrix_coordinator
    }

    /// Access mutable reference to the internal 4D matrix coordinator.
    pub fn matrix_coordinator_mut(&mut self) -> &mut ScreenMatrixCoordinator {
        &mut self.matrix_coordinator
    }

    /// Check if target frame pacing interval has elapsed.
    pub fn is_pacing_ready(&self, now_us: u64) -> bool {
        self.fps_pacer.is_ready(now_us)
    }

    /// Check and consume a pacing tick if target frame pacing interval has elapsed.
    pub fn consume_pacing(&mut self, now_us: u64) -> bool {
        self.fps_pacer.check_and_consume(now_us)
    }

    /// Hardware cursor decoupling mode (v2.02 / ADR-088).
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        self.cursor_controller.mode()
    }

    /// Explicitly configure hardware cursor decoupling mode.
    pub fn set_cursor_mode(&mut self, mode: ScreenCursorMode) -> ScreenCursorMode {
        self.cursor_controller.set_mode(mode);
        self.stats.cursor_mode = mode.tag();
        mode
    }

    /// Access reference to internal cursor controller.
    pub fn cursor_controller(&self) -> &ScreenCursorController {
        &self.cursor_controller
    }

    /// Access mutable reference to internal cursor controller.
    pub fn cursor_controller_mut(&mut self) -> &mut ScreenCursorController {
        &mut self.cursor_controller
    }

    /// Push high-frequency cursor update. Returns ScreenCursorUpdate if candidate passed dedup and rate-limiting.
    #[allow(clippy::too_many_arguments)]
    pub fn push_cursor_with_time(
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
        if self.state != ScreenState::Streaming {
            return None;
        }
        let update = self.cursor_controller.create_update(
            x,
            y,
            visible,
            shape,
            hotspot,
            dimensions,
            custom_rgba,
            now_us,
        )?;
        self.stats.cursor_x = x;
        self.stats.cursor_y = y;
        self.stats.cursor_visible = visible;
        self.stats.cursor_shape = shape.tag();
        self.stats.cursor_updates_sent = self.cursor_controller.stats().updates_sent;
        Some(update)
    }

    /// Access snapshot of cursor controller statistics.
    pub fn cursor_stats(&self) -> ScreenCursorStats {
        self.cursor_controller.stats()
    }

    /// Push high-frequency cursor update from existing ScreenCursorUpdate.
    pub fn push_cursor_update_with_time(
        &mut self,
        update: ScreenCursorUpdate,
        now_us: u64,
    ) -> Option<ScreenCursorUpdate> {
        self.push_cursor_with_time(
            update.x,
            update.y,
            update.visible,
            update.shape,
            (update.hotspot_x, update.hotspot_y),
            (update.width, update.height),
            update.custom_rgba,
            now_us,
        )
    }

    /// Screen streaming adaptive squelch mode (v2.03 / ADR-089).
    pub fn squelch_mode(&self) -> ScreenSquelchMode {
        self.squelch_controller.mode()
    }

    /// Explicitly configure screen streaming adaptive squelch mode.
    pub fn set_squelch_mode(&mut self, mode: ScreenSquelchMode) -> ScreenSquelchMode {
        let actual = self.squelch_controller.set_mode(mode);
        self.stats.squelch_mode = actual.tag();
        self.stats.is_squelched = self.squelch_controller.is_squelched();
        actual
    }

    /// Whether frame transmission is currently squelched due to static screen.
    pub fn is_squelched(&self) -> bool {
        self.squelch_controller.is_squelched()
    }

    /// Real-time squelch controller telemetry and statistics.
    pub fn squelch_stats(&self) -> ScreenSquelchStats {
        self.squelch_controller.stats()
    }

    /// Access reference to internal squelch controller.
    pub fn squelch_controller(&self) -> &ScreenSquelchController {
        &self.squelch_controller
    }

    /// Access mutable reference to internal squelch controller.
    pub fn squelch_controller_mut(&mut self) -> &mut ScreenSquelchController {
        &mut self.squelch_controller
    }

    /// Wake up squelch controller immediately (e.g. upon user input or client interaction).
    pub fn force_wake_squelch(&mut self, now_us: u64) {
        self.squelch_controller.force_wake(now_us);
        self.stats.is_squelched = false;
    }

    /// Screen forward error correction (FEC) mode (v2.04 / ADR-090).
    pub fn fec_mode(&self) -> ScreenFecMode {
        self.fec_controller.mode()
    }

    /// Explicitly configure screen forward error correction (FEC) mode.
    pub fn set_fec_mode(&mut self, mode: ScreenFecMode) -> ScreenFecMode {
        let actual = self.fec_controller.set_mode(mode);
        self.stats.fec_mode = actual.tag();
        self.stats.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;
        actual
    }

    /// Access reference to internal FEC controller.
    pub fn fec_controller(&self) -> &ScreenFecController {
        &self.fec_controller
    }

    /// Access mutable reference to internal FEC controller.
    pub fn fec_controller_mut(&mut self) -> &mut ScreenFecController {
        &mut self.fec_controller
    }

    /// Real-time FEC controller telemetry and statistics.
    pub fn fec_stats(&self) -> ScreenFecStats {
        self.fec_controller.stats()
    }

    /// Screen packet pacing mode (v2.05 / ADR-091).
    pub fn pacer_mode(&self) -> ScreenPacerMode {
        self.packet_pacer.mode()
    }

    /// Explicitly configure screen packet pacing mode.
    pub fn set_pacer_mode(&mut self, mode: ScreenPacerMode) -> ScreenPacerMode {
        let actual = self.packet_pacer.set_mode(mode);
        self.stats.pacer_mode = actual.tag();
        actual
    }

    /// Access reference to internal packet pacer.
    pub fn packet_pacer(&self) -> &ScreenPacketPacer {
        &self.packet_pacer
    }

    /// Access mutable reference to internal packet pacer.
    pub fn packet_pacer_mut(&mut self) -> &mut ScreenPacketPacer {
        &mut self.packet_pacer
    }

    /// Real-time packet pacer statistics.
    pub fn pacer_stats(&self) -> ScreenPacerStats {
        self.packet_pacer.stats()
    }

    /// Computes packet departure delay offsets (in microseconds) for smoothing bursts.
    pub fn compute_packet_delays(&mut self, packet_count: usize) -> Vec<u64> {
        let delays = self
            .packet_pacer
            .compute_packet_delays(packet_count, self.fps_pacer.fps_target());
        let st = self.packet_pacer.stats();
        self.stats.pacer_packets_paced = st.packets_paced;
        self.stats.pacer_bursts_smoothed = st.bursts_smoothed;
        self.stats.pacer_avg_delay_us = st.avg_delay_us;
        delays
    }

    /// Convert a ScreenFrame into outbound InnerFrames.
    /// If FEC mode is active (redundancy > 0), encodes payload into Cauchy RS shards (TYPE_SCREEN_FEC).
    /// Otherwise, falls back to standard fragmentation (TYPE_SCREEN_FRAME).
    pub fn encode_frame_to_inner_frames(
        &mut self,
        frame: &ScreenFrame,
        packet_loss_ratio: f32,
    ) -> Result<Vec<InnerFrame>, CoreError> {
        let payload = frame.encode_payload()?;
        let m_parity = self
            .fec_controller
            .calculate_parity_shards(1, packet_loss_ratio);
        if m_parity == 0 {
            return frame.to_frames();
        }

        // Split payload into shards that comfortably fit within INNER_PAYLOAD_MAX
        let shard_chunk_max = INNER_PAYLOAD_MAX - FEC_HEADER_LEN - 32;
        let k = payload.len().div_ceil(shard_chunk_max).max(1);
        if k > FEC_MAX_DATA_SHARDS {
            // If too large for single FEC block, fallback to standard fragmentation
            return frame.to_frames();
        }

        let mut data_buffers = Vec::with_capacity(k);
        for chunk in payload.chunks(shard_chunk_max) {
            data_buffers.push(chunk.to_vec());
        }

        let m = self
            .fec_controller
            .calculate_parity_shards(k, packet_loss_ratio);
        if m == 0 {
            return frame.to_frames();
        }

        let shards = ScreenFecEncoder::encode_block(frame.seq, 0, &data_buffers, m)?;
        self.fec_controller.record_block_encoded(k, m);
        self.stats.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;

        Ok(shards.iter().map(|s| s.to_inner_frame()).collect())
    }

    pub fn drain(&mut self) -> Vec<ScreenFrame> {
        self.pending.drain(..).collect()
    }

    /// Peer gone: back to Idle, sequence reset so a reconnect starts clean and
    /// cannot replay.
    pub fn on_peer_gone(&mut self) {
        self.state = ScreenState::Idle;
        self.next_seq = 1;
        self.pending.clear();
        self.in_flight_records.clear();
        self.rtt_tracker.clear();
        self.stats.in_flight = 0;
        self.fps_pacer.reset();
        self.cursor_controller.reset();
        self.squelch_controller = ScreenSquelchController::new(self.squelch_controller.mode());
        self.fec_controller = ScreenFecController::new(self.fec_controller.mode());
        self.packet_pacer = ScreenPacketPacer::new(self.packet_pacer.mode());
        self.stats.is_squelched = false;
        self.stats.squelched_frames = 0;
    }
}

/// Receiver side. Enforces the sequence window and never presents a frame it
/// could not fully validate.
pub struct ScreenReceiver {
    last_seq: u64,
    stats: ScreenStats,
    trace: VecDeque<u64>,
    playout_buffer: ScreenPlayoutBuffer,
    cursor_controller: ScreenCursorController,
    last_cursor: Option<ScreenCursorUpdate>,
    fec_assembler: ScreenFecAssembler,
}

impl Default for ScreenReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenReceiver {
    pub fn new() -> Self {
        let cursor_controller = ScreenCursorController::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            ..Default::default()
        };
        Self {
            last_seq: 0,
            stats,
            trace: VecDeque::with_capacity(SCREEN_TRACE_MAX),
            playout_buffer: ScreenPlayoutBuffer::default(),
            cursor_controller,
            last_cursor: None,
            fec_assembler: ScreenFecAssembler::new(),
        }
    }

    pub fn with_playout_mode(mode: ScreenPlayoutMode) -> Self {
        let cursor_controller = ScreenCursorController::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            ..Default::default()
        };
        Self {
            last_seq: 0,
            stats,
            trace: VecDeque::with_capacity(SCREEN_TRACE_MAX),
            playout_buffer: ScreenPlayoutBuffer::new(mode),
            cursor_controller,
            last_cursor: None,
            fec_assembler: ScreenFecAssembler::new(),
        }
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn stats(&self) -> ScreenStats {
        let mut st = self.stats;
        let pstats = self.playout_buffer.stats();
        st.playout_mode = pstats.mode as u8;
        st.playout_delay_us = pstats.target_delay_us.min(u32::MAX as u64) as u32;
        st.playout_jitter_us = pstats.current_jitter_us.min(u32::MAX as u64) as u32;
        let cstats = self.cursor_controller.stats();
        st.cursor_mode = cstats.mode.tag();
        st.cursor_updates_sent = cstats.updates_sent;
        st.cursor_updates_received = self.stats.cursor_updates_received;
        if let Some(ref cur) = self.last_cursor {
            st.cursor_x = cur.x;
            st.cursor_y = cur.y;
            st.cursor_visible = cur.visible;
            st.cursor_shape = cur.shape.tag();
        }
        st.fec_recovered_shards = self.fec_assembler.recovered_shards();
        st.fec_unrecoverable_blocks = self.fec_assembler.unrecoverable_blocks();
        st
    }

    /// Access playout buffer reference.
    pub fn playout_buffer(&self) -> &ScreenPlayoutBuffer {
        &self.playout_buffer
    }

    /// Access mutable playout buffer reference.
    pub fn playout_buffer_mut(&mut self) -> &mut ScreenPlayoutBuffer {
        &mut self.playout_buffer
    }

    /// Current playout mode.
    pub fn playout_mode(&self) -> ScreenPlayoutMode {
        self.playout_buffer.mode()
    }

    /// Set playout mode.
    pub fn set_playout_mode(&mut self, mode: ScreenPlayoutMode) {
        self.playout_buffer.set_mode(mode);
    }

    /// Playout buffer stats and telemetry.
    pub fn playout_stats(&self) -> ScreenPlayoutStats {
        self.playout_buffer.stats()
    }

    /// Push an accepted frame to the playout buffer with its arrival timestamp (microseconds).
    pub fn push_playout(&mut self, frame: ScreenFrame, arrival_us: u64) {
        self.playout_buffer.push_frame(frame, arrival_us);
    }

    /// Pop due frame for presentation at current clock timestamp `now_us`.
    pub fn pop_playout(&mut self, now_us: u64) -> ScreenPlayoutAction {
        self.playout_buffer.pop_frame(now_us)
    }

    /// Hardware cursor decoupling mode (v2.02 / ADR-088).
    pub fn cursor_mode(&self) -> ScreenCursorMode {
        self.cursor_controller.mode()
    }

    /// Set hardware cursor decoupling mode.
    pub fn set_cursor_mode(&mut self, mode: ScreenCursorMode) -> ScreenCursorMode {
        self.cursor_controller.set_mode(mode);
        self.stats.cursor_mode = mode.tag();
        mode
    }

    /// Handle incoming high-frequency cursor update frame.
    pub fn on_cursor(&mut self, update: ScreenCursorUpdate) {
        self.stats.cursor_x = update.x;
        self.stats.cursor_y = update.y;
        self.stats.cursor_visible = update.visible;
        self.stats.cursor_shape = update.shape.tag();
        self.stats.cursor_updates_received = self.stats.cursor_updates_received.saturating_add(1);
        self.last_cursor = Some(update);
    }

    /// Current cursor state as received from host.
    pub fn current_cursor(&self) -> Option<&ScreenCursorUpdate> {
        self.last_cursor.as_ref()
    }

    /// Receiver cursor telemetry statistics.
    pub fn cursor_stats(&self) -> ScreenCursorStats {
        let mut st = self.cursor_controller.stats();
        st.updates_received = self.stats.cursor_updates_received;
        if let Some(ref cur) = self.last_cursor {
            st.current_x = cur.x;
            st.current_y = cur.y;
            st.current_visible = cur.visible;
            st.current_shape = cur.shape;
        }
        st
    }

    /// Access reference to internal FEC assembler (v2.04 / ADR-090).
    pub fn fec_assembler(&self) -> &ScreenFecAssembler {
        &self.fec_assembler
    }

    /// Access mutable reference to internal FEC assembler (v2.04 / ADR-090).
    pub fn fec_assembler_mut(&mut self) -> &mut ScreenFecAssembler {
        &mut self.fec_assembler
    }

    /// Handle incoming forward error correction (FEC) shard packet (v2.04 / ADR-090).
    /// If shard satisfies K data shards or reconstruction threshold, assembles and returns ScreenFrame.
    pub fn on_fec_shard(
        &mut self,
        shard: ScreenFecShard,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        let res = self.fec_assembler.push_shard(shard);
        self.stats.fec_recovered_shards = self.fec_assembler.recovered_shards();
        self.stats.fec_unrecoverable_blocks = self.fec_assembler.unrecoverable_blocks();
        res
    }

    /// Most recently accepted sequence numbers, oldest first.
    pub fn trace(&self) -> Vec<u64> {
        self.trace.iter().copied().collect()
    }

    /// Validate and accept one frame. Duplicates and stale frames are dropped
    /// (counted), a forward jump beyond the window is an error, and a valid
    /// frame is recorded. `Ok(None)` means "not for presentation".
    pub fn apply(&mut self, frame: &ScreenFrame) -> Result<Option<u64>, CoreError> {
        if frame.seq <= self.last_seq {
            self.stats.dropped = self.stats.dropped.saturating_add(1);
            return Ok(None);
        }
        if self.last_seq != 0 && frame.seq > self.last_seq.saturating_add(SCREEN_SEQ_WINDOW) {
            return Err(CoreError::ScreenSeqJump);
        }
        if self.last_seq != 0 && frame.seq > self.last_seq.saturating_add(1) {
            self.stats.gaps = self.stats.gaps.saturating_add(1);
        }
        // A raw frame must match its rect; the codec already enforced this on
        // the wire, but a locally built frame could still lie.
        if !frame.compressed() && frame.bytes.len() != frame.raw_len() {
            return Err(CoreError::ScreenFrame);
        }
        self.last_seq = frame.seq;
        self.stats.received = self.stats.received.saturating_add(1);
        self.stats.bytes = self.stats.bytes.saturating_add(frame.bytes.len() as u64);
        if self.trace.len() >= SCREEN_TRACE_MAX {
            self.trace.pop_front();
        }
        self.trace.push_back(frame.seq);
        Ok(Some(frame.seq))
    }

    /// Peer gone: clear the cursor so a reconnect cannot be mistaken for a
    /// continuation of the old stream.
    pub fn on_peer_gone(&mut self) {
        self.last_seq = 0;
        self.trace.clear();
        self.playout_buffer.reset();
        self.last_cursor = None;
        self.cursor_controller.reset();
        self.fec_assembler.reset();
    }
}

/// Reassembles `TMF1` fragment streams back into one `ScreenFrame`. A whole
/// `TMV1` payload (small frame) passes through unchanged. Fragments that
/// disagree with the current stream (a different seq, index, or shape) reset
/// the buffer — the wire is a live stream and a stale fragment never pollutes
/// a newer frame. Bounded: the assembled body is capped at
/// `SCREEN_PAYLOAD_MAX`, so a hostile peer cannot grow memory without limit.
#[derive(Debug, Default)]
pub struct ScreenAssembler {
    seq: u64,
    total: usize,
    count: usize,
    /// Reassembly body, allocated once per stream and filled in place at each
    /// fragment's offset — one buffer instead of per-chunk Vecs plus a final
    /// concatenation copy.
    body: Vec<u8>,
    mask: Vec<bool>,
    accepted: usize,
}

impl ScreenAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one inner frame. Returns the complete frame when the last fragment
    /// of a stream arrives (or immediately for a whole small frame).
    pub fn push(&mut self, frame: &InnerFrame) -> Result<Option<ScreenFrame>, CoreError> {
        if frame.ty != TYPE_SCREEN_FRAME {
            return Err(CoreError::ScreenFrame);
        }
        if frame.payload.starts_with(&SCREEN_MAGIC) {
            // A whole frame supersedes any half-assembled fragment stream; drop
            // the stale fragments so a later fragment cannot resurrect them.
            self.reset();
            return ScreenFrame::from_frame(frame).map(Some);
        }
        if !frame.payload.starts_with(&SCREEN_FRAG_MAGIC) {
            return Err(CoreError::ScreenFragment);
        }
        let p = &frame.payload;
        if p.len() < SCREEN_FRAG_HEADER {
            return Err(CoreError::ScreenFragment);
        }
        let total = u32::from_be_bytes(p[4..8].try_into().unwrap()) as usize;
        let idx = u32::from_be_bytes(p[8..12].try_into().unwrap()) as usize;
        let count = u32::from_be_bytes(p[12..16].try_into().unwrap()) as usize;
        let seq = u64::from_be_bytes(p[16..24].try_into().unwrap());
        let chunk = &p[SCREEN_FRAG_HEADER..];

        // Every fragment must be under the inner budget and the whole stream
        // under SCREEN_PAYLOAD_MAX; a lying count is refused, not allocated.
        // `count` must also equal the chunk count implied by `total`, or a
        // short `count` would let the stream "complete" with `body` unfilled.
        let expected_count = total.div_ceil(SCREEN_FRAG_CHUNK_MAX);
        if p.len() > INNER_PAYLOAD_MAX
            || total > SCREEN_PAYLOAD_MAX
            || total == 0
            || count != expected_count
            || idx >= count
        {
            return Err(CoreError::ScreenFragment);
        }

        // A new stream supersedes an incomplete older one.
        if seq != self.seq && !self.mask.is_empty() {
            self.reset();
        }
        if self.mask.is_empty() || self.total != total || self.count != count {
            self.seq = seq;
            self.total = total;
            self.count = count;
            self.body = vec![0u8; total];
            self.mask = vec![false; count];
            self.accepted = 0;
        }
        // `idx` indexes a chunk of a `count`-chunk stream, but a lying sender
        // can still point an index past `total`; compute the offset with a
        // checked multiply so neither step can overflow or underflow.
        let Some(off) = idx.checked_mul(SCREEN_FRAG_CHUNK_MAX) else {
            return Err(CoreError::ScreenFragment);
        };
        if off >= self.total {
            return Err(CoreError::ScreenFragment);
        }
        let expected = (self.total - off).min(SCREEN_FRAG_CHUNK_MAX);
        if chunk.len() != expected {
            return Err(CoreError::ScreenFragment);
        }
        if !self.mask[idx] {
            self.body[off..off + chunk.len()].copy_from_slice(chunk);
            self.mask[idx] = true;
            self.accepted += 1;
        }
        if self.accepted < self.count {
            return Ok(None);
        }
        let frame = ScreenFrame::decode_payload(&self.body)?;
        self.reset();
        Ok(Some(frame))
    }

    fn reset(&mut self) {
        self.seq = 0;
        self.total = 0;
        self.count = 0;
        self.body = Vec::new();
        self.mask = Vec::new();
        self.accepted = 0;
    }
}

/// In-memory capture source for tests and CI. Deterministic: the same `grab`
/// calls always yield the same pixels, so latency and sequence assertions are
/// reproducible without a display.
#[derive(Debug)]
pub struct MemoryScreenSource {
    rect: ScreenRect,
    format: PixelFormat,
    seq: std::sync::atomic::AtomicU64,
    /// Optional per-grab failure injection (index of the grab call, 1-based).
    fail_at: std::sync::atomic::AtomicU64,
}

impl MemoryScreenSource {
    pub fn new(w: u16, h: u16) -> Result<Self, CoreError> {
        Ok(Self {
            rect: ScreenRect::new(0, 0, w, h)?,
            format: PixelFormat::Bgra8,
            seq: std::sync::atomic::AtomicU64::new(1),
            fail_at: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Make the `n`-th grab (1-based) return an error once.
    pub fn fail_on_grab(&self, n: u64) {
        self.fail_at.store(n, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn rect(&self) -> ScreenRect {
        self.rect
    }
}

impl crate::ports::ScreenSource for MemoryScreenSource {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        let seq = self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail_at.load(std::sync::atomic::Ordering::SeqCst) == seq {
            return Err(CoreError::ScreenRefused("injected grab failure".into()));
        }
        let mut bytes = vec![0u8; self.rect.pixel_bytes(self.format)];
        // A cheap deterministic ramp so frames differ from one another and a
        // decompressor cannot trivially collapse them all to one value.
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i.wrapping_add(seq as usize) & 0xFF) as u8;
        }
        Ok(Some(ScreenFrame::raw(seq, self.rect, self.format, bytes)?))
    }
}

/// In-memory presentation sink for tests. Records the frames it was asked to
/// present, bounded so a long stream cannot grow the trace without limit.
/// Clone shares the same inner state (backed by `Arc<Mutex<_>>`).
#[derive(Debug, Clone, Default)]
pub struct MemoryScreenSink {
    inner: std::sync::Arc<std::sync::Mutex<MemoryScreenSinkInner>>,
}

#[derive(Debug, Default)]
struct MemoryScreenSinkInner {
    presented: VecDeque<u64>,
    last: Option<ScreenFrame>,
    fail: bool,
}

impl MemoryScreenSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Make every subsequent `present` fail, to prove a caller surfaces the
    /// error instead of swallowing it.
    pub fn set_fail(&self, fail: bool) {
        self.inner.lock().expect("sink").fail = fail;
    }

    pub fn presented(&self) -> Vec<u64> {
        self.inner
            .lock()
            .expect("sink")
            .presented
            .iter()
            .copied()
            .collect()
    }

    pub fn count(&self) -> usize {
        self.inner.lock().expect("sink").presented.len()
    }

    pub fn last(&self) -> Option<ScreenFrame> {
        self.inner.lock().expect("sink").last.clone()
    }
}

impl crate::ports::ScreenSink for MemoryScreenSink {
    fn present(&self, frame: &ScreenFrame) -> Result<(), CoreError> {
        let mut g = self.inner.lock().expect("sink");
        if g.fail {
            return Err(CoreError::ScreenRefused("injected present failure".into()));
        }
        if g.presented.len() >= SCREEN_TRACE_MAX {
            g.presented.pop_front();
        }
        g.presented.push_back(frame.seq);
        g.last = Some(frame.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ScreenSink, ScreenSource};

    fn rect() -> ScreenRect {
        ScreenRect::new(0, 0, 4, 2).unwrap()
    }

    fn raw_frame(seq: u64) -> ScreenFrame {
        ScreenFrame::raw(seq, rect(), PixelFormat::Bgra8, vec![7u8; 32]).unwrap()
    }

    #[test]
    fn rect_rejects_zero_dimensions() {
        assert!(ScreenRect::new(0, 0, 0, 10).is_err());
        assert!(ScreenRect::new(0, 0, 10, 0).is_err());
        assert_eq!(rect().pixel_bytes(PixelFormat::Bgra8), 32);
    }

    #[test]
    fn raw_frame_length_must_match_rect() {
        assert!(ScreenFrame::raw(1, rect(), PixelFormat::Bgra8, vec![0u8; 31]).is_err());
        assert!(ScreenFrame::raw(1, rect(), PixelFormat::Bgra8, vec![0u8; 32]).is_ok());
    }

    #[test]
    fn frame_round_trips_through_the_wire() {
        let f = raw_frame(42);
        let wire = f.to_frame().unwrap();
        assert_eq!(wire.ty, TYPE_SCREEN_FRAME);
        let back = ScreenFrame::from_frame(&wire).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn decode_rejects_a_short_header_and_a_bad_magic() {
        assert_eq!(
            ScreenFrame::decode_payload(&[0u8; 10]).unwrap_err(),
            CoreError::TruncatedFrame
        );
        let mut payload = raw_frame(1).encode_payload().unwrap();
        payload[0] = b'X';
        assert_eq!(
            ScreenFrame::decode_payload(&payload).unwrap_err(),
            CoreError::ScreenFrame
        );
    }

    #[test]
    fn decode_rejects_a_raw_body_that_does_not_match_the_rect() {
        let mut payload = raw_frame(1).encode_payload().unwrap();
        payload.push(0);
        assert_eq!(
            ScreenFrame::decode_payload(&payload).unwrap_err(),
            CoreError::ScreenFrame
        );
    }

    #[test]
    fn decode_rejects_an_unknown_compression_tag() {
        let mut payload = raw_frame(1).encode_payload().unwrap();
        payload[26] = 9;
        assert_eq!(
            ScreenFrame::decode_payload(&payload).unwrap_err(),
            CoreError::ScreenFrame
        );
    }

    #[test]
    fn control_round_trips_and_rejects_a_short_body() {
        for msg in [
            ControlMsg::Start,
            ControlMsg::Stop,
            ControlMsg::Ack { seq: 9 },
            ControlMsg::KeyframeRequest,
        ] {
            let wire = msg.to_frame().unwrap();
            assert_eq!(ControlMsg::from_frame(&wire).unwrap(), msg);
        }
        assert_eq!(
            ControlMsg::decode_payload(&[0u8; 5]).unwrap_err(),
            CoreError::TruncatedFrame
        );
    }

    #[test]
    fn start_is_refused_until_the_local_machine_allows() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        assert_eq!(s.state(), ScreenState::Requested);
        // Arriving at the channel is not consent.
        assert_eq!(s.on_control(ControlMsg::Start).unwrap(), None);
        assert_eq!(s.state(), ScreenState::Requested);
        assert_eq!(s.stats().refused, 1);
        // A push while un-allowed is dropped, not queued.
        assert!(s.push(raw_frame(1)).is_none());
        assert!(s.drain().is_empty());

        s.allow();
        assert_eq!(s.state(), ScreenState::Allowed);
        s.on_control(ControlMsg::Start).unwrap();
        assert_eq!(s.state(), ScreenState::Streaming);
        assert_eq!(s.push(raw_frame(1)), Some(1));
        assert_eq!(s.drain().len(), 1);
    }

    #[test]
    fn revoke_stops_streaming_and_clears_the_queue() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();
        s.push(raw_frame(1));
        s.revoke();
        assert_eq!(s.state(), ScreenState::Requested);
        assert!(s.drain().is_empty());
        // After a revoke, Start is refused again.
        s.on_control(ControlMsg::Start).unwrap();
        assert_eq!(s.state(), ScreenState::Requested);
        assert_eq!(s.stats().refused, 1);
    }

    #[test]
    fn leaving_the_channel_resets_the_sequence() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();
        assert_eq!(s.push(raw_frame(1)), Some(1));
        s.on_peer_gone();
        assert_eq!(s.state(), ScreenState::Idle);
        assert_eq!(s.next_seq(), 1);
        // A late frame from the dead peer is dropped.
        assert!(s.push(raw_frame(2)).is_none());
    }

    #[test]
    fn receiver_drops_stale_and_counts_the_drop() {
        let mut r = ScreenReceiver::new();
        assert_eq!(r.apply(&raw_frame(1)).unwrap(), Some(1));
        assert_eq!(r.apply(&raw_frame(1)).unwrap(), None);
        assert_eq!(r.apply(&raw_frame(0)).unwrap(), None);
        assert_eq!(r.stats().dropped, 2);
        assert_eq!(r.stats().received, 1);
        assert_eq!(r.last_seq(), 1);
    }

    #[test]
    fn receiver_refuses_a_forward_jump_beyond_the_window() {
        let mut r = ScreenReceiver::new();
        r.apply(&raw_frame(1)).unwrap();
        let far = SCREEN_SEQ_WINDOW + 10;
        assert_eq!(
            r.apply(&raw_frame(far)).unwrap_err(),
            CoreError::ScreenSeqJump
        );
        // The stride is untouched by the refusal.
        assert_eq!(r.last_seq(), 1);
    }

    #[test]
    fn receiver_forgets_the_cursor_on_peer_gone() {
        let mut r = ScreenReceiver::new();
        r.apply(&raw_frame(5)).unwrap();
        r.on_peer_gone();
        assert_eq!(r.last_seq(), 0);
        assert!(r.trace().is_empty());
        // seq 1 is a fresh start again, not a stale drop.
        assert_eq!(r.apply(&raw_frame(1)).unwrap(), Some(1));
    }

    #[test]
    fn receiver_trace_is_bounded() {
        let mut r = ScreenReceiver::new();
        for seq in 1..=(SCREEN_TRACE_MAX as u64 + 20) {
            r.apply(&raw_frame(seq)).unwrap();
        }
        assert_eq!(r.trace().len(), SCREEN_TRACE_MAX);
        assert_eq!(*r.trace().last().unwrap(), SCREEN_TRACE_MAX as u64 + 20);
    }

    #[test]
    fn memory_source_is_deterministic_and_can_inject_a_failure() {
        let a = MemoryScreenSource::new(4, 2).unwrap();
        let b = MemoryScreenSource::new(4, 2).unwrap();
        assert_eq!(a.grab().unwrap(), b.grab().unwrap());
        assert_eq!(a.grab().unwrap().unwrap().seq, 2);

        let src = MemoryScreenSource::new(4, 2).unwrap();
        src.fail_on_grab(2);
        assert!(src.grab().is_ok());
        assert!(src.grab().is_err());
        assert!(src.grab().is_ok());
    }

    #[test]
    fn memory_sink_records_and_surfaces_a_failure() {
        let sink = MemoryScreenSink::new();
        sink.present(&raw_frame(1)).unwrap();
        sink.present(&raw_frame(2)).unwrap();
        assert_eq!(sink.presented(), vec![1, 2]);
        assert_eq!(sink.last().unwrap().seq, 2);

        sink.set_fail(true);
        assert!(sink.present(&raw_frame(3)).is_err());
        assert_eq!(sink.count(), 2);
    }

    #[test]
    fn pixel_format_swap_round_trips() {
        let mut px = vec![1u8, 2, 3, 4];
        PixelFormat::Bgra8.converted(PixelFormat::Rgba8, &mut px);
        assert_eq!(px, vec![3u8, 2, 1, 4]);
        PixelFormat::Rgba8.converted(PixelFormat::Bgra8, &mut px);
        assert_eq!(px, vec![1u8, 2, 3, 4]);
    }

    #[test]
    fn effective_stride_defaults_to_packed() {
        let f = raw_frame(1);
        assert_eq!(f.stride, 0);
        assert_eq!(f.effective_stride(), 4 * 4);
        assert_eq!(f.raw_len(), 32);
    }

    #[test]
    fn screen_types_are_recognized_for_the_must_understand_exemption() {
        assert!(is_screen_type(TYPE_SCREEN_FRAME));
        assert!(is_screen_type(TYPE_SCREEN_CONTROL));
        assert!(!is_screen_type(0x0999));
    }

    #[test]
    fn small_frame_takes_the_whole_frame_fast_path() {
        let f = raw_frame(7);
        let frames = f.to_frames().unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].payload.starts_with(&SCREEN_MAGIC));
        let mut a = ScreenAssembler::new();
        let out = a.push(&frames[0]).unwrap();
        assert_eq!(out, Some(f));
    }

    #[test]
    fn large_frame_fragments_and_reassembles_round_trip() {
        // 4 MiB raw BGRA body — far past the 64 KiB inner budget.
        let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
        let bytes = vec![0xABu8; rect.pixel_bytes(PixelFormat::Bgra8)];
        let f = ScreenFrame::raw(9, rect, PixelFormat::Bgra8, bytes).unwrap();
        let frames = f.to_frames().unwrap();
        assert!(frames.len() > 2, "must be fragmented, got {}", frames.len());
        for fr in &frames {
            assert!(fr.payload.starts_with(&SCREEN_FRAG_MAGIC));
            assert!(fr.payload.len() <= INNER_PAYLOAD_MAX);
        }
        let mut a = ScreenAssembler::new();
        let mut out = None;
        for fr in &frames {
            out = a.push(fr).unwrap();
        }
        let out = out.expect("last fragment yields the frame");
        assert_eq!(out, f);
        assert_eq!(out.bytes.len(), rect.pixel_bytes(PixelFormat::Bgra8));
    }

    #[test]
    fn assembler_rejects_a_lying_fragment_stream() {
        let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
        let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 4 * 1024 * 1024]).unwrap();
        let frames = f.to_frames().unwrap();
        assert!(frames.len() > 1, "must be fragmented");
        // Corrupt the first fragment: total_len far above the cap.
        let mut bad = frames[0].payload.clone();
        bad[4..8].copy_from_slice(&(SCREEN_PAYLOAD_MAX as u32 + 1000).to_be_bytes());
        let mut a = ScreenAssembler::new();
        assert_eq!(
            a.push(&InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload: bad,
            })
            .unwrap_err(),
            CoreError::ScreenFragment
        );
        // A non-TMV1 non-TMF1 payload is refused too.
        let mut a = ScreenAssembler::new();
        assert_eq!(
            a.push(&InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload: b"not a frame".to_vec(),
            })
            .unwrap_err(),
            CoreError::ScreenFragment
        );
    }

    #[test]
    fn assembler_rejects_an_index_past_the_stream_end() {
        // A sender that keeps `total` small but claims a high `idx` must be
        // refused before `total - off` can underflow (a remote panic).
        let mut a = ScreenAssembler::new();
        let mut frag = vec![0u8; SCREEN_FRAG_HEADER + 16];
        frag[..4].copy_from_slice(&SCREEN_FRAG_MAGIC);
        frag[4..8].copy_from_slice(&16u32.to_be_bytes()); // total = 16
        frag[8..12].copy_from_slice(&1u32.to_be_bytes()); // idx = 1 -> off = 64K
        frag[12..16].copy_from_slice(&2u32.to_be_bytes()); // count = 2
        frag[16..24].copy_from_slice(&5u64.to_be_bytes());
        assert_eq!(
            a.push(&InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload: frag,
            })
            .unwrap_err(),
            CoreError::ScreenFragment
        );
    }

    #[test]
    fn assembler_stale_stream_is_superseded_by_a_new_seq() {
        let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
        let f1 =
            ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0x11u8; 4 * 1024 * 1024]).unwrap();
        let f2 =
            ScreenFrame::raw(2, rect, PixelFormat::Bgra8, vec![0x22u8; 4 * 1024 * 1024]).unwrap();
        let (mut f1f, mut f2f) = (f1.to_frames().unwrap(), f2.to_frames().unwrap());
        let mut a = ScreenAssembler::new();
        // Half of stream 1, then a chunk of stream 2 — the old buffer resets.
        for fr in f1f.drain(..f1f.len() / 2) {
            assert!(a.push(&fr).unwrap().is_none());
        }
        assert!(a.push(&f2f.remove(0)).unwrap().is_none());
        let mut out = None;
        for fr in f2f {
            out = a.push(&fr).unwrap();
        }
        assert_eq!(out, Some(f2));
    }

    #[test]
    fn assembler_rejects_a_short_count_that_would_leave_the_body_unfilled() {
        // `count` smaller than the chunks implied by `total` would otherwise let
        // the stream "complete" with holes in `body` — a silently corrupt frame.
        let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
        let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 4 * 1024 * 1024]).unwrap();
        let frames = f.to_frames().unwrap();
        assert!(frames.len() > 1);
        let mut bad = frames[0].payload.clone();
        bad[12..16].copy_from_slice(&1u32.to_be_bytes()); // count = 1, total is many
        let mut a = ScreenAssembler::new();
        assert_eq!(
            a.push(&InnerFrame {
                ty: TYPE_SCREEN_FRAME,
                flags: FLAG_MUST_UNDERSTAND,
                msg_id: 0,
                payload: bad,
            })
            .unwrap_err(),
            CoreError::ScreenFragment
        );
    }

    #[test]
    fn assembler_whole_frame_fast_path_drops_stale_fragments() {
        // Half a fragmented stream, then a whole frame: the whole frame must
        // win and the leftover fragments must not resurrect.
        let rect = ScreenRect::new(0, 0, 1024, 1024).unwrap();
        let big =
            ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0x33u8; 4 * 1024 * 1024]).unwrap();
        let frags = big.to_frames().unwrap();
        let small = raw_frame(2);
        let whole = small.to_frames().unwrap();
        assert!(frags.len() > 1 && whole.len() == 1);

        let mut a = ScreenAssembler::new();
        assert!(a.push(&frags[0]).unwrap().is_none());
        assert_eq!(a.push(&whole[0]).unwrap(), Some(small.clone()));
        // Feeding the remaining old fragments must re-anchor against a fresh
        // stream (they may start a new partial), never blend with the whole
        // frame that already shipped.
        for fr in &frags[1..] {
            let _ = a.push(fr);
        }
    }

    #[test]
    fn screen_matrix_coordinator_health_state_transitions() {
        let mut coord = ScreenMatrixCoordinator::default();
        assert_eq!(coord.health(), NetworkHealth::Healthy);
        assert_eq!(coord.mode(), ScreenMatrixMode::Auto);

        // Low RTT, 0 inflight -> Healthy
        let out1 = coord.on_ack(1_000_000, 50_000, 0, 60, 100, 60);
        assert_eq!(out1.health, NetworkHealth::Healthy);

        // Elevated RTT (150ms) -> Warning
        let out2 = coord.on_ack(1_100_000, 150_000, 0, 60, 100, 60);
        assert_eq!(out2.health, NetworkHealth::Warning);

        // High RTT (280ms) -> Congested
        let out3 = coord.on_ack(1_200_000, 280_000, 0, 60, 100, 60);
        assert_eq!(out3.health, NetworkHealth::Congested);

        // High in-flight (8) -> Critical
        let out4 = coord.on_ack(1_300_000, 100_000, 8, 60, 100, 60);
        assert_eq!(out4.health, NetworkHealth::Critical);
    }

    #[test]
    fn screen_matrix_coordinator_demotion_ladder_on_drops() {
        let mut coord = ScreenMatrixCoordinator::default();
        let now_us = 10_000_000;

        // 1 drop -> Congested -> demote FPS from 60 to 30
        let out1 = coord.on_drop(now_us, 60, 100);
        assert_eq!(out1.health, NetworkHealth::Congested);
        assert_eq!(out1.new_fps, Some(30));
        assert_eq!(out1.new_scale_pct, None);

        // 2nd drop (exceeds cooldown: +500ms) -> Critical -> demote scale to 75 and FPS to 24
        let out2 = coord.on_drop(now_us + 500_000, 30, 100);
        assert_eq!(out2.health, NetworkHealth::Critical);
        assert_eq!(out2.new_scale_pct, Some(75));
        assert_eq!(out2.new_fps, Some(24));
        assert!(out2.force_idr);
    }

    #[test]
    fn screen_matrix_coordinator_promotion_ladder_on_sustained_healthy_acks() {
        let mut coord = ScreenMatrixCoordinator::default();
        let mut now_us = 10_000_000;

        // Start degraded at 50% scale, 15 FPS
        let mut current_scale = 50;
        let current_fps = 15;

        // Send 14 healthy acks (not yet 15)
        for _ in 0..14 {
            now_us += 100_000;
            let out = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
            assert_eq!(out.new_scale_pct, None);
            assert_eq!(out.new_fps, None);
        }

        // 15th ack (+3.5s elapsed): should promote scale first: 50% -> 75%
        now_us += 3_500_000;
        let out_promo1 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        assert_eq!(out_promo1.new_scale_pct, Some(75));
        assert!(out_promo1.force_idr);
        current_scale = 75;

        // Another 15 acks (+3.5s): promote scale 75% -> 100%
        for _ in 0..14 {
            now_us += 100_000;
            let _ = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        }
        now_us += 3_500_000;
        let out_promo2 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        assert_eq!(out_promo2.new_scale_pct, Some(100));
        current_scale = 100;

        // Once scale is 100%, next promotion promotes FPS: 15 -> 24
        for _ in 0..14 {
            now_us += 100_000;
            let _ = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        }
        now_us += 3_500_000;
        let out_promo3 = coord.on_ack(now_us, 30_000, 0, current_fps, current_scale, 60);
        assert_eq!(out_promo3.new_fps, Some(24));
        assert_eq!(out_promo3.new_scale_pct, None);
    }

    #[test]
    fn screen_matrix_coordinator_modes_manual_and_conservative() {
        let mut coord_manual = ScreenMatrixCoordinator::new(ScreenMatrixMode::Manual);
        let out_m = coord_manual.on_drop(1_000_000, 60, 100);
        assert_eq!(out_m.health, NetworkHealth::Congested);
        assert_eq!(out_m.new_fps, None);
        assert_eq!(out_m.new_scale_pct, None);

        let mut coord_cons = ScreenMatrixCoordinator::new(ScreenMatrixMode::Conservative);
        let out_c1 = coord_cons.on_drop(1_000_000, 60, 100);
        assert_eq!(out_c1.new_fps, Some(30));
        assert_eq!(out_c1.new_scale_pct, None); // Conservative never downscales resolution

        let out_c2 = coord_cons.on_drop(2_000_000, 30, 100);
        assert_eq!(out_c2.health, NetworkHealth::Critical);
        assert_eq!(out_c2.new_fps, Some(24));
        assert_eq!(out_c2.new_scale_pct, None); // Still no scale change
    }

    #[test]
    fn screen_sender_4d_autonomous_matrix_closed_loop() {
        let mut sender = ScreenSender::new();
        assert_eq!(sender.stats().matrix_mode(), ScreenMatrixMode::Auto);
        assert_eq!(sender.stats().network_health(), NetworkHealth::Healthy);
        assert_eq!(sender.stats().fps_target, 30);
        assert_eq!(sender.stats().scale_pct, 100);

        sender.set_fps(60);
        assert_eq!(sender.stats().fps_target, 60);

        // Record a congestion drop at t=10_000_000
        sender.record_drop_with_time(10_000_000);
        assert_eq!(sender.stats().network_health(), NetworkHealth::Congested);
        assert_eq!(sender.stats().fps_target, 30); // 60 -> 30 demoted
        assert_eq!(sender.stats().scale_pct, 100);

        // Mode switch check
        sender.set_matrix_mode(ScreenMatrixMode::Conservative);
        assert_eq!(sender.stats().matrix_mode(), ScreenMatrixMode::Conservative);
    }

    #[test]
    fn sender_tracks_in_flight_and_computes_rtt_on_ack() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();

        // Push 3 frames with timestamps (t = 1000, 2000, 3000 us)
        assert_eq!(s.push_with_time(raw_frame(1), 1000), Some(1));
        assert_eq!(s.push_with_time(raw_frame(2), 2000), Some(2));
        assert_eq!(s.push_with_time(raw_frame(3), 3000), Some(3));

        assert_eq!(s.stats().sent, 3);
        assert_eq!(s.stats().in_flight, 3);
        assert_eq!(s.in_flight_count(), 3);
        assert!(!s.is_congested());

        // Ack frame 2 at t = 2050 us (RTT for frame 2 = 50 us; frame 1 is also cumulatively acknowledged)
        s.on_control_with_time(ControlMsg::Ack { seq: 2 }, 2050)
            .unwrap();
        assert_eq!(s.stats().acked, 1);
        assert_eq!(s.stats().last_acked_seq, 2);
        assert_eq!(s.stats().in_flight, 1); // Only frame 3 remains in-flight
        assert_eq!(s.stats().rtt_latest_us, Some(50));
        assert_eq!(s.stats().rtt_p95_us, Some(50));

        // Ack frame 3 at t = 3100 us (RTT for frame 3 = 100 us)
        s.on_control_with_time(ControlMsg::Ack { seq: 3 }, 3100)
            .unwrap();
        assert_eq!(s.stats().acked, 2);
        assert_eq!(s.stats().last_acked_seq, 3);
        assert_eq!(s.stats().in_flight, 0);
        assert_eq!(s.stats().rtt_latest_us, Some(100));
        assert_eq!(s.stats().rtt_p95_us, Some(100));
        assert!(!s.is_congested());
    }

    #[test]
    fn sender_congestion_detection_and_in_flight_bounds() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();

        // Push up to congestion threshold (8 frames)
        for i in 1..=8 {
            s.push_with_time(raw_frame(i), i * 1000);
        }
        assert_eq!(s.in_flight_count(), 8);
        assert!(s.is_congested()); // In-flight threshold reached

        // Ack frame 8: clears in-flight queue, RTT = 20 us
        s.on_control_with_time(ControlMsg::Ack { seq: 8 }, 8020)
            .unwrap();
        assert_eq!(s.in_flight_count(), 0);
        assert!(!s.is_congested());

        // Push frame 9, ack with high RTT (300ms)
        s.push_with_time(raw_frame(9), 10_000);
        s.on_control_with_time(ControlMsg::Ack { seq: 9 }, 10_000 + 300_000)
            .unwrap();
        assert_eq!(s.stats().rtt_latest_us, Some(300_000));
        assert_eq!(s.in_flight_count(), 0);
        assert!(!s.is_congested(), "drained queue must not be congested");

        // When a new frame 10 is pushed under high RTT, while it is in-flight, channel becomes congested
        s.push_with_time(raw_frame(10), 400_000);
        assert_eq!(s.in_flight_count(), 1);
        assert!(
            s.is_congested(),
            "in-flight frame with high RTT triggers congestion pacing"
        );
    }

    #[test]
    fn keyframe_request_and_gap_detection_tracking() {
        let mut sender = ScreenSender::new();
        sender.on_peer_connected();
        sender.allow();
        sender.on_control(ControlMsg::Start).unwrap();

        assert_eq!(sender.stats().keyframes_requested, 0);
        sender.on_control(ControlMsg::KeyframeRequest).unwrap();
        assert_eq!(sender.stats().keyframes_requested, 1);
        sender.on_control(ControlMsg::KeyframeRequest).unwrap();
        assert_eq!(sender.stats().keyframes_requested, 2);

        let mut receiver = ScreenReceiver::new();
        assert_eq!(receiver.stats().gaps, 0);

        // Frame 1 arrives
        assert_eq!(receiver.apply(&raw_frame(1)).unwrap(), Some(1));
        assert_eq!(receiver.stats().gaps, 0);

        // Frame 2 arrives (consecutive, no gap)
        assert_eq!(receiver.apply(&raw_frame(2)).unwrap(), Some(2));
        assert_eq!(receiver.stats().gaps, 0);

        // Frame 5 arrives (frames 3 and 4 were dropped -> 1 gap detected)
        assert_eq!(receiver.apply(&raw_frame(5)).unwrap(), Some(5));
        assert_eq!(receiver.stats().gaps, 1);

        // Frame 6 arrives (consecutive, no additional gap)
        assert_eq!(receiver.apply(&raw_frame(6)).unwrap(), Some(6));
        assert_eq!(receiver.stats().gaps, 1);

        // Frame 9 arrives (frames 7 and 8 dropped -> gap count becomes 2)
        assert_eq!(receiver.apply(&raw_frame(9)).unwrap(), Some(9));
        assert_eq!(receiver.stats().gaps, 2);
    }

    #[test]
    fn rate_controller_aimd_adaptation() {
        let mut s = ScreenSender::new();
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();

        assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_DEFAULT_BPS);
        assert_eq!(s.stats().bitrate_bps, SCREEN_BITRATE_DEFAULT_BPS);

        // 1. High RTT congestion triggers Multiplicative Decrease (80% of 4_000_000 = 3_200_000)
        s.push_with_time(raw_frame(1), 1_000);
        s.on_control_with_time(ControlMsg::Ack { seq: 1 }, 1_000 + 300_000)
            .unwrap();
        assert_eq!(s.bitrate_bps(), 3_200_000);

        // 2. Cooldown damping: another high RTT within 400ms does NOT decrease again
        s.push_with_time(raw_frame(2), 310_000);
        s.on_control_with_time(ControlMsg::Ack { seq: 2 }, 310_000 + 260_000)
            .unwrap();
        assert_eq!(s.bitrate_bps(), 3_200_000);

        // 3. Sender drop past cooldown triggers another decrease (80% of 3_200_000 = 2_560_000)
        s.record_drop_with_time(1_000_000);
        assert_eq!(s.bitrate_bps(), 2_560_000);

        // 4. Clean network (low RTT, in_flight=0): 5 consecutive good frames trigger Additive Increase (+150k)
        let mut now = 1_500_000;
        for i in 3..8 {
            s.push_with_time(raw_frame(i), now);
            now += 50_000;
            s.on_control_with_time(ControlMsg::Ack { seq: i }, now)
                .unwrap();
        }
        assert_eq!(
            s.bitrate_bps(),
            2_560_000 + SCREEN_BITRATE_STEP_INCREASE_BPS
        );

        // 5. Manual override clamps to bounds
        s.set_bitrate_bps(50_000_000);
        assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_MAX_BPS);

        s.set_bitrate_bps(10_000);
        assert_eq!(s.bitrate_bps(), SCREEN_BITRATE_MIN_BPS);
    }

    #[test]
    fn fps_pacer_and_frame_rate_throttling() {
        let mut pacer = ScreenFpsPacer::default();
        assert_eq!(pacer.fps_target(), SCREEN_FPS_DEFAULT);
        assert_eq!(pacer.interval_us(), 1_000_000 / 30);

        // 1. Clamping bounds: upper limit (120 FPS) and lower limit (5 FPS)
        assert_eq!(pacer.set_fps(200), SCREEN_FPS_MAX);
        assert_eq!(pacer.fps_target(), SCREEN_FPS_MAX);
        assert_eq!(pacer.interval_us(), 1_000_000 / 120);

        assert_eq!(pacer.set_fps(1), SCREEN_FPS_MIN);
        assert_eq!(pacer.fps_target(), SCREEN_FPS_MIN);
        assert_eq!(pacer.interval_us(), 1_000_000 / 5);

        // Set to 30 FPS (interval = 33_333 us)
        pacer.set_fps(30);
        assert_eq!(pacer.fps_target(), 30);
        let interval = pacer.interval_us();

        // 2. Initial frame tick consumes immediately
        assert!(pacer.is_ready(100_000));
        assert!(pacer.check_and_consume(100_000));

        // 3. Early tick before interval elapses is throttled
        assert!(!pacer.is_ready(100_000 + 15_000));
        assert!(!pacer.check_and_consume(100_000 + 15_000));

        // 4. Tick after interval elapses is permitted
        assert!(pacer.is_ready(100_000 + interval));
        assert!(pacer.check_and_consume(100_000 + interval));

        // 5. Sender integration and stats
        let mut s = ScreenSender::new();
        assert_eq!(s.fps_target(), SCREEN_FPS_DEFAULT);
        assert_eq!(s.stats().fps_target, SCREEN_FPS_DEFAULT);

        assert_eq!(s.set_fps(60), 60);
        assert_eq!(s.fps_target(), 60);
        assert_eq!(s.stats().fps_target, 60);

        assert!(s.consume_pacing(500_000));
        assert!(!s.consume_pacing(500_000 + 5_000));
        assert!(s.consume_pacing(500_000 + 17_000));

        s.on_peer_gone();
        // Reset resets last frame timestamp, allowing immediate next tick
        assert!(s.is_pacing_ready(600_000));
    }

    #[test]
    fn resolution_scale_controller_and_downscaling() {
        let mut ctrl = ScreenScaleController::default();
        assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_DEFAULT_PCT);

        // 1. Clamping bounds
        assert_eq!(ctrl.set_scale_pct(150), SCREEN_SCALE_MAX_PCT);
        assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_MAX_PCT);

        assert_eq!(ctrl.set_scale_pct(10), SCREEN_SCALE_MIN_PCT);
        assert_eq!(ctrl.scale_pct(), SCREEN_SCALE_MIN_PCT);

        // 2. Dimension calculation with even constraint (minimum 2x2)
        assert_eq!(
            ScreenScaleController::scale_dims(1920, 1080, 100),
            (1920, 1080)
        );
        assert_eq!(
            ScreenScaleController::scale_dims(1920, 1080, 50),
            (960, 540)
        );
        assert_eq!(
            ScreenScaleController::scale_dims(1920, 1080, 75),
            (1440, 810)
        );
        assert_eq!(
            ScreenScaleController::scale_dims(1920, 1080, 25),
            (480, 270)
        );

        // Odd dimensions rounded to even
        let (w, h) = ScreenScaleController::scale_dims(1366, 768, 50);
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert_eq!((w, h), (682, 384));

        // 3. Downscale raw frame
        let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
        let mut bgra = vec![0u8; 100 * 100 * 4];
        for i in 0..100 * 100 {
            bgra[i * 4] = 255; // Blue
            bgra[i * 4 + 1] = 128; // Green
            bgra[i * 4 + 2] = 64; // Red
            bgra[i * 4 + 3] = 255; // Alpha
        }
        let frame = ScreenFrame::raw(42, rect, PixelFormat::Bgra8, bgra).unwrap();

        // 50% scale -> 50x50 frame
        let scaled = downscale_frame(&frame, 50).unwrap();
        assert_eq!(scaled.rect.w, 50);
        assert_eq!(scaled.rect.h, 50);
        assert_eq!(scaled.bytes.len(), 50 * 50 * 4);
        assert_eq!(scaled.seq, 42);
        // Verify color preservation in downscaled pixels
        assert_eq!(scaled.bytes[0], 255);
        assert_eq!(scaled.bytes[1], 128);
        assert_eq!(scaled.bytes[2], 64);
        assert_eq!(scaled.bytes[3], 255);

        // 100% scale is a no-op clone
        let native = downscale_frame(&frame, 100).unwrap();
        assert_eq!(native.rect.w, 100);
        assert_eq!(native.rect.h, 100);

        // 4. Sender integration and stats
        let mut s = ScreenSender::new();
        assert_eq!(s.scale_pct(), SCREEN_SCALE_DEFAULT_PCT);
        assert_eq!(s.stats().scale_pct, SCREEN_SCALE_DEFAULT_PCT);

        assert_eq!(s.set_scale_pct(50), 50);
        assert_eq!(s.scale_pct(), 50);
        assert_eq!(s.stats().scale_pct, 50);
    }

    #[test]
    fn quality_controller_and_content_profiles() {
        let mut ctrl = ScreenQualityController::default();
        assert_eq!(ctrl.profile(), ScreenContentProfile::Text);
        assert_eq!(ctrl.min_qp(), SCREEN_QP_TEXT_DEFAULT.0);
        assert_eq!(ctrl.max_qp(), SCREEN_QP_TEXT_DEFAULT.1);
        assert_eq!(ctrl.qp_range(), SCREEN_QP_TEXT_DEFAULT);
        assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Text);
        assert!(!ctrl.custom_qp());

        // 1. Switch to Motion profile updates default QP range to motion defaults
        assert_eq!(
            ctrl.set_profile(ScreenContentProfile::Motion),
            ScreenContentProfile::Motion
        );
        assert_eq!(ctrl.profile(), ScreenContentProfile::Motion);
        assert_eq!(ctrl.min_qp(), SCREEN_QP_MOTION_DEFAULT.0);
        assert_eq!(ctrl.max_qp(), SCREEN_QP_MOTION_DEFAULT.1);
        assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Motion);

        // 2. Switch to Auto profile resolves effective profile to Text for desktop sharing
        assert_eq!(
            ctrl.set_profile(ScreenContentProfile::Auto),
            ScreenContentProfile::Auto
        );
        assert_eq!(ctrl.effective_profile(), ScreenContentProfile::Text);
        assert_eq!(ctrl.qp_range(), SCREEN_QP_TEXT_DEFAULT);

        // 3. Custom QP range configuration and clamping
        let custom = ctrl.set_qp_range(10, 30);
        assert_eq!(custom, (10, 30));
        assert_eq!(ctrl.qp_range(), (10, 30));
        assert!(ctrl.custom_qp());

        // Clamping invalid inputs (min > max clamped, bounds 0..51)
        let clamped = ctrl.set_qp_range(60, 20);
        assert_eq!(clamped, (51, 51));

        // 4. Reset default QP restores profile defaults
        let reset = ctrl.reset_default_qp();
        assert_eq!(reset, SCREEN_QP_TEXT_DEFAULT);
        assert!(!ctrl.custom_qp());

        // 5. String parsing and display
        assert_eq!(
            "text".parse::<ScreenContentProfile>().unwrap(),
            ScreenContentProfile::Text
        );
        assert_eq!(
            "desktop".parse::<ScreenContentProfile>().unwrap(),
            ScreenContentProfile::Text
        );
        assert_eq!(
            "motion".parse::<ScreenContentProfile>().unwrap(),
            ScreenContentProfile::Motion
        );
        assert_eq!(
            "video".parse::<ScreenContentProfile>().unwrap(),
            ScreenContentProfile::Motion
        );
        assert_eq!(
            "auto".parse::<ScreenContentProfile>().unwrap(),
            ScreenContentProfile::Auto
        );
        assert!("invalid".parse::<ScreenContentProfile>().is_err());
        assert_eq!(ScreenContentProfile::Text.to_string(), "text");
        assert_eq!(ScreenContentProfile::Motion.to_string(), "motion");
        assert_eq!(ScreenContentProfile::Auto.to_string(), "auto");

        // 6. Sender integration and stats
        let mut s = ScreenSender::new();
        assert_eq!(s.content_profile(), ScreenContentProfile::Text);
        assert_eq!(s.stats().content_profile(), ScreenContentProfile::Text);
        assert_eq!(s.stats().profile_tag, 0);
        assert_eq!(s.stats().qp_min, SCREEN_QP_TEXT_DEFAULT.0);
        assert_eq!(s.stats().qp_max, SCREEN_QP_TEXT_DEFAULT.1);

        assert_eq!(
            s.set_content_profile(ScreenContentProfile::Motion),
            ScreenContentProfile::Motion
        );
        assert_eq!(s.content_profile(), ScreenContentProfile::Motion);
        assert_eq!(s.stats().profile_tag, 1);
        assert_eq!(s.stats().content_profile(), ScreenContentProfile::Motion);
        assert_eq!(s.stats().qp_min, SCREEN_QP_MOTION_DEFAULT.0);
        assert_eq!(s.stats().qp_max, SCREEN_QP_MOTION_DEFAULT.1);

        s.set_qp_range(8, 28);
        assert_eq!(s.qp_range(), (8, 28));
        assert_eq!(s.stats().qp_min, 8);
        assert_eq!(s.stats().qp_max, 28);
    }

    #[test]
    fn screen_playout_modes_and_target_delays() {
        let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);
        assert_eq!(buf.mode(), ScreenPlayoutMode::Adaptive);
        assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US);

        // Switch to ZeroLatency
        buf.set_mode(ScreenPlayoutMode::ZeroLatency);
        assert_eq!(buf.mode(), ScreenPlayoutMode::ZeroLatency);
        assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_MIN_DELAY_US);

        // Switch to Smooth
        buf.set_mode(ScreenPlayoutMode::Smooth);
        assert_eq!(buf.mode(), ScreenPlayoutMode::Smooth);
        assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US);

        // Custom delay override
        buf.set_custom_target_delay_us(Some(50_000));
        assert_eq!(buf.custom_target_delay_us(), Some(50_000));
        assert_eq!(buf.target_delay_us(), 50_000);

        // Clear custom delay restores mode default
        buf.set_custom_target_delay_us(None);
        assert_eq!(buf.custom_target_delay_us(), None);
        assert_eq!(buf.target_delay_us(), SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US);

        // Parsing and string conversion
        assert_eq!(
            ScreenPlayoutMode::parse("adaptive"),
            Some(ScreenPlayoutMode::Adaptive)
        );
        assert_eq!(
            ScreenPlayoutMode::parse("auto"),
            Some(ScreenPlayoutMode::Adaptive)
        );
        assert_eq!(
            ScreenPlayoutMode::parse("zerolatency"),
            Some(ScreenPlayoutMode::ZeroLatency)
        );
        assert_eq!(
            ScreenPlayoutMode::parse("zero"),
            Some(ScreenPlayoutMode::ZeroLatency)
        );
        assert_eq!(
            ScreenPlayoutMode::parse("smooth"),
            Some(ScreenPlayoutMode::Smooth)
        );
        assert_eq!(ScreenPlayoutMode::parse("invalid"), None);

        assert_eq!(ScreenPlayoutMode::from_tag(0), ScreenPlayoutMode::Adaptive);
        assert_eq!(
            ScreenPlayoutMode::from_tag(1),
            ScreenPlayoutMode::ZeroLatency
        );
        assert_eq!(ScreenPlayoutMode::from_tag(2), ScreenPlayoutMode::Smooth);
    }

    #[test]
    fn screen_playout_jitter_calculation_and_pacing() {
        let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);

        // 1. Initial frame arrival at t=100_000 us
        let f1 = raw_frame(1);
        buf.push_frame(f1, 100_000);
        assert_eq!(buf.queue_len(), 1);
        assert_eq!(buf.current_jitter_us(), 0);
        // Initial playout deadline = 100_000 + 15_000 = 115_000 us
        assert_eq!(buf.next_scheduled_playout_us(), Some(115_000));

        // Pop before scheduled playout returns WaitUntil
        match buf.pop_frame(110_000) {
            ScreenPlayoutAction::WaitUntil(ts) => assert_eq!(ts, 115_000),
            other => panic!("expected WaitUntil, got {other:?}"),
        }

        // Pop at or after scheduled playout returns Play
        match buf.pop_frame(115_000) {
            ScreenPlayoutAction::Play(f) => assert_eq!(f.seq, 1),
            other => panic!("expected Play, got {other:?}"),
        }
        assert_eq!(buf.queue_len(), 0);

        // 2. Second frame arrives with severe jitter (gap of 90ms instead of nominal 33.3ms)
        let f2 = raw_frame(2);
        buf.push_frame(f2, 190_000);
        // Interarrival = 90_000, nominal = 33_333, diff = 56_667. Jitter updated via EWMA.
        assert!(buf.current_jitter_us() > 0);
        // Adaptive target delay expands beyond base 15ms due to jitter
        assert!(buf.target_delay_us() > SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US);
    }

    #[test]
    fn screen_playout_late_frame_dropping_and_overflow() {
        let mut buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::ZeroLatency);

        // Push 3 frames
        buf.push_frame(raw_frame(1), 10_000);
        buf.push_frame(raw_frame(2), 20_000);
        buf.push_frame(raw_frame(3), 30_000);

        // Advance clock way past late threshold (e.g. t = 500_000 us)
        // Frames 1 and 2 are hopelessly late; pop should drop late frames and play frame 3
        match buf.pop_frame(500_000) {
            ScreenPlayoutAction::Play(f) => assert_eq!(f.seq, 3),
            other => panic!("expected Play frame 3, got {other:?}"),
        }
        let st = buf.stats();
        assert_eq!(st.dropped_late_frames, 2);
        assert_eq!(st.played_frames, 1);

        // Overflow test: push 15 frames into max_capacity 12 buffer
        for i in 10..25 {
            buf.push_frame(raw_frame(i), 1_000_000 + (i - 10) * 10_000);
        }
        assert_eq!(buf.queue_len(), 12);
        assert_eq!(buf.stats().dropped_overflow_frames, 3);
    }

    #[test]
    fn screen_receiver_playout_buffer_integration() {
        let mut recv = ScreenReceiver::with_playout_mode(ScreenPlayoutMode::Smooth);
        assert_eq!(recv.playout_mode(), ScreenPlayoutMode::Smooth);

        let f = raw_frame(1);
        let accepted = recv.apply(&f).unwrap();
        assert_eq!(accepted, Some(1));

        recv.push_playout(f, 50_000);
        assert_eq!(recv.playout_stats().buffered_frames, 1);

        let stats = recv.stats();
        assert_eq!(stats.playout_mode, ScreenPlayoutMode::Smooth as u8);
        assert_eq!(
            stats.playout_delay_us,
            SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US as u32
        );

        // On peer gone resets playout queue
        recv.on_peer_gone();
        assert_eq!(recv.playout_stats().buffered_frames, 0);
    }

    #[test]
    fn screen_cursor_update_codec_and_roundtrip() {
        // 1. Basic standard cursor without custom bitmap
        let update1 = ScreenCursorUpdate {
            seq: 42,
            x: 1024,
            y: 768,
            visible: true,
            shape: CursorShape::Hand,
            hotspot_x: 6,
            hotspot_y: 2,
            width: 32,
            height: 32,
            custom_rgba: None,
        };

        let payload = update1.encode_payload().unwrap();
        assert_eq!(payload.len(), ScreenCursorUpdate::HEADER_LEN);
        let decoded1 = ScreenCursorUpdate::decode_payload(&payload).unwrap();
        assert_eq!(decoded1, update1);

        // InnerFrame conversion
        let frame = update1.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_SCREEN_CURSOR);
        let decoded_frame = ScreenCursorUpdate::from_frame(&frame).unwrap();
        assert_eq!(decoded_frame, update1);

        // 2. Custom cursor with RGBA bitmap payload
        let custom_bytes = vec![255u8; 16 * 16 * 4];
        let update2 = ScreenCursorUpdate {
            seq: 99,
            x: 500,
            y: 300,
            visible: true,
            shape: CursorShape::Custom,
            hotspot_x: 0,
            hotspot_y: 0,
            width: 16,
            height: 16,
            custom_rgba: Some(custom_bytes.clone()),
        };

        let payload2 = update2.encode_payload().unwrap();
        assert_eq!(
            payload2.len(),
            ScreenCursorUpdate::HEADER_LEN + custom_bytes.len()
        );
        let decoded2 = ScreenCursorUpdate::decode_payload(&payload2).unwrap();
        assert_eq!(decoded2, update2);

        // 3. Negative coordinates (multi-monitor topology)
        let update3 = ScreenCursorUpdate {
            seq: 101,
            x: -1920,
            y: -1080,
            visible: false,
            shape: CursorShape::IBeam,
            hotspot_x: 4,
            hotspot_y: 8,
            width: 16,
            height: 24,
            custom_rgba: None,
        };
        let payload3 = update3.encode_payload().unwrap();
        let decoded3 = ScreenCursorUpdate::decode_payload(&payload3).unwrap();
        assert_eq!(decoded3.x, -1920);
        assert_eq!(decoded3.y, -1080);
        assert!(!decoded3.visible);
        assert_eq!(decoded3.shape, CursorShape::IBeam);
    }

    #[test]
    fn screen_cursor_controller_dedup_and_rate_limiting() {
        let mut ctrl = ScreenCursorController::new(ScreenCursorMode::Decoupled);
        assert_eq!(ctrl.mode(), ScreenCursorMode::Decoupled);
        assert_eq!(ctrl.min_interval_us(), SCREEN_CURSOR_MIN_INTERVAL_US);

        // 1. First update succeeds
        let u1 = ctrl.create_update(
            100,
            100,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            10_000,
        );
        assert!(u1.is_some());
        assert_eq!(u1.unwrap().seq, 1);
        assert_eq!(ctrl.stats().updates_sent, 1);

        // 2. Exact position deduplication drops redundant call
        let u2 = ctrl.create_update(
            100,
            100,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            10_000 + 50_000,
        );
        assert!(u2.is_none());
        assert_eq!(ctrl.stats().updates_dropped_dedup, 1);

        // 3. Movement before min_interval_us (8ms) elapses is rate-limited
        let u3 = ctrl.create_update(
            105,
            102,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            10_000 + 4_000, // only 4ms elapsed
        );
        assert!(u3.is_none());
        assert_eq!(ctrl.stats().updates_dropped_rate, 1);

        // 4. Movement after min_interval_us elapses succeeds
        let u4 = ctrl.create_update(
            110,
            105,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            10_000 + 10_000, // 10ms elapsed >= 8ms
        );
        assert!(u4.is_some());
        assert_eq!(u4.unwrap().seq, 2);
        assert_eq!(ctrl.stats().updates_sent, 2);

        // 5. Shape change bypasses rate limiting immediately
        let u5 = ctrl.create_update(
            112,
            106,
            true,
            CursorShape::Hand, // shape changed!
            (6, 2),
            (32, 32),
            None,
            10_000 + 11_000, // only 1ms after last send
        );
        assert!(u5.is_some());
        assert_eq!(u5.unwrap().shape, CursorShape::Hand);
        assert_eq!(ctrl.stats().updates_sent, 3);

        // 6. Embedded mode produces no outbound cursor updates
        ctrl.set_mode(ScreenCursorMode::Embedded);
        let u6 = ctrl.create_update(
            200,
            200,
            true,
            CursorShape::Arrow,
            (0, 0),
            (32, 32),
            None,
            1_000_000,
        );
        assert!(u6.is_none());
    }

    #[test]
    fn screen_cursor_sender_and_receiver_integration() {
        let mut sender = ScreenSender::new();
        let mut receiver = ScreenReceiver::new();

        // 1. Initial default mode is decoupled
        assert_eq!(sender.cursor_mode(), ScreenCursorMode::Decoupled);
        assert_eq!(receiver.cursor_mode(), ScreenCursorMode::Decoupled);

        // Cursor push before streaming returns None
        assert!(sender
            .push_cursor_with_time(
                50,
                50,
                true,
                CursorShape::Arrow,
                (0, 0),
                (32, 32),
                None,
                1000
            )
            .is_none());

        // Connect, allow, and start streaming
        sender.on_peer_connected();
        sender.allow();
        sender.on_control(ControlMsg::Start).unwrap();
        assert!(sender.state().is_streaming());

        // 2. Push cursor update on sender
        let update = sender
            .push_cursor_with_time(
                300,
                400,
                true,
                CursorShape::IBeam,
                (4, 8),
                (16, 24),
                None,
                20_000,
            )
            .expect("should produce update");

        assert_eq!(update.x, 300);
        assert_eq!(update.y, 400);
        assert_eq!(update.shape, CursorShape::IBeam);
        assert_eq!(sender.stats().cursor_updates_sent, 1);
        assert_eq!(sender.stats().cursor_x, 300);
        assert_eq!(sender.stats().cursor_y, 400);

        // 3. Receive cursor update on receiver
        receiver.on_cursor(update.clone());
        assert_eq!(receiver.stats().cursor_updates_received, 1);
        assert_eq!(receiver.stats().cursor_x, 300);
        assert_eq!(receiver.stats().cursor_y, 400);
        assert_eq!(receiver.stats().cursor_shape, CursorShape::IBeam.tag());
        assert_eq!(receiver.current_cursor(), Some(&update));

        // 4. Mode switching updates stats
        sender.set_cursor_mode(ScreenCursorMode::Hidden);
        assert_eq!(sender.cursor_mode(), ScreenCursorMode::Hidden);
        assert_eq!(sender.stats().cursor_mode, ScreenCursorMode::Hidden.tag());

        receiver.set_cursor_mode(ScreenCursorMode::Embedded);
        assert_eq!(receiver.cursor_mode(), ScreenCursorMode::Embedded);
        assert_eq!(
            receiver.stats().cursor_mode,
            ScreenCursorMode::Embedded.tag()
        );

        // 5. On peer gone resets receiver cursor
        receiver.on_peer_gone();
        assert_eq!(receiver.current_cursor(), None);
    }

    #[test]
    fn screen_damage_tracker_clean_and_dirty_eval() {
        let mut tracker = ScreenDamageTracker::new();
        let w = 128u32;
        let h = 128u32;
        let block_w = w.div_ceil(SCREEN_DAMAGE_BLOCK_SIZE as u32);
        let block_h = h.div_ceil(SCREEN_DAMAGE_BLOCK_SIZE as u32);
        let total_blocks = (block_w * block_h) as usize;
        assert_eq!(total_blocks, 16);

        // 1. Initial frame comparison with empty previous buffer -> full dirty
        let mut curr = vec![0x11u8; (w * h * 4) as usize];
        let rep1 = tracker.evaluate_rgba(&[], &curr, w, h);
        assert!(rep1.is_dirty());
        assert_eq!(rep1.num_dirty_blocks, 1); // fallback returns 1 whole dirty block covering full rect
        assert_eq!(rep1.dirty_pixels, 128 * 128);
        assert!((rep1.dirty_ratio - 1.0).abs() < 1e-4);
        assert_eq!(rep1.rect(), Some(ScreenDamageRect::new(0, 0, 128, 128)));

        // 2. Second frame comparison with identical buffer -> clean (zero dirty)
        let rep2 = tracker.evaluate_rgba(&curr, &curr, w, h);
        assert!(!rep2.is_dirty());
        assert_eq!(rep2.num_dirty_blocks, 0);
        assert_eq!(rep2.total_blocks, 16);
        assert_eq!(rep2.dirty_pixels, 0);
        assert_eq!(rep2.dirty_ratio, 0.0);
        assert_eq!(rep2.rect(), None);

        // 3. Modify single pixel in macroblock (1, 1) -> (x: 32..64, y: 32..64)
        let prev = curr.clone();
        let target_idx = ((35 * w + 40) * 4) as usize;
        curr[target_idx] = 0x99;
        let rep3 = tracker.evaluate_rgba(&prev, &curr, w, h);
        assert!(rep3.is_dirty());
        assert_eq!(rep3.num_dirty_blocks, 1);
        assert_eq!(rep3.total_blocks, 16);
        assert_eq!(rep3.dirty_pixels, 32 * 32);
        assert_eq!(rep3.dirty_ratio, 1.0 / 16.0);
        assert_eq!(rep3.rect(), Some(ScreenDamageRect::new(32, 32, 32, 32)));

        // 4. Modify top-left (0, 0) and bottom-right (127, 127) -> bounding box covers whole screen
        let prev = curr.clone();
        curr[0] = 0xAA;
        let last_idx = ((127 * w + 127) * 4) as usize;
        curr[last_idx] = 0xBB;
        let rep4 = tracker.evaluate_rgba(&prev, &curr, w, h);
        assert!(rep4.is_dirty());
        assert_eq!(rep4.num_dirty_blocks, 2);
        assert_eq!(rep4.rect(), Some(ScreenDamageRect::new(0, 0, 128, 128)));
    }

    #[test]
    fn screen_squelch_modes_and_decisions() {
        // 1. Parsing and serialization
        assert_eq!(
            "auto".parse::<ScreenSquelchMode>().unwrap(),
            ScreenSquelchMode::Auto
        );
        assert_eq!(
            "aggressive".parse::<ScreenSquelchMode>().unwrap(),
            ScreenSquelchMode::Aggressive
        );
        assert_eq!(
            "agg".parse::<ScreenSquelchMode>().unwrap(),
            ScreenSquelchMode::Aggressive
        );
        assert_eq!(
            "disabled".parse::<ScreenSquelchMode>().unwrap(),
            ScreenSquelchMode::Disabled
        );
        assert_eq!(
            "off".parse::<ScreenSquelchMode>().unwrap(),
            ScreenSquelchMode::Disabled
        );
        assert!("invalid".parse::<ScreenSquelchMode>().is_err());
        assert_eq!(ScreenSquelchMode::from_tag(0), ScreenSquelchMode::Auto);
        assert_eq!(
            ScreenSquelchMode::from_tag(1),
            ScreenSquelchMode::Aggressive
        );
        assert_eq!(ScreenSquelchMode::from_tag(2), ScreenSquelchMode::Disabled);

        // 2. Auto mode lifecycle
        let mut ctrl = ScreenSquelchController::new(ScreenSquelchMode::Auto);
        let t0 = 1_000_000u64;

        // Dirty frame -> SendFrame
        assert_eq!(
            ctrl.evaluate_decision(true, 0.5, t0),
            ScreenSquelchDecision::SendFrame
        );
        assert!(!ctrl.stats().is_squelched);
        assert_eq!(ctrl.stats().frames_passed, 1);
        assert_eq!(ctrl.stats().consecutive_static_frames, 0);

        // Static frame within 500ms grace window (t0 + 200ms) -> SendFrame
        let t1 = t0 + 200_000;
        assert_eq!(
            ctrl.evaluate_decision(false, 0.0, t1),
            ScreenSquelchDecision::SendFrame
        );
        assert!(!ctrl.stats().is_squelched);
        assert_eq!(ctrl.stats().frames_passed, 2);
        assert_eq!(ctrl.stats().consecutive_static_frames, 1);

        // Static frame at t0 + 500ms -> Squelch activated, frame skipped
        let t2 = t0 + 500_000;
        assert_eq!(
            ctrl.evaluate_decision(false, 0.0, t2),
            ScreenSquelchDecision::SquelchSkip
        );
        assert!(ctrl.stats().is_squelched);
        assert_eq!(ctrl.stats().frames_squelched, 1);
        assert_eq!(ctrl.stats().consecutive_static_frames, 2);

        // Repeated static frames remain squelched
        for i in 1..10 {
            let t = t2 + i * 33_333;
            assert_eq!(
                ctrl.evaluate_decision(false, 0.0, t),
                ScreenSquelchDecision::SquelchSkip
            );
        }
        assert_eq!(ctrl.stats().frames_squelched, 10);

        // Keepalive heartbeat fires after 2.0s since last sent frame
        let t_keepalive = t0 + 200_000 + SCREEN_SQUELCH_AUTO_KEEPALIVE_US;
        assert_eq!(
            ctrl.evaluate_decision(false, 0.0, t_keepalive),
            ScreenSquelchDecision::SendFrame
        );
        assert_eq!(ctrl.stats().keepalive_frames, 1);
        assert_eq!(ctrl.stats().frames_passed, 3);
        assert!(ctrl.stats().is_squelched);

        // User input or force_wake awakens immediately
        ctrl.force_wake(t_keepalive + 10_000);
        assert!(!ctrl.stats().is_squelched);
        assert_eq!(
            ctrl.evaluate_decision(false, 0.0, t_keepalive + 10_000),
            ScreenSquelchDecision::SendFrame
        );

        // 3. Aggressive mode squelches after 150ms
        let mut agg = ScreenSquelchController::new(ScreenSquelchMode::Aggressive);
        assert_eq!(
            agg.evaluate_decision(true, 1.0, 100_000),
            ScreenSquelchDecision::SendFrame
        );
        assert_eq!(
            agg.evaluate_decision(false, 0.0, 100_000 + 100_000),
            ScreenSquelchDecision::SendFrame
        );
        // Squelched at 150ms
        assert_eq!(
            agg.evaluate_decision(false, 0.0, 100_000 + 150_000),
            ScreenSquelchDecision::SquelchSkip
        );
        assert!(agg.stats().is_squelched);

        // 4. Disabled mode never squelches
        let mut dis = ScreenSquelchController::new(ScreenSquelchMode::Disabled);
        assert_eq!(
            dis.evaluate_decision(true, 1.0, 100_000),
            ScreenSquelchDecision::SendFrame
        );
        assert_eq!(
            dis.evaluate_decision(false, 0.0, 100_000 + 600_000),
            ScreenSquelchDecision::SendFrame
        );
        assert!(!dis.stats().is_squelched);
    }

    #[test]
    fn screen_sender_squelch_integration() {
        let mut s = ScreenSender::new();
        assert_eq!(s.squelch_mode(), ScreenSquelchMode::Auto);
        assert_eq!(s.stats().squelch_mode, ScreenSquelchMode::Auto.tag());

        // Mode mutation
        s.set_squelch_mode(ScreenSquelchMode::Aggressive);
        assert_eq!(s.squelch_mode(), ScreenSquelchMode::Aggressive);
        assert_eq!(s.stats().squelch_mode, ScreenSquelchMode::Aggressive.tag());

        s.set_squelch_mode(ScreenSquelchMode::Auto);
        s.on_peer_connected();
        s.allow();
        s.on_control(ControlMsg::Start).unwrap();
        assert!(s.state().is_streaming());

        let rect = ScreenRect::new(0, 0, 64, 64).unwrap();
        let frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 64 * 64 * 4]).unwrap();

        // 1. Initial dirty push
        let seq = s.push_with_damage_time(frame.clone(), true, 1.0, 1_000_000);
        assert_eq!(seq, Some(1));
        assert_eq!(s.stats().sent, 1);
        assert!(!s.stats().is_squelched);

        // 2. Static push within 500ms
        let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_200_000);
        assert_eq!(seq, Some(2));
        assert_eq!(s.stats().sent, 2);

        // 3. Static push past 500ms -> Squelch skips
        let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_600_000);
        assert_eq!(seq, None);
        assert_eq!(s.stats().sent, 2);
        assert!(s.stats().is_squelched);
        assert_eq!(s.stats().squelched_frames, 1);

        // 4. On-demand IDR keyframe wakes squelch
        s.on_control_with_time(ControlMsg::KeyframeRequest, 1_605_000)
            .unwrap();
        assert!(!s.stats().is_squelched);
        let seq = s.push_with_damage_time(frame.clone(), false, 0.0, 1_610_000);
        assert_eq!(seq, Some(3));
    }

    #[test]
    fn gf_arithmetic_and_cauchy_matrix() {
        // 1. Inverses over GF(2^8)
        for a in 1..=255u8 {
            let inv = gf_inv(a);
            assert_ne!(inv, 0);
            assert_eq!(gf_mul(a, inv), 1, "gf_mul failed for a={a}");
        }
        assert_eq!(gf_inv(0), 0);

        // 2. Division
        assert_eq!(gf_div(100, 100), 1);
        assert_eq!(gf_div(42, 1), 42);
        assert_eq!(gf_div(0, 50), 0);

        // 3. Cauchy matrix non-zero and invertible submatrices
        let k = 4;
        let m = 2;
        for r in 0..m {
            for c in 0..k {
                let coeff = cauchy_coeff(r, c, k);
                assert_ne!(coeff, 0);
            }
        }
    }

    #[test]
    fn screen_fec_encoder_decoder_recovery() {
        let frame_seq = 42;
        let block_id = 1;
        let data_shards = vec![
            b"Hello World - Shard 0 payload".to_vec(),
            b"Second buffer of variable length 12345".to_vec(),
            b"Short".to_vec(),
            b"The fourth and final data shard in this FEC test block!".to_vec(),
        ];
        let k = data_shards.len(); // 4
        let m = 2; // 2 parity shards (can tolerate any 2 lost shards)

        let shards = ScreenFecEncoder::encode_block(frame_seq, block_id, &data_shards, m).unwrap();
        assert_eq!(shards.len(), k + m); // 6 shards total

        // Case A: Zero loss (all data shards present)
        let dec_a = ScreenFecDecoder::decode_block(&shards[0..k]).unwrap();
        assert_eq!(dec_a, Some(data_shards.clone()));

        // Case B: 1 data shard lost (shard 1 missing, replaced by parity shard 0)
        let loss_1 = vec![
            shards[0].clone(),
            shards[2].clone(),
            shards[3].clone(),
            shards[4].clone(), // parity 0
        ];
        let dec_b = ScreenFecDecoder::decode_block(&loss_1).unwrap();
        assert_eq!(dec_b, Some(data_shards.clone()));

        // Case C: 2 data shards lost (shards 0 and 3 missing, parity 0 and 1 present)
        let loss_2 = vec![
            shards[1].clone(),
            shards[2].clone(),
            shards[4].clone(), // parity 0
            shards[5].clone(), // parity 1
        ];
        let dec_c = ScreenFecDecoder::decode_block(&loss_2).unwrap();
        assert_eq!(dec_c, Some(data_shards.clone()));

        // Case D: 3 data shards lost (only 3 total shards received, need 4) -> unrecoverable (None)
        let loss_3 = vec![shards[1].clone(), shards[4].clone(), shards[5].clone()];
        let dec_d = ScreenFecDecoder::decode_block(&loss_3).unwrap();
        assert_eq!(dec_d, None);
    }

    #[test]
    fn screen_fec_shard_wire_codec() {
        let shard = ScreenFecShard {
            frame_seq: 12345678,
            block_id: 7,
            k_data_shards: 4,
            m_parity_shards: 2,
            shard_index: 1,
            is_parity: true,
            data: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02],
        };

        let inner = shard.to_inner_frame();
        assert_eq!(inner.ty, TYPE_SCREEN_FEC);

        let decoded = ScreenFecShard::from_inner_frame(&inner).unwrap();
        assert_eq!(decoded, shard);
    }

    #[test]
    fn screen_fec_controller_redundancy_adaptation() {
        // 1. Parsing & Modes
        assert_eq!(
            "auto".parse::<ScreenFecMode>().unwrap(),
            ScreenFecMode::Auto
        );
        assert_eq!(
            "conservative".parse::<ScreenFecMode>().unwrap(),
            ScreenFecMode::Conservative
        );
        assert_eq!(
            "aggressive".parse::<ScreenFecMode>().unwrap(),
            ScreenFecMode::Aggressive
        );
        assert_eq!(
            "disabled".parse::<ScreenFecMode>().unwrap(),
            ScreenFecMode::Disabled
        );

        // 2. Disabled mode
        let mut ctrl = ScreenFecController::new(ScreenFecMode::Disabled);
        assert_eq!(ctrl.calculate_parity_shards(4, 0.10), 0);
        assert_eq!(ctrl.stats().redundancy_pct, 0);

        // 3. Conservative mode (10%)
        let mut cons = ScreenFecController::new(ScreenFecMode::Conservative);
        assert_eq!(cons.calculate_parity_shards(10, 0.0), 1);
        assert_eq!(cons.stats().redundancy_pct, 10);

        // 4. Aggressive mode (25%)
        let mut agg = ScreenFecController::new(ScreenFecMode::Aggressive);
        assert_eq!(agg.calculate_parity_shards(8, 0.0), 2);
        assert_eq!(agg.stats().redundancy_pct, 25);

        // 5. Auto mode adaptation
        let mut auto = ScreenFecController::new(ScreenFecMode::Auto);
        // < 0.5% loss -> 0% redundancy
        assert_eq!(auto.calculate_parity_shards(10, 0.002), 0);
        // 0.5%~3% loss -> 10% redundancy
        assert_eq!(auto.calculate_parity_shards(10, 0.015), 1);
        // 3%~8% loss -> 20% redundancy
        assert_eq!(auto.calculate_parity_shards(10, 0.05), 2);
        // >= 8% loss -> 30% redundancy
        assert_eq!(auto.calculate_parity_shards(10, 0.12), 3);
    }

    #[test]
    fn screen_fec_assembler_and_sender_receiver_flow() {
        let mut sender = ScreenSender::new();
        sender.set_fec_mode(ScreenFecMode::Aggressive);
        sender.on_peer_connected();
        sender.allow();
        sender.on_control(ControlMsg::Start).unwrap();

        let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
        let frame =
            ScreenFrame::raw(10, rect, PixelFormat::Bgra8, vec![0x77; 100 * 100 * 4]).unwrap();

        // Encode frame to FEC inner frames
        let inners = sender.encode_frame_to_inner_frames(&frame, 0.0).unwrap();
        assert!(!inners.is_empty());
        assert_eq!(inners[0].ty, TYPE_SCREEN_FEC);

        let shards: Vec<ScreenFecShard> = inners
            .iter()
            .map(|f| ScreenFecShard::from_inner_frame(f).unwrap())
            .collect();

        // Receiver assembly with 1 dropped data shard
        let mut receiver = ScreenReceiver::new();
        assert_eq!(receiver.stats().fec_recovered_shards, 0);

        let mut reconstructed_frame = None;
        for (i, shard) in shards.into_iter().enumerate() {
            // Intentionally skip shard 0 (simulate network loss)
            if i == 0 {
                continue;
            }
            if let Some(f) = receiver.on_fec_shard(shard).unwrap() {
                reconstructed_frame = Some(f);
                break;
            }
        }

        assert!(reconstructed_frame.is_some());
        let rf = reconstructed_frame.unwrap();
        assert_eq!(rf.seq, frame.seq);
        assert_eq!(rf.bytes.len(), frame.bytes.len());
        assert_eq!(receiver.stats().fec_recovered_shards, 1);
    }

    #[test]
    fn screen_packet_pacer_burst_smoothing_and_modes() {
        // 1. Parsing and serialization
        assert_eq!(
            "auto".parse::<ScreenPacerMode>().unwrap(),
            ScreenPacerMode::Auto
        );
        assert_eq!(
            "aggressive".parse::<ScreenPacerMode>().unwrap(),
            ScreenPacerMode::Aggressive
        );
        assert_eq!(
            "agg".parse::<ScreenPacerMode>().unwrap(),
            ScreenPacerMode::Aggressive
        );
        assert_eq!(
            "disabled".parse::<ScreenPacerMode>().unwrap(),
            ScreenPacerMode::Disabled
        );
        assert_eq!(
            "off".parse::<ScreenPacerMode>().unwrap(),
            ScreenPacerMode::Disabled
        );
        assert!("invalid".parse::<ScreenPacerMode>().is_err());
        assert_eq!(ScreenPacerMode::from_tag(0), ScreenPacerMode::Auto);
        assert_eq!(ScreenPacerMode::from_tag(1), ScreenPacerMode::Aggressive);
        assert_eq!(ScreenPacerMode::from_tag(2), ScreenPacerMode::Disabled);

        // 2. Auto mode pacing (burst limit = 4, target 30 FPS -> frame_duration 33333 us, budget = 26666 us)
        let mut pacer = ScreenPacketPacer::new(ScreenPacerMode::Auto);
        assert_eq!(pacer.mode(), ScreenPacerMode::Auto);

        // Under burst limit: 3 packets -> all zero delay
        let delays_under = pacer.compute_packet_delays(3, 30);
        assert_eq!(delays_under, vec![0, 0, 0]);
        assert_eq!(pacer.stats().packets_paced, 3);
        assert_eq!(pacer.stats().bursts_smoothed, 0);

        // Over burst limit: 8 packets -> 4 burst packets at 0, 4 paced packets with increasing delay
        let delays_over = pacer.compute_packet_delays(8, 30);
        assert_eq!(delays_over.len(), 8);
        assert_eq!(&delays_over[0..4], &[0, 0, 0, 0]);
        assert!(delays_over[4] > 0);
        assert!(delays_over[5] > delays_over[4]);
        assert!(delays_over[6] > delays_over[5]);
        assert!(delays_over[7] > delays_over[6]);
        assert!(delays_over[7] <= SCREEN_PACER_MAX_DELAY_US);
        assert_eq!(pacer.stats().bursts_smoothed, 1);
        assert_eq!(pacer.stats().packets_paced, 11);
        assert!(pacer.stats().avg_delay_us > 0);

        // 3. Aggressive mode (burst limit = 2)
        pacer.set_mode(ScreenPacerMode::Aggressive);
        let agg_delays = pacer.compute_packet_delays(5, 60);
        assert_eq!(agg_delays.len(), 5);
        assert_eq!(&agg_delays[0..2], &[0, 0]);
        assert!(agg_delays[2] > 0);
        assert!(agg_delays[3] > agg_delays[2]);
        assert!(agg_delays[4] > agg_delays[3]);

        // 4. Disabled mode -> all 0
        pacer.set_mode(ScreenPacerMode::Disabled);
        let dis_delays = pacer.compute_packet_delays(10, 30);
        assert_eq!(dis_delays, vec![0; 10]);

        // 5. Sender integration test
        let mut sender = ScreenSender::new();
        assert_eq!(sender.pacer_mode(), ScreenPacerMode::Auto);
        let sender_delays = sender.compute_packet_delays(6);
        assert_eq!(sender_delays.len(), 6);
        assert_eq!(&sender_delays[0..4], &[0, 0, 0, 0]);
        assert!(sender_delays[4] > 0);
        assert_eq!(sender.stats().pacer_mode, ScreenPacerMode::Auto.tag());
        assert_eq!(sender.stats().pacer_bursts_smoothed, 1);
        assert_eq!(sender.stats().pacer_packets_paced, 6);
    }

    #[test]
    fn screen_trendline_filter_bufferbloat_detection_and_modes() {
        // 1. Parsing and enum tags
        assert_eq!(
            "auto".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Auto
        );
        assert_eq!(
            "aggressive".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Aggressive
        );
        assert_eq!(
            "agg".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Aggressive
        );
        assert_eq!(
            "conservative".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Conservative
        );
        assert_eq!(
            "cons".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Conservative
        );
        assert_eq!(
            "disabled".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Disabled
        );
        assert_eq!(
            "off".parse::<ScreenTrendlineMode>().unwrap(),
            ScreenTrendlineMode::Disabled
        );
        assert!("unknown".parse::<ScreenTrendlineMode>().is_err());

        assert_eq!(ScreenTrendlineMode::from_tag(0), ScreenTrendlineMode::Auto);
        assert_eq!(
            ScreenTrendlineMode::from_tag(1),
            ScreenTrendlineMode::Aggressive
        );
        assert_eq!(
            ScreenTrendlineMode::from_tag(2),
            ScreenTrendlineMode::Conservative
        );
        assert_eq!(
            ScreenTrendlineMode::from_tag(3),
            ScreenTrendlineMode::Disabled
        );

        assert_eq!(
            ScreenBandwidthUsageState::from_tag(0),
            ScreenBandwidthUsageState::Normal
        );
        assert_eq!(
            ScreenBandwidthUsageState::from_tag(1),
            ScreenBandwidthUsageState::Overusing
        );
        assert_eq!(
            ScreenBandwidthUsageState::from_tag(2),
            ScreenBandwidthUsageState::Underusing
        );
        assert_eq!(ScreenBandwidthUsageState::Normal.as_str(), "normal");
        assert_eq!(ScreenBandwidthUsageState::Overusing.as_str(), "overusing");
        assert_eq!(ScreenBandwidthUsageState::Underusing.as_str(), "underusing");

        // 2. Linear regression & constant delay -> normal state, slope ~ 0
        let mut filter = ScreenTrendlineFilter::new(ScreenTrendlineMode::Auto);
        assert_eq!(filter.mode(), ScreenTrendlineMode::Auto);
        assert_eq!(filter.state(), ScreenBandwidthUsageState::Normal);

        let frame_interval_us = 33_333; // ~30 fps
        let mut send_ts = 1_000_000;
        let rtt = 30_000; // 30ms

        for _ in 0..15 {
            let recv_ts = send_ts + rtt;
            let state = filter.on_ack(send_ts, recv_ts, Some(rtt));
            assert_eq!(state, ScreenBandwidthUsageState::Normal);
            send_ts += frame_interval_us;
        }
        assert_eq!(filter.stats().overuse_count, 0);
        assert_eq!(filter.stats().underuse_count, 0);
        assert!(filter.stats().current_slope_us_per_frame.abs() < 1.0);

        // 3. Bufferbloat queue buildup: increasing delay (+200 us per frame) -> triggers Overusing
        let mut queue_delay = rtt;
        let mut reached_overuse = false;
        for _ in 0..20 {
            queue_delay += 200;
            let recv_ts = send_ts + queue_delay;
            let state = filter.on_ack(send_ts, recv_ts, Some(queue_delay));
            if state == ScreenBandwidthUsageState::Overusing {
                reached_overuse = true;
            }
            send_ts += frame_interval_us;
        }
        assert!(reached_overuse);
        assert_eq!(filter.state(), ScreenBandwidthUsageState::Overusing);
        assert!(filter.stats().overuse_count >= 1);
        assert!(filter.stats().current_slope_us_per_frame > 10.0);

        // 4. Queue draining: decreasing delay (-200 us per frame) -> triggers Underusing
        let mut reached_underuse = false;
        for _ in 0..25 {
            queue_delay = queue_delay.saturating_sub(200);
            let recv_ts = send_ts + queue_delay;
            let state = filter.on_ack(send_ts, recv_ts, Some(queue_delay));
            if state == ScreenBandwidthUsageState::Underusing {
                reached_underuse = true;
            }
            send_ts += frame_interval_us;
        }
        assert!(reached_underuse);
        assert!(filter.stats().underuse_count >= 1);
        assert!(filter.stats().current_slope_us_per_frame < -10.0);

        // 5. Threshold adaptation across modes
        let mut filter_auto = ScreenTrendlineFilter::new(ScreenTrendlineMode::Auto);
        // Low RTT (50ms): factor clamp 0.5 -> threshold = 12.5 * 0.5 = 6.25
        filter_auto.on_ack(1_000_000, 1_050_000, Some(50_000));
        assert!((filter_auto.stats().threshold_us_per_frame - 6.25).abs() < 0.01);
        // High RTT (200ms): factor 2.0 -> threshold = 12.5 * 2.0 = 25.0
        filter_auto.on_ack(2_000_000, 2_200_000, Some(200_000));
        assert!((filter_auto.stats().threshold_us_per_frame - 25.0).abs() < 0.01);

        let mut filter_agg = ScreenTrendlineFilter::new(ScreenTrendlineMode::Aggressive);
        filter_agg.on_ack(1_000_000, 1_050_000, Some(50_000));
        assert!((filter_agg.stats().threshold_us_per_frame - 6.0).abs() < 0.01);

        let mut filter_cons = ScreenTrendlineFilter::new(ScreenTrendlineMode::Conservative);
        filter_cons.on_ack(1_000_000, 1_050_000, Some(50_000));
        assert!((filter_cons.stats().threshold_us_per_frame - 25.0).abs() < 0.01);

        let mut filter_dis = ScreenTrendlineFilter::new(ScreenTrendlineMode::Disabled);
        let dis_state = filter_dis.on_ack(1_000_000, 1_500_000, Some(500_000));
        assert_eq!(dis_state, ScreenBandwidthUsageState::Normal);

        // 6. Sender integration test
        let mut sender = ScreenSender::new();
        assert_eq!(sender.trendline_mode(), ScreenTrendlineMode::Auto);
        sender.set_trendline_mode(ScreenTrendlineMode::Aggressive);
        assert_eq!(sender.trendline_mode(), ScreenTrendlineMode::Aggressive);
        assert_eq!(
            sender.stats().trendline_mode,
            ScreenTrendlineMode::Aggressive.tag()
        );
        assert_eq!(
            sender.stats().trendline_state,
            ScreenBandwidthUsageState::Normal.tag()
        );
    }
}
