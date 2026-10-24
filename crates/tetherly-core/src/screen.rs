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
use std::collections::VecDeque;

/// Screen channel inner types. Not listed in the JSON control-plane table.
pub const TYPE_SCREEN_FRAME: u16 = 0x0501;
pub const TYPE_SCREEN_CONTROL: u16 = 0x0502;

pub const SCREEN_MAGIC: [u8; 4] = *b"TMV1";
pub const SCREEN_PROTO: u8 = 1;

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
    matches!(ty, TYPE_SCREEN_FRAME | TYPE_SCREEN_CONTROL)
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
}

impl ScreenStats {
    /// Active screen content profile resolved from profile tag.
    pub fn content_profile(&self) -> ScreenContentProfile {
        ScreenContentProfile::from_tag(self.profile_tag)
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
        let stats = ScreenStats {
            bitrate_bps: rate_controller.current_bps(),
            fps_target: fps_pacer.fps_target(),
            scale_pct: scale_controller.scale_pct(),
            profile_tag: quality_controller.profile().tag(),
            qp_min: quality_controller.min_qp(),
            qp_max: quality_controller.max_qp(),
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
        }
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
        self.stats
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
                    self.rate_controller
                        .on_ack(rtt, self.in_flight_records.len() as u64, now_us);
                    self.stats.bitrate_bps = self.rate_controller.current_bps();
                }
                self.stats.in_flight = self.in_flight_records.len() as u64;
                Ok(None)
            }
            ControlMsg::KeyframeRequest => {
                self.stats.keyframes_requested = self.stats.keyframes_requested.saturating_add(1);
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

    /// Record a frame dropped on sender side (e.g. due to congestion or backpressure).
    pub fn record_drop(&mut self) {
        self.record_drop_with_time(0);
    }

    /// Record a frame dropped on sender side with timestamp for rate adaptation cooldown.
    pub fn record_drop_with_time(&mut self, now_us: u64) {
        self.stats.dropped = self.stats.dropped.saturating_add(1);
        self.rate_controller.on_congestion(now_us);
        self.stats.bitrate_bps = self.rate_controller.current_bps();
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

    /// Check if target frame pacing interval has elapsed.
    pub fn is_pacing_ready(&self, now_us: u64) -> bool {
        self.fps_pacer.is_ready(now_us)
    }

    /// Check and consume a pacing tick if target frame pacing interval has elapsed.
    pub fn consume_pacing(&mut self, now_us: u64) -> bool {
        self.fps_pacer.check_and_consume(now_us)
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
    }
}

/// Receiver side. Enforces the sequence window and never presents a frame it
/// could not fully validate.
pub struct ScreenReceiver {
    last_seq: u64,
    stats: ScreenStats,
    trace: VecDeque<u64>,
}

impl Default for ScreenReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenReceiver {
    pub fn new() -> Self {
        Self {
            last_seq: 0,
            stats: ScreenStats::default(),
            trace: VecDeque::with_capacity(SCREEN_TRACE_MAX),
        }
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn stats(&self) -> ScreenStats {
        self.stats
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
}
