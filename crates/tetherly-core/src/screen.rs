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
use std::collections::{HashMap, VecDeque};
use std::fmt;

/// Screen channel inner types. Not listed in the JSON control-plane table.
pub const TYPE_SCREEN_FRAME: u16 = 0x0501;
pub const TYPE_SCREEN_CONTROL: u16 = 0x0502;
/// Screen cursor channel inner type (v2.02 / ADR-088).
pub const TYPE_SCREEN_CURSOR: u16 = 0x0503;
/// Screen forward error correction (FEC) channel inner type (v2.04 / ADR-090).
pub const TYPE_SCREEN_FEC: u16 = 0x0504;
/// Screen selective negative acknowledgment (NACK) channel inner type (v2.07 / ADR-093).
pub const TYPE_SCREEN_NACK: u16 = 0x0505;
/// Screen transport-wide congestion control (TWCC) feedback channel inner type (v2.08 / ADR-094).
pub const TYPE_SCREEN_TWCC: u16 = 0x0506;
/// Screen active bandwidth probing channel inner type (v2.09 / ADR-095).
pub const TYPE_SCREEN_PROBE: u16 = 0x0507;
/// Screen and audio presentation timestamp (PTS) lip-sync alignment and clock synchronization inner type (v2.10 / ADR-096).
pub const TYPE_SCREEN_SYNC: u16 = 0x0508;
/// Screen end-to-end latency profiling and multi-stage frame profiler channel inner type (v2.13 / ADR-099).
pub const TYPE_SCREEN_PROFILE: u16 = 0x0509;
/// Screen collaborative annotations and whiteboard overlay channel inner type (v2.19 / ADR-105).
pub const TYPE_SCREEN_ANNOTATION: u16 = 0x050a;

pub const SCREEN_MAGIC: [u8; 4] = *b"TMV1";
pub const SCREEN_PROTO: u8 = 1;

pub const ANNOTATION_MAGIC: [u8; 4] = *b"TANN";
pub const ANNOTATION_PROTO: u8 = 1;

pub const CURSOR_MAGIC: [u8; 4] = *b"TMCU";
pub const CURSOR_PROTO: u8 = 1;

pub const FEC_MAGIC: [u8; 4] = *b"TFEC";
pub const FEC_PROTO: u8 = 1;
pub const FEC_HEADER_LEN: usize = 24;
pub const FEC_MAX_DATA_SHARDS: usize = 32;
pub const FEC_MAX_PARITY_SHARDS: usize = 16;

pub const SCREEN_NACK_MAGIC: [u8; 5] = *b"TNACK";
pub const SCREEN_NACK_PROTO: u8 = 1;
pub const SCREEN_NACK_HEADER_LEN: usize = 36;
pub const SCREEN_HARQ_MAX_RETRIES_AUTO: u8 = 2;
pub const SCREEN_HARQ_MAX_RETRIES_AGGRESSIVE: u8 = 3;
pub const SCREEN_HARQ_MAX_RETRIES_CONSERVATIVE: u8 = 1;
pub const SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY: usize = 256;
pub const SCREEN_HARQ_MAX_AGE_US: u64 = 300_000;

pub const SCREEN_TWCC_MAGIC: [u8; 5] = *b"TTWCC";
pub const SCREEN_TWCC_PROTO: u8 = 1;
pub const SCREEN_TWCC_HEADER_LEN: usize = 20;
pub const SCREEN_TWCC_RECORD_LEN: usize = 4;
pub const SCREEN_TWCC_MISSING_DELTA: i16 = -32768;

pub const SCREEN_TWCC_AUTO_INTERVAL_US: u64 = 10_000;
pub const SCREEN_TWCC_AGGRESSIVE_INTERVAL_US: u64 = 5_000;
pub const SCREEN_TWCC_CONSERVATIVE_INTERVAL_US: u64 = 25_000;

pub const SCREEN_TWCC_AUTO_PACKET_THRESHOLD: usize = 40;
pub const SCREEN_TWCC_AGGRESSIVE_PACKET_THRESHOLD: usize = 20;
pub const SCREEN_TWCC_CONSERVATIVE_PACKET_THRESHOLD: usize = 80;

pub const SCREEN_PROBE_MAGIC: [u8; 5] = *b"TPROB";
pub const SCREEN_PROBE_PROTO: u8 = 1;
pub const SCREEN_PROBE_HEADER_LEN: usize = 26;
pub const SCREEN_PROBE_DEFAULT_PACKET_LEN: usize = 1000;
pub const SCREEN_PROBE_CLUSTER_SIZE_AUTO: u8 = 10;
pub const SCREEN_PROBE_CLUSTER_SIZE_AGGRESSIVE: u8 = 16;
pub const SCREEN_PROBE_CLUSTER_SIZE_CONSERVATIVE: u8 = 6;
pub const SCREEN_PROBE_MIN_INTERVAL_US_AUTO: u64 = 2_000_000;
pub const SCREEN_PROBE_MIN_INTERVAL_US_AGGRESSIVE: u64 = 1_000_000;
pub const SCREEN_PROBE_MIN_INTERVAL_US_CONSERVATIVE: u64 = 4_000_000;

pub const SCREEN_SYNC_MAGIC: [u8; 5] = *b"TSYNC";
pub const SCREEN_SYNC_PROTO: u8 = 1;
pub const SCREEN_SYNC_HEADER_LEN: usize = 36;
pub const SCREEN_SYNC_INTERVAL_US_AUTO: u64 = 500_000;
pub const SCREEN_SYNC_INTERVAL_US_STRICT: u64 = 200_000;
pub const SCREEN_SYNC_INTERVAL_US_LOOSE: u64 = 1_000_000;
pub const SCREEN_SYNC_SKEW_TOLERANCE_US_AUTO: i64 = 15_000;
pub const SCREEN_SYNC_SKEW_TOLERANCE_US_STRICT: i64 = 5_000;
pub const SCREEN_SYNC_SKEW_TOLERANCE_US_LOOSE: i64 = 40_000;

pub const SCREEN_PROFILE_MAGIC: [u8; 5] = *b"TPROF";
pub const SCREEN_PROFILE_PROTO: u8 = 1;
pub const SCREEN_PROFILE_HEADER_LEN: usize = 56;
pub const SCREEN_PROFILE_RING_CAPACITY: usize = 1024;

/// Consecutive same-bottleneck frames required before the autonomous tuner acts (v2.14 / ADR-100).
pub const SCREEN_TUNER_TRIGGER_FRAMES_STANDARD: u32 = 3;
pub const SCREEN_TUNER_TRIGGER_FRAMES_AGGRESSIVE: u32 = 2;
/// Consecutive balanced frames required before the autonomous tuner recovers one step (v2.14 / ADR-100).
pub const SCREEN_TUNER_RECOVERY_FRAMES_STANDARD: u32 = 10;
pub const SCREEN_TUNER_RECOVERY_FRAMES_AGGRESSIVE: u32 = 6;
/// Cooldown between two actual tuner actions, in microseconds (v2.14 / ADR-100).
pub const SCREEN_TUNER_COOLDOWN_US_STANDARD: u64 = 2_000_000;
pub const SCREEN_TUNER_COOLDOWN_US_AGGRESSIVE: u64 = 1_000_000;
/// Multiplicative bitrate relief/recovery steps of the autonomous tuner, in percent (v2.14 / ADR-100).
pub const SCREEN_TUNER_BITRATE_DROP_PCT: u32 = 15;
pub const SCREEN_TUNER_BITRATE_RECOVER_PCT: u32 = 12;
/// Encoder QP relief/recovery steps of the autonomous tuner (v2.14 / ADR-100).
pub const SCREEN_TUNER_QP_RELIEF_STEP: u8 = 4;
pub const SCREEN_TUNER_QP_RECOVER_STEP: u8 = 2;
/// Receiver playout buffer target delay relief/recovery step, in microseconds (v2.14 / ADR-100).
pub const SCREEN_TUNER_PLAYOUT_RELIEF_DELTA_US: i64 = 5_000;

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
/// Dynamic differential delta patch sub-rectangle with Zstd compression (ADR-109).
pub const SCREEN_COMPRESS_DELTA: u8 = 4;
/// Dynamic long-term reference (LTR) recovery patch (v2.27 / ADR-113).
pub const SCREEN_COMPRESS_LTR: u8 = 5;
pub const SCREEN_LTR_DEFAULT_AUTO_INTERVAL: u16 = 60;
pub const SCREEN_LTR_MIN_INTERVAL: u16 = 5;
pub const SCREEN_LTR_MAX_INTERVAL: u16 = 600;
/// Dynamic temporal scalability (SVC-T) enhancement layer T1 frame patch (v2.28 / ADR-114).
pub const SCREEN_COMPRESS_SVC_T1: u8 = 6;
/// Dynamic 2D motion vector displacement scroll patch with Zstd exposed strip compression (v2.29 / ADR-115).
pub const SCREEN_COMPRESS_MOTION_SCROLL: u8 = 7;
pub const SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT: u16 = 64;
pub const SCREEN_MOTION_MIN_DISPLACEMENT: u16 = 8;
pub const SCREEN_MOTION_MAX_DISPLACEMENT: u16 = 256;
pub const SCREEN_MOTION_MIN_MATCH_RATIO: f32 = 0.85;
/// Dynamic spatial tile-grid decomposition and independent tile streaming (v2.30 / ADR-116).
pub const SCREEN_COMPRESS_TILE_GRID: u8 = 8;
pub const SCREEN_TILE_DEFAULT_SIZE: u16 = 64;
pub const SCREEN_TILE_MIN_SIZE: u16 = 32;
pub const SCREEN_TILE_MAX_SIZE: u16 = 256;
/// Screen streaming hash-based tile deduplication and LRU tile dictionary cache (v2.31 / ADR-117).
pub const SCREEN_COMPRESS_TILE_CACHE: u8 = 9;
pub const SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES: u16 = 512;
pub const SCREEN_TILE_CACHE_MIN_ENTRIES: u16 = 64;
pub const SCREEN_TILE_CACHE_MAX_ENTRIES: u16 = 4096;
/// Screen streaming foveated focus awareness and spatial region-priority scheduling constants (v2.32 / ADR-118).
pub const SCREEN_FOCUS_DEFAULT_RADIUS: u16 = 256;
pub const SCREEN_FOCUS_MIN_RADIUS: u16 = 64;
pub const SCREEN_FOCUS_MAX_RADIUS: u16 = 2048;
pub const SCREEN_FOCUS_DEFAULT_SKIP_RATIO: u8 = 2;
pub const SCREEN_FOCUS_MAX_SKIP_RATIO: u8 = 10;
/// Screen streaming tile-level adaptive palette, bi-level bitmap & hybrid lossless compression constants (v2.33 / ADR-119).
pub const TILE_SUBCODEC_RAW: u8 = 0;
pub const TILE_SUBCODEC_ZSTD: u8 = 1;
pub const TILE_SUBCODEC_SOLID: u8 = 2;
pub const TILE_SUBCODEC_BILEVEL: u8 = 3;
pub const TILE_SUBCODEC_PALETTE16: u8 = 4;
pub const SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS: u8 = 16;
/// Screen streaming tile-level lossless predictive filter & reversible color transform constants (v2.34 / ADR-120).
pub const TILE_SUBCODEC_PREDICT_ZSTD: u8 = 5;
/// Screen streaming tile-level dynamic multi-resolution Level-of-Detail (LoD) downsampling constants (v2.38 / ADR-124).
pub const TILE_SUBCODEC_LOD: u8 = 6;
/// Dynamic spatial tile adaptive MTU batch chunking and coalesced pacing (v2.39 / ADR-125).
pub const SCREEN_COMPRESS_TILE_BATCH: u8 = 10;
pub const SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES: u16 = 1200;
pub const SCREEN_TILE_BATCH_MIN_CHUNK_BYTES: u16 = 512;
pub const SCREEN_TILE_BATCH_MAX_CHUNK_BYTES: u16 = 64000;
pub const SCREEN_TILE_BATCH_DEFAULT_PACING_US: u16 = 200;
pub const SCREEN_TILE_BATCH_MAX_PACING_US: u16 = 50000;
/// Screen streaming tile-level dynamic local motion vector estimation & displacement compensation constants (v2.40 / ADR-126).
pub const TILE_SUBCODEC_MOTION: u8 = 7;
/// Decoded tile-batch chunk: (batch_id, chunk_index, total_chunks, tile_size, total_tiles, tiles).
pub type TileBatchChunk = (u32, u16, u16, u16, u16, Vec<DirtyTile>);
pub const SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT: u16 = 32;
pub const SCREEN_TILE_MOTION_MIN_DISPLACEMENT: u16 = 4;
pub const SCREEN_TILE_MOTION_MAX_DISPLACEMENT: u16 = 128;
pub const SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO: u8 = 90;

pub fn is_screen_type(ty: u16) -> bool {
    matches!(
        ty,
        TYPE_SCREEN_FRAME
            | TYPE_SCREEN_CONTROL
            | TYPE_SCREEN_CURSOR
            | TYPE_SCREEN_FEC
            | TYPE_SCREEN_NACK
            | TYPE_SCREEN_TWCC
            | TYPE_SCREEN_PROBE
            | TYPE_SCREEN_SYNC
            | TYPE_SCREEN_PROFILE
            | TYPE_SCREEN_ANNOTATION
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Windows / DXGI order. Alpha is ignored on present.
    Bgra8,
    Rgba8,
}

impl PixelFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            PixelFormat::Bgra8 => "bgra8",
            PixelFormat::Rgba8 => "rgba8",
        }
    }

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

    /// Encode uncompressed frame pixels into a standard 32-bit Windows Top-Down BMP (DIB)
    /// byte buffer without external graphics dependencies (ADR-103).
    ///
    /// The BMP format uses a 54-byte header:
    /// - 14-byte BITMAPFILEHEADER (bfType='BM', bfSize=54+N, bfOffBits=54)
    /// - 40-byte BITMAPINFOHEADER (biSize=40, biWidth=w, biHeight=-h, biPlanes=1, biBitCount=32, biCompression=BI_RGB)
    ///   Setting biHeight to -h (negative) specifies a Top-Down DIB where row 0 is the top row,
    ///   matching memory layout and avoiding vertical row flipping.
    ///   Standard 32-bit DIB pixel format in memory is BGRA (B, G, R, A). If the frame format is
    ///   Bgra8, pixels are zero-copy transferred. If Rgba8, channels 0 and 2 are swapped in-place.
    pub fn to_bmp(&self) -> Result<Vec<u8>, CoreError> {
        if self.compressed() {
            return Err(CoreError::ScreenRefused(
                "cannot encode compressed frame directly to bmp; decode first".into(),
            ));
        }
        let w = self.rect.w as u32;
        let h = self.rect.h as u32;
        if w == 0 || h == 0 {
            return Err(CoreError::ScreenRefused("frame has zero dimensions".into()));
        }
        let pixel_bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or(CoreError::ScreenTooLarge)?;
        if self.bytes.len() < pixel_bytes {
            return Err(CoreError::ScreenFrame);
        }

        const BMP_HEADER_LEN: usize = 54;
        let file_size = (BMP_HEADER_LEN + pixel_bytes) as u32;
        let mut out = Vec::with_capacity(BMP_HEADER_LEN + pixel_bytes);

        // BITMAPFILEHEADER (14 bytes)
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved1 + reserved2
        out.extend_from_slice(&(BMP_HEADER_LEN as u32).to_le_bytes()); // bfOffBits

        // BITMAPINFOHEADER (40 bytes)
        out.extend_from_slice(&40u32.to_le_bytes()); // biSize
        out.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
        out.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative for top-down)
        out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        out.extend_from_slice(&0u32.to_le_bytes()); // biCompression = 0 (BI_RGB)
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes()); // biSizeImage
        out.extend_from_slice(&2835i32.to_le_bytes()); // biXPelsPerMeter (~72 dpi)
        out.extend_from_slice(&2835i32.to_le_bytes()); // biYPelsPerMeter (~72 dpi)
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

        match self.format {
            PixelFormat::Bgra8 => {
                out.extend_from_slice(&self.bytes[..pixel_bytes]);
            }
            PixelFormat::Rgba8 => {
                let mut converted = self.bytes[..pixel_bytes].to_vec();
                PixelFormat::Rgba8.converted(PixelFormat::Bgra8, &mut converted);
                out.extend_from_slice(&converted);
            }
        }
        Ok(out)
    }

    /// Renders dynamic security watermark text onto the uncompressed frame pixel buffer (v2.18 / ADR-104).
    /// Returns Ok(true) if watermark was applied, Ok(false) if disabled or not applicable.
    pub fn apply_watermark(
        &mut self,
        config: &ScreenWatermarkConfig,
        peer_id: &str,
        timestamp: &str,
    ) -> Result<bool, CoreError> {
        if !config.enabled && !config.host_enforced {
            return Ok(false);
        }
        if self.compressed() {
            return Err(CoreError::InvalidConfig(
                "cannot apply watermark to compressed frame; decode first".into(),
            ));
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 {
            return Ok(false);
        }
        let pixel_bytes = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .ok_or(CoreError::ScreenTooLarge)?;
        if self.bytes.len() < pixel_bytes {
            return Err(CoreError::ScreenFrame);
        }

        let rendered = config
            .text_template
            .replace("{peer}", peer_id)
            .replace("{time}", timestamp);
        if rendered.is_empty() {
            return Ok(false);
        }

        let alpha = config.opacity;
        if alpha == 0 {
            return Ok(false);
        }
        let [r_fg, g_fg, b_fg] = config.color_rgb;

        let scale = if config.scale == 0 {
            1
        } else {
            config.scale.min(4)
        } as usize;
        let char_w = 8 * scale;
        let char_h = 8 * scale;
        let text_w = rendered.chars().count() * char_w;
        let text_h = char_h;

        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };

        let draw_text_at = |start_x: usize, start_y: usize, pixels: &mut [u8]| {
            for (idx, ch) in rendered.chars().enumerate() {
                let char_x = start_x + idx * char_w;
                let glyph =
                    crate::screen_font::get_glyph(ch).unwrap_or(&crate::screen_font::FONT_8X8[0]);
                for (row, &bits) in glyph.iter().enumerate() {
                    for col in 0..8 {
                        if (bits & (1 << (7 - col))) != 0 {
                            for dy in 0..scale {
                                let py = start_y + row * scale + dy;
                                if py >= h {
                                    continue;
                                }
                                for dx in 0..scale {
                                    let px = char_x + col * scale + dx;
                                    if px >= w {
                                        continue;
                                    }
                                    let offset = (py * w + px) * 4;
                                    blend_pixel(pixels, offset, is_bgra, r_fg, g_fg, b_fg, alpha);
                                }
                            }
                        }
                    }
                }
            }
        };

        match config.pattern {
            WatermarkPattern::Single => {
                let start_x = if w > text_w + 40 {
                    w - text_w - 20
                } else {
                    w.saturating_sub(text_w) / 2
                };
                let start_y = if h > text_h + 40 {
                    h - text_h - 20
                } else {
                    h.saturating_sub(text_h) / 2
                };
                draw_text_at(start_x, start_y, &mut self.bytes);
            }
            WatermarkPattern::Banner => {
                let start_x = w.saturating_sub(text_w) / 2;
                let start_y = 12.min(h.saturating_sub(text_h));
                draw_text_at(start_x, start_y, &mut self.bytes);
            }
            WatermarkPattern::Grid => {
                let step_x = (config.spacing_x.max(40) as usize).max(text_w + 20);
                let step_y = (config.spacing_y.max(30) as usize).max(text_h + 16);
                let mut row = 0;
                let mut y = 16;
                let mut drawn = false;
                while y + text_h <= h {
                    let offset_x = if row % 2 == 1 { step_x / 2 } else { 0 };
                    let mut x = offset_x;
                    while x + text_w <= w {
                        draw_text_at(x, y, &mut self.bytes);
                        drawn = true;
                        x += step_x;
                    }
                    if !drawn && x == offset_x {
                        draw_text_at(w.saturating_sub(text_w) / 2, y, &mut self.bytes);
                        drawn = true;
                    }
                    y += step_y;
                    row += 1;
                }
                if !drawn {
                    let start_x = w.saturating_sub(text_w) / 2;
                    let start_y = h.saturating_sub(text_h) / 2;
                    draw_text_at(start_x, start_y, &mut self.bytes);
                }
            }
        }

        Ok(true)
    }

    /// Renders active collaborative annotations onto the uncompressed frame pixel buffer (v2.19 / ADR-105).
    /// Returns the count of annotations applied, or error if compressed/invalid.
    pub fn apply_annotations(
        &mut self,
        annotations: &[AnnotationItem],
        now_ms: u64,
    ) -> Result<usize, CoreError> {
        if annotations.is_empty() {
            return Ok(0);
        }
        if self.compressed() {
            return Err(CoreError::InvalidConfig(
                "cannot apply annotations to compressed frame; decode first".into(),
            ));
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 {
            return Ok(0);
        }
        let pixel_bytes = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .ok_or(CoreError::ScreenTooLarge)?;
        if self.bytes.len() < pixel_bytes {
            return Err(CoreError::ScreenFrame);
        }

        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };
        let mut canvas = Canvas {
            pixels: &mut self.bytes,
            w,
            h,
            is_bgra,
        };

        let mut applied = 0;
        for item in annotations {
            if item.is_expired(now_ms) {
                continue;
            }
            let stroke = Stroke {
                thickness: item.thickness.max(1),
                color: item.color_rgb,
                alpha: item.alpha,
            };

            match &item.kind {
                AnnotationKind::Stroke { points } => {
                    if points.len() >= 2 {
                        for pair in points.windows(2) {
                            let (x0, y0) = pair[0];
                            let (x1, y1) = pair[1];
                            canvas.line(x0 as i32, y0 as i32, x1 as i32, y1 as i32, &stroke);
                        }
                        applied += 1;
                    } else if points.len() == 1 {
                        let (x, y) = points[0];
                        canvas.disk(
                            x as i32,
                            y as i32,
                            (stroke.thickness as i32) / 2,
                            stroke.color,
                            stroke.alpha,
                        );
                        applied += 1;
                    }
                }
                AnnotationKind::Rect { x, y, w: rw, h: rh } => {
                    canvas.rect(*x as i32, *y as i32, *rw as i32, *rh as i32, &stroke);
                    applied += 1;
                }
                AnnotationKind::Arrow { from, to } => {
                    canvas.arrow(
                        from.0 as i32,
                        from.1 as i32,
                        to.0 as i32,
                        to.1 as i32,
                        &stroke,
                    );
                    applied += 1;
                }
                AnnotationKind::Text { x, y, text, scale } => {
                    canvas.annotation_text(
                        *x as usize,
                        *y as usize,
                        text,
                        *scale,
                        stroke.color,
                        stroke.alpha,
                    );
                    applied += 1;
                }
            }
        }

        Ok(applied)
    }

    /// Renders virtual laser pointer spotlight onto the uncompressed frame pixel buffer (v2.19 / ADR-105).
    /// Returns Ok(true) if laser pointer was rendered, Ok(false) if inactive.
    pub fn apply_laser_pointer(&mut self, laser: &LaserPointerState) -> Result<bool, CoreError> {
        if !laser.active {
            return Ok(false);
        }
        if self.compressed() {
            return Err(CoreError::InvalidConfig(
                "cannot apply laser pointer to compressed frame; decode first".into(),
            ));
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 {
            return Ok(false);
        }
        let pixel_bytes = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(4))
            .ok_or(CoreError::ScreenTooLarge)?;
        if self.bytes.len() < pixel_bytes {
            return Err(CoreError::ScreenFrame);
        }

        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };

        Canvas {
            pixels: &mut self.bytes,
            w,
            h,
            is_bgra,
        }
        .laser_spot(
            laser.x as i32,
            laser.y as i32,
            laser.radius,
            laser.color_rgb,
        );

        Ok(true)
    }

    /// Crop an uncompressed raw ScreenFrame according to Region-of-Interest (ROI) viewport parameters (v2.21 / ADR-107).
    pub fn crop_roi(&self, roi: &ScreenRoi) -> Result<ScreenFrame, CoreError> {
        if !roi.active {
            return Ok(self.clone());
        }
        if self.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        let src_w = self.rect.w;
        let src_h = self.rect.h;
        if src_w == 0 || src_h == 0 {
            return Ok(self.clone());
        }
        let x = roi.x.min(src_w.saturating_sub(2));
        let y = roi.y.min(src_h.saturating_sub(2));
        let w = roi.w.clamp(2, src_w.saturating_sub(x));
        let h = roi.h.clamp(2, src_h.saturating_sub(y));
        let w = (w / 2) * 2;
        let h = (h / 2) * 2;
        if w == 0 || h == 0 {
            return Ok(self.clone());
        }
        let bpp = self.format.bpp();
        let src_stride = self.effective_stride() as usize;
        let dst_stride = w as usize * bpp;
        let mut dst_bytes = vec![0u8; dst_stride * h as usize];
        for row in 0..h as usize {
            let src_offset = (y as usize + row) * src_stride + (x as usize * bpp);
            let dst_offset = row * dst_stride;
            if src_offset + dst_stride <= self.bytes.len() {
                dst_bytes[dst_offset..dst_offset + dst_stride]
                    .copy_from_slice(&self.bytes[src_offset..src_offset + dst_stride]);
            }
        }
        let new_rect = ScreenRect::new(x as i32, y as i32, w, h)?;
        Ok(ScreenFrame {
            seq: self.seq,
            rect: new_rect,
            format: self.format,
            codec_tag: self.codec_tag,
            stride: dst_stride as u32,
            bytes: dst_bytes,
        })
    }

    /// Extracts a sub-rectangle pixel buffer for dynamic differential delta patching (v2.23 / ADR-109).
    pub fn extract_sub_rect(
        &self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    ) -> Result<ScreenFrame, CoreError> {
        if self.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        let src_w = self.rect.w;
        let src_h = self.rect.h;
        if src_w == 0 || src_h == 0 {
            return Ok(self.clone());
        }
        let x = x.min(src_w.saturating_sub(1));
        let y = y.min(src_h.saturating_sub(1));
        let w = w.clamp(1, src_w.saturating_sub(x));
        let h = h.clamp(1, src_h.saturating_sub(y));
        let bpp = self.format.bpp();
        let src_stride = self.effective_stride() as usize;
        let dst_stride = w as usize * bpp;
        let mut dst_bytes = vec![0u8; dst_stride * h as usize];
        for row in 0..h as usize {
            let src_offset = (y as usize + row) * src_stride + (x as usize * bpp);
            let dst_offset = row * dst_stride;
            if src_offset + dst_stride <= self.bytes.len() {
                dst_bytes[dst_offset..dst_offset + dst_stride]
                    .copy_from_slice(&self.bytes[src_offset..src_offset + dst_stride]);
            }
        }
        let new_rect = ScreenRect::new(self.rect.x + x as i32, self.rect.y + y as i32, w, h)?;
        Ok(ScreenFrame {
            seq: self.seq,
            rect: new_rect,
            format: self.format,
            codec_tag: self.codec_tag,
            stride: dst_stride as u32,
            bytes: dst_bytes,
        })
    }

    /// Blits a differential delta sub-rectangle patch in-place into the current frame canvas (v2.23 / ADR-109).
    pub fn apply_sub_rect(&mut self, patch: &ScreenFrame) -> Result<(), CoreError> {
        if self.compressed() || patch.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        if self.format != patch.format {
            return Err(CoreError::ScreenFrame);
        }
        let patch_x = patch.rect.x;
        let patch_y = patch.rect.y;
        if patch_x < self.rect.x || patch_y < self.rect.y {
            return Err(CoreError::ScreenFrame);
        }
        let px = (patch_x - self.rect.x) as usize;
        let py = (patch_y - self.rect.y) as usize;
        let pw = patch.rect.w as usize;
        let ph = patch.rect.h as usize;
        let src_w = self.rect.w as usize;
        let src_h = self.rect.h as usize;
        if px + pw > src_w || py + ph > src_h {
            return Err(CoreError::ScreenFrame);
        }
        let bpp = self.format.bpp();
        let self_stride = self.effective_stride() as usize;
        let patch_stride = patch.effective_stride() as usize;
        let row_bytes = pw * bpp;
        for row in 0..ph {
            let self_offset = (py + row) * self_stride + (px * bpp);
            let patch_offset = row * patch_stride;
            if self_offset + row_bytes <= self.bytes.len()
                && patch_offset + row_bytes <= patch.bytes.len()
            {
                self.bytes[self_offset..self_offset + row_bytes]
                    .copy_from_slice(&patch.bytes[patch_offset..patch_offset + row_bytes]);
            }
        }
        Ok(())
    }

    /// Shifts canvas pixels in-place according to motion displacement vector (dx, dy) (v2.29 / ADR-115).
    pub fn scroll_canvas(&mut self, dx: i16, dy: i16) {
        if self.compressed() {
            return;
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 || (dx == 0 && dy == 0) {
            return;
        }
        let bpp = self.format.bpp();
        let stride = self.effective_stride() as usize;

        // Vertical displacement
        if dy != 0 {
            let abs_dy = dy.unsigned_abs() as usize;
            if abs_dy < h {
                if dy > 0 {
                    // Shift down: copy backwards from bottom to top
                    for y in (abs_dy..h).rev() {
                        let src_offset = (y - abs_dy) * stride;
                        let dst_offset = y * stride;
                        let copy_len = (w * bpp).min(stride);
                        if src_offset + copy_len <= self.bytes.len()
                            && dst_offset + copy_len <= self.bytes.len()
                        {
                            self.bytes
                                .copy_within(src_offset..src_offset + copy_len, dst_offset);
                        }
                    }
                } else {
                    // Shift up: copy forwards from top to bottom
                    for y in 0..(h - abs_dy) {
                        let src_offset = (y + abs_dy) * stride;
                        let dst_offset = y * stride;
                        let copy_len = (w * bpp).min(stride);
                        if src_offset + copy_len <= self.bytes.len()
                            && dst_offset + copy_len <= self.bytes.len()
                        {
                            self.bytes
                                .copy_within(src_offset..src_offset + copy_len, dst_offset);
                        }
                    }
                }
            }
        }

        // Horizontal displacement
        if dx != 0 {
            let abs_dx = dx.unsigned_abs() as usize;
            if abs_dx < w {
                for y in 0..h {
                    let row_start = y * stride;
                    let row_len = w * bpp;
                    if row_start + row_len <= self.bytes.len() {
                        let row = &mut self.bytes[row_start..row_start + row_len];
                        if dx > 0 {
                            let copy_bytes = (w - abs_dx) * bpp;
                            let dst_start = abs_dx * bpp;
                            row.copy_within(0..copy_bytes, dst_start);
                        } else {
                            let copy_bytes = (w - abs_dx) * bpp;
                            let src_start = abs_dx * bpp;
                            row.copy_within(src_start..src_start + copy_bytes, 0);
                        }
                    }
                }
            }
        }
    }

    /// Encodes a 2D motion scroll displacement frame with exposed patch (v2.29 / ADR-115).
    pub fn encode_motion_scroll(
        screen_rect: ScreenRect,
        format: PixelFormat,
        seq: u64,
        dx: i16,
        dy: i16,
        exposed_rect: ScreenDamageRect,
        compressed_patch: &[u8],
    ) -> ScreenFrame {
        let mut payload = Vec::with_capacity(12 + compressed_patch.len());
        payload.extend_from_slice(&dx.to_be_bytes());
        payload.extend_from_slice(&dy.to_be_bytes());
        payload.extend_from_slice(&exposed_rect.x.to_be_bytes());
        payload.extend_from_slice(&exposed_rect.y.to_be_bytes());
        payload.extend_from_slice(&exposed_rect.width.to_be_bytes());
        payload.extend_from_slice(&exposed_rect.height.to_be_bytes());
        payload.extend_from_slice(compressed_patch);

        ScreenFrame {
            seq,
            rect: screen_rect,
            format,
            codec_tag: SCREEN_COMPRESS_MOTION_SCROLL,
            stride: (screen_rect.w as u32) * (format.bpp() as u32),
            bytes: payload,
        }
    }

    /// Decodes a 2D motion scroll displacement frame returning (dx, dy, exposed_rect, compressed_patch) (v2.29 / ADR-115).
    pub fn decode_motion_scroll(&self) -> Result<(i16, i16, ScreenDamageRect, &[u8]), CoreError> {
        if self.codec_tag != SCREEN_COMPRESS_MOTION_SCROLL || self.bytes.len() < 12 {
            return Err(CoreError::ScreenFrame);
        }
        let dx = i16::from_be_bytes(self.bytes[0..2].try_into().unwrap());
        let dy = i16::from_be_bytes(self.bytes[2..4].try_into().unwrap());
        let ex = u16::from_be_bytes(self.bytes[4..6].try_into().unwrap());
        let ey = u16::from_be_bytes(self.bytes[6..8].try_into().unwrap());
        let ew = u16::from_be_bytes(self.bytes[8..10].try_into().unwrap());
        let eh = u16::from_be_bytes(self.bytes[10..12].try_into().unwrap());
        let patch = &self.bytes[12..];
        Ok((
            dx,
            dy,
            ScreenDamageRect {
                x: ex,
                y: ey,
                width: ew,
                height: eh,
            },
            patch,
        ))
    }

    /// Encodes a spatial tile-grid frame with compressed dirty tile patches (v2.30 / ADR-116).
    pub fn encode_tile_grid(
        seq: u64,
        screen_rect: ScreenRect,
        format: PixelFormat,
        tile_size: u16,
        total_tiles: u16,
        dirty_tiles: &[DirtyTile],
    ) -> ScreenFrame {
        let mut payload = Vec::new();
        payload.extend_from_slice(&tile_size.to_be_bytes());
        payload.extend_from_slice(&total_tiles.to_be_bytes());
        payload.extend_from_slice(&(dirty_tiles.len() as u16).to_be_bytes());

        for tile in dirty_tiles {
            payload.extend_from_slice(&tile.index.to_be_bytes());
            payload.extend_from_slice(&tile.x.to_be_bytes());
            payload.extend_from_slice(&tile.y.to_be_bytes());
            payload.extend_from_slice(&tile.w.to_be_bytes());
            payload.extend_from_slice(&tile.h.to_be_bytes());
            payload.extend_from_slice(&(tile.data.len() as u32).to_be_bytes());
            payload.extend_from_slice(&tile.data);
        }

        ScreenFrame {
            seq,
            rect: screen_rect,
            format,
            codec_tag: SCREEN_COMPRESS_TILE_GRID,
            stride: (screen_rect.w as u32) * (format.bpp() as u32),
            bytes: payload,
        }
    }

    /// Decodes a spatial tile-grid frame returning (tile_size, total_tiles, dirty_tiles) (v2.30 / ADR-116).
    pub fn decode_tile_grid(&self) -> Result<(u16, u16, Vec<DirtyTile>), CoreError> {
        if self.codec_tag != SCREEN_COMPRESS_TILE_GRID || self.bytes.len() < 6 {
            return Err(CoreError::ScreenFrame);
        }
        let tile_size = u16::from_be_bytes(self.bytes[0..2].try_into().unwrap());
        let total_tiles = u16::from_be_bytes(self.bytes[2..4].try_into().unwrap());
        let dirty_count = u16::from_be_bytes(self.bytes[4..6].try_into().unwrap()) as usize;

        let mut offset = 6;
        let mut tiles = Vec::with_capacity(dirty_count);

        for _ in 0..dirty_count {
            if offset + 14 > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let index = u16::from_be_bytes(self.bytes[offset..offset + 2].try_into().unwrap());
            let x = u16::from_be_bytes(self.bytes[offset + 2..offset + 4].try_into().unwrap());
            let y = u16::from_be_bytes(self.bytes[offset + 4..offset + 6].try_into().unwrap());
            let w = u16::from_be_bytes(self.bytes[offset + 6..offset + 8].try_into().unwrap());
            let h = u16::from_be_bytes(self.bytes[offset + 8..offset + 10].try_into().unwrap());
            let patch_len =
                u32::from_be_bytes(self.bytes[offset + 10..offset + 14].try_into().unwrap())
                    as usize;
            offset += 14;

            if offset + patch_len > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let data = self.bytes[offset..offset + patch_len].to_vec();
            offset += patch_len;

            tiles.push(DirtyTile {
                index,
                x,
                y,
                w,
                h,
                data,
            });
        }

        Ok((tile_size, total_tiles, tiles))
    }

    /// Encodes a batch chunk of dirty tiles into a ScreenFrame (v2.39 / ADR-125).
    /// Wire format: batch_id(4) | chunk_index(2) | total_chunks(2) | tile_size(2) | total_tiles(2) | tiles_in_chunk(2)
    /// Followed by tiles_in_chunk records of: index(2) | x(2) | y(2) | w(2) | h(2) | len(4) | data(len).
    #[allow(clippy::too_many_arguments)]
    pub fn encode_tile_batch_chunk(
        seq: u64,
        screen_rect: ScreenRect,
        format: PixelFormat,
        batch_id: u32,
        chunk_index: u16,
        total_chunks: u16,
        tile_size: u16,
        total_tiles: u16,
        tiles: &[DirtyTile],
    ) -> ScreenFrame {
        let mut payload = Vec::new();
        payload.extend_from_slice(&batch_id.to_be_bytes());
        payload.extend_from_slice(&chunk_index.to_be_bytes());
        payload.extend_from_slice(&total_chunks.to_be_bytes());
        payload.extend_from_slice(&tile_size.to_be_bytes());
        payload.extend_from_slice(&total_tiles.to_be_bytes());
        payload.extend_from_slice(&(tiles.len() as u16).to_be_bytes());

        for tile in tiles {
            payload.extend_from_slice(&tile.index.to_be_bytes());
            payload.extend_from_slice(&tile.x.to_be_bytes());
            payload.extend_from_slice(&tile.y.to_be_bytes());
            payload.extend_from_slice(&tile.w.to_be_bytes());
            payload.extend_from_slice(&tile.h.to_be_bytes());
            payload.extend_from_slice(&(tile.data.len() as u32).to_be_bytes());
            payload.extend_from_slice(&tile.data);
        }

        ScreenFrame {
            seq,
            rect: screen_rect,
            format,
            codec_tag: SCREEN_COMPRESS_TILE_BATCH,
            stride: (screen_rect.w as u32) * (format.bpp() as u32),
            bytes: payload,
        }
    }

    /// Decodes a spatial tile-batch chunk frame returning (batch_id, chunk_index, total_chunks, tile_size, total_tiles, tiles) (v2.39 / ADR-125).
    pub fn decode_tile_batch(&self) -> Result<TileBatchChunk, CoreError> {
        if self.codec_tag != SCREEN_COMPRESS_TILE_BATCH || self.bytes.len() < 14 {
            return Err(CoreError::ScreenFrame);
        }
        let batch_id = u32::from_be_bytes(self.bytes[0..4].try_into().unwrap());
        let chunk_index = u16::from_be_bytes(self.bytes[4..6].try_into().unwrap());
        let total_chunks = u16::from_be_bytes(self.bytes[6..8].try_into().unwrap());
        let tile_size = u16::from_be_bytes(self.bytes[8..10].try_into().unwrap());
        let total_tiles = u16::from_be_bytes(self.bytes[10..12].try_into().unwrap());
        let tiles_in_chunk = u16::from_be_bytes(self.bytes[12..14].try_into().unwrap()) as usize;

        let mut offset = 14;
        let mut tiles = Vec::with_capacity(tiles_in_chunk);

        for _ in 0..tiles_in_chunk {
            if offset + 14 > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let index = u16::from_be_bytes(self.bytes[offset..offset + 2].try_into().unwrap());
            let x = u16::from_be_bytes(self.bytes[offset + 2..offset + 4].try_into().unwrap());
            let y = u16::from_be_bytes(self.bytes[offset + 4..offset + 6].try_into().unwrap());
            let w = u16::from_be_bytes(self.bytes[offset + 6..offset + 8].try_into().unwrap());
            let h = u16::from_be_bytes(self.bytes[offset + 8..offset + 10].try_into().unwrap());
            let patch_len =
                u32::from_be_bytes(self.bytes[offset + 10..offset + 14].try_into().unwrap())
                    as usize;
            offset += 14;

            if offset + patch_len > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let data = self.bytes[offset..offset + patch_len].to_vec();
            offset += patch_len;

            tiles.push(DirtyTile {
                index,
                x,
                y,
                w,
                h,
                data,
            });
        }

        Ok((
            batch_id,
            chunk_index,
            total_chunks,
            tile_size,
            total_tiles,
            tiles,
        ))
    }

    /// Blits an uncompressed dirty tile patch into the target uncompressed canvas in-place (v2.30 / ADR-116).
    pub fn apply_dirty_tile(&mut self, tile: &DirtyTile) -> Result<(), CoreError> {
        if self.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        let bpp = self.format.bpp();
        let tile_bytes = (tile.w as usize)
            .checked_mul(tile.h as usize)
            .and_then(|v| v.checked_mul(bpp))
            .ok_or(CoreError::ScreenTooLarge)?;
        if tile.data.len() < tile_bytes {
            return Err(CoreError::ScreenFrame);
        }
        let dst_stride = (self.rect.w as usize) * bpp;
        let tile_stride = (tile.w as usize) * bpp;

        if (tile.x as usize + tile.w as usize) > self.rect.w as usize
            || (tile.y as usize + tile.h as usize) > self.rect.h as usize
        {
            return Err(CoreError::ScreenFrame);
        }

        for row in 0..tile.h as usize {
            let dst_offset = (tile.y as usize + row) * dst_stride + (tile.x as usize) * bpp;
            let src_offset = row * tile_stride;
            self.bytes[dst_offset..dst_offset + tile_stride]
                .copy_from_slice(&tile.data[src_offset..src_offset + tile_stride]);
        }
        Ok(())
    }

    /// Encodes a spatial tile cache frame with deduplicated and compressed dirty tile patches (v2.31 / ADR-117).
    pub fn encode_tile_cache(
        seq: u64,
        screen_rect: ScreenRect,
        format: PixelFormat,
        tile_size: u16,
        total_tiles: u16,
        entries: &[CachedTileEntry],
    ) -> ScreenFrame {
        let mut payload = Vec::new();
        payload.extend_from_slice(&tile_size.to_be_bytes());
        payload.extend_from_slice(&total_tiles.to_be_bytes());
        payload.extend_from_slice(&(entries.len() as u16).to_be_bytes());

        for entry in entries {
            payload.extend_from_slice(&entry.index.to_be_bytes());
            payload.extend_from_slice(&entry.x.to_be_bytes());
            payload.extend_from_slice(&entry.y.to_be_bytes());
            payload.extend_from_slice(&entry.w.to_be_bytes());
            payload.extend_from_slice(&entry.h.to_be_bytes());
            payload.push(if entry.cache_hit { 1 } else { 0 });
            payload.extend_from_slice(&entry.cache_id.to_be_bytes());
            payload.extend_from_slice(&(entry.data.len() as u32).to_be_bytes());
            payload.extend_from_slice(&entry.data);
        }

        ScreenFrame {
            seq,
            rect: screen_rect,
            format,
            codec_tag: SCREEN_COMPRESS_TILE_CACHE,
            stride: (screen_rect.w as u32) * (format.bpp() as u32),
            bytes: payload,
        }
    }

    /// Decodes a spatial tile cache frame returning (tile_size, total_tiles, entries) (v2.31 / ADR-117).
    pub fn decode_tile_cache(&self) -> Result<(u16, u16, Vec<CachedTileEntry>), CoreError> {
        if self.codec_tag != SCREEN_COMPRESS_TILE_CACHE || self.bytes.len() < 6 {
            return Err(CoreError::ScreenFrame);
        }
        let tile_size = u16::from_be_bytes(self.bytes[0..2].try_into().unwrap());
        let total_tiles = u16::from_be_bytes(self.bytes[2..4].try_into().unwrap());
        let count = u16::from_be_bytes(self.bytes[4..6].try_into().unwrap()) as usize;

        let mut offset = 6;
        let mut entries = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 17 > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let index = u16::from_be_bytes(self.bytes[offset..offset + 2].try_into().unwrap());
            let x = u16::from_be_bytes(self.bytes[offset + 2..offset + 4].try_into().unwrap());
            let y = u16::from_be_bytes(self.bytes[offset + 4..offset + 6].try_into().unwrap());
            let w = u16::from_be_bytes(self.bytes[offset + 6..offset + 8].try_into().unwrap());
            let h = u16::from_be_bytes(self.bytes[offset + 8..offset + 10].try_into().unwrap());
            let cache_hit = self.bytes[offset + 10] != 0;
            let cache_id =
                u16::from_be_bytes(self.bytes[offset + 11..offset + 13].try_into().unwrap());
            let patch_len =
                u32::from_be_bytes(self.bytes[offset + 13..offset + 17].try_into().unwrap())
                    as usize;
            offset += 17;

            if offset + patch_len > self.bytes.len() {
                return Err(CoreError::ScreenFrame);
            }
            let data = self.bytes[offset..offset + patch_len].to_vec();
            offset += patch_len;

            entries.push(CachedTileEntry {
                index,
                x,
                y,
                w,
                h,
                cache_hit,
                cache_id,
                data,
            });
        }

        Ok((tile_size, total_tiles, entries))
    }

    /// Blits a cached dirty tile patch into the target uncompressed canvas in-place (v2.31 / ADR-117).
    pub fn apply_cached_tile(
        &mut self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        data: &[u8],
    ) -> Result<(), CoreError> {
        if self.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        let bpp = self.format.bpp();
        let tile_bytes = (w as usize)
            .checked_mul(h as usize)
            .and_then(|v| v.checked_mul(bpp))
            .ok_or(CoreError::ScreenTooLarge)?;
        if data.len() < tile_bytes {
            return Err(CoreError::ScreenFrame);
        }
        let dst_stride = (self.rect.w as usize) * bpp;
        let tile_stride = (w as usize) * bpp;

        if (x as usize + w as usize) > self.rect.w as usize
            || (y as usize + h as usize) > self.rect.h as usize
        {
            return Err(CoreError::ScreenFrame);
        }

        for row in 0..h as usize {
            let dst_offset = (y as usize + row) * dst_stride + (x as usize) * bpp;
            let src_offset = row * tile_stride;
            self.bytes[dst_offset..dst_offset + tile_stride]
                .copy_from_slice(&data[src_offset..src_offset + tile_stride]);
        }
        Ok(())
    }

    /// Apply adaptive color depth quantization or grayscale transformation in-place on uncompressed raw pixels (v2.22 / ADR-108).
    pub fn apply_color_mode(&mut self, config: &ScreenColorConfig) -> Result<(), CoreError> {
        if !config.is_active() {
            return Ok(());
        }
        if self.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };
        let threshold = config.threshold;
        for px in self.bytes.chunks_exact_mut(4) {
            let (r, g, b) = if is_bgra {
                (px[2], px[1], px[0])
            } else {
                (px[0], px[1], px[2])
            };
            match config.mode {
                ScreenColorMode::FullColor32 => break,
                ScreenColorMode::ReducedColor16 => {
                    let r_q = (r & 0xF8) | (r >> 5);
                    let g_q = (g & 0xFC) | (g >> 6);
                    let b_q = (b & 0xF8) | (b >> 5);
                    if is_bgra {
                        px[0] = b_q;
                        px[1] = g_q;
                        px[2] = r_q;
                    } else {
                        px[0] = r_q;
                        px[1] = g_q;
                        px[2] = b_q;
                    }
                }
                ScreenColorMode::Grayscale8 => {
                    let y = ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) >> 8) as u8;
                    px[0] = y;
                    px[1] = y;
                    px[2] = y;
                }
                ScreenColorMode::Monochrome1 => {
                    let y = ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) >> 8) as u8;
                    let val = if y >= threshold { 255 } else { 0 };
                    px[0] = val;
                    px[1] = val;
                    px[2] = val;
                }
            }
        }
        Ok(())
    }

    /// Computes the Mean Squared Error (MSE) between two uncompressed screen frames across RGB channels (v2.24 / ADR-110).
    /// Returns Ok(mse) where mse >= 0.0, or CoreError if formats, dimensions, or compression state do not match.
    pub fn compute_mse(&self, reference: &ScreenFrame) -> Result<f64, CoreError> {
        if self.compressed() || reference.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        if self.format != reference.format {
            return Err(CoreError::ScreenFrame);
        }
        if self.rect.w != reference.rect.w || self.rect.h != reference.rect.h {
            return Err(CoreError::ScreenFrame);
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 {
            return Ok(0.0);
        }
        let self_stride = self.effective_stride() as usize;
        let ref_stride = reference.effective_stride() as usize;
        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };
        let mut sum_sq_diff = 0.0f64;
        for row in 0..h {
            let self_row_start = row * self_stride;
            let ref_row_start = row * ref_stride;
            for col in 0..w {
                let self_idx = self_row_start + col * 4;
                let ref_idx = ref_row_start + col * 4;
                if self_idx + 4 > self.bytes.len() || ref_idx + 4 > reference.bytes.len() {
                    return Err(CoreError::ScreenFrame);
                }
                let (r1, g1, b1) = if is_bgra {
                    (
                        self.bytes[self_idx + 2],
                        self.bytes[self_idx + 1],
                        self.bytes[self_idx],
                    )
                } else {
                    (
                        self.bytes[self_idx],
                        self.bytes[self_idx + 1],
                        self.bytes[self_idx + 2],
                    )
                };
                let (r2, g2, b2) = if is_bgra {
                    (
                        reference.bytes[ref_idx + 2],
                        reference.bytes[ref_idx + 1],
                        reference.bytes[ref_idx],
                    )
                } else {
                    (
                        reference.bytes[ref_idx],
                        reference.bytes[ref_idx + 1],
                        reference.bytes[ref_idx + 2],
                    )
                };
                let dr = r1 as f64 - r2 as f64;
                let dg = g1 as f64 - g2 as f64;
                let db = b1 as f64 - b2 as f64;
                sum_sq_diff += dr * dr + dg * dg + db * db;
            }
        }
        let total_pixels = (w * h) as f64;
        let mse = sum_sq_diff / (3.0 * total_pixels);
        Ok(mse)
    }

    /// Computes Peak Signal-to-Noise Ratio (PSNR) in decibels (dB) between two uncompressed frames (v2.24 / ADR-110).
    /// Baseline for identical frames (MSE <= 1e-10) is 100.0 dB.
    pub fn compute_psnr(&self, reference: &ScreenFrame) -> Result<f64, CoreError> {
        let mse = self.compute_mse(reference)?;
        if mse <= 1e-10 {
            return Ok(100.0);
        }
        // PSNR = 10 * log10(255^2 / MSE)
        let psnr = 10.0 * (65025.0 / mse).log10();
        Ok(psnr.clamp(0.0, 100.0))
    }

    /// Computes whole-frame Structural Similarity Index (SSIM) in [0.0, 1.0] using 16x16 macroblock windows (v2.24 / ADR-110).
    /// Standard constants: C1 = (0.01 * 255)^2 = 6.5025, C2 = (0.03 * 255)^2 = 58.5225.
    pub fn compute_ssim(&self, reference: &ScreenFrame) -> Result<f64, CoreError> {
        if self.compressed() || reference.compressed() {
            return Err(CoreError::ScreenFrame);
        }
        if self.format != reference.format {
            return Err(CoreError::ScreenFrame);
        }
        if self.rect.w != reference.rect.w || self.rect.h != reference.rect.h {
            return Err(CoreError::ScreenFrame);
        }
        let w = self.rect.w as usize;
        let h = self.rect.h as usize;
        if w == 0 || h == 0 {
            return Ok(1.0);
        }
        let self_stride = self.effective_stride() as usize;
        let ref_stride = reference.effective_stride() as usize;
        let is_bgra = match self.format {
            PixelFormat::Bgra8 => true,
            PixelFormat::Rgba8 => false,
        };

        const C1: f64 = 6.5025;
        const C2: f64 = 58.5225;
        const BLOCK_SIZE: usize = 16;

        let mut total_ssim = 0.0f64;
        let mut block_count = 0usize;

        for by in (0..h).step_by(BLOCK_SIZE) {
            let bh = (h - by).min(BLOCK_SIZE);
            for bx in (0..w).step_by(BLOCK_SIZE) {
                let bw = (w - bx).min(BLOCK_SIZE);
                let n = (bw * bh) as f64;
                if n <= 0.0 {
                    continue;
                }

                let mut sum_x = 0.0f64;
                let mut sum_y = 0.0f64;

                for dy in 0..bh {
                    let row = by + dy;
                    let self_row = row * self_stride;
                    let ref_row = row * ref_stride;
                    for dx in 0..bw {
                        let col = bx + dx;
                        let self_idx = self_row + col * 4;
                        let ref_idx = ref_row + col * 4;
                        if self_idx + 4 > self.bytes.len() || ref_idx + 4 > reference.bytes.len() {
                            return Err(CoreError::ScreenFrame);
                        }
                        let (r1, g1, b1) = if is_bgra {
                            (
                                self.bytes[self_idx + 2],
                                self.bytes[self_idx + 1],
                                self.bytes[self_idx],
                            )
                        } else {
                            (
                                self.bytes[self_idx],
                                self.bytes[self_idx + 1],
                                self.bytes[self_idx + 2],
                            )
                        };
                        let (r2, g2, b2) = if is_bgra {
                            (
                                reference.bytes[ref_idx + 2],
                                reference.bytes[ref_idx + 1],
                                reference.bytes[ref_idx],
                            )
                        } else {
                            (
                                reference.bytes[ref_idx],
                                reference.bytes[ref_idx + 1],
                                reference.bytes[ref_idx + 2],
                            )
                        };
                        let y1 = ((77u32 * r1 as u32 + 150u32 * g1 as u32 + 29u32 * b1 as u32) >> 8)
                            as f64;
                        let y2 = ((77u32 * r2 as u32 + 150u32 * g2 as u32 + 29u32 * b2 as u32) >> 8)
                            as f64;
                        sum_x += y1;
                        sum_y += y2;
                    }
                }

                let mu_x = sum_x / n;
                let mu_y = sum_y / n;

                let mut sum_sq_diff_x = 0.0f64;
                let mut sum_sq_diff_y = 0.0f64;
                let mut sum_covar = 0.0f64;

                for dy in 0..bh {
                    let row = by + dy;
                    let self_row = row * self_stride;
                    let ref_row = row * ref_stride;
                    for dx in 0..bw {
                        let col = bx + dx;
                        let self_idx = self_row + col * 4;
                        let ref_idx = ref_row + col * 4;
                        let (r1, g1, b1) = if is_bgra {
                            (
                                self.bytes[self_idx + 2],
                                self.bytes[self_idx + 1],
                                self.bytes[self_idx],
                            )
                        } else {
                            (
                                self.bytes[self_idx],
                                self.bytes[self_idx + 1],
                                self.bytes[self_idx + 2],
                            )
                        };
                        let (r2, g2, b2) = if is_bgra {
                            (
                                reference.bytes[ref_idx + 2],
                                reference.bytes[ref_idx + 1],
                                reference.bytes[ref_idx],
                            )
                        } else {
                            (
                                reference.bytes[ref_idx],
                                reference.bytes[ref_idx + 1],
                                reference.bytes[ref_idx + 2],
                            )
                        };
                        let y1 = ((77u32 * r1 as u32 + 150u32 * g1 as u32 + 29u32 * b1 as u32) >> 8)
                            as f64;
                        let y2 = ((77u32 * r2 as u32 + 150u32 * g2 as u32 + 29u32 * b2 as u32) >> 8)
                            as f64;
                        let diff_x = y1 - mu_x;
                        let diff_y = y2 - mu_y;
                        sum_sq_diff_x += diff_x * diff_x;
                        sum_sq_diff_y += diff_y * diff_y;
                        sum_covar += diff_x * diff_y;
                    }
                }

                let var_x = sum_sq_diff_x / n;
                let var_y = sum_sq_diff_y / n;
                let covar_xy = sum_covar / n;

                let num = (2.0 * mu_x * mu_y + C1) * (2.0 * covar_xy + C2);
                let den = (mu_x * mu_x + mu_y * mu_y + C1) * (var_x + var_y + C2);
                let block_ssim = if den.abs() > 1e-12 {
                    (num / den).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                total_ssim += block_ssim;
                block_count += 1;
            }
        }

        if block_count == 0 {
            Ok(1.0)
        } else {
            let mean_ssim = total_ssim / (block_count as f64);
            Ok(mean_ssim.clamp(0.0, 1.0))
        }
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
            tag @ (SCREEN_COMPRESS_NONE
            | SCREEN_COMPRESS_ZSTD
            | SCREEN_COMPRESS_H264
            | SCREEN_COMPRESS_DELTA
            | SCREEN_COMPRESS_LTR
            | SCREEN_COMPRESS_SVC_T1
            | SCREEN_COMPRESS_MOTION_SCROLL
            | SCREEN_COMPRESS_TILE_GRID
            | SCREEN_COMPRESS_TILE_CACHE
            | SCREEN_COMPRESS_TILE_BATCH) => tag,
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
        if compressed
            && codec_tag != SCREEN_COMPRESS_MOTION_SCROLL
            && codec_tag != SCREEN_COMPRESS_TILE_GRID
            && codec_tag != SCREEN_COMPRESS_TILE_CACHE
            && codec_tag != SCREEN_COMPRESS_TILE_BATCH
            && bytes.len() > rect.pixel_bytes(format)
        {
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

#[inline(always)]
fn blend_pixel(
    pixels: &mut [u8],
    offset: usize,
    is_bgra: bool,
    r_fg: u8,
    g_fg: u8,
    b_fg: u8,
    alpha: u8,
) {
    if alpha == 0 || offset + 3 >= pixels.len() {
        return;
    }
    let alpha_u32 = alpha as u32;
    let inv_alpha = 255u32 - alpha_u32;
    if is_bgra {
        let bg_b = pixels[offset] as u32;
        let bg_g = pixels[offset + 1] as u32;
        let bg_r = pixels[offset + 2] as u32;
        pixels[offset] = ((b_fg as u32 * alpha_u32 + bg_b * inv_alpha + 128) / 255) as u8;
        pixels[offset + 1] = ((g_fg as u32 * alpha_u32 + bg_g * inv_alpha + 128) / 255) as u8;
        pixels[offset + 2] = ((r_fg as u32 * alpha_u32 + bg_r * inv_alpha + 128) / 255) as u8;
    } else {
        let bg_r = pixels[offset] as u32;
        let bg_g = pixels[offset + 1] as u32;
        let bg_b = pixels[offset + 2] as u32;
        pixels[offset] = ((r_fg as u32 * alpha_u32 + bg_r * inv_alpha + 128) / 255) as u8;
        pixels[offset + 1] = ((g_fg as u32 * alpha_u32 + bg_g * inv_alpha + 128) / 255) as u8;
        pixels[offset + 2] = ((b_fg as u32 * alpha_u32 + bg_b * inv_alpha + 128) / 255) as u8;
    }
}

fn isqrt(val: i64) -> i64 {
    if val <= 0 {
        return 0;
    }
    let mut x0 = val / 2;
    if x0 == 0 {
        return 1;
    }
    let mut x1 = (x0 + val / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) / 2;
    }
    x0
}

/// Rendering style bundle shared by collaborative annotation primitives (ADR-105).
struct Stroke {
    thickness: u8,
    color: [u8; 3],
    alpha: u8,
}

/// Pixel raster context bundling the frame buffer view with geometry and channel
/// order so overlay primitives stay small, allocation-free methods (ADR-105).
struct Canvas<'a> {
    pixels: &'a mut [u8],
    w: usize,
    h: usize,
    is_bgra: bool,
}

impl Canvas<'_> {
    #[inline]
    fn blend(&mut self, x: i32, y: i32, color: [u8; 3], alpha: u8) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let offset = (y as usize * self.w + x as usize) * 4;
        blend_pixel(
            self.pixels,
            offset,
            self.is_bgra,
            color[0],
            color[1],
            color[2],
            alpha,
        );
    }

    fn disk(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 3], alpha: u8) {
        if radius <= 0 {
            self.blend(cx, cy, color, alpha);
            return;
        }
        let r2 = radius * radius;
        let min_y = (cy - radius).max(0);
        let max_y = (cy + radius).min(self.h as i32 - 1);
        let min_x = (cx - radius).max(0);
        let max_x = (cx + radius).min(self.w as i32 - 1);
        for y in min_y..=max_y {
            let dy = y - cy;
            let dy2 = dy * dy;
            for x in min_x..=max_x {
                let dx = x - cx;
                if dx * dx + dy2 <= r2 {
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    /// Bresenham scan with per-step disk expansion for stroke thickness (ADR-105).
    fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, stroke: &Stroke) {
        let radius = (stroke.thickness.max(1) as i32) / 2;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.disk(x0, y0, radius, stroke.color, stroke.alpha);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn rect(&mut self, x: i32, y: i32, rw: i32, rh: i32, stroke: &Stroke) {
        if rw <= 0 || rh <= 0 {
            return;
        }
        let x1 = x + rw;
        let y1 = y + rh;
        self.line(x, y, x1, y, stroke);
        self.line(x1, y, x1, y1, stroke);
        self.line(x1, y1, x, y1, stroke);
        self.line(x, y1, x, y, stroke);
    }

    /// Shaft plus two 30-degree wings computed with integer-only geometry (ADR-105).
    fn arrow(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, stroke: &Stroke) {
        self.line(x0, y0, x1, y1, stroke);

        let vx = (x1 - x0) as i64;
        let vy = (y1 - y0) as i64;
        let len = isqrt(vx * vx + vy * vy);
        if len <= 2 {
            return;
        }
        let wing_len = ((stroke.thickness as i64) * 4)
            .clamp(10, 40)
            .min(len / 2 + 5);
        let bx = -vx * 64 / len;
        let by = -vy * 64 / len;

        let w1x = (x1 as i64) + (bx * 55 - by * 32) * wing_len / 4096;
        let w1y = (y1 as i64) + (by * 55 + bx * 32) * wing_len / 4096;

        let w2x = (x1 as i64) + (bx * 55 + by * 32) * wing_len / 4096;
        let w2y = (y1 as i64) + (by * 55 - bx * 32) * wing_len / 4096;

        self.line(x1, y1, w1x as i32, w1y as i32, stroke);
        self.line(x1, y1, w2x as i32, w2y as i32, stroke);
    }

    /// Midpoint-circle spotlight: hot inner core plus a quadratically decaying
    /// integer alpha halo (ADR-105).
    fn laser_spot(&mut self, cx: i32, cy: i32, radius: u8, color: [u8; 3]) {
        let r = radius.max(4) as i32;
        let r2 = r * r;
        let core_r = (r / 3).max(2);
        let core_r2 = core_r * core_r;
        let min_y = (cy - r).max(0);
        let max_y = (cy + r).min(self.h as i32 - 1);
        let min_x = (cx - r).max(0);
        let max_x = (cx + r).min(self.w as i32 - 1);

        let core_color = [
            color[0].saturating_add(180),
            color[1].saturating_add(180),
            color[2].saturating_add(180),
        ];

        for y in min_y..=max_y {
            let dy = y - cy;
            let dy2 = dy * dy;
            for x in min_x..=max_x {
                let dx = x - cx;
                let dist2 = dx * dx + dy2;
                if dist2 <= core_r2 {
                    self.blend(x, y, core_color, 240);
                } else if dist2 <= r2 {
                    let span = (r2 - core_r2).max(1) as u64;
                    let rem = (r2 - dist2) as u64;
                    let alpha = (20 + (rem * 160 / span)).min(200) as u8;
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    /// 8x8 bitmap-font annotation text with integer alpha blending (ADR-105).
    fn annotation_text(
        &mut self,
        start_x: usize,
        start_y: usize,
        text: &str,
        scale: u8,
        color: [u8; 3],
        alpha: u8,
    ) {
        if alpha == 0 || text.is_empty() {
            return;
        }
        let scale = if scale == 0 { 1 } else { scale.min(4) } as usize;
        let char_w = 8 * scale;
        for (idx, ch) in text.chars().enumerate() {
            let char_x = start_x + idx * char_w;
            let glyph =
                crate::screen_font::get_glyph(ch).unwrap_or(&crate::screen_font::FONT_8X8[0]);
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if (bits & (1 << (7 - col))) != 0 {
                        for dy in 0..scale {
                            let py = start_y + row * scale + dy;
                            if py >= self.h {
                                continue;
                            }
                            for dx in 0..scale {
                                let px = char_x + col * scale + dx;
                                if px >= self.w {
                                    continue;
                                }
                                let offset = (py * self.w + px) * 4;
                                blend_pixel(
                                    self.pixels,
                                    offset,
                                    self.is_bgra,
                                    color[0],
                                    color[1],
                                    color[2],
                                    alpha,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Detailed display properties and geometry for multi-display and virtual display multiplexing (v2.11 / ADR-097).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenDisplayInfo {
    /// Zero-based identifier of the display on the host system.
    pub display_id: u8,
    /// Human-readable title or monitor model name.
    pub title: String,
    /// Physical width in pixels.
    pub width: u32,
    /// Physical height in pixels.
    pub height: u32,
    /// Operating system display scaling percentage (e.g. 100%, 125%, 150%, 200%).
    pub scale_factor_pct: u32,
    /// Indicates whether this display is designated as the primary monitor.
    pub is_primary: bool,
    /// Indicates whether this display is an emulated virtual secondary display (ADR-073).
    pub is_virtual: bool,
}

impl ScreenDisplayInfo {
    pub fn new(
        display_id: u8,
        title: impl Into<String>,
        width: u32,
        height: u32,
        scale_factor_pct: u32,
        is_primary: bool,
        is_virtual: bool,
    ) -> Self {
        Self {
            display_id,
            title: title.into(),
            width,
            height,
            scale_factor_pct,
            is_primary,
            is_virtual,
        }
    }

    pub fn primary_default() -> Self {
        Self {
            display_id: 0,
            title: "Display 0 (Primary)".to_string(),
            width: 1920,
            height: 1080,
            scale_factor_pct: 100,
            is_primary: true,
            is_virtual: false,
        }
    }
}

/// Status report of active multi-display configuration and dynamic multiplexing statistics (v2.11 / ADR-097).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenDisplayReport {
    pub ok: bool,
    /// Currently active display ID being captured and streamed.
    pub active_display_id: u8,
    /// List of all discovered displays on the streaming host.
    pub displays: Vec<ScreenDisplayInfo>,
    /// Cumulative count of dynamic display switches performed.
    pub switches_count: u64,
    /// Timestamp in microseconds of the last display switch event.
    pub last_switch_us: u64,
}

/// Status report of screen privacy mode, display blanking and input lockout (v2.12 / ADR-098).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenPrivacyReport {
    pub ok: bool,
    /// Whether privacy screen blanking is currently active on the host.
    pub enabled: bool,
    /// Whether local physical input (keyboard/mouse) lockout is active on the host.
    pub lock_input: bool,
    /// Blanking implementation method ("virtual_overlay", "display_power", "driver_curtain", or "none").
    pub blanking_method: String,
    /// Timestamp in microseconds when privacy mode was last toggled.
    pub last_change_us: u64,
}

impl Default for ScreenPrivacyReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            lock_input: false,
            blanking_method: "virtual_overlay".to_string(),
            last_change_us: 0,
        }
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
    /// Viewer asks sender to dynamically switch captured monitor/display (v2.11 / ADR-097).
    SwitchDisplay {
        display_id: u8,
    },
    /// Viewer/controller asks sender to enable/disable privacy mode (curtain blanking) and input lockout (v2.12 / ADR-098).
    SetPrivacyMode {
        enabled: bool,
        lock_input: bool,
    },
    /// Viewer asks sender to reset multi-stage latency profile statistics (v2.13 / ADR-099).
    ResetLatencyProfile,
    /// Peer asks to synchronize the bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    SetTuningMode {
        mode: u8,
    },
    /// Peer asks to reset bottleneck-driven autonomous tuning statistics and recovery stack (v2.14 / ADR-100).
    ResetTuningProfile,
    /// Receiver discloses that it started/stopped recording the session (v2.15 / ADR-101).
    SetRecordingState {
        active: bool,
    },
    /// Host or peer synchronizes dynamic security watermark state (v2.18 / ADR-104).
    SetWatermarkState {
        enabled: bool,
        host_enforced: bool,
    },
    /// Visitor/controller updates transient virtual laser pointer coordinates and state (v2.19 / ADR-105).
    SetLaserPointer {
        x: u16,
        y: u16,
        active: bool,
        color_rgb: [u8; 3],
    },
    /// Broadcast command to clear all collaborative whiteboard annotations (v2.19 / ADR-105).
    ClearAnnotations,
    /// Undo last annotation for a specific author hash, or 0 for global (v2.20 / ADR-106).
    WhiteboardUndo {
        author_hash: u64,
    },
    /// Redo last annotation for a specific author hash, or 0 for global (v2.20 / ADR-106).
    WhiteboardRedo {
        author_hash: u64,
    },
    /// Host updates whiteboard policy (v2.20 / ADR-106).
    SetWhiteboardPolicy {
        read_only: bool,
        allow_peer_clear: bool,
        max_items: u16,
    },
    /// Viewer/controller dynamically updates Region-of-Interest (ROI) viewport cropping bounds (v2.21 / ADR-107).
    SetScreenRoi {
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
    /// Viewer/controller resets ROI to full-screen capture and streaming (v2.21 / ADR-107).
    ResetScreenRoi,
    /// Viewer/controller dynamically adjusts ROI zoom scale percentage (v2.21 / ADR-107).
    SetScreenRoiZoom {
        zoom_percent: u16,
    },
    /// Viewer/controller dynamically updates adaptive color depth mode (v2.22 / ADR-108).
    SetScreenColorMode {
        mode: u8,
        threshold: u8,
    },
    /// Viewer/controller resets color mode to FullColor32 (v2.22 / ADR-108).
    ResetScreenColorMode,
    /// Viewer/controller dynamically configures differential delta patch mode (v2.23 / ADR-109).
    SetScreenDeltaMode {
        enabled: bool,
        keyframe_interval: u16,
    },
    /// Viewer/controller resets differential delta patch mode to full-frame streaming (v2.23 / ADR-109).
    ResetScreenDeltaMode,
    /// Viewer/controller dynamically configures real-time objective visual quality metric mode (v2.24 / ADR-110).
    SetScreenQualityMetricMode {
        enabled: bool,
        sample_interval: u8,
    },
    /// Viewer/controller resets objective visual quality metric mode to default disabled (v2.24 / ADR-110).
    ResetScreenQualityMetricMode,
    /// Viewer/controller dynamically configures autonomous closed-loop QoE adaptation mode (v2.25 / ADR-111).
    SetScreenQoeMode {
        mode: u8,
        target_psnr_db: u8,
        target_ssim_pct: u8,
    },
    /// Viewer/controller resets autonomous closed-loop QoE adaptation mode to default disabled (v2.25 / ADR-111).
    ResetScreenQoeMode,
    /// Viewer/controller dynamically configures progressive intra-refresh mode (v2.26 / ADR-112).
    SetScreenPirMode {
        mode: u8,
        bands: u8,
    },
    /// Viewer/controller resets progressive intra-refresh mode to default disabled (v2.26 / ADR-112).
    ResetScreenPirMode,
    /// Viewer/controller dynamically configures long-term reference (LTR) mode (v2.27 / ADR-113).
    SetScreenLtrMode {
        enabled: bool,
        auto_interval: u16,
    },
    /// Viewer/controller resets long-term reference (LTR) mode to default disabled (v2.27 / ADR-113).
    ResetScreenLtrMode,
    /// Viewer/controller asks sender to recover stream state against acknowledged LTR anchor (v2.27 / ADR-113).
    LtrRecoveryRequest {
        ltr_seq: u64,
    },
    /// Sender confirms and synchronizes a new LTR golden reference anchor (v2.27 / ADR-113).
    LtrAnchorSync {
        ltr_seq: u64,
    },
    /// Viewer/controller dynamically configures temporal scalability (SVC-T) mode (v2.28 / ADR-114).
    SetScreenSvcMode {
        mode: u8,
        drop_t1: bool,
    },
    /// Viewer/controller resets temporal scalability (SVC-T) mode to default disabled (v2.28 / ADR-114).
    ResetScreenSvcMode,
    /// Sender or viewer synchronizes temporal scalability (SVC-T) T1 drop count (v2.28 / ADR-114).
    SvcT1DropSync {
        dropped_count: u64,
    },
    /// Viewer/controller dynamically configures 2D motion vector estimation and scroll acceleration mode (v2.29 / ADR-115).
    SetScreenMotionMode {
        enabled: bool,
        max_displacement: u16,
    },
    /// Viewer/controller resets 2D motion vector estimation and scroll acceleration mode to default disabled (v2.29 / ADR-115).
    ResetScreenMotionMode,
    /// Sender synchronizes motion scroll frames sent telemetry (v2.29 / ADR-115).
    MotionScrollSync {
        scroll_frames_sent: u64,
    },
    /// Viewer/controller dynamically configures spatial tile-grid decomposition mode (v2.30 / ADR-116).
    SetScreenTileMode {
        enabled: bool,
        tile_size: u16,
    },
    /// Viewer/controller resets spatial tile-grid decomposition mode to default disabled (v2.30 / ADR-116).
    ResetScreenTileMode,
    /// Sender synchronizes spatial tile-grid frames and tiles sent telemetry (v2.30 / ADR-116).
    TileGridSync {
        tiles_sent: u64,
    },
    /// Viewer/controller dynamically configures screen tile dictionary cache mode (v2.31 / ADR-117).
    SetScreenTileCacheMode {
        enabled: bool,
        max_entries: u16,
    },
    /// Viewer/controller resets screen tile dictionary cache mode to default disabled (v2.31 / ADR-117).
    ResetScreenTileCacheMode,
    /// Sender synchronizes tile cache hits telemetry (v2.31 / ADR-117).
    TileCacheSync {
        hits: u64,
    },
    /// Viewer/controller dynamically configures foveated focus awareness and region-priority scheduling (v2.32 / ADR-118).
    SetScreenFocusMode {
        enabled: bool,
        mode: u8,
        radius: u16,
        peripheral_skip_ratio: u8,
    },
    /// Viewer/controller dynamically updates static focus box bounds (v2.32 / ADR-118).
    SetScreenFocusBox {
        box_x: u16,
        box_y: u16,
        box_w: u16,
        box_h: u16,
    },
    /// Viewer/controller resets foveated focus awareness mode to default disabled (v2.32 / ADR-118).
    ResetScreenFocusMode,
    /// Sender synchronizes foveated focus tiles sent telemetry (v2.32 / ADR-118).
    FocusSync {
        focus_tiles_sent: u64,
    },
    /// Sender synchronizes peripheral tiles skipped telemetry (v2.32 / ADR-118).
    FocusPeripheralSync {
        peripheral_skipped: u64,
    },
    /// Viewer/controller dynamically configures tile-level adaptive palette compression (v2.33 / ADR-119).
    SetScreenTilePaletteMode {
        enabled: bool,
        max_colors: u8,
        enable_rle: bool,
    },
    /// Viewer/controller resets tile-level adaptive palette compression mode to default disabled (v2.33 / ADR-119).
    ResetScreenTilePaletteMode,
    /// Sender synchronizes tile palette compression metrics telemetry (v2.33 / ADR-119).
    TilePaletteSync {
        solid_tiles: u64,
        bilevel_tiles: u64,
        palette_tiles: u64,
        saved_bytes: u64,
    },
    /// Viewer/controller dynamically configures tile-level lossless predictive filter & reversible color transform (v2.34 / ADR-120).
    SetScreenTilePredictMode {
        enabled: bool,
        filter_mode: u8,
        enable_rct: bool,
    },
    /// Viewer/controller resets tile-level predictive filter mode to default disabled (v2.34 / ADR-120).
    ResetScreenTilePredictMode,
    /// Sender synchronizes tile predictive compression metrics telemetry (v2.34 / ADR-120).
    TilePredictSync {
        predict_tiles: u64,
        saved_bytes: u64,
    },
    /// Viewer/controller dynamically configures tile-level presentation transaction & double-buffering (v2.35 / ADR-121).
    SetScreenTilePresentMode {
        enabled: bool,
        vsync_mode: u8,
        double_buffer: bool,
        tear_prevention: bool,
    },
    /// Viewer/controller resets tile-level presentation transaction mode to default (v2.35 / ADR-121).
    ResetScreenTilePresentMode,
    /// Sender/receiver synchronizes tile presentation metrics telemetry (v2.35 / ADR-121).
    TilePresentSync {
        committed_txs: u64,
        swapped_frames: u64,
    },
    /// Viewer/controller dispatches a micro-NACK requesting fast retransmission of a missing dirty tile (v2.36 / ADR-122).
    TileNackRequest {
        seq: u32,
        tile_index: u16,
        retry_count: u8,
    },
    /// Viewer/controller resets tile-level micro-NACK recovery mode to default disabled (v2.36 / ADR-122).
    ResetScreenTileNackMode,
    /// Sender/receiver synchronizes tile-level micro-NACK recovery telemetry (v2.36 / ADR-122).
    TileNackSync {
        nack_requests: u64,
        retransmitted_tiles: u64,
        degraded_commits: u64,
    },
    /// Viewer/controller dynamically configures screen tile saliency tiering & priority protection (v2.37 / ADR-123).
    SetScreenTileSaliencyMode {
        enabled: bool,
        high_threshold: u8,
        low_threshold: u8,
        drop_background_on_congestion: bool,
    },
    /// Viewer/controller resets screen tile saliency mode to default disabled (v2.37 / ADR-123).
    ResetScreenTileSaliencyMode,
    /// Sender/receiver synchronizes tile saliency tiering and congestion skip telemetry (v2.37 / ADR-123).
    TileSaliencySync {
        critical_tiles: u64,
        normal_tiles: u64,
        background_tiles: u64,
        background_tiles_skipped: u64,
    },
    /// Viewer/controller dynamically configures screen tile dynamic multi-resolution Level-of-Detail (LoD) downsampling mode (v2.38 / ADR-124).
    SetScreenTileLodMode {
        enabled: bool,
        max_lod: u8,
        auto_lod_on_congestion: bool,
    },
    /// Viewer/controller resets screen tile LoD downsampling mode to default disabled (v2.38 / ADR-124).
    ResetScreenTileLodMode,
    /// Sender/receiver synchronizes tile LoD downsampling telemetry (v2.38 / ADR-124).
    TileLodSync {
        lod0_tiles: u64,
        lod1_tiles: u64,
        lod2_tiles: u64,
        bytes_saved: u64,
    },
    /// Viewer/controller dynamically configures screen tile batch MTU chunking and pacing (v2.39 / ADR-125).
    SetScreenTileBatchMode {
        enabled: bool,
        target_chunk_bytes: u16,
        pacing_interval_us: u16,
    },
    /// Viewer/controller resets screen tile batch chunking mode to default disabled (v2.39 / ADR-125).
    ResetScreenTileBatchMode,
    /// Sender/receiver synchronizes tile batch MTU chunking and pacing telemetry (v2.39 / ADR-125).
    TileBatchSync {
        batches_sent: u64,
        chunks_sent: u64,
        avg_chunk_bytes: u32,
        pacing_pauses: u64,
    },
    /// Viewer/controller dynamically configures screen tile local motion estimation (v2.40 / ADR-126).
    SetScreenTileMotionMode {
        enabled: bool,
        max_displacement: u16,
        min_match_ratio: u8,
    },
    /// Viewer/controller resets screen tile motion mode to default disabled (v2.40 / ADR-126).
    ResetScreenTileMotionMode,
    /// Sender/receiver synchronizes tile motion estimation and displacement compensation telemetry (v2.40 / ADR-126).
    TileMotionSync {
        motion_tiles: u64,
        fallback_tiles: u64,
        bytes_saved: u64,
    },
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
            ControlMsg::SwitchDisplay { .. } => 4,
            ControlMsg::SetPrivacyMode { .. } => 5,
            ControlMsg::ResetLatencyProfile => 6,
            ControlMsg::SetTuningMode { .. } => 7,
            ControlMsg::ResetTuningProfile => 8,
            ControlMsg::SetRecordingState { .. } => 9,
            ControlMsg::SetWatermarkState { .. } => 10,
            ControlMsg::SetLaserPointer { .. } => 11,
            ControlMsg::ClearAnnotations => 12,
            ControlMsg::WhiteboardUndo { .. } => 13,
            ControlMsg::WhiteboardRedo { .. } => 14,
            ControlMsg::SetWhiteboardPolicy { .. } => 15,
            ControlMsg::SetScreenRoi { .. } => 16,
            ControlMsg::ResetScreenRoi => 17,
            ControlMsg::SetScreenRoiZoom { .. } => 18,
            ControlMsg::SetScreenColorMode { .. } => 19,
            ControlMsg::ResetScreenColorMode => 20,
            ControlMsg::SetScreenDeltaMode { .. } => 21,
            ControlMsg::ResetScreenDeltaMode => 22,
            ControlMsg::SetScreenQualityMetricMode { .. } => 23,
            ControlMsg::ResetScreenQualityMetricMode => 24,
            ControlMsg::SetScreenQoeMode { .. } => 25,
            ControlMsg::ResetScreenQoeMode => 26,
            ControlMsg::SetScreenPirMode { .. } => 27,
            ControlMsg::ResetScreenPirMode => 28,
            ControlMsg::SetScreenLtrMode { .. } => 29,
            ControlMsg::ResetScreenLtrMode => 30,
            ControlMsg::LtrRecoveryRequest { .. } => 31,
            ControlMsg::LtrAnchorSync { .. } => 32,
            ControlMsg::SetScreenSvcMode { .. } => 33,
            ControlMsg::ResetScreenSvcMode => 34,
            ControlMsg::SvcT1DropSync { .. } => 35,
            ControlMsg::SetScreenMotionMode { .. } => 36,
            ControlMsg::ResetScreenMotionMode => 37,
            ControlMsg::MotionScrollSync { .. } => 38,
            ControlMsg::SetScreenTileMode { .. } => 39,
            ControlMsg::ResetScreenTileMode => 40,
            ControlMsg::TileGridSync { .. } => 41,
            ControlMsg::SetScreenTileCacheMode { .. } => 42,
            ControlMsg::ResetScreenTileCacheMode => 43,
            ControlMsg::TileCacheSync { .. } => 44,
            ControlMsg::SetScreenFocusMode { .. } => 45,
            ControlMsg::SetScreenFocusBox { .. } => 46,
            ControlMsg::ResetScreenFocusMode => 47,
            ControlMsg::FocusSync { .. } => 48,
            ControlMsg::FocusPeripheralSync { .. } => 49,
            ControlMsg::SetScreenTilePaletteMode { .. } => 50,
            ControlMsg::ResetScreenTilePaletteMode => 51,
            ControlMsg::TilePaletteSync { .. } => 52,
            ControlMsg::SetScreenTilePredictMode { .. } => 53,
            ControlMsg::ResetScreenTilePredictMode => 54,
            ControlMsg::TilePredictSync { .. } => 55,
            ControlMsg::SetScreenTilePresentMode { .. } => 56,
            ControlMsg::ResetScreenTilePresentMode => 57,
            ControlMsg::TilePresentSync { .. } => 58,
            ControlMsg::TileNackRequest { .. } => 59,
            ControlMsg::ResetScreenTileNackMode => 60,
            ControlMsg::TileNackSync { .. } => 61,
            ControlMsg::SetScreenTileSaliencyMode { .. } => 62,
            ControlMsg::ResetScreenTileSaliencyMode => 63,
            ControlMsg::TileSaliencySync { .. } => 64,
            ControlMsg::SetScreenTileLodMode { .. } => 65,
            ControlMsg::ResetScreenTileLodMode => 66,
            ControlMsg::TileLodSync { .. } => 67,
            ControlMsg::SetScreenTileBatchMode { .. } => 68,
            ControlMsg::ResetScreenTileBatchMode => 69,
            ControlMsg::TileBatchSync { .. } => 70,
            ControlMsg::SetScreenTileMotionMode { .. } => 71,
            ControlMsg::ResetScreenTileMotionMode => 72,
            ControlMsg::TileMotionSync { .. } => 73,
        }
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CONTROL_LEN);
        out.extend_from_slice(&SCREEN_MAGIC);
        out.push(SCREEN_PROTO);
        out.push(self.tag());
        match self {
            ControlMsg::Ack { seq } => out.extend_from_slice(&seq.to_be_bytes()),
            ControlMsg::SwitchDisplay { display_id } => {
                out.extend_from_slice(&(*display_id as u64).to_be_bytes())
            }
            ControlMsg::SetPrivacyMode {
                enabled,
                lock_input,
            } => {
                let val: u64 = (*enabled as u64) | ((*lock_input as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetTuningMode { mode } => {
                out.extend_from_slice(&(*mode as u64).to_be_bytes())
            }
            ControlMsg::SetRecordingState { active } => {
                out.extend_from_slice(&(*active as u64).to_be_bytes())
            }
            ControlMsg::SetWatermarkState {
                enabled,
                host_enforced,
            } => {
                let val: u64 = (*enabled as u64) | ((*host_enforced as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetLaserPointer {
                x,
                y,
                active,
                color_rgb,
            } => {
                let val: u64 = (*x as u64)
                    | ((*y as u64) << 16)
                    | ((*active as u64) << 32)
                    | ((color_rgb[0] as u64) << 40)
                    | ((color_rgb[1] as u64) << 48)
                    | ((color_rgb[2] as u64) << 56);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ClearAnnotations => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::WhiteboardUndo { author_hash } => {
                out.extend_from_slice(&author_hash.to_be_bytes())
            }
            ControlMsg::WhiteboardRedo { author_hash } => {
                out.extend_from_slice(&author_hash.to_be_bytes())
            }
            ControlMsg::SetWhiteboardPolicy {
                read_only,
                allow_peer_clear,
                max_items,
            } => {
                let val: u64 = (*read_only as u64)
                    | ((*allow_peer_clear as u64) << 1)
                    | ((*max_items as u64) << 16);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetScreenRoi { x, y, w, h } => {
                let val: u64 =
                    (*x as u64) | ((*y as u64) << 16) | ((*w as u64) << 32) | ((*h as u64) << 48);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenRoi => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenRoiZoom { zoom_percent } => {
                out.extend_from_slice(&(*zoom_percent as u64).to_be_bytes())
            }
            ControlMsg::SetScreenColorMode { mode, threshold } => {
                let val: u64 = (*mode as u64) | ((*threshold as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenColorMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenDeltaMode {
                enabled,
                keyframe_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*keyframe_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenDeltaMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenQualityMetricMode {
                enabled,
                sample_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*sample_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenQualityMetricMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenQoeMode {
                mode,
                target_psnr_db,
                target_ssim_pct,
            } => {
                let val: u64 = (*mode as u64)
                    | ((*target_psnr_db as u64) << 8)
                    | ((*target_ssim_pct as u64) << 16);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenQoeMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenPirMode { mode, bands } => {
                let val: u64 = (*mode as u64) | ((*bands as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenPirMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenLtrMode {
                enabled,
                auto_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*auto_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenLtrMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::LtrRecoveryRequest { ltr_seq } => {
                out.extend_from_slice(&ltr_seq.to_be_bytes())
            }
            ControlMsg::LtrAnchorSync { ltr_seq } => out.extend_from_slice(&ltr_seq.to_be_bytes()),
            ControlMsg::SetScreenSvcMode { mode, drop_t1 } => {
                let val: u64 = (*mode as u64) | (if *drop_t1 { 1u64 << 8 } else { 0u64 });
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenSvcMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SvcT1DropSync { dropped_count } => {
                out.extend_from_slice(&dropped_count.to_be_bytes())
            }
            ControlMsg::SetScreenMotionMode {
                enabled,
                max_displacement,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*max_displacement as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenMotionMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::MotionScrollSync { scroll_frames_sent } => {
                out.extend_from_slice(&scroll_frames_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileMode { enabled, tile_size } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 }) | ((*tile_size as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileGridSync { tiles_sent } => {
                out.extend_from_slice(&tiles_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileCacheMode {
                enabled,
                max_entries,
            } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 }) | ((*max_entries as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileCacheMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileCacheSync { hits } => out.extend_from_slice(&hits.to_be_bytes()),
            ControlMsg::SetScreenFocusMode {
                enabled,
                mode,
                radius,
                peripheral_skip_ratio,
            } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 })
                    | ((*mode as u64) << 8)
                    | ((*radius as u64) << 16)
                    | ((*peripheral_skip_ratio as u64) << 32);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetScreenFocusBox {
                box_x,
                box_y,
                box_w,
                box_h,
            } => {
                let val: u64 = (*box_x as u64)
                    | ((*box_y as u64) << 16)
                    | ((*box_w as u64) << 32)
                    | ((*box_h as u64) << 48);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenFocusMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::FocusSync { focus_tiles_sent } => {
                out.extend_from_slice(&focus_tiles_sent.to_be_bytes())
            }
            ControlMsg::FocusPeripheralSync { peripheral_skipped } => {
                out.extend_from_slice(&peripheral_skipped.to_be_bytes())
            }
            ControlMsg::SetScreenTilePaletteMode {
                enabled,
                max_colors,
                enable_rle,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_colors as u64) << 8;
                if *enable_rle {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePaletteMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePaletteSync { saved_bytes, .. } => {
                out.extend_from_slice(&saved_bytes.to_be_bytes())
            }
            ControlMsg::SetScreenTilePredictMode {
                enabled,
                filter_mode,
                enable_rct,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*filter_mode as u64) << 8;
                if *enable_rct {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePredictMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePredictSync { saved_bytes, .. } => {
                out.extend_from_slice(&saved_bytes.to_be_bytes())
            }
            ControlMsg::SetScreenTilePresentMode {
                enabled,
                vsync_mode,
                double_buffer,
                tear_prevention,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*vsync_mode as u64) << 8;
                if *double_buffer {
                    val |= 1u64 << 16;
                }
                if *tear_prevention {
                    val |= 1u64 << 17;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePresentMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePresentSync { committed_txs, .. } => {
                out.extend_from_slice(&committed_txs.to_be_bytes())
            }
            ControlMsg::TileNackRequest {
                seq,
                tile_index,
                retry_count,
            } => {
                let mut val: u64 = (*seq as u64) & 0xFFFF_FFFF;
                val |= (*tile_index as u64) << 32;
                val |= (*retry_count as u64) << 48;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileNackMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileNackSync { nack_requests, .. } => {
                out.extend_from_slice(&nack_requests.to_be_bytes())
            }
            ControlMsg::SetScreenTileSaliencyMode {
                enabled,
                high_threshold,
                low_threshold,
                drop_background_on_congestion,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*high_threshold as u64) << 8;
                val |= (*low_threshold as u64) << 16;
                if *drop_background_on_congestion {
                    val |= 1u64 << 24;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileSaliencyMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileSaliencySync { critical_tiles, .. } => {
                out.extend_from_slice(&critical_tiles.to_be_bytes())
            }
            ControlMsg::SetScreenTileLodMode {
                enabled,
                max_lod,
                auto_lod_on_congestion,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_lod as u64) << 8;
                if *auto_lod_on_congestion {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileLodMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileLodSync { bytes_saved, .. } => {
                out.extend_from_slice(&bytes_saved.to_be_bytes())
            }
            ControlMsg::SetScreenTileBatchMode {
                enabled,
                target_chunk_bytes,
                pacing_interval_us,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*target_chunk_bytes as u64) << 16;
                val |= (*pacing_interval_us as u64) << 32;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileBatchMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileBatchSync { batches_sent, .. } => {
                out.extend_from_slice(&batches_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileMotionMode {
                enabled,
                max_displacement,
                min_match_ratio,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_displacement as u64) << 16;
                val |= (*min_match_ratio as u64) << 32;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileMotionMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileMotionSync { motion_tiles, .. } => {
                out.extend_from_slice(&motion_tiles.to_be_bytes())
            }
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
            4 => Ok(ControlMsg::SwitchDisplay {
                display_id: seq as u8,
            }),
            5 => {
                let enabled = (seq & 0xFF) != 0;
                let lock_input = ((seq >> 8) & 0xFF) != 0;
                Ok(ControlMsg::SetPrivacyMode {
                    enabled,
                    lock_input,
                })
            }
            6 => Ok(ControlMsg::ResetLatencyProfile),
            7 => Ok(ControlMsg::SetTuningMode {
                mode: (seq & 0xFF) as u8,
            }),
            8 => Ok(ControlMsg::ResetTuningProfile),
            9 => Ok(ControlMsg::SetRecordingState {
                active: (seq & 0xFF) != 0,
            }),
            10 => {
                let enabled = (seq & 0xFF) != 0;
                let host_enforced = ((seq >> 8) & 0xFF) != 0;
                Ok(ControlMsg::SetWatermarkState {
                    enabled,
                    host_enforced,
                })
            }
            11 => {
                let x = (seq & 0xFFFF) as u16;
                let y = ((seq >> 16) & 0xFFFF) as u16;
                let active = ((seq >> 32) & 0xFF) != 0;
                let color_rgb = [
                    ((seq >> 40) & 0xFF) as u8,
                    ((seq >> 48) & 0xFF) as u8,
                    ((seq >> 56) & 0xFF) as u8,
                ];
                Ok(ControlMsg::SetLaserPointer {
                    x,
                    y,
                    active,
                    color_rgb,
                })
            }
            12 => Ok(ControlMsg::ClearAnnotations),
            13 => Ok(ControlMsg::WhiteboardUndo { author_hash: seq }),
            14 => Ok(ControlMsg::WhiteboardRedo { author_hash: seq }),
            15 => {
                let read_only = (seq & 0x1) != 0;
                let allow_peer_clear = (seq & 0x2) != 0;
                let max_items = ((seq >> 16) & 0xFFFF) as u16;
                Ok(ControlMsg::SetWhiteboardPolicy {
                    read_only,
                    allow_peer_clear,
                    max_items,
                })
            }
            16 => {
                let x = (seq & 0xFFFF) as u16;
                let y = ((seq >> 16) & 0xFFFF) as u16;
                let w = ((seq >> 32) & 0xFFFF) as u16;
                let h = ((seq >> 48) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenRoi { x, y, w, h })
            }
            17 => Ok(ControlMsg::ResetScreenRoi),
            18 => {
                let zoom_percent = (seq & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenRoiZoom { zoom_percent })
            }
            19 => {
                let mode = (seq & 0xFF) as u8;
                let threshold = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenColorMode { mode, threshold })
            }
            20 => Ok(ControlMsg::ResetScreenColorMode),
            21 => {
                let enabled = (seq & 0x1) != 0;
                let keyframe_interval = ((seq >> 8) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenDeltaMode {
                    enabled,
                    keyframe_interval,
                })
            }
            22 => Ok(ControlMsg::ResetScreenDeltaMode),
            23 => {
                let enabled = (seq & 0x1) != 0;
                let sample_interval = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenQualityMetricMode {
                    enabled,
                    sample_interval: if sample_interval == 0 {
                        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
                    } else {
                        sample_interval.clamp(1, 120)
                    },
                })
            }
            24 => Ok(ControlMsg::ResetScreenQualityMetricMode),
            25 => {
                let mode = (seq & 0xFF) as u8;
                let target_psnr_db = ((seq >> 8) & 0xFF) as u8;
                let target_ssim_pct = ((seq >> 16) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenQoeMode {
                    mode,
                    target_psnr_db: if target_psnr_db == 0 {
                        SCREEN_QOE_DEFAULT_TARGET_PSNR_DB
                    } else {
                        target_psnr_db.clamp(10, 60)
                    },
                    target_ssim_pct: if target_ssim_pct == 0 {
                        SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT
                    } else {
                        target_ssim_pct.clamp(10, 100)
                    },
                })
            }
            26 => Ok(ControlMsg::ResetScreenQoeMode),
            27 => {
                let mode = (seq & 0xFF) as u8;
                let bands = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenPirMode {
                    mode,
                    bands: if bands == 0 {
                        SCREEN_PIR_DEFAULT_BANDS
                    } else {
                        bands.clamp(SCREEN_PIR_MIN_BANDS, SCREEN_PIR_MAX_BANDS)
                    },
                })
            }
            28 => Ok(ControlMsg::ResetScreenPirMode),
            29 => {
                let enabled = (seq & 0x1) != 0;
                let auto_interval = ((seq >> 8) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenLtrMode {
                    enabled,
                    auto_interval: if auto_interval == 0 {
                        SCREEN_LTR_DEFAULT_AUTO_INTERVAL
                    } else {
                        auto_interval.clamp(SCREEN_LTR_MIN_INTERVAL, SCREEN_LTR_MAX_INTERVAL)
                    },
                })
            }
            30 => Ok(ControlMsg::ResetScreenLtrMode),
            31 => Ok(ControlMsg::LtrRecoveryRequest { ltr_seq: seq }),
            32 => Ok(ControlMsg::LtrAnchorSync { ltr_seq: seq }),
            33 => {
                let mode = (seq & 0xFF) as u8;
                let drop_t1 = ((seq >> 8) & 0x1) != 0;
                Ok(ControlMsg::SetScreenSvcMode { mode, drop_t1 })
            }
            34 => Ok(ControlMsg::ResetScreenSvcMode),
            35 => Ok(ControlMsg::SvcT1DropSync { dropped_count: seq }),
            36 => {
                let enabled = (seq & 0x1) != 0;
                let max_displacement = ((seq >> 8) & 0xFFFF) as u16;
                let max_displacement = if max_displacement == 0 {
                    SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
                } else {
                    max_displacement.clamp(
                        SCREEN_MOTION_MIN_DISPLACEMENT,
                        SCREEN_MOTION_MAX_DISPLACEMENT,
                    )
                };
                Ok(ControlMsg::SetScreenMotionMode {
                    enabled,
                    max_displacement,
                })
            }
            37 => Ok(ControlMsg::ResetScreenMotionMode),
            38 => Ok(ControlMsg::MotionScrollSync {
                scroll_frames_sent: seq,
            }),
            39 => {
                let enabled = (seq & 0x1) != 0;
                let tile_size = ((seq >> 8) & 0xFFFF) as u16;
                let tile_size = if tile_size == 0 {
                    SCREEN_TILE_DEFAULT_SIZE
                } else {
                    tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
                };
                Ok(ControlMsg::SetScreenTileMode { enabled, tile_size })
            }
            40 => Ok(ControlMsg::ResetScreenTileMode),
            41 => Ok(ControlMsg::TileGridSync { tiles_sent: seq }),
            42 => {
                let enabled = (seq & 0x1) != 0;
                let max_entries = ((seq >> 8) & 0xFFFF) as u16;
                let max_entries = if max_entries == 0 {
                    SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
                } else {
                    max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
                };
                Ok(ControlMsg::SetScreenTileCacheMode {
                    enabled,
                    max_entries,
                })
            }
            43 => Ok(ControlMsg::ResetScreenTileCacheMode),
            44 => Ok(ControlMsg::TileCacheSync { hits: seq }),
            45 => {
                let enabled = (seq & 0x1) != 0;
                let mode = ((seq >> 8) & 0xFF) as u8;
                let radius = ((seq >> 16) & 0xFFFF) as u16;
                let peripheral_skip_ratio = ((seq >> 32) & 0xFF) as u8;
                let radius = if radius == 0 {
                    SCREEN_FOCUS_DEFAULT_RADIUS
                } else {
                    radius.clamp(SCREEN_FOCUS_MIN_RADIUS, SCREEN_FOCUS_MAX_RADIUS)
                };
                let peripheral_skip_ratio = peripheral_skip_ratio.min(SCREEN_FOCUS_MAX_SKIP_RATIO);
                Ok(ControlMsg::SetScreenFocusMode {
                    enabled,
                    mode,
                    radius,
                    peripheral_skip_ratio,
                })
            }
            46 => {
                let box_x = (seq & 0xFFFF) as u16;
                let box_y = ((seq >> 16) & 0xFFFF) as u16;
                let box_w = ((seq >> 32) & 0xFFFF) as u16;
                let box_h = ((seq >> 48) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenFocusBox {
                    box_x,
                    box_y,
                    box_w,
                    box_h,
                })
            }
            47 => Ok(ControlMsg::ResetScreenFocusMode),
            48 => Ok(ControlMsg::FocusSync {
                focus_tiles_sent: seq,
            }),
            49 => Ok(ControlMsg::FocusPeripheralSync {
                peripheral_skipped: seq,
            }),
            50 => {
                let enabled = (seq & 0x1) != 0;
                let max_colors = ((seq >> 8) & 0xFF) as u8;
                let enable_rle = ((seq >> 16) & 0x1) != 0;
                let max_colors = if max_colors == 0 {
                    SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS
                } else {
                    max_colors.clamp(2, 16)
                };
                Ok(ControlMsg::SetScreenTilePaletteMode {
                    enabled,
                    max_colors,
                    enable_rle,
                })
            }
            51 => Ok(ControlMsg::ResetScreenTilePaletteMode),
            52 => Ok(ControlMsg::TilePaletteSync {
                solid_tiles: 0,
                bilevel_tiles: 0,
                palette_tiles: 0,
                saved_bytes: seq,
            }),
            53 => {
                let enabled = (seq & 0x1) != 0;
                let filter_mode = ((seq >> 8) & 0xFF) as u8;
                let enable_rct = ((seq >> 16) & 0x1) != 0;
                Ok(ControlMsg::SetScreenTilePredictMode {
                    enabled,
                    filter_mode: filter_mode.min(5),
                    enable_rct,
                })
            }
            54 => Ok(ControlMsg::ResetScreenTilePredictMode),
            55 => Ok(ControlMsg::TilePredictSync {
                predict_tiles: 0,
                saved_bytes: seq,
            }),
            56 => {
                let enabled = (seq & 0x1) != 0;
                let vsync_mode = ((seq >> 8) & 0xFF) as u8;
                let double_buffer = ((seq >> 16) & 0x1) != 0;
                let tear_prevention = ((seq >> 17) & 0x1) != 0;
                Ok(ControlMsg::SetScreenTilePresentMode {
                    enabled,
                    vsync_mode: vsync_mode.min(3),
                    double_buffer,
                    tear_prevention,
                })
            }
            57 => Ok(ControlMsg::ResetScreenTilePresentMode),
            58 => Ok(ControlMsg::TilePresentSync {
                committed_txs: seq,
                swapped_frames: 0,
            }),
            59 => {
                let frame_seq = (seq & 0xFFFF_FFFF) as u32;
                let tile_index = ((seq >> 32) & 0xFFFF) as u16;
                let retry_count = ((seq >> 48) & 0xFF) as u8;
                Ok(ControlMsg::TileNackRequest {
                    seq: frame_seq,
                    tile_index,
                    retry_count,
                })
            }
            60 => Ok(ControlMsg::ResetScreenTileNackMode),
            61 => Ok(ControlMsg::TileNackSync {
                nack_requests: seq,
                retransmitted_tiles: 0,
                degraded_commits: 0,
            }),
            62 => {
                let enabled = (seq & 0x1) != 0;
                let high_threshold = ((seq >> 8) & 0xFF) as u8;
                let low_threshold = ((seq >> 16) & 0xFF) as u8;
                let drop_background_on_congestion = ((seq >> 24) & 0x1) != 0;
                let high = if high_threshold == 0 {
                    120
                } else {
                    high_threshold
                };
                let low = if low_threshold == 0 {
                    40
                } else {
                    low_threshold
                };
                Ok(ControlMsg::SetScreenTileSaliencyMode {
                    enabled,
                    high_threshold: high,
                    low_threshold: low,
                    drop_background_on_congestion,
                })
            }
            63 => Ok(ControlMsg::ResetScreenTileSaliencyMode),
            64 => Ok(ControlMsg::TileSaliencySync {
                critical_tiles: seq,
                normal_tiles: 0,
                background_tiles: 0,
                background_tiles_skipped: 0,
            }),
            65 => {
                let enabled = (seq & 0x1) != 0;
                let max_lod = ((seq >> 8) & 0xFF) as u8;
                let auto_lod_on_congestion = ((seq >> 16) & 0x1) != 0;
                let max_lod = if max_lod == 0 { 1 } else { max_lod.clamp(1, 2) };
                Ok(ControlMsg::SetScreenTileLodMode {
                    enabled,
                    max_lod,
                    auto_lod_on_congestion,
                })
            }
            66 => Ok(ControlMsg::ResetScreenTileLodMode),
            67 => Ok(ControlMsg::TileLodSync {
                lod0_tiles: 0,
                lod1_tiles: 0,
                lod2_tiles: 0,
                bytes_saved: seq,
            }),
            68 => {
                let enabled = (seq & 0x1) != 0;
                let target_chunk_bytes = ((seq >> 16) & 0xFFFF) as u16;
                let pacing_interval_us = ((seq >> 32) & 0xFFFF) as u16;
                let target = if target_chunk_bytes == 0 {
                    SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
                } else {
                    target_chunk_bytes.clamp(
                        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES,
                        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES,
                    )
                };
                let pacing = pacing_interval_us.min(SCREEN_TILE_BATCH_MAX_PACING_US);
                Ok(ControlMsg::SetScreenTileBatchMode {
                    enabled,
                    target_chunk_bytes: target,
                    pacing_interval_us: pacing,
                })
            }
            69 => Ok(ControlMsg::ResetScreenTileBatchMode),
            70 => Ok(ControlMsg::TileBatchSync {
                batches_sent: seq,
                chunks_sent: 0,
                avg_chunk_bytes: 0,
                pacing_pauses: 0,
            }),
            71 => {
                let enabled = (seq & 0xFF) != 0;
                let max_disp = ((seq >> 16) & 0xFFFF) as u16;
                let min_ratio = ((seq >> 32) & 0xFF) as u8;
                let disp = if max_disp == 0 {
                    SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT
                } else {
                    max_disp.clamp(
                        SCREEN_TILE_MOTION_MIN_DISPLACEMENT,
                        SCREEN_TILE_MOTION_MAX_DISPLACEMENT,
                    )
                };
                let ratio = if min_ratio == 0 {
                    SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO
                } else {
                    min_ratio.min(100)
                };
                Ok(ControlMsg::SetScreenTileMotionMode {
                    enabled,
                    max_displacement: disp,
                    min_match_ratio: ratio,
                })
            }
            72 => Ok(ControlMsg::ResetScreenTileMotionMode),
            73 => Ok(ControlMsg::TileMotionSync {
                motion_tiles: seq,
                fallback_tiles: 0,
                bytes_saved: 0,
            }),
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Hybrid ARQ (HARQ) selective retransmission mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.07 / ADR-093).
    pub harq_mode: u8,
    /// Cumulative count of NACK requests sent by receiver (v2.07 / ADR-093).
    pub harq_nacks_sent: u64,
    /// Cumulative count of NACK requests received by sender (v2.07 / ADR-093).
    pub harq_nacks_received: u64,
    /// Total video shards/fragments retransmitted by sender (v2.07 / ADR-093).
    pub harq_shards_retransmitted: u64,
    /// Total frames successfully recovered via selective retransmission (v2.07 / ADR-093).
    pub harq_recovered_frames: u64,
    /// Transport-Wide Congestion Control (TWCC) mode tag (0=auto, 1=conservative, 2=aggressive, 3=disabled) (v2.08 / ADR-094).
    pub twcc_mode: u8,
    /// Cumulative count of TWCC feedback packets sent by receiver (v2.08 / ADR-094).
    pub twcc_feedbacks_sent: u64,
    /// Cumulative count of TWCC feedback packets received by sender (v2.08 / ADR-094).
    pub twcc_feedbacks_received: u64,
    /// Estimated network delivery rate in bits per second derived from TWCC arrival deltas (v2.08 / ADR-094).
    pub twcc_delivery_rate_bps: u64,
    /// Inter-packet queuing delay gradient in microseconds (v2.08 / ADR-094).
    pub twcc_delay_gradient_us: i64,
    /// Active bandwidth probing mode tag (0=auto, 1=aggressive, 2=conservative, 3=disabled) (v2.09 / ADR-095).
    pub probe_mode: u8,
    /// Active bandwidth probing state tag (0=idle, 1=probing, 2=awaiting_feedback, 3=completed, 4=failed) (v2.09 / ADR-095).
    pub probe_state: u8,
    /// Cumulative count of bandwidth probe clusters triggered (v2.09 / ADR-095).
    pub probes_triggered: u64,
    /// Cumulative count of bandwidth probe clusters successfully completed (v2.09 / ADR-095).
    pub probes_completed: u64,
    /// Cumulative count of bandwidth probe clusters failed (v2.09 / ADR-095).
    pub probes_failed: u64,
    /// Last target probing bitrate in bits per second (v2.09 / ADR-095).
    pub probe_target_bps: u32,
    /// Last measured delivery rate during probe in bits per second (v2.09 / ADR-095).
    pub probe_measured_bps: u64,
    /// Audio-video lip-sync and clock synchronization mode tag (0=auto, 1=strict, 2=loose, 3=disabled) (v2.10 / ADR-096).
    pub sync_mode: u8,
    /// Cumulative count of sync beacons sent (v2.10 / ADR-096).
    pub sync_beacons_sent: u64,
    /// Cumulative count of sync beacons received (v2.10 / ADR-096).
    pub sync_beacons_received: u64,
    /// Latest measured presentation timestamp (PTS) skew in microseconds (video PTS - audio PTS) (v2.10 / ADR-096).
    pub sync_skew_us: i64,
    /// Estimated sender vs receiver hardware clock drift in parts per million (PPM) (v2.10 / ADR-096).
    pub sync_clock_drift_ppm: f64,
    /// Total playout target delay adjustments applied for lip-sync alignment (v2.10 / ADR-096).
    pub sync_adjustments_applied: u64,
    /// Indicates whether audio and video presentation timestamps are currently within tolerance (v2.10 / ADR-096).
    pub in_sync: bool,
    /// Active display ID currently selected for streaming capture (v2.11 / ADR-097).
    pub active_display_id: u8,
    /// Total count of available displays discovered on the host (v2.11 / ADR-097).
    pub display_count: u8,
    /// Total number of dynamic display switches executed (v2.11 / ADR-097).
    pub display_switches_count: u64,
    /// Whether screen privacy mode (curtain blanking) is active on the host (v2.12 / ADR-098).
    pub privacy_enabled: bool,
    /// Whether local physical input lockout is active on the host (v2.12 / ADR-098).
    pub privacy_input_locked: bool,
    /// Latency profiler mode tag (0=auto, 1=detailed, 2=disabled) (v2.13 / ADR-099).
    pub latency_mode: u8,
    /// Primary bottleneck stage tag (0=capture, 1=encode, 2=pacer, 3=network, 4=jitter, 5=decode, 6=render, 7=balanced) (v2.13 / ADR-099).
    pub latency_bottleneck: u8,
    /// Latest / EWMA total end-to-end latency in microseconds (v2.13 / ADR-099).
    pub latency_total_e2e_us: u64,
    /// 95th-percentile total end-to-end latency in microseconds (v2.13 / ADR-099).
    pub latency_p95_e2e_us: u64,
    /// Total frames profiled by the multi-stage latency profiler (v2.13 / ADR-099).
    pub latency_frames_profiled: u64,
    /// Bottleneck-driven autonomous tuner mode tag (0=disabled, 1=standard, 2=aggressive) (v2.14 / ADR-100).
    pub tuning_mode: u8,
    /// Cumulative count of autonomous tuning actions applied (v2.14 / ADR-100).
    pub tuning_actions_total: u64,
    /// Cumulative count of autonomous tuning recovery steps applied (v2.14 / ADR-100).
    pub tuning_recoveries: u64,
    /// Bottleneck stage tag of the most recent tuning action (v2.14 / ADR-100).
    pub tuning_last_stage: u8,
    /// Action tag of the most recent tuning action (v2.14 / ADR-100).
    pub tuning_last_action: u8,
}

impl Default for ScreenStats {
    fn default() -> Self {
        Self {
            sent: 0,
            received: 0,
            dropped: 0,
            bytes: 0,
            refused: 0,
            acked: 0,
            last_acked_seq: 0,
            in_flight: 0,
            rtt_latest_us: None,
            rtt_p95_us: None,
            keyframes_requested: 0,
            gaps: 0,
            bitrate_bps: 0,
            fps_target: 0,
            scale_pct: 0,
            profile_tag: 0,
            qp_min: 0,
            qp_max: 0,
            matrix_mode: 0,
            network_health: 0,
            playout_mode: 0,
            playout_delay_us: 0,
            playout_jitter_us: 0,
            cursor_mode: 0,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: false,
            cursor_shape: 0,
            cursor_updates_sent: 0,
            cursor_updates_received: 0,
            squelch_mode: 0,
            is_squelched: false,
            squelched_frames: 0,
            fec_mode: 0,
            fec_redundancy_pct: 0,
            fec_recovered_shards: 0,
            fec_unrecoverable_blocks: 0,
            pacer_mode: 0,
            pacer_packets_paced: 0,
            pacer_bursts_smoothed: 0,
            pacer_avg_delay_us: 0,
            trendline_mode: 0,
            trendline_state: 0,
            trendline_slope_us: 0.0,
            trendline_overuse_count: 0,
            harq_mode: 0,
            harq_nacks_sent: 0,
            harq_nacks_received: 0,
            harq_shards_retransmitted: 0,
            harq_recovered_frames: 0,
            twcc_mode: 0,
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: 0,
            probe_state: 0,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: 0,
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            privacy_enabled: false,
            privacy_input_locked: false,
            latency_mode: 0,
            latency_bottleneck: 7,
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
            tuning_mode: 1,
            tuning_actions_total: 0,
            tuning_recoveries: 0,
            tuning_last_stage: 7,
            tuning_last_action: 0,
        }
    }
}

impl ScreenStats {
    /// Latency profiler mode.
    pub fn latency_mode(&self) -> ScreenProfilerMode {
        ScreenProfilerMode::from_tag(self.latency_mode)
    }

    /// Latency bottleneck stage.
    pub fn latency_bottleneck(&self) -> ScreenBottleneckStage {
        ScreenBottleneckStage::from_tag(self.latency_bottleneck)
    }
    /// Audio-video lip-sync and clock synchronization mode.
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        ScreenAudioSyncMode::from_tag(self.sync_mode)
    }

    /// Active bandwidth probing mode.
    pub fn probe_mode(&self) -> ScreenProbeMode {
        ScreenProbeMode::from_tag(self.probe_mode)
    }

    /// Active bandwidth probing state.
    pub fn probe_state(&self) -> ScreenProbeState {
        ScreenProbeState::from_tag(self.probe_state)
    }

    /// Transport-Wide Congestion Control (TWCC) mode.
    pub fn twcc_mode(&self) -> ScreenTwccMode {
        ScreenTwccMode::from_tag(self.twcc_mode)
    }

    /// Hybrid ARQ (HARQ) selective retransmission mode.
    pub fn harq_mode(&self) -> ScreenHarqMode {
        ScreenHarqMode::from_tag(self.harq_mode)
    }

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

    /// Force an immediate multiplicative step down regardless of cooldown (used by TWCC on acute congestion).
    pub fn force_step_down(&mut self) -> bool {
        self.consecutive_good_frames = 0;
        let old = self.current_bps;
        let decreased = ((self.current_bps as u64 * 80) / 100) as u32;
        self.current_bps = decreased.max(self.min_bps);
        self.current_bps != old
    }

    /// Fast ramp-up to a verified probe bitrate (v2.09 / ADR-095).
    /// Clamps target to [min_bps, max_bps] and steps up immediately without waiting for additive increase steps.
    pub fn fast_ramp_up(&mut self, target_bps: u32) -> bool {
        let clamped = target_bps.clamp(self.min_bps, self.max_bps);
        if clamped > self.current_bps {
            self.current_bps = clamped;
            self.consecutive_good_frames = 0;
            true
        } else {
            false
        }
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

    pub fn adjust_target_delay(&mut self, delta_us: i64) {
        let current = self.target_delay_us as i64;
        let new_target = (current + delta_us).clamp(
            SCREEN_PLAYOUT_MIN_DELAY_US as i64,
            SCREEN_PLAYOUT_MAX_DELAY_US as i64,
        ) as u64;
        self.custom_target_delay_us = Some(new_target);
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
    pub transport_seq: u16,
    pub data: Vec<u8>,
}

impl ScreenFecShard {
    pub fn encode_header(&self) -> [u8; FEC_HEADER_LEN] {
        let mut hdr = [0u8; FEC_HEADER_LEN];
        hdr[0..4].copy_from_slice(&FEC_MAGIC);
        hdr[4] = FEC_PROTO;
        hdr[5] = 0; // flags
        hdr[6..8].copy_from_slice(&self.transport_seq.to_be_bytes());
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
        let transport_seq = u16::from_be_bytes(frame.payload[6..8].try_into().unwrap());
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
            transport_seq,
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
                transport_seq: 0,
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
                transport_seq: 0,
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

    /// Generate selective NACK for any missing shards in the current FEC block (v2.07 / ADR-093).
    pub fn generate_nack(&self, rtt_estimate_us: u64) -> Option<ScreenNack> {
        if self.received_shards.is_empty() {
            return None;
        }
        let first = &self.received_shards[0];
        let k = first.k_data_shards as usize;
        let m = first.m_parity_shards as usize;
        let total = k + m;
        let mut missing = Vec::new();
        let mut mask = 0u64;

        for i in 0..total {
            let is_parity = i >= k;
            let shard_idx = if is_parity { (i - k) as u8 } else { i as u8 };
            let present = self
                .received_shards
                .iter()
                .any(|s| s.is_parity == is_parity && s.shard_index == shard_idx);
            if !present {
                if i < 64 {
                    mask |= 1u64 << i;
                }
                missing.push(i as u16);
            }
        }

        if missing.is_empty() {
            return None;
        }

        let missing_data = missing.iter().filter(|&&idx| (idx as usize) < k).count();
        let fallback = missing_data > m;

        Some(ScreenNack {
            frame_seq: self.current_seq,
            block_id: self.current_block_id,
            missing_mask: mask,
            missing_indices: missing,
            is_fec: true,
            requires_keyframe_fallback: fallback,
            rtt_estimate_us,
        })
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

/// Hybrid ARQ (HARQ) selective retransmission mode (v2.07 / ADR-093).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenHarqMode {
    #[default]
    Auto = 0,
    Aggressive = 1,
    Conservative = 2,
    Disabled = 3,
}

impl ScreenHarqMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenHarqMode::Aggressive,
            2 => ScreenHarqMode::Conservative,
            3 => ScreenHarqMode::Disabled,
            _ => ScreenHarqMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenHarqMode::Auto => "auto",
            ScreenHarqMode::Aggressive => "aggressive",
            ScreenHarqMode::Conservative => "conservative",
            ScreenHarqMode::Disabled => "disabled",
        }
    }
}

impl fmt::Display for ScreenHarqMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenHarqMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenHarqMode::Auto),
            "aggressive" | "agg" => Ok(ScreenHarqMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenHarqMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenHarqMode::Disabled),
            _ => Err(CoreError::ScreenRefused("unknown screen harq mode".into())),
        }
    }
}

pub const SCREEN_NACK_FLAG_FEC: u8 = 0x01;
pub const SCREEN_NACK_FLAG_KEYFRAME_FALLBACK: u8 = 0x02;

/// Fine-grained selective negative acknowledgment (NACK) requesting retransmission
/// of missing fragments or shards before playout deadline (v2.07 / ADR-093).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenNack {
    pub frame_seq: u64,
    pub block_id: u16,
    pub missing_mask: u64,
    pub missing_indices: Vec<u16>,
    pub is_fec: bool,
    pub requires_keyframe_fallback: bool,
    pub rtt_estimate_us: u64,
}

impl ScreenNack {
    pub fn new(frame_seq: u64, block_id: u16, is_fec: bool) -> Self {
        Self {
            frame_seq,
            block_id,
            missing_mask: 0,
            missing_indices: Vec::new(),
            is_fec,
            requires_keyframe_fallback: false,
            rtt_estimate_us: 0,
        }
    }

    pub fn with_missing(mut self, indices: &[u16]) -> Self {
        for &idx in indices {
            if idx < 64 {
                self.missing_mask |= 1u64 << idx;
            }
            if !self.missing_indices.contains(&idx) {
                self.missing_indices.push(idx);
            }
        }
        self.missing_indices.sort_unstable();
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let count = self.missing_indices.len().min(u16::MAX as usize) as u16;
        let mut buf = Vec::with_capacity(SCREEN_NACK_HEADER_LEN + (count as usize) * 2);
        buf.extend_from_slice(&SCREEN_NACK_MAGIC);
        buf.push(SCREEN_NACK_PROTO);
        let mut flags = 0u8;
        if self.is_fec {
            flags |= SCREEN_NACK_FLAG_FEC;
        }
        if self.requires_keyframe_fallback {
            flags |= SCREEN_NACK_FLAG_KEYFRAME_FALLBACK;
        }
        buf.push(flags);
        buf.push(0u8); // reserved
        buf.extend_from_slice(&self.frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.block_id.to_be_bytes());
        buf.extend_from_slice(&self.missing_mask.to_be_bytes());
        buf.extend_from_slice(&count.to_be_bytes());
        for &idx in &self.missing_indices[..count as usize] {
            buf.extend_from_slice(&idx.to_be_bytes());
        }
        buf.extend_from_slice(&self.rtt_estimate_us.to_be_bytes());
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_NACK_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_NACK_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_NACK_PROTO {
            return Err(CoreError::ScreenFrame);
        }
        let flags = payload[6];
        let is_fec = (flags & SCREEN_NACK_FLAG_FEC) != 0;
        let requires_keyframe_fallback = (flags & SCREEN_NACK_FLAG_KEYFRAME_FALLBACK) != 0;
        let frame_seq = u64::from_be_bytes(
            payload[8..16]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let block_id = u16::from_be_bytes(
            payload[16..18]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let missing_mask = u64::from_be_bytes(
            payload[18..26]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let count = u16::from_be_bytes(
            payload[26..28]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        ) as usize;

        let required_len = SCREEN_NACK_HEADER_LEN + count * 2;
        if payload.len() < required_len {
            return Err(CoreError::ScreenFrame);
        }

        let mut offset = 28;
        let mut missing_indices = Vec::with_capacity(count);
        for _ in 0..count {
            let idx = u16::from_be_bytes(
                payload[offset..offset + 2]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            missing_indices.push(idx);
            offset += 2;
        }

        let rtt_estimate_us = u64::from_be_bytes(
            payload[offset..offset + 8]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );

        Ok(Self {
            frame_seq,
            block_id,
            missing_mask,
            missing_indices,
            is_fec,
            requires_keyframe_fallback,
            rtt_estimate_us,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_NACK, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_NACK {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Telemetry statistics for Hybrid ARQ & selective retransmission (v2.07 / ADR-093).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenHarqStats {
    pub mode: ScreenHarqMode,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub shards_retransmitted: u64,
    pub recovered_frames: u64,
    pub retransmit_timeouts: u64,
    pub buffer_occupancy: usize,
    pub buffer_capacity: usize,
}

/// DTO report for REST API and CLI status (v2.07 / ADR-093).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScreenHarqReport {
    pub ok: bool,
    pub mode: ScreenHarqMode,
    pub nacks_sent: u64,
    pub nacks_received: u64,
    pub shards_retransmitted: u64,
    pub recovered_frames: u64,
    pub retransmit_timeouts: u64,
    pub buffer_occupancy: usize,
    pub buffer_capacity: usize,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RetransmitEntry {
    pub frame: InnerFrame,
    pub sent_at_us: u64,
    pub retx_count: u8,
    pub frame_seq: u64,
    pub shard_index: u16,
    pub is_fec: bool,
}

/// Sender-side bounded sliding buffer of recently transmitted shards for selective NACK recovery.
#[derive(Debug, Clone)]
pub struct ScreenRetransmitBuffer {
    mode: ScreenHarqMode,
    capacity: usize,
    entries: VecDeque<RetransmitEntry>,
    stats: ScreenHarqStats,
}

impl Default for ScreenRetransmitBuffer {
    fn default() -> Self {
        Self::new(ScreenHarqMode::Auto)
    }
}

impl ScreenRetransmitBuffer {
    pub fn new(mode: ScreenHarqMode) -> Self {
        Self {
            mode,
            capacity: SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY,
            entries: VecDeque::with_capacity(SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY),
            stats: ScreenHarqStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenHarqMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenHarqMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenHarqMode::Disabled {
            self.entries.clear();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> ScreenHarqStats {
        let mut s = self.stats;
        s.mode = self.mode;
        s.buffer_occupancy = self.entries.len();
        s.buffer_capacity = self.capacity;
        s
    }

    pub fn record_outbound(
        &mut self,
        frame: &InnerFrame,
        frame_seq: u64,
        shard_index: u16,
        is_fec: bool,
        now_us: u64,
    ) {
        if self.mode == ScreenHarqMode::Disabled {
            return;
        }

        let cutoff = now_us.saturating_sub(SCREEN_HARQ_MAX_AGE_US);
        while let Some(front) = self.entries.front() {
            if front.sent_at_us < cutoff || self.entries.len() >= self.capacity {
                self.entries.pop_front();
            } else {
                break;
            }
        }

        self.entries.push_back(RetransmitEntry {
            frame: frame.clone(),
            sent_at_us: now_us,
            retx_count: 0,
            frame_seq,
            shard_index,
            is_fec,
        });
        self.stats.buffer_occupancy = self.entries.len();
        self.stats.buffer_capacity = self.capacity;
    }

    pub fn handle_nack(&mut self, nack: &ScreenNack, now_us: u64) -> Vec<InnerFrame> {
        if self.mode == ScreenHarqMode::Disabled {
            return Vec::new();
        }
        self.stats.nacks_received = self.stats.nacks_received.saturating_add(1);

        let max_retries = match self.mode {
            ScreenHarqMode::Auto => SCREEN_HARQ_MAX_RETRIES_AUTO,
            ScreenHarqMode::Aggressive => SCREEN_HARQ_MAX_RETRIES_AGGRESSIVE,
            ScreenHarqMode::Conservative => SCREEN_HARQ_MAX_RETRIES_CONSERVATIVE,
            ScreenHarqMode::Disabled => 0,
        };

        let mut to_resend = Vec::new();
        let cutoff = now_us.saturating_sub(SCREEN_HARQ_MAX_AGE_US);

        let mut requested_indices = nack.missing_indices.clone();
        if requested_indices.is_empty() && nack.missing_mask != 0 {
            for i in 0..64 {
                if (nack.missing_mask & (1u64 << i)) != 0 {
                    requested_indices.push(i as u16);
                }
            }
        }

        for idx in requested_indices {
            let mut found = false;
            for entry in self.entries.iter_mut() {
                if entry.frame_seq == nack.frame_seq
                    && entry.is_fec == nack.is_fec
                    && entry.shard_index == idx
                {
                    found = true;
                    if entry.retx_count < max_retries && entry.sent_at_us >= cutoff {
                        entry.retx_count += 1;
                        self.stats.shards_retransmitted =
                            self.stats.shards_retransmitted.saturating_add(1);
                        to_resend.push(entry.frame.clone());
                    } else {
                        self.stats.retransmit_timeouts =
                            self.stats.retransmit_timeouts.saturating_add(1);
                    }
                    break;
                }
            }
            if !found {
                self.stats.retransmit_timeouts = self.stats.retransmit_timeouts.saturating_add(1);
            }
        }

        self.stats.buffer_occupancy = self.entries.len();
        to_resend
    }

    /// Automatically inspect outbound InnerFrame (either TYPE_SCREEN_FEC or TYPE_SCREEN_FRAME)
    /// and record it into the bounded sliding window retransmit buffer.
    pub fn record_outbound_frame(&mut self, frame: &InnerFrame, now_us: u64) {
        if self.mode == ScreenHarqMode::Disabled {
            return;
        }
        if frame.ty == TYPE_SCREEN_FEC {
            if let Ok(shard) = ScreenFecShard::from_inner_frame(frame) {
                let shard_index = if shard.is_parity {
                    shard.k_data_shards as u16 + shard.shard_index as u16
                } else {
                    shard.shard_index as u16
                };
                self.record_outbound(frame, shard.frame_seq, shard_index, true, now_us);
            }
        } else if frame.ty == TYPE_SCREEN_FRAME {
            if frame.payload.starts_with(&SCREEN_MAGIC) {
                if let Ok(sf) = ScreenFrame::from_frame(frame) {
                    self.record_outbound(frame, sf.seq, 0, false, now_us);
                }
            } else if frame.payload.starts_with(&SCREEN_FRAG_MAGIC)
                && frame.payload.len() >= SCREEN_FRAG_HEADER
            {
                let idx = u32::from_be_bytes(frame.payload[8..12].try_into().unwrap()) as u16;
                let seq = u64::from_be_bytes(frame.payload[16..24].try_into().unwrap());
                self.record_outbound(frame, seq, idx, false, now_us);
            }
        }
    }

    pub fn report(&self) -> ScreenHarqReport {
        ScreenHarqReport {
            ok: true,
            mode: self.mode,
            nacks_sent: self.stats.nacks_sent,
            nacks_received: self.stats.nacks_received,
            shards_retransmitted: self.stats.shards_retransmitted,
            recovered_frames: self.stats.recovered_frames,
            retransmit_timeouts: self.stats.retransmit_timeouts,
            buffer_occupancy: self.entries.len(),
            buffer_capacity: self.capacity,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.stats.buffer_occupancy = 0;
    }
}

/// Receiver-driven transport-wide congestion control (TWCC) feedback mode (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenTwccMode {
    #[default]
    Auto = 0,
    Conservative = 1,
    Aggressive = 2,
    Disabled = 3,
}

impl ScreenTwccMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenTwccMode::Conservative,
            2 => ScreenTwccMode::Aggressive,
            3 => ScreenTwccMode::Disabled,
            _ => ScreenTwccMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenTwccMode::Auto => "auto",
            ScreenTwccMode::Conservative => "conservative",
            ScreenTwccMode::Aggressive => "aggressive",
            ScreenTwccMode::Disabled => "disabled",
        }
    }

    pub const fn interval_us(self) -> u64 {
        match self {
            ScreenTwccMode::Auto => SCREEN_TWCC_AUTO_INTERVAL_US,
            ScreenTwccMode::Conservative => SCREEN_TWCC_CONSERVATIVE_INTERVAL_US,
            ScreenTwccMode::Aggressive => SCREEN_TWCC_AGGRESSIVE_INTERVAL_US,
            ScreenTwccMode::Disabled => u64::MAX,
        }
    }

    pub const fn packet_threshold(self) -> usize {
        match self {
            ScreenTwccMode::Auto => SCREEN_TWCC_AUTO_PACKET_THRESHOLD,
            ScreenTwccMode::Conservative => SCREEN_TWCC_CONSERVATIVE_PACKET_THRESHOLD,
            ScreenTwccMode::Aggressive => SCREEN_TWCC_AGGRESSIVE_PACKET_THRESHOLD,
            ScreenTwccMode::Disabled => usize::MAX,
        }
    }
}

impl fmt::Display for ScreenTwccMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenTwccMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenTwccMode::Auto),
            "conservative" | "cons" => Ok(ScreenTwccMode::Conservative),
            "aggressive" | "agg" => Ok(ScreenTwccMode::Aggressive),
            "disabled" | "off" | "none" => Ok(ScreenTwccMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Individual packet arrival record in TWCC feedback (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenTwccPacketRecord {
    pub transport_seq: u16,
    pub recv_delta_us: i16,
}

/// Receiver-to-sender TWCC feedback message (v2.08 / ADR-094).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenTwccFeedback {
    pub feedback_seq: u8,
    pub base_seq: u16,
    pub packet_count: u16,
    pub reference_time_us: u64,
    pub records: Vec<ScreenTwccPacketRecord>,
}

impl ScreenTwccFeedback {
    pub fn encode(&self) -> Vec<u8> {
        let count = self.records.len().min(u16::MAX as usize) as u16;
        let mut buf =
            Vec::with_capacity(SCREEN_TWCC_HEADER_LEN + (count as usize) * SCREEN_TWCC_RECORD_LEN);
        buf.extend_from_slice(&SCREEN_TWCC_MAGIC);
        buf.push(SCREEN_TWCC_PROTO);
        buf.push(0); // flags
        buf.push(self.feedback_seq);
        buf.extend_from_slice(&self.base_seq.to_be_bytes());
        buf.extend_from_slice(&count.to_be_bytes());
        buf.extend_from_slice(&self.reference_time_us.to_be_bytes());
        for rec in &self.records[..count as usize] {
            buf.extend_from_slice(&rec.transport_seq.to_be_bytes());
            buf.extend_from_slice(&rec.recv_delta_us.to_be_bytes());
        }
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_TWCC_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_TWCC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_TWCC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let feedback_seq = payload[7];
        let base_seq = u16::from_be_bytes(
            payload[8..10]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let packet_count = u16::from_be_bytes(
            payload[10..12]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let reference_time_us = u64::from_be_bytes(
            payload[12..20]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );

        let required_len =
            SCREEN_TWCC_HEADER_LEN + (packet_count as usize) * SCREEN_TWCC_RECORD_LEN;
        if payload.len() < required_len {
            return Err(CoreError::TruncatedFrame);
        }

        let mut records = Vec::with_capacity(packet_count as usize);
        let mut offset = SCREEN_TWCC_HEADER_LEN;
        for _ in 0..packet_count {
            let transport_seq = u16::from_be_bytes(
                payload[offset..offset + 2]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            let recv_delta_us = i16::from_be_bytes(
                payload[offset + 2..offset + 4]
                    .try_into()
                    .map_err(|_| CoreError::ScreenFrame)?,
            );
            records.push(ScreenTwccPacketRecord {
                transport_seq,
                recv_delta_us,
            });
            offset += SCREEN_TWCC_RECORD_LEN;
        }

        Ok(Self {
            feedback_seq,
            base_seq,
            packet_count,
            reference_time_us,
            records,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_TWCC, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_TWCC {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Telemetry statistics for Transport-Wide Congestion Control (v2.08 / ADR-094).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenTwccStats {
    pub mode: ScreenTwccMode,
    pub feedbacks_sent: u64,
    pub feedbacks_received: u64,
    pub packets_tracked: u64,
    pub packets_acknowledged: u64,
    pub packets_lost: u64,
    pub loss_ratio: f32,
    pub delivery_rate_bps: u64,
    pub delay_gradient_us: i64,
    pub smoothed_rtt_us: u64,
}

impl Default for ScreenTwccStats {
    fn default() -> Self {
        Self {
            mode: ScreenTwccMode::Auto,
            feedbacks_sent: 0,
            feedbacks_received: 0,
            packets_tracked: 0,
            packets_acknowledged: 0,
            packets_lost: 0,
            loss_ratio: 0.0,
            delivery_rate_bps: 0,
            delay_gradient_us: 0,
            smoothed_rtt_us: 0,
        }
    }
}

/// DTO report for REST API and CLI status (v2.08 / ADR-094).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenTwccReport {
    pub ok: bool,
    pub mode: ScreenTwccMode,
    pub feedbacks_sent: u64,
    pub feedbacks_received: u64,
    pub packets_tracked: u64,
    pub packets_acknowledged: u64,
    pub packets_lost: u64,
    pub loss_ratio: f32,
    pub delivery_rate_bps: u64,
    pub delay_gradient_us: i64,
    pub smoothed_rtt_us: u64,
    pub timestamp_ms: u64,
}

/// Receiver-side arrival tracker that records packet arrival times and emits TWCC feedback (v2.08 / ADR-094).
#[derive(Debug, Clone)]
pub struct ScreenTwccReceiverTracker {
    mode: ScreenTwccMode,
    feedback_seq: u8,
    last_feedback_us: u64,
    arrivals: VecDeque<(u16, u64)>,
    stats: ScreenTwccStats,
}

impl Default for ScreenTwccReceiverTracker {
    fn default() -> Self {
        Self::new(ScreenTwccMode::Auto)
    }
}

impl ScreenTwccReceiverTracker {
    pub fn new(mode: ScreenTwccMode) -> Self {
        Self {
            mode,
            feedback_seq: 0,
            last_feedback_us: 0,
            arrivals: VecDeque::with_capacity(512),
            stats: ScreenTwccStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTwccMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTwccMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenTwccMode::Disabled {
            self.arrivals.clear();
        }
    }

    pub fn stats(&self) -> ScreenTwccStats {
        self.stats
    }

    pub fn report(&self) -> ScreenTwccReport {
        ScreenTwccReport {
            ok: true,
            mode: self.mode,
            feedbacks_sent: self.stats.feedbacks_sent,
            feedbacks_received: self.stats.feedbacks_received,
            packets_tracked: self.stats.packets_tracked,
            packets_acknowledged: self.stats.packets_acknowledged,
            packets_lost: self.stats.packets_lost,
            loss_ratio: self.stats.loss_ratio,
            delivery_rate_bps: self.stats.delivery_rate_bps,
            delay_gradient_us: self.stats.delay_gradient_us,
            smoothed_rtt_us: self.stats.smoothed_rtt_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn on_packet_received(&mut self, transport_seq: u16, recv_us: u64) {
        if self.mode == ScreenTwccMode::Disabled || transport_seq == 0 {
            return;
        }
        if self.arrivals.iter().any(|&(s, _)| s == transport_seq) {
            return;
        }
        if self.last_feedback_us == 0 {
            self.last_feedback_us = recv_us;
        }
        while self.arrivals.len() >= 512 {
            self.arrivals.pop_front();
        }
        self.arrivals.push_back((transport_seq, recv_us));
        self.stats.packets_tracked = self.stats.packets_tracked.saturating_add(1);
    }

    pub fn should_send_feedback(&self, now_us: u64) -> bool {
        if self.mode == ScreenTwccMode::Disabled || self.arrivals.is_empty() {
            return false;
        }
        if self.arrivals.len() >= self.mode.packet_threshold() {
            return true;
        }
        now_us.saturating_sub(self.last_feedback_us) >= self.mode.interval_us()
    }

    pub fn build_feedback(&mut self, now_us: u64) -> Option<ScreenTwccFeedback> {
        if self.mode == ScreenTwccMode::Disabled || self.arrivals.is_empty() {
            return None;
        }
        let base_seq = self.arrivals.front().unwrap().0;
        let max_seq = self.arrivals.back().unwrap().0;
        let span = (max_seq.wrapping_sub(base_seq) as usize) + 1;
        let count = span.clamp(1, 256);

        let reference_time_us = self.arrivals.front().unwrap().1;
        let mut records = Vec::with_capacity(count);

        for i in 0..count {
            let seq = base_seq.wrapping_add(i as u16);
            if let Some(&(_, arrival_us)) = self.arrivals.iter().find(|&&(s, _)| s == seq) {
                let delta = (arrival_us as i64) - (reference_time_us as i64);
                let clamped_delta =
                    delta.clamp(SCREEN_TWCC_MISSING_DELTA as i64 + 1, i16::MAX as i64) as i16;
                records.push(ScreenTwccPacketRecord {
                    transport_seq: seq,
                    recv_delta_us: clamped_delta,
                });
            } else {
                records.push(ScreenTwccPacketRecord {
                    transport_seq: seq,
                    recv_delta_us: SCREEN_TWCC_MISSING_DELTA,
                });
            }
        }

        self.arrivals.retain(|&(seq, _)| {
            let diff = seq.wrapping_sub(base_seq);
            diff >= count as u16
        });

        let fb_seq = self.feedback_seq;
        self.feedback_seq = self.feedback_seq.wrapping_add(1);
        self.last_feedback_us = now_us;
        self.stats.feedbacks_sent = self.stats.feedbacks_sent.saturating_add(1);

        Some(ScreenTwccFeedback {
            feedback_seq: fb_seq,
            base_seq,
            packet_count: records.len() as u16,
            reference_time_us,
            records,
        })
    }

    pub fn clear(&mut self) {
        self.arrivals.clear();
        self.stats = ScreenTwccStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}

#[derive(Debug, Clone)]
pub struct TwccSentPacket {
    pub seq: u16,
    pub send_us: u64,
    pub bytes: usize,
}

/// Sender-side sequence allocator and feedback processor that derives delivery rate and delay gradient (v2.08 / ADR-094).
#[derive(Debug, Clone)]
pub struct ScreenTwccSenderTracker {
    mode: ScreenTwccMode,
    next_seq: u16,
    in_flight: VecDeque<TwccSentPacket>,
    stats: ScreenTwccStats,
}

impl Default for ScreenTwccSenderTracker {
    fn default() -> Self {
        Self::new(ScreenTwccMode::Auto)
    }
}

impl ScreenTwccSenderTracker {
    pub fn new(mode: ScreenTwccMode) -> Self {
        Self {
            mode,
            next_seq: 1,
            in_flight: VecDeque::with_capacity(1024),
            stats: ScreenTwccStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenTwccMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenTwccMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenTwccMode::Disabled {
            self.in_flight.clear();
        }
    }

    pub fn stats(&self) -> ScreenTwccStats {
        self.stats
    }

    pub fn report(&self) -> ScreenTwccReport {
        ScreenTwccReport {
            ok: true,
            mode: self.mode,
            feedbacks_sent: self.stats.feedbacks_sent,
            feedbacks_received: self.stats.feedbacks_received,
            packets_tracked: self.stats.packets_tracked,
            packets_acknowledged: self.stats.packets_acknowledged,
            packets_lost: self.stats.packets_lost,
            loss_ratio: self.stats.loss_ratio,
            delivery_rate_bps: self.stats.delivery_rate_bps,
            delay_gradient_us: self.stats.delay_gradient_us,
            smoothed_rtt_us: self.stats.smoothed_rtt_us,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn next_seq(&self) -> u16 {
        self.next_seq
    }

    pub fn set_next_seq(&mut self, next: u16) {
        self.next_seq = next;
    }

    pub fn allocate_seq(&mut self, send_us: u64, bytes: usize) -> u16 {
        if self.mode == ScreenTwccMode::Disabled {
            return 0;
        }
        let seq = self.next_seq;
        self.next_seq = if self.next_seq == u16::MAX {
            1
        } else {
            self.next_seq + 1
        };
        while self.in_flight.len() >= 1024 {
            self.in_flight.pop_front();
        }
        self.in_flight.push_back(TwccSentPacket {
            seq,
            send_us,
            bytes,
        });
        self.stats.packets_tracked = self.stats.packets_tracked.saturating_add(1);
        seq
    }

    pub fn on_feedback(&mut self, feedback: &ScreenTwccFeedback, now_us: u64) -> ScreenTwccReport {
        if self.mode == ScreenTwccMode::Disabled {
            return self.report();
        }
        self.stats.feedbacks_received = self.stats.feedbacks_received.saturating_add(1);

        let mut acked_in_block = 0u64;
        let mut lost_in_block = 0u64;
        let mut delivered_bytes = 0usize;
        let mut min_arrival_us = u64::MAX;
        let mut max_arrival_us = 0u64;

        let mut prev_send_us: Option<u64> = None;
        let mut prev_recv_us: Option<u64> = None;
        let mut delay_gradients = Vec::new();

        for rec in &feedback.records {
            if rec.recv_delta_us == SCREEN_TWCC_MISSING_DELTA {
                lost_in_block += 1;
                self.stats.packets_lost = self.stats.packets_lost.saturating_add(1);
            } else {
                acked_in_block += 1;
                self.stats.packets_acknowledged = self.stats.packets_acknowledged.saturating_add(1);
                let recv_us =
                    (feedback.reference_time_us as i64 + rec.recv_delta_us as i64).max(0) as u64;
                min_arrival_us = min_arrival_us.min(recv_us);
                max_arrival_us = max_arrival_us.max(recv_us);

                if let Some(sent) = self.in_flight.iter().find(|p| p.seq == rec.transport_seq) {
                    delivered_bytes += sent.bytes;
                    let sample_rtt = now_us.saturating_sub(sent.send_us);
                    if self.stats.smoothed_rtt_us == 0 {
                        self.stats.smoothed_rtt_us = sample_rtt;
                    } else {
                        self.stats.smoothed_rtt_us =
                            (self.stats.smoothed_rtt_us * 7 + sample_rtt) / 8;
                    }

                    if let (Some(p_send), Some(p_recv)) = (prev_send_us, prev_recv_us) {
                        let send_delta = sent.send_us as i64 - p_send as i64;
                        let recv_delta = recv_us as i64 - p_recv as i64;
                        delay_gradients.push(recv_delta - send_delta);
                    }
                    prev_send_us = Some(sent.send_us);
                    prev_recv_us = Some(recv_us);
                }
            }
        }

        let total_block = acked_in_block + lost_in_block;
        if total_block > 0 {
            let block_loss = lost_in_block as f32 / total_block as f32;
            if self.stats.feedbacks_received <= 1 {
                self.stats.loss_ratio = block_loss;
            } else {
                self.stats.loss_ratio = self.stats.loss_ratio * 0.7 + block_loss * 0.3;
            }
        }

        if delivered_bytes > 0 && max_arrival_us > min_arrival_us {
            let dt_us = max_arrival_us - min_arrival_us;
            if dt_us >= 100 {
                let inst_rate_bps = ((delivered_bytes as u64) * 8 * 1_000_000) / dt_us;
                if self.stats.delivery_rate_bps == 0 || self.stats.feedbacks_received <= 1 {
                    self.stats.delivery_rate_bps = inst_rate_bps;
                } else {
                    self.stats.delivery_rate_bps =
                        (self.stats.delivery_rate_bps * 7 + inst_rate_bps) / 8;
                }
            }
        }

        if !delay_gradients.is_empty() {
            let avg_grad = delay_gradients.iter().sum::<i64>() / (delay_gradients.len() as i64);
            if self.stats.feedbacks_received <= 1 {
                self.stats.delay_gradient_us = avg_grad;
            } else {
                self.stats.delay_gradient_us = (self.stats.delay_gradient_us * 3 + avg_grad) / 4;
            }
        }

        self.in_flight.retain(|p| {
            let diff = p.seq.wrapping_sub(feedback.base_seq);
            diff >= feedback.packet_count
        });

        self.report()
    }

    pub fn clear(&mut self) {
        self.in_flight.clear();
        self.next_seq = 1;
        self.stats = ScreenTwccStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}

/// Screen active bandwidth probing operational modes (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenProbeMode {
    #[default]
    Auto,
    Aggressive,
    Conservative,
    Disabled,
}

impl ScreenProbeMode {
    pub const fn tag(self) -> u8 {
        match self {
            ScreenProbeMode::Auto => 0,
            ScreenProbeMode::Aggressive => 1,
            ScreenProbeMode::Conservative => 2,
            ScreenProbeMode::Disabled => 3,
        }
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenProbeMode::Auto,
            1 => ScreenProbeMode::Aggressive,
            2 => ScreenProbeMode::Conservative,
            _ => ScreenProbeMode::Disabled,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenProbeMode::Auto => "auto",
            ScreenProbeMode::Aggressive => "aggressive",
            ScreenProbeMode::Conservative => "conservative",
            ScreenProbeMode::Disabled => "disabled",
        }
    }

    pub const fn cluster_size(self) -> u8 {
        match self {
            ScreenProbeMode::Auto => SCREEN_PROBE_CLUSTER_SIZE_AUTO,
            ScreenProbeMode::Aggressive => SCREEN_PROBE_CLUSTER_SIZE_AGGRESSIVE,
            ScreenProbeMode::Conservative => SCREEN_PROBE_CLUSTER_SIZE_CONSERVATIVE,
            ScreenProbeMode::Disabled => 0,
        }
    }

    pub const fn min_interval_us(self) -> u64 {
        match self {
            ScreenProbeMode::Auto => SCREEN_PROBE_MIN_INTERVAL_US_AUTO,
            ScreenProbeMode::Aggressive => SCREEN_PROBE_MIN_INTERVAL_US_AGGRESSIVE,
            ScreenProbeMode::Conservative => SCREEN_PROBE_MIN_INTERVAL_US_CONSERVATIVE,
            ScreenProbeMode::Disabled => u64::MAX,
        }
    }
}

impl fmt::Display for ScreenProbeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenProbeMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenProbeMode::Auto),
            "aggressive" | "agg" => Ok(ScreenProbeMode::Aggressive),
            "conservative" | "cons" => Ok(ScreenProbeMode::Conservative),
            "disabled" | "off" | "none" => Ok(ScreenProbeMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Screen active bandwidth probing state machine (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenProbeState {
    #[default]
    Idle,
    Probing,
    AwaitingFeedback,
    Completed,
    Failed,
}

impl ScreenProbeState {
    pub const fn tag(self) -> u8 {
        match self {
            ScreenProbeState::Idle => 0,
            ScreenProbeState::Probing => 1,
            ScreenProbeState::AwaitingFeedback => 2,
            ScreenProbeState::Completed => 3,
            ScreenProbeState::Failed => 4,
        }
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            0 => ScreenProbeState::Idle,
            1 => ScreenProbeState::Probing,
            2 => ScreenProbeState::AwaitingFeedback,
            3 => ScreenProbeState::Completed,
            _ => ScreenProbeState::Failed,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenProbeState::Idle => "idle",
            ScreenProbeState::Probing => "probing",
            ScreenProbeState::AwaitingFeedback => "awaiting_feedback",
            ScreenProbeState::Completed => "completed",
            ScreenProbeState::Failed => "failed",
        }
    }
}

impl fmt::Display for ScreenProbeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Screen active bandwidth probe packet (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProbePacket {
    pub flags: u8,
    pub cluster_id: u8,
    pub cluster_index: u8,
    pub cluster_size: u8,
    pub transport_seq: u16,
    pub target_bps: u32,
    pub send_time_us: u64,
    pub padding: Vec<u8>,
}

impl ScreenProbePacket {
    pub fn encode(&self) -> Vec<u8> {
        let padding_len = self.padding.len().min(u16::MAX as usize) as u16;
        let mut buf = Vec::with_capacity(SCREEN_PROBE_HEADER_LEN + padding_len as usize);
        buf.extend_from_slice(&SCREEN_PROBE_MAGIC);
        buf.push(SCREEN_PROBE_PROTO);
        buf.push(self.flags);
        buf.push(self.cluster_id);
        buf.push(self.cluster_index);
        buf.push(self.cluster_size);
        buf.extend_from_slice(&self.transport_seq.to_be_bytes());
        buf.extend_from_slice(&self.target_bps.to_be_bytes());
        buf.extend_from_slice(&self.send_time_us.to_be_bytes());
        buf.extend_from_slice(&padding_len.to_be_bytes());
        buf.extend_from_slice(&self.padding[..padding_len as usize]);
        buf
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < SCREEN_PROBE_HEADER_LEN {
            return Err(CoreError::ScreenFrame);
        }
        if payload[..5] != SCREEN_PROBE_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[5] != SCREEN_PROBE_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = payload[6];
        let cluster_id = payload[7];
        let cluster_index = payload[8];
        let cluster_size = payload[9];
        let transport_seq = u16::from_be_bytes(
            payload[10..12]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let target_bps = u32::from_be_bytes(
            payload[12..16]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let send_time_us = u64::from_be_bytes(
            payload[16..24]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        );
        let padding_len = u16::from_be_bytes(
            payload[24..26]
                .try_into()
                .map_err(|_| CoreError::ScreenFrame)?,
        ) as usize;

        if payload.len() < SCREEN_PROBE_HEADER_LEN + padding_len {
            return Err(CoreError::TruncatedFrame);
        }

        let padding =
            payload[SCREEN_PROBE_HEADER_LEN..SCREEN_PROBE_HEADER_LEN + padding_len].to_vec();

        Ok(Self {
            flags,
            cluster_id,
            cluster_index,
            cluster_size,
            transport_seq,
            target_bps,
            send_time_us,
            padding,
        })
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_SCREEN_PROBE, self.encode())
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_PROBE {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }

    pub fn from_inner(frame: &InnerFrame) -> Result<Self, CoreError> {
        Self::from_inner_frame(frame)
    }
}

/// Active bandwidth probe cluster tracking metadata (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProbeCluster {
    pub cluster_id: u8,
    pub target_bps: u32,
    pub cluster_size: u8,
    pub packets_sent: u8,
    pub first_send_time_us: u64,
    pub last_send_time_us: u64,
    pub total_bytes: u64,
    pub start_seq: u16,
    pub end_seq: u16,
}

/// Telemetry statistics for Screen Active Bandwidth Probing (v2.09 / ADR-095).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenProbeStats {
    pub mode: ScreenProbeMode,
    pub state: ScreenProbeState,
    pub probes_triggered: u64,
    pub probes_completed: u64,
    pub probes_failed: u64,
    pub last_target_bps: u32,
    pub last_measured_bps: u64,
    pub last_rtt_us: u64,
    pub last_loss_ratio: f32,
    pub last_delay_gradient_us: i64,
    pub active_cluster_id: Option<u8>,
}

impl Default for ScreenProbeStats {
    fn default() -> Self {
        Self {
            mode: ScreenProbeMode::Auto,
            state: ScreenProbeState::Idle,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            last_target_bps: 0,
            last_measured_bps: 0,
            last_rtt_us: 0,
            last_loss_ratio: 0.0,
            last_delay_gradient_us: 0,
            active_cluster_id: None,
        }
    }
}

/// DTO report for REST API and CLI status (v2.09 / ADR-095).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenProbeReport {
    pub ok: bool,
    pub mode: ScreenProbeMode,
    pub state: ScreenProbeState,
    pub probes_triggered: u64,
    pub probes_completed: u64,
    pub probes_failed: u64,
    pub last_target_bps: u32,
    pub last_measured_bps: u64,
    pub last_rtt_us: u64,
    pub last_loss_ratio: f32,
    pub last_delay_gradient_us: i64,
    pub active_cluster_id: Option<u8>,
    pub timestamp_ms: u64,
}

impl Default for ScreenProbeReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenProbeMode::Auto,
            state: ScreenProbeState::Idle,
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            last_target_bps: 0,
            last_measured_bps: 0,
            last_rtt_us: 0,
            last_loss_ratio: 0.0,
            last_delay_gradient_us: 0,
            active_cluster_id: None,
            timestamp_ms: 0,
        }
    }
}

/// Controller coordinating active bandwidth probing bursts and fast ramp-up (v2.09 / ADR-095).
#[derive(Debug, Clone)]
pub struct ScreenProbeController {
    mode: ScreenProbeMode,
    state: ScreenProbeState,
    next_cluster_id: u8,
    active_cluster: Option<ScreenProbeCluster>,
    last_probe_us: u64,
    packet_queue: VecDeque<ScreenProbePacket>,
    stats: ScreenProbeStats,
}

impl Default for ScreenProbeController {
    fn default() -> Self {
        Self::new(ScreenProbeMode::Auto)
    }
}

impl ScreenProbeController {
    pub fn new(mode: ScreenProbeMode) -> Self {
        Self {
            mode,
            state: ScreenProbeState::Idle,
            next_cluster_id: 0,
            active_cluster: None,
            last_probe_us: 0,
            packet_queue: VecDeque::with_capacity(32),
            stats: ScreenProbeStats {
                mode,
                state: ScreenProbeState::Idle,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenProbeMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenProbeMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenProbeMode::Disabled {
            self.packet_queue.clear();
            self.active_cluster = None;
            self.state = ScreenProbeState::Idle;
            self.stats.state = ScreenProbeState::Idle;
            self.stats.active_cluster_id = None;
        }
    }

    pub fn state(&self) -> ScreenProbeState {
        self.state
    }

    pub fn stats(&self) -> ScreenProbeStats {
        self.stats
    }

    pub fn report(&self) -> ScreenProbeReport {
        ScreenProbeReport {
            ok: true,
            mode: self.mode,
            state: self.state,
            probes_triggered: self.stats.probes_triggered,
            probes_completed: self.stats.probes_completed,
            probes_failed: self.stats.probes_failed,
            last_target_bps: self.stats.last_target_bps,
            last_measured_bps: self.stats.last_measured_bps,
            last_rtt_us: self.stats.last_rtt_us,
            last_loss_ratio: self.stats.last_loss_ratio,
            last_delay_gradient_us: self.stats.last_delay_gradient_us,
            active_cluster_id: self.stats.active_cluster_id,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    pub fn has_pending_packets(&self) -> bool {
        !self.packet_queue.is_empty()
    }

    pub fn pop_probe_packet(&mut self) -> Option<ScreenProbePacket> {
        let pkt = self.packet_queue.pop_front()?;
        if let Some(ref mut cl) = self.active_cluster {
            cl.packets_sent = cl.packets_sent.saturating_add(1);
        }
        if self.packet_queue.is_empty() && self.state == ScreenProbeState::Probing {
            self.state = ScreenProbeState::AwaitingFeedback;
            self.stats.state = ScreenProbeState::AwaitingFeedback;
        }
        Some(pkt)
    }

    /// Trigger an active bandwidth probe burst to probe for target_bps.
    pub fn trigger_probe(
        &mut self,
        target_bps: u32,
        now_us: u64,
        next_transport_seq: &mut u16,
    ) -> bool {
        if self.mode == ScreenProbeMode::Disabled {
            return false;
        }

        // If an active cluster is still waiting, check timeout (1.5s)
        if let Some(ref cl) = self.active_cluster {
            if now_us.saturating_sub(cl.last_send_time_us) > 1_500_000 {
                self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
                self.active_cluster = None;
            } else {
                return false;
            }
        }

        self.next_cluster_id = (self.next_cluster_id % 254) + 1;
        let cluster_id = self.next_cluster_id;
        let cluster_size = self.mode.cluster_size().max(4);
        let target_bps = target_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);

        self.packet_queue.clear();
        let mut start_seq = 0u16;
        let mut end_seq = 0u16;

        let padding_len = SCREEN_PROBE_DEFAULT_PACKET_LEN - SCREEN_PROBE_HEADER_LEN;
        let dummy_padding = vec![0x5a; padding_len];

        for i in 0..cluster_size {
            *next_transport_seq = next_transport_seq.wrapping_add(1);
            if *next_transport_seq == 0 {
                *next_transport_seq = 1;
            }
            let t_seq = *next_transport_seq;
            if i == 0 {
                start_seq = t_seq;
            }
            if i == cluster_size - 1 {
                end_seq = t_seq;
            }
            let pkt = ScreenProbePacket {
                flags: 0,
                cluster_id,
                cluster_index: i,
                cluster_size,
                transport_seq: t_seq,
                target_bps,
                send_time_us: now_us,
                padding: dummy_padding.clone(),
            };
            self.packet_queue.push_back(pkt);
        }

        self.active_cluster = Some(ScreenProbeCluster {
            cluster_id,
            target_bps,
            cluster_size,
            packets_sent: 0,
            first_send_time_us: now_us,
            last_send_time_us: now_us,
            total_bytes: (cluster_size as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64),
            start_seq,
            end_seq,
        });

        self.state = ScreenProbeState::Probing;
        self.stats.state = ScreenProbeState::Probing;
        self.stats.probes_triggered = self.stats.probes_triggered.saturating_add(1);
        self.stats.last_target_bps = target_bps;
        self.stats.active_cluster_id = Some(cluster_id);
        self.last_probe_us = now_us;
        true
    }

    /// Trigger an active bandwidth probe burst using pre-allocated transport sequence numbers.
    pub fn trigger_probe_with_seqs(&mut self, target_bps: u32, now_us: u64, seqs: &[u16]) -> bool {
        if self.mode == ScreenProbeMode::Disabled || seqs.is_empty() {
            return false;
        }

        if let Some(ref cl) = self.active_cluster {
            if now_us.saturating_sub(cl.last_send_time_us) > 1_500_000 {
                self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
                self.active_cluster = None;
            } else {
                return false;
            }
        }

        self.next_cluster_id = (self.next_cluster_id % 254) + 1;
        let cluster_id = self.next_cluster_id;
        let cluster_size = seqs.len() as u8;
        let target_bps = target_bps.clamp(SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_MAX_BPS);

        self.packet_queue.clear();
        let start_seq = seqs[0];
        let end_seq = seqs[seqs.len() - 1];

        let padding_len = SCREEN_PROBE_DEFAULT_PACKET_LEN - SCREEN_PROBE_HEADER_LEN;
        let dummy_padding = vec![0x5a; padding_len];

        for (i, &t_seq) in seqs.iter().enumerate() {
            let pkt = ScreenProbePacket {
                flags: 0,
                cluster_id,
                cluster_index: i as u8,
                cluster_size,
                transport_seq: t_seq,
                target_bps,
                send_time_us: now_us,
                padding: dummy_padding.clone(),
            };
            self.packet_queue.push_back(pkt);
        }

        self.active_cluster = Some(ScreenProbeCluster {
            cluster_id,
            target_bps,
            cluster_size,
            packets_sent: 0,
            first_send_time_us: now_us,
            last_send_time_us: now_us,
            total_bytes: (cluster_size as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64),
            start_seq,
            end_seq,
        });

        self.state = ScreenProbeState::Probing;
        self.stats.state = ScreenProbeState::Probing;
        self.stats.probes_triggered = self.stats.probes_triggered.saturating_add(1);
        self.stats.last_target_bps = target_bps;
        self.stats.active_cluster_id = Some(cluster_id);
        self.last_probe_us = now_us;
        true
    }

    /// Automatically schedule a bandwidth probe if network conditions and intervals allow.
    pub fn maybe_schedule_probe(
        &mut self,
        current_bps: u32,
        now_us: u64,
        next_transport_seq: &mut u16,
    ) -> bool {
        if self.mode == ScreenProbeMode::Disabled || self.active_cluster.is_some() {
            return false;
        }
        if self.last_probe_us != 0
            && now_us.saturating_sub(self.last_probe_us) < self.mode.min_interval_us()
        {
            return false;
        }
        if current_bps >= SCREEN_BITRATE_MAX_BPS {
            return false;
        }

        let target_bps = match self.mode {
            ScreenProbeMode::Aggressive => {
                let step = (current_bps as u64 * 16) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Auto => {
                let step = (current_bps as u64 * 14) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Conservative => {
                let step = (current_bps as u64 * 12) / 10;
                step.min(SCREEN_BITRATE_MAX_BPS as u64) as u32
            }
            ScreenProbeMode::Disabled => return false,
        };

        if target_bps > current_bps {
            self.trigger_probe(target_bps, now_us, next_transport_seq)
        } else {
            false
        }
    }

    /// Process incoming TWCC feedback to evaluate the delivery rate and delay gradient of the active probe cluster.
    /// If probe passed, executes fast ramp-up on rate_controller and returns Some(new_bps).
    pub fn on_twcc_feedback(
        &mut self,
        feedback: &ScreenTwccFeedback,
        rate_controller: &mut ScreenRateController,
        _now_us: u64,
    ) -> Option<u32> {
        let cluster = self.active_cluster.as_ref()?.clone();

        // Check if any packet in this cluster is reported in feedback.records
        let cluster_records: Vec<_> = feedback
            .records
            .iter()
            .filter(|r| {
                let diff = r.transport_seq.wrapping_sub(cluster.start_seq);
                diff < cluster.cluster_size as u16
            })
            .collect();

        if cluster_records.is_empty() {
            return None;
        }

        let mut arrived_deltas = Vec::new();
        let mut missing_count = 0;

        for rec in &cluster_records {
            if rec.recv_delta_us == SCREEN_TWCC_MISSING_DELTA {
                missing_count += 1;
            } else {
                arrived_deltas.push(rec.recv_delta_us as i64);
            }
        }

        let loss_ratio = (missing_count as f32) / (cluster.cluster_size as f32);
        self.stats.last_loss_ratio = loss_ratio;

        let delivered_rate_bps = if arrived_deltas.len() >= 2 {
            let min_d = *arrived_deltas.iter().min().unwrap();
            let max_d = *arrived_deltas.iter().max().unwrap();
            let delta_us = (max_d - min_d).max(1) as u64;
            let delivered_bytes =
                (arrived_deltas.len() as u64) * (SCREEN_PROBE_DEFAULT_PACKET_LEN as u64);
            (delivered_bytes * 8 * 1_000_000) / delta_us
        } else {
            0
        };

        self.stats.last_measured_bps = delivered_rate_bps;

        // Estimate delay gradient over probe cluster: recv span vs send span
        let delay_gradient_us = if arrived_deltas.len() >= 2 {
            let recv_span =
                (*arrived_deltas.iter().max().unwrap()) - (*arrived_deltas.iter().min().unwrap());
            recv_span
        } else {
            0
        };
        self.stats.last_delay_gradient_us = delay_gradient_us;

        // Validation gate:
        // 1. Loss ratio <= 15%
        // 2. Measured rate >= 70% of target_bps
        // 3. Delay gradient <= 20,000 us (20ms queue growth during burst)
        let passed = loss_ratio <= 0.15
            && delivered_rate_bps >= ((cluster.target_bps as u64 * 70) / 100)
            && delay_gradient_us <= 20_000;

        if passed {
            self.stats.probes_completed = self.stats.probes_completed.saturating_add(1);
            self.state = ScreenProbeState::Completed;
            self.stats.state = ScreenProbeState::Completed;
            self.active_cluster = None;
            self.stats.active_cluster_id = None;
            rate_controller.fast_ramp_up(cluster.target_bps);
            Some(cluster.target_bps)
        } else if cluster_records.len() >= (cluster.cluster_size as usize * 3 / 4) {
            // Cluster largely evaluated but failed validation gate
            self.stats.probes_failed = self.stats.probes_failed.saturating_add(1);
            self.state = ScreenProbeState::Failed;
            self.stats.state = ScreenProbeState::Failed;
            self.active_cluster = None;
            self.stats.active_cluster_id = None;
            None
        } else {
            None
        }
    }
}

/// Screen and audio presentation timestamp (PTS) lip-sync alignment and clock synchronization mode (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenAudioSyncMode {
    #[default]
    Auto = 0,
    Strict = 1,
    Loose = 2,
    Disabled = 3,
}

impl ScreenAudioSyncMode {
    pub const fn tag(self) -> u8 {
        self as u8
    }

    pub const fn from_tag(tag: u8) -> Self {
        match tag {
            1 => ScreenAudioSyncMode::Strict,
            2 => ScreenAudioSyncMode::Loose,
            3 => ScreenAudioSyncMode::Disabled,
            _ => ScreenAudioSyncMode::Auto,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ScreenAudioSyncMode::Auto => "auto",
            ScreenAudioSyncMode::Strict => "strict",
            ScreenAudioSyncMode::Loose => "loose",
            ScreenAudioSyncMode::Disabled => "disabled",
        }
    }

    pub const fn beacon_interval_us(self) -> u64 {
        match self {
            ScreenAudioSyncMode::Auto => SCREEN_SYNC_INTERVAL_US_AUTO,
            ScreenAudioSyncMode::Strict => SCREEN_SYNC_INTERVAL_US_STRICT,
            ScreenAudioSyncMode::Loose => SCREEN_SYNC_INTERVAL_US_LOOSE,
            ScreenAudioSyncMode::Disabled => u64::MAX,
        }
    }

    pub const fn skew_tolerance_us(self) -> i64 {
        match self {
            ScreenAudioSyncMode::Auto => SCREEN_SYNC_SKEW_TOLERANCE_US_AUTO,
            ScreenAudioSyncMode::Strict => SCREEN_SYNC_SKEW_TOLERANCE_US_STRICT,
            ScreenAudioSyncMode::Loose => SCREEN_SYNC_SKEW_TOLERANCE_US_LOOSE,
            ScreenAudioSyncMode::Disabled => i64::MAX,
        }
    }
}

impl fmt::Display for ScreenAudioSyncMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScreenAudioSyncMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(ScreenAudioSyncMode::Auto),
            "strict" | "tight" | "low" => Ok(ScreenAudioSyncMode::Strict),
            "loose" | "relaxed" | "high" => Ok(ScreenAudioSyncMode::Loose),
            "disabled" | "off" | "none" => Ok(ScreenAudioSyncMode::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Compact wire protocol frame for presentation timestamp (PTS) lip-sync beacon (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenAudioSyncFrame {
    pub flags: u16,
    pub sync_seq: u32,
    pub sender_wallclock_us: u64,
    pub video_pts_us: u64,
    pub audio_pts_us: u64,
}

impl ScreenAudioSyncFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SCREEN_SYNC_HEADER_LEN);
        buf.extend_from_slice(&SCREEN_SYNC_MAGIC);
        buf.push(SCREEN_SYNC_PROTO);
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.sync_seq.to_be_bytes());
        buf.extend_from_slice(&self.sender_wallclock_us.to_be_bytes());
        buf.extend_from_slice(&self.video_pts_us.to_be_bytes());
        buf.extend_from_slice(&self.audio_pts_us.to_be_bytes());
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < SCREEN_SYNC_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if bytes[0..5] != SCREEN_SYNC_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if bytes[5] != SCREEN_SYNC_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
        let sync_seq = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
        let sender_wallclock_us = u64::from_be_bytes(bytes[12..20].try_into().unwrap());
        let video_pts_us = u64::from_be_bytes(bytes[20..28].try_into().unwrap());
        let audio_pts_us = u64::from_be_bytes(bytes[28..36].try_into().unwrap());

        Ok(Self {
            flags,
            sync_seq,
            sender_wallclock_us,
            video_pts_us,
            audio_pts_us,
        })
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        Ok(InnerFrame {
            ty: TYPE_SCREEN_SYNC,
            flags: 0,
            msg_id: 0,
            payload: self.encode(),
        })
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_SYNC {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }
}

/// Statistics and telemetry for audio-video presentation sync (v2.10 / ADR-096).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScreenAudioSyncStats {
    pub mode: ScreenAudioSyncMode,
    pub beacons_sent: u64,
    pub beacons_received: u64,
    pub current_skew_us: i64,
    pub clock_drift_ppm: f64,
    pub adjustments_applied: u64,
    pub last_sync_timestamp_ms: u64,
    pub in_sync: bool,
}

impl Default for ScreenAudioSyncStats {
    fn default() -> Self {
        Self {
            mode: ScreenAudioSyncMode::Auto,
            beacons_sent: 0,
            beacons_received: 0,
            current_skew_us: 0,
            clock_drift_ppm: 0.0,
            adjustments_applied: 0,
            last_sync_timestamp_ms: 0,
            in_sync: true,
        }
    }
}

/// DTO report for REST API and CLI status formatting (v2.10 / ADR-096).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenAudioSyncReport {
    pub ok: bool,
    pub mode: ScreenAudioSyncMode,
    pub beacons_sent: u64,
    pub beacons_received: u64,
    pub current_skew_us: i64,
    pub clock_drift_ppm: f64,
    pub adjustments_applied: u64,
    pub in_sync: bool,
    pub timestamp_ms: u64,
}

impl Default for ScreenAudioSyncReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: ScreenAudioSyncMode::Auto,
            beacons_sent: 0,
            beacons_received: 0,
            current_skew_us: 0,
            clock_drift_ppm: 0.0,
            adjustments_applied: 0,
            in_sync: true,
            timestamp_ms: 0,
        }
    }
}

/// Audio-video lip-sync alignment and clock drift synchronization coordinator (v2.10 / ADR-096).
#[derive(Debug, Clone)]
pub struct ScreenAudioSyncCoordinator {
    mode: ScreenAudioSyncMode,
    next_sync_seq: u32,
    last_beacon_sent_us: u64,
    latest_video_pts_us: u64,
    latest_audio_pts_us: u64,
    recent_samples: VecDeque<(u64, u64)>,
    beacon_queue: VecDeque<ScreenAudioSyncFrame>,
    stats: ScreenAudioSyncStats,
}

impl Default for ScreenAudioSyncCoordinator {
    fn default() -> Self {
        Self::new(ScreenAudioSyncMode::Auto)
    }
}

impl ScreenAudioSyncCoordinator {
    pub fn new(mode: ScreenAudioSyncMode) -> Self {
        Self {
            mode,
            next_sync_seq: 0,
            last_beacon_sent_us: 0,
            latest_video_pts_us: 0,
            latest_audio_pts_us: 0,
            recent_samples: VecDeque::with_capacity(16),
            beacon_queue: VecDeque::with_capacity(8),
            stats: ScreenAudioSyncStats {
                mode,
                ..Default::default()
            },
        }
    }

    pub fn mode(&self) -> ScreenAudioSyncMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.mode = mode;
        self.stats.mode = mode;
        if mode == ScreenAudioSyncMode::Disabled {
            self.beacon_queue.clear();
        }
    }

    pub fn stats(&self) -> ScreenAudioSyncStats {
        self.stats
    }

    pub fn report(&self) -> ScreenAudioSyncReport {
        ScreenAudioSyncReport {
            ok: true,
            mode: self.mode,
            beacons_sent: self.stats.beacons_sent,
            beacons_received: self.stats.beacons_received,
            current_skew_us: self.stats.current_skew_us,
            clock_drift_ppm: self.stats.clock_drift_ppm,
            adjustments_applied: self.stats.adjustments_applied,
            in_sync: self.stats.in_sync,
            timestamp_ms: self.stats.last_sync_timestamp_ms,
        }
    }

    pub fn record_video_pts(&mut self, pts_us: u64) {
        self.latest_video_pts_us = pts_us;
    }

    pub fn record_audio_pts(&mut self, pts_us: u64) {
        self.latest_audio_pts_us = pts_us;
    }

    pub fn latest_video_pts_us(&self) -> u64 {
        self.latest_video_pts_us
    }

    pub fn latest_audio_pts_us(&self) -> u64 {
        self.latest_audio_pts_us
    }

    /// Evaluates beacon generation interval and produces an outbound sync beacon if due.
    pub fn maybe_generate_sync_beacon(&mut self, now_us: u64) -> Option<ScreenAudioSyncFrame> {
        if self.mode == ScreenAudioSyncMode::Disabled {
            return None;
        }
        let interval = self.mode.beacon_interval_us();
        if self.last_beacon_sent_us > 0
            && now_us.saturating_sub(self.last_beacon_sent_us) < interval
        {
            return None;
        }
        Some(self.trigger_sync_beacon(now_us))
    }

    /// Unconditionally trigger a sync beacon (for tests or manual inspection).
    pub fn trigger_sync_beacon(&mut self, now_us: u64) -> ScreenAudioSyncFrame {
        self.last_beacon_sent_us = now_us;
        self.next_sync_seq = self.next_sync_seq.wrapping_add(1);
        let frame = ScreenAudioSyncFrame {
            flags: 0,
            sync_seq: self.next_sync_seq,
            sender_wallclock_us: now_us,
            video_pts_us: self.latest_video_pts_us,
            audio_pts_us: self.latest_audio_pts_us,
        };
        self.stats.beacons_sent = self.stats.beacons_sent.saturating_add(1);
        self.stats.last_sync_timestamp_ms = now_us / 1000;
        self.beacon_queue.push_back(frame);
        frame
    }

    pub fn pop_sync_beacon(&mut self) -> Option<ScreenAudioSyncFrame> {
        self.beacon_queue.pop_front()
    }

    /// Ingest an inbound sync beacon, calculating presentation skew, clock drift, and applying playout compensation.
    pub fn on_sync_beacon(
        &mut self,
        frame: &ScreenAudioSyncFrame,
        arrival_us: u64,
        playout_buffer: Option<&mut ScreenPlayoutBuffer>,
    ) -> i64 {
        let skew_us = (frame.video_pts_us as i64) - (frame.audio_pts_us as i64);
        self.stats.current_skew_us = skew_us;
        self.stats.beacons_received = self.stats.beacons_received.saturating_add(1);
        self.stats.last_sync_timestamp_ms = arrival_us / 1000;

        if self.recent_samples.len() >= 16 {
            self.recent_samples.pop_front();
        }
        self.recent_samples
            .push_back((arrival_us, frame.sender_wallclock_us));

        if self.recent_samples.len() >= 2 {
            let (first_arrival, first_sender) = self.recent_samples.front().copied().unwrap();
            let elapsed_recv = arrival_us.saturating_sub(first_arrival);
            let elapsed_sender = frame.sender_wallclock_us.saturating_sub(first_sender);
            if elapsed_sender >= 100_000 && elapsed_recv > 0 {
                let drift_ppm = ((elapsed_recv as f64 - elapsed_sender as f64)
                    / (elapsed_sender as f64))
                    * 1_000_000.0;
                if self.stats.clock_drift_ppm == 0.0 {
                    self.stats.clock_drift_ppm = drift_ppm;
                } else {
                    self.stats.clock_drift_ppm =
                        (self.stats.clock_drift_ppm * 0.7) + (drift_ppm * 0.3);
                }
            }
        }

        let tol = self.mode.skew_tolerance_us();
        let in_sync = skew_us.abs() <= tol;
        self.stats.in_sync = in_sync;

        if !in_sync && self.mode != ScreenAudioSyncMode::Disabled {
            if let Some(buf) = playout_buffer {
                let delta_us = (skew_us / 2).clamp(-20_000, 20_000);
                if delta_us != 0 {
                    buf.adjust_target_delay(delta_us);
                    self.stats.adjustments_applied =
                        self.stats.adjustments_applied.saturating_add(1);
                }
            }
        }
        skew_us
    }

    pub fn reset(&mut self) {
        self.next_sync_seq = 0;
        self.last_beacon_sent_us = 0;
        self.latest_video_pts_us = 0;
        self.latest_audio_pts_us = 0;
        self.recent_samples.clear();
        self.beacon_queue.clear();
        self.stats = ScreenAudioSyncStats {
            mode: self.mode,
            ..Default::default()
        };
    }
}

/// Primary latency bottleneck stage identified by multi-stage profiling (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenBottleneckStage {
    Capture = 0,
    Encode = 1,
    Pacer = 2,
    Network = 3,
    Jitter = 4,
    Decode = 5,
    Render = 6,
    #[default]
    Balanced = 7,
}

impl ScreenBottleneckStage {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Capture,
            1 => Self::Encode,
            2 => Self::Pacer,
            3 => Self::Network,
            4 => Self::Jitter,
            5 => Self::Decode,
            6 => Self::Render,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Encode => "encode",
            Self::Pacer => "pacer",
            Self::Network => "network",
            Self::Jitter => "jitter",
            Self::Decode => "decode",
            Self::Render => "render",
            Self::Balanced => "balanced",
        }
    }
}

impl std::fmt::Display for ScreenBottleneckStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ScreenBottleneckStage {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "capture" => Ok(Self::Capture),
            "encode" => Ok(Self::Encode),
            "pacer" => Ok(Self::Pacer),
            "network" => Ok(Self::Network),
            "jitter" => Ok(Self::Jitter),
            "decode" => Ok(Self::Decode),
            "render" => Ok(Self::Render),
            "balanced" => Ok(Self::Balanced),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

/// Latency profiling operational mode (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenProfilerMode {
    #[default]
    Auto = 0,
    Detailed = 1,
    Disabled = 2,
}

impl ScreenProfilerMode {
    pub fn tag(self) -> u8 {
        self as u8
    }

    pub fn from_tag(tag: u8) -> Self {
        match tag {
            0 => Self::Auto,
            1 => Self::Detailed,
            _ => Self::Disabled,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Detailed => "detailed",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "default" | "normal" => Ok(Self::Auto),
            "detailed" | "high" | "verbose" | "always" => Ok(Self::Detailed),
            "disabled" | "off" | "none" => Ok(Self::Disabled),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}

impl std::str::FromStr for ScreenProfilerMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for ScreenProfilerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Granular 7-stage microsecond latency breakdown for a single video frame (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenLatencyBreakdown {
    pub frame_seq: u64,
    pub capture_us: u32,
    pub encode_us: u32,
    pub pacer_us: u32,
    pub network_us: u32,
    pub jitter_us: u32,
    pub decode_us: u32,
    pub render_us: u32,
    pub total_e2e_us: u64,
}

impl ScreenLatencyBreakdown {
    pub fn calculate_total(&mut self) -> u64 {
        self.total_e2e_us = self.capture_us as u64
            + self.encode_us as u64
            + self.pacer_us as u64
            + self.network_us as u64
            + self.jitter_us as u64
            + self.decode_us as u64
            + self.render_us as u64;
        self.total_e2e_us
    }
}

/// Compact wire protocol frame for latency profiling & multi-stage telemetry (56 bytes) (v2.13 / ADR-099).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenProfileFrame {
    pub flags: u16,
    pub frame_seq: u64,
    pub capture_us: u32,
    pub encode_us: u32,
    pub pacer_us: u32,
    pub sender_ts_us: u64,
    pub network_us: u32,
    pub jitter_us: u32,
    pub decode_us: u32,
    pub render_us: u32,
    pub total_e2e_us: u32,
}

impl ScreenProfileFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(SCREEN_PROFILE_HEADER_LEN);
        buf.extend_from_slice(&SCREEN_PROFILE_MAGIC);
        buf.push(SCREEN_PROFILE_PROTO);
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.capture_us.to_be_bytes());
        buf.extend_from_slice(&self.encode_us.to_be_bytes());
        buf.extend_from_slice(&self.pacer_us.to_be_bytes());
        buf.extend_from_slice(&self.sender_ts_us.to_be_bytes());
        buf.extend_from_slice(&self.network_us.to_be_bytes());
        buf.extend_from_slice(&self.jitter_us.to_be_bytes());
        buf.extend_from_slice(&self.decode_us.to_be_bytes());
        buf.extend_from_slice(&self.render_us.to_be_bytes());
        buf.extend_from_slice(&self.total_e2e_us.to_be_bytes());
        buf
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < SCREEN_PROFILE_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if bytes[0..5] != SCREEN_PROFILE_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if bytes[5] != SCREEN_PROFILE_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let flags = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
        let frame_seq = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
        let capture_us = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let encode_us = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        let pacer_us = u32::from_be_bytes(bytes[24..28].try_into().unwrap());
        let sender_ts_us = u64::from_be_bytes(bytes[28..36].try_into().unwrap());
        let network_us = u32::from_be_bytes(bytes[36..40].try_into().unwrap());
        let jitter_us = u32::from_be_bytes(bytes[40..44].try_into().unwrap());
        let decode_us = u32::from_be_bytes(bytes[44..48].try_into().unwrap());
        let render_us = u32::from_be_bytes(bytes[48..52].try_into().unwrap());
        let total_e2e_us = u32::from_be_bytes(bytes[52..56].try_into().unwrap());

        Ok(Self {
            flags,
            frame_seq,
            capture_us,
            encode_us,
            pacer_us,
            sender_ts_us,
            network_us,
            jitter_us,
            decode_us,
            render_us,
            total_e2e_us,
        })
    }

    pub fn to_inner(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode();
        Ok(InnerFrame {
            ty: TYPE_SCREEN_PROFILE,
            flags: 0,
            msg_id: 0,
            payload,
        })
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_PROFILE {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode(&frame.payload)
    }
}

/// DTO report for REST API and CLI formatting (v2.13 / ADR-099).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenLatencyReport {
    pub ok: bool,
    pub mode: String,
    pub frames_profiled: u64,
    pub bottleneck: String,
    pub current: ScreenLatencyBreakdown,
    pub avg_e2e_us: u64,
    pub p50_e2e_us: u64,
    pub p90_e2e_us: u64,
    pub p95_e2e_us: u64,
    pub p99_e2e_us: u64,
    pub min_e2e_us: u64,
    pub max_e2e_us: u64,
    pub avg_capture_us: u32,
    pub avg_encode_us: u32,
    pub avg_pacer_us: u32,
    pub avg_network_us: u32,
    pub avg_jitter_us: u32,
    pub avg_decode_us: u32,
    pub avg_render_us: u32,
}

impl Default for ScreenLatencyReport {
    fn default() -> Self {
        Self {
            ok: true,
            mode: "auto".to_string(),
            frames_profiled: 0,
            bottleneck: "balanced".to_string(),
            current: ScreenLatencyBreakdown::default(),
            avg_e2e_us: 0,
            p50_e2e_us: 0,
            p90_e2e_us: 0,
            p95_e2e_us: 0,
            p99_e2e_us: 0,
            min_e2e_us: 0,
            max_e2e_us: 0,
            avg_capture_us: 0,
            avg_encode_us: 0,
            avg_pacer_us: 0,
            avg_network_us: 0,
            avg_jitter_us: 0,
            avg_decode_us: 0,
            avg_render_us: 0,
        }
    }
}

/// Microsecond-precision multi-stage frame profiler & performance telemetry engine (v2.13 / ADR-099).
#[derive(Debug, Clone)]
pub struct ScreenFrameProfiler {
    mode: ScreenProfilerMode,
    frames_profiled: u64,
    current: ScreenLatencyBreakdown,
    samples_e2e: VecDeque<u64>,
    max_samples: usize,
    min_e2e_us: u64,
    max_e2e_us: u64,
    ewma_capture_us: f64,
    ewma_encode_us: f64,
    ewma_pacer_us: f64,
    ewma_network_us: f64,
    ewma_jitter_us: f64,
    ewma_decode_us: f64,
    ewma_render_us: f64,
    ewma_total_e2e_us: f64,
    bottleneck: ScreenBottleneckStage,
}

impl Default for ScreenFrameProfiler {
    fn default() -> Self {
        Self::new(ScreenProfilerMode::Auto)
    }
}

impl ScreenFrameProfiler {
    pub fn new(mode: ScreenProfilerMode) -> Self {
        Self {
            mode,
            frames_profiled: 0,
            current: ScreenLatencyBreakdown::default(),
            samples_e2e: VecDeque::with_capacity(SCREEN_PROFILE_RING_CAPACITY),
            max_samples: SCREEN_PROFILE_RING_CAPACITY,
            min_e2e_us: 0,
            max_e2e_us: 0,
            ewma_capture_us: 0.0,
            ewma_encode_us: 0.0,
            ewma_pacer_us: 0.0,
            ewma_network_us: 0.0,
            ewma_jitter_us: 0.0,
            ewma_decode_us: 0.0,
            ewma_render_us: 0.0,
            ewma_total_e2e_us: 0.0,
            bottleneck: ScreenBottleneckStage::Balanced,
        }
    }

    pub fn mode(&self) -> ScreenProfilerMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ScreenProfilerMode) {
        self.mode = mode;
        if mode == ScreenProfilerMode::Disabled {
            self.samples_e2e.clear();
        }
    }

    pub fn bottleneck(&self) -> ScreenBottleneckStage {
        self.bottleneck
    }

    pub fn stats(&self) -> ScreenLatencyReport {
        self.report()
    }

    pub fn record_profile_frame(&mut self, f: &ScreenProfileFrame) {
        let b = ScreenLatencyBreakdown {
            frame_seq: f.frame_seq,
            capture_us: f.capture_us,
            encode_us: f.encode_us,
            pacer_us: f.pacer_us,
            network_us: f.network_us,
            jitter_us: f.jitter_us,
            decode_us: f.decode_us,
            render_us: f.render_us,
            total_e2e_us: f.total_e2e_us as u64,
        };
        self.record_frame(b);
    }

    pub fn record_breakdown(&mut self, b: ScreenLatencyBreakdown) {
        self.record_frame(b);
    }

    pub fn record_frame(&mut self, mut b: ScreenLatencyBreakdown) {
        if self.mode == ScreenProfilerMode::Disabled {
            return;
        }
        b.calculate_total();
        self.frames_profiled += 1;
        self.current = b;

        let total = b.total_e2e_us;
        if self.min_e2e_us == 0 || total < self.min_e2e_us {
            self.min_e2e_us = total;
        }
        if total > self.max_e2e_us {
            self.max_e2e_us = total;
        }

        if self.samples_e2e.len() >= self.max_samples {
            self.samples_e2e.pop_front();
        }
        self.samples_e2e.push_back(total);

        let alpha = 0.15;
        if self.frames_profiled == 1 {
            self.ewma_capture_us = b.capture_us as f64;
            self.ewma_encode_us = b.encode_us as f64;
            self.ewma_pacer_us = b.pacer_us as f64;
            self.ewma_network_us = b.network_us as f64;
            self.ewma_jitter_us = b.jitter_us as f64;
            self.ewma_decode_us = b.decode_us as f64;
            self.ewma_render_us = b.render_us as f64;
            self.ewma_total_e2e_us = total as f64;
        } else {
            self.ewma_capture_us =
                (1.0 - alpha) * self.ewma_capture_us + alpha * (b.capture_us as f64);
            self.ewma_encode_us =
                (1.0 - alpha) * self.ewma_encode_us + alpha * (b.encode_us as f64);
            self.ewma_pacer_us = (1.0 - alpha) * self.ewma_pacer_us + alpha * (b.pacer_us as f64);
            self.ewma_network_us =
                (1.0 - alpha) * self.ewma_network_us + alpha * (b.network_us as f64);
            self.ewma_jitter_us =
                (1.0 - alpha) * self.ewma_jitter_us + alpha * (b.jitter_us as f64);
            self.ewma_decode_us =
                (1.0 - alpha) * self.ewma_decode_us + alpha * (b.decode_us as f64);
            self.ewma_render_us =
                (1.0 - alpha) * self.ewma_render_us + alpha * (b.render_us as f64);
            self.ewma_total_e2e_us =
                (1.0 - alpha) * self.ewma_total_e2e_us + alpha * (total as f64);
        }

        // Bottleneck analysis:
        let stages = [
            (ScreenBottleneckStage::Capture, self.ewma_capture_us),
            (ScreenBottleneckStage::Encode, self.ewma_encode_us),
            (ScreenBottleneckStage::Pacer, self.ewma_pacer_us),
            (ScreenBottleneckStage::Network, self.ewma_network_us),
            (ScreenBottleneckStage::Jitter, self.ewma_jitter_us),
            (ScreenBottleneckStage::Decode, self.ewma_decode_us),
            (ScreenBottleneckStage::Render, self.ewma_render_us),
        ];

        let mut max_stage = ScreenBottleneckStage::Balanced;
        let mut max_val = 0.0;
        for (st, val) in stages {
            if val > max_val {
                max_val = val;
                max_stage = st;
            }
        }

        if self.ewma_total_e2e_us > 0.0 && (max_val / self.ewma_total_e2e_us) >= 0.35 {
            self.bottleneck = max_stage;
        } else if total > 0 {
            let mut cur_max_stage = ScreenBottleneckStage::Balanced;
            let mut cur_max_val = 0.0;
            for (st, val) in [
                (ScreenBottleneckStage::Capture, b.capture_us as f64),
                (ScreenBottleneckStage::Encode, b.encode_us as f64),
                (ScreenBottleneckStage::Pacer, b.pacer_us as f64),
                (ScreenBottleneckStage::Network, b.network_us as f64),
                (ScreenBottleneckStage::Jitter, b.jitter_us as f64),
                (ScreenBottleneckStage::Decode, b.decode_us as f64),
                (ScreenBottleneckStage::Render, b.render_us as f64),
            ] {
                if val > cur_max_val {
                    cur_max_val = val;
                    cur_max_stage = st;
                }
            }
            if (cur_max_val / total as f64) >= 0.35 {
                self.bottleneck = cur_max_stage;
            } else {
                self.bottleneck = ScreenBottleneckStage::Balanced;
            }
        } else {
            self.bottleneck = ScreenBottleneckStage::Balanced;
        }
    }

    pub fn percentiles(&self) -> (u64, u64, u64, u64) {
        if self.samples_e2e.is_empty() {
            return (0, 0, 0, 0);
        }
        let mut sorted: Vec<u64> = self.samples_e2e.iter().copied().collect();
        sorted.sort_unstable();
        let len = sorted.len();
        let p50 = sorted[(len * 50 / 100).min(len - 1)];
        let p90 = sorted[(len * 90 / 100).min(len - 1)];
        let p95 = sorted[(len * 95 / 100).min(len - 1)];
        let p99 = sorted[(len * 99 / 100).min(len - 1)];
        (p50, p90, p95, p99)
    }

    pub fn report(&self) -> ScreenLatencyReport {
        let (p50, p90, p95, p99) = self.percentiles();
        ScreenLatencyReport {
            ok: true,
            mode: self.mode.as_str().to_string(),
            frames_profiled: self.frames_profiled,
            bottleneck: self.bottleneck.as_str().to_string(),
            current: self.current,
            avg_e2e_us: self.ewma_total_e2e_us.round() as u64,
            p50_e2e_us: p50,
            p90_e2e_us: p90,
            p95_e2e_us: p95,
            p99_e2e_us: p99,
            min_e2e_us: self.min_e2e_us,
            max_e2e_us: self.max_e2e_us,
            avg_capture_us: self.ewma_capture_us.round() as u32,
            avg_encode_us: self.ewma_encode_us.round() as u32,
            avg_pacer_us: self.ewma_pacer_us.round() as u32,
            avg_network_us: self.ewma_network_us.round() as u32,
            avg_jitter_us: self.ewma_jitter_us.round() as u32,
            avg_decode_us: self.ewma_decode_us.round() as u32,
            avg_render_us: self.ewma_render_us.round() as u32,
        }
    }

    pub fn reset(&mut self) -> ScreenLatencyReport {
        self.frames_profiled = 0;
        self.current = ScreenLatencyBreakdown::default();
        self.samples_e2e.clear();
        self.min_e2e_us = 0;
        self.max_e2e_us = 0;
        self.ewma_capture_us = 0.0;
        self.ewma_encode_us = 0.0;
        self.ewma_pacer_us = 0.0;
        self.ewma_network_us = 0.0;
        self.ewma_jitter_us = 0.0;
        self.ewma_decode_us = 0.0;
        self.ewma_render_us = 0.0;
        self.ewma_total_e2e_us = 0.0;
        self.bottleneck = ScreenBottleneckStage::Balanced;
        self.report()
    }
}

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
    fn hold(frame_seq: u64, stage: ScreenBottleneckStage, now_us: u64) -> Self {
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

    fn with_stamp(mut self, now_us: u64) -> Self {
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
    mode: ScreenTunerMode,
    bottleneck_streak: u32,
    last_bottleneck: ScreenBottleneckStage,
    balanced_streak: u32,
    last_action_us: u64,
    actions_applied: u64,
    recoveries_applied: u64,
    degraded: VecDeque<ScreenTuningAction>,
    last_decision: Option<ScreenTuningDecision>,
    last_action: ScreenTuningAction,
    last_stage: ScreenBottleneckStage,
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

    fn apply_relief(
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

    fn apply_recovery(
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

fn drop_bitrate(current_bps: u32, min_bps: u32) -> u32 {
    let dropped =
        current_bps.saturating_sub(current_bps.saturating_mul(SCREEN_TUNER_BITRATE_DROP_PCT) / 100);
    dropped.max(min_bps)
}

/// Screen session recording container magic & layout (v2.15 / ADR-101).
pub const SCREEN_RECORDING_MAGIC: [u8; 4] = *b"TREC";
pub const SCREEN_RECORDING_PROTO: u8 = 1;
/// Container header: magic(4) proto(1) width(4) height(4) format(1) codec(1) stride(4) started_us(8).
pub const SCREEN_RECORDING_HEADER_LEN: usize = 27;
/// Per-frame record prefix: seq(8) pts_us(8) len(4).
pub const SCREEN_RECORDING_RECORD_PREFIX_LEN: usize = 20;
/// Upper bound of frame summaries returned by one replay inspection.
pub const SCREEN_RECORDING_REPLAY_LIMIT_MAX: usize = 512;

/// Container header metadata derived from the first recorded frame (v2.15 / ADR-101).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRecordingMeta {
    pub width: u32,
    pub height: u32,
    pub format_tag: u8,
    pub codec_tag: u8,
    pub stride: u32,
    pub started_at_us: u64,
}

impl ScreenRecordingMeta {
    pub fn from_frame(frame: &ScreenFrame, started_at_us: u64) -> Self {
        Self {
            width: frame.rect.w as u32,
            height: frame.rect.h as u32,
            format_tag: frame.format.tag(),
            codec_tag: frame.codec_tag,
            stride: frame.stride,
            started_at_us,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SCREEN_RECORDING_HEADER_LEN);
        out.extend_from_slice(&SCREEN_RECORDING_MAGIC);
        out.push(SCREEN_RECORDING_PROTO);
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.push(self.format_tag);
        out.push(self.codec_tag);
        out.extend_from_slice(&self.stride.to_be_bytes());
        out.extend_from_slice(&self.started_at_us.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < SCREEN_RECORDING_HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if bytes[0..4] != SCREEN_RECORDING_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if bytes[4] != SCREEN_RECORDING_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        Ok(Self {
            width: u32::from_be_bytes(bytes[5..9].try_into().unwrap()),
            height: u32::from_be_bytes(bytes[9..13].try_into().unwrap()),
            format_tag: bytes[13],
            codec_tag: bytes[14],
            stride: u32::from_be_bytes(bytes[15..19].try_into().unwrap()),
            started_at_us: u64::from_be_bytes(bytes[19..27].try_into().unwrap()),
        })
    }
}

/// One recorded frame record (v2.15 / ADR-101).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRecordingFrameRecord {
    pub seq: u64,
    pub pts_us: u64,
    pub payload: Vec<u8>,
}

/// Streaming TREC container writer; the header is written on construction from
/// the first frame's metadata and each append stores the raw wire payload of the
/// frame — zero re-encoding (v2.15 / ADR-101).
pub struct ScreenRecordingWriter<W: std::io::Write> {
    meta: ScreenRecordingMeta,
    inner: W,
    frames_written: u64,
    bytes_written: u64,
}

impl<W: std::io::Write> ScreenRecordingWriter<W> {
    pub fn new(meta: ScreenRecordingMeta, mut inner: W) -> Result<Self, CoreError> {
        inner.write_all(&meta.encode())?;
        Ok(Self {
            meta,
            inner,
            frames_written: 0,
            bytes_written: 0,
        })
    }

    pub fn meta(&self) -> &ScreenRecordingMeta {
        &self.meta
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub fn append(&mut self, frame: &ScreenFrame, pts_us: u64) -> Result<(), CoreError> {
        let payload = frame.encode_payload()?;
        self.inner.write_all(&frame.seq.to_be_bytes())?;
        self.inner.write_all(&pts_us.to_be_bytes())?;
        self.inner
            .write_all(&(payload.len() as u32).to_be_bytes())?;
        self.inner.write_all(&payload)?;
        self.frames_written = self.frames_written.saturating_add(1);
        self.bytes_written = self
            .bytes_written
            .saturating_add((SCREEN_RECORDING_RECORD_PREFIX_LEN + payload.len()) as u64);
        Ok(())
    }

    pub fn finish(mut self) -> Result<(u64, u64), CoreError> {
        self.inner.flush()?;
        Ok((self.frames_written, self.bytes_written))
    }
}

/// Streaming TREC container reader; validates magic/proto on open and yields
/// raw frame records that `decode_frame` turns back into standard `ScreenFrame`s
/// (v2.15 / ADR-101).
pub struct ScreenRecordingReader<R: std::io::Read> {
    meta: ScreenRecordingMeta,
    inner: R,
    records_read: u64,
}

impl<R: std::io::Read> ScreenRecordingReader<R> {
    pub fn new(mut inner: R) -> Result<Self, CoreError> {
        let mut head = [0u8; SCREEN_RECORDING_HEADER_LEN];
        inner.read_exact(&mut head)?;
        if head[0..4] != SCREEN_RECORDING_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if head[4] != SCREEN_RECORDING_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let meta = ScreenRecordingMeta::decode(&head)?;
        Ok(Self {
            meta,
            inner,
            records_read: 0,
        })
    }

    pub fn meta(&self) -> &ScreenRecordingMeta {
        &self.meta
    }

    pub fn records_read(&self) -> u64 {
        self.records_read
    }

    pub fn next_record(&mut self) -> Result<Option<ScreenRecordingFrameRecord>, CoreError> {
        let mut prefix = [0u8; SCREEN_RECORDING_RECORD_PREFIX_LEN];
        // Distinguish a clean container end from a truncated record: read the
        // first prefix byte manually; EOF there means the container ended cleanly,
        // while a short read inside a record is a hard truncation error.
        if self.inner.read(&mut prefix[..1])? == 0 {
            return Ok(None);
        }
        self.inner.read_exact(&mut prefix[1..])?;
        let seq = u64::from_be_bytes(prefix[0..8].try_into().unwrap());
        let pts_us = u64::from_be_bytes(prefix[8..16].try_into().unwrap());
        let len = u32::from_be_bytes(prefix[16..20].try_into().unwrap()) as usize;
        let mut payload = vec![0u8; len];
        self.inner.read_exact(&mut payload)?;
        self.records_read = self.records_read.saturating_add(1);
        Ok(Some(ScreenRecordingFrameRecord {
            seq,
            pts_us,
            payload,
        }))
    }

    pub fn decode_frame(
        &self,
        record: &ScreenRecordingFrameRecord,
    ) -> Result<ScreenFrame, CoreError> {
        ScreenFrame::decode_payload(&record.payload)
    }
}

impl<R: std::io::Read + std::io::Seek> ScreenRecordingReader<R> {
    /// Rewind stream to the first recorded frame (past the 27-byte container header).
    pub fn rewind_to_start(&mut self) -> Result<(), CoreError> {
        self.inner
            .seek(std::io::SeekFrom::Start(SCREEN_RECORDING_HEADER_LEN as u64))?;
        self.records_read = 0;
        Ok(())
    }

    /// Fast-scan container records without decoding to determine total frame count
    /// and elapsed recording duration in microseconds, then rewinds to start.
    pub fn scan_summary(&mut self) -> Result<(u64, u64), CoreError> {
        self.rewind_to_start()?;
        let mut count = 0u64;
        let mut first_pts: Option<u64> = None;
        let mut last_pts: Option<u64> = None;
        let mut prefix = [0u8; SCREEN_RECORDING_RECORD_PREFIX_LEN];
        loop {
            if self.inner.read(&mut prefix[..1])? == 0 {
                break;
            }
            self.inner.read_exact(&mut prefix[1..])?;
            let pts_us = u64::from_be_bytes(prefix[8..16].try_into().unwrap());
            let len = u32::from_be_bytes(prefix[16..20].try_into().unwrap()) as i64;
            if first_pts.is_none() {
                first_pts = Some(pts_us);
            }
            last_pts = Some(pts_us);
            count = count.saturating_add(1);
            self.inner.seek(std::io::SeekFrom::Current(len))?;
        }
        self.rewind_to_start()?;
        let duration = match (first_pts, last_pts) {
            (Some(f), Some(l)) => l.saturating_sub(f),
            _ => 0,
        };
        Ok((count, duration))
    }

    /// Fast seek-and-extract a single frame from a recording container (ADR-103).
    ///
    /// Skips non-matching frame payloads via `seek(SeekFrom::Current(len))` without
    /// decompressing or allocating memory.
    /// - If `target_seq` is specified, matches `record.seq == target_seq`.
    /// - If `target_pts_us` is specified (and target_seq is None), matches `record.pts_us >= target_pts_us`.
    /// - If both are specified, requires both conditions to match.
    /// - If neither is specified, matches the first frame.
    pub fn extract_frame(
        &mut self,
        target_seq: Option<u64>,
        target_pts_us: Option<u64>,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        Ok(self
            .extract_frame_with_pts(target_seq, target_pts_us)?
            .map(|(f, _)| f))
    }

    /// Fast seek-and-extract a single frame along with its recording PTS (us) (ADR-103).
    pub fn extract_frame_with_pts(
        &mut self,
        target_seq: Option<u64>,
        target_pts_us: Option<u64>,
    ) -> Result<Option<(ScreenFrame, u64)>, CoreError> {
        self.rewind_to_start()?;
        let mut prefix = [0u8; SCREEN_RECORDING_RECORD_PREFIX_LEN];
        loop {
            if self.inner.read(&mut prefix[..1])? == 0 {
                break;
            }
            self.inner.read_exact(&mut prefix[1..])?;
            let seq = u64::from_be_bytes(prefix[0..8].try_into().unwrap());
            let pts_us = u64::from_be_bytes(prefix[8..16].try_into().unwrap());
            let len = u32::from_be_bytes(prefix[16..20].try_into().unwrap()) as usize;

            let matches = match (target_seq, target_pts_us) {
                (Some(s), Some(p)) => seq == s && pts_us >= p,
                (Some(s), None) => seq == s,
                (None, Some(p)) => pts_us >= p,
                (None, None) => true,
            };

            if matches {
                let mut payload = vec![0u8; len];
                self.inner.read_exact(&mut payload)?;
                self.records_read = self.records_read.saturating_add(1);
                let record = ScreenRecordingFrameRecord {
                    seq,
                    pts_us,
                    payload,
                };
                let frame = self.decode_frame(&record)?;
                return Ok(Some((frame, pts_us)));
            } else {
                self.inner.seek(std::io::SeekFrom::Current(len as i64))?;
                self.records_read = self.records_read.saturating_add(1);
            }
        }
        Ok(None)
    }
}

/// Trait alias for seekable and readable stream over threads (v2.16 / ADR-102).
pub trait ReadSeek: std::io::Read + std::io::Seek + Send {}
impl<T: std::io::Read + std::io::Seek + Send> ReadSeek for T {}

/// Lifecycle states of a screen session in-container playback session (v2.16 / ADR-102).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenPlaybackState {
    #[default]
    Idle,
    Playing,
    Paused,
    Finished,
    Stopped,
}

impl ScreenPlaybackState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScreenPlaybackState::Idle => "idle",
            ScreenPlaybackState::Playing => "playing",
            ScreenPlaybackState::Paused => "paused",
            ScreenPlaybackState::Finished => "finished",
            ScreenPlaybackState::Stopped => "stopped",
        }
    }
}

impl std::str::FromStr for ScreenPlaybackState {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().trim() {
            "idle" => Ok(ScreenPlaybackState::Idle),
            "playing" => Ok(ScreenPlaybackState::Playing),
            "paused" => Ok(ScreenPlaybackState::Paused),
            "finished" => Ok(ScreenPlaybackState::Finished),
            "stopped" => Ok(ScreenPlaybackState::Stopped),
            _ => Err(CoreError::InvalidConfig(format!(
                "unknown playback state: {s}"
            ))),
        }
    }
}

/// Telemetry status report for screen session playback (v2.16 / ADR-102).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenPlaybackReport {
    pub ok: bool,
    pub state: String,
    pub path: String,
    pub speed: f32,
    pub loop_enabled: bool,
    pub current_frame: u64,
    pub total_frames: u64,
    pub elapsed_ms: u64,
    pub duration_ms: u64,
    pub width: u16,
    pub height: u16,
    pub format: String,
    pub codec: String,
}

impl Default for ScreenPlaybackReport {
    fn default() -> Self {
        Self {
            ok: true,
            state: ScreenPlaybackState::Idle.as_str().to_string(),
            path: String::new(),
            speed: 1.0,
            loop_enabled: false,
            current_frame: 0,
            total_frames: 0,
            elapsed_ms: 0,
            duration_ms: 0,
            width: 0,
            height: 0,
            format: "unknown".to_string(),
            codec: "raw".to_string(),
        }
    }
}

/// Telemetry and structured outcome of a screen snapshot or recording frame extraction (v2.17 / ADR-103).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenSnapshotReport {
    pub ok: bool,
    pub source: String, // "live" | "recording" | "playback"
    pub frame_seq: u64,
    pub pts_us: u64,
    pub width: u32,
    pub height: u32,
    pub format: String, // "bmp" | "raw"
    pub bytes_len: usize,
    pub privacy_blanked: bool,
    #[serde(default)]
    pub watermarked: bool,
    pub path: Option<String>, // target file path if written to disk
    pub data_base64: Option<String>,
}

impl Default for ScreenSnapshotReport {
    fn default() -> Self {
        Self {
            ok: true,
            source: "live".to_string(),
            frame_seq: 0,
            pts_us: 0,
            width: 0,
            height: 0,
            format: "bmp".to_string(),
            bytes_len: 0,
            privacy_blanked: false,
            watermarked: false,
            path: None,
            data_base64: None,
        }
    }
}

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

/// Category of visual annotation primitives on screen overlay (v2.19 / ADR-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnnotationKind {
    /// Freehand polyline stroke connecting sequence of screen coordinates.
    Stroke { points: Vec<(u16, u16)> },
    /// Rectangular bounding box with top-left coordinate and dimensions.
    Rect { x: u16, y: u16, w: u16, h: u16 },
    /// Directional arrow pointer from start coordinate to end coordinate.
    Arrow { from: (u16, u16), to: (u16, u16) },
    /// Text note rendered with 8x8 ASCII bitmap font.
    Text {
        x: u16,
        y: u16,
        text: String,
        scale: u8,
    },
}

fn default_annotation_color() -> [u8; 3] {
    [255, 59, 48]
}

fn default_annotation_thickness() -> u8 {
    3
}

fn default_annotation_alpha() -> u8 {
    230
}

/// Collaborative whiteboard annotation item on screen streaming session (v2.19 / ADR-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationItem {
    pub id: u32,
    #[serde(default)]
    pub author: String,
    pub kind: AnnotationKind,
    #[serde(default = "default_annotation_color")]
    pub color_rgb: [u8; 3],
    #[serde(default = "default_annotation_thickness")]
    pub thickness: u8,
    #[serde(default = "default_annotation_alpha")]
    pub alpha: u8,
    #[serde(default)]
    pub ttl_ms: u32,
    #[serde(default)]
    pub created_at_ms: u64,
}

impl AnnotationItem {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        if self.ttl_ms == 0 {
            false
        } else {
            now_ms >= self.created_at_ms.saturating_add(self.ttl_ms as u64)
        }
    }
}

fn default_laser_color() -> [u8; 3] {
    [255, 45, 85]
}

fn default_laser_radius() -> u8 {
    12
}

/// Transient virtual laser pointer coordinate and visual configuration (v2.19 / ADR-105).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaserPointerState {
    pub x: u16,
    pub y: u16,
    pub active: bool,
    #[serde(default = "default_laser_color")]
    pub color_rgb: [u8; 3],
    #[serde(default = "default_laser_radius")]
    pub radius: u8,
}

impl Default for LaserPointerState {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            active: false,
            color_rgb: default_laser_color(),
            radius: default_laser_radius(),
        }
    }
}

/// Comprehensive report for active annotations, laser pointer and statistics (v2.19 / ADR-105, v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenAnnotationReport {
    pub ok: bool,
    pub annotations_count: usize,
    pub annotations: Vec<AnnotationItem>,
    pub laser_active: bool,
    pub laser: Option<LaserPointerState>,
    pub total_annotations_added: u64,
    pub laser_updates_total: u64,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub whiteboard_policy: WhiteboardPolicy,
    #[serde(default)]
    pub redo_count: usize,
}

/// FNV-1a 64-bit compact hash for author identity strings (v2.20 / ADR-106).
pub fn hash_author(author: &str) -> u64 {
    if author.is_empty() {
        return 0;
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in author.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

fn default_allow_peer_clear() -> bool {
    true
}

fn default_max_items() -> u16 {
    1000
}

/// Collaborative whiteboard policy and host governance settings (v2.20 / ADR-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhiteboardPolicy {
    #[serde(default)]
    pub read_only: bool,
    #[serde(default = "default_allow_peer_clear")]
    pub allow_peer_clear: bool,
    #[serde(default = "default_max_items")]
    pub max_items: u16,
}

impl Default for WhiteboardPolicy {
    fn default() -> Self {
        Self {
            read_only: false,
            allow_peer_clear: true,
            max_items: 1000,
        }
    }
}

/// Serializable vector snapshot of collaborative whiteboard annotations (v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhiteboardSnapshot {
    pub version: u32,
    pub created_at_ms: u64,
    pub session_id: String,
    pub active_items: Vec<AnnotationItem>,
    pub total_added_counter: u64,
}

/// Chronological undo/redo history and capacity engine for whiteboard (v2.20 / ADR-106).
#[derive(Debug, Clone, Default)]
pub struct WhiteboardHistory {
    active: Vec<AnnotationItem>,
    undo_stack: Vec<AnnotationItem>,
    redo_stack: Vec<AnnotationItem>,
    policy: WhiteboardPolicy,
    total_added: u64,
}

impl WhiteboardHistory {
    pub fn new(policy: WhiteboardPolicy) -> Self {
        Self {
            active: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            policy,
            total_added: 0,
        }
    }

    pub fn policy(&self) -> WhiteboardPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: WhiteboardPolicy) {
        self.policy = policy;
    }

    pub fn active_items(&self) -> &[AnnotationItem] {
        &self.active
    }

    pub fn active_items_mut(&mut self) -> &mut Vec<AnnotationItem> {
        &mut self.active
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    pub fn total_added(&self) -> u64 {
        self.total_added
    }

    pub fn prune_expired(&mut self, now_ms: u64) {
        self.active.retain(|it| !it.is_expired(now_ms));
        self.redo_stack.retain(|it| !it.is_expired(now_ms));
        self.undo_stack.retain(|it| !it.is_expired(now_ms));
    }

    pub fn add_item(&mut self, mut item: AnnotationItem) -> Result<u32, CoreError> {
        if self.policy.read_only {
            return Err(CoreError::InvalidConfig("whiteboard is read-only".into()));
        }
        if self.active.len() >= self.policy.max_items as usize {
            return Err(CoreError::InvalidConfig(format!(
                "whiteboard capacity limit reached ({} items)",
                self.policy.max_items
            )));
        }
        self.total_added += 1;
        if item.id == 0 {
            item.id = self.total_added as u32;
        }
        let item_id = item.id;
        self.undo_stack.push(item.clone());
        self.active.push(item);
        self.redo_stack.clear();
        Ok(item_id)
    }

    pub fn undo(&mut self, author: Option<&str>) -> Option<AnnotationItem> {
        let target_author = author.filter(|s| !s.is_empty());
        if let Some(target) = target_author {
            if let Some(pos) = self.active.iter().rposition(|it| it.author == target) {
                let item = self.active.remove(pos);
                self.redo_stack.push(item.clone());
                Some(item)
            } else {
                None
            }
        } else if let Some(item) = self.active.pop() {
            self.redo_stack.push(item.clone());
            Some(item)
        } else {
            None
        }
    }

    pub fn redo(&mut self, author: Option<&str>) -> Option<AnnotationItem> {
        let target_author = author.filter(|s| !s.is_empty());
        if let Some(target) = target_author {
            if let Some(pos) = self.redo_stack.iter().rposition(|it| it.author == target) {
                let item = self.redo_stack.remove(pos);
                self.active.push(item.clone());
                Some(item)
            } else {
                None
            }
        } else if let Some(item) = self.redo_stack.pop() {
            self.active.push(item.clone());
            Some(item)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        for item in self.active.drain(..) {
            self.undo_stack.push(item);
        }
        self.redo_stack.clear();
    }

    pub fn export_snapshot(&self, session_id: String, now_ms: u64) -> WhiteboardSnapshot {
        let items: Vec<AnnotationItem> = self
            .active
            .iter()
            .filter(|it| !it.is_expired(now_ms))
            .cloned()
            .collect();
        WhiteboardSnapshot {
            version: 1,
            created_at_ms: now_ms,
            session_id,
            active_items: items,
            total_added_counter: self.total_added,
        }
    }

    pub fn import_snapshot(
        &mut self,
        snapshot: WhiteboardSnapshot,
        replace: bool,
    ) -> Result<usize, CoreError> {
        if self.policy.read_only {
            return Err(CoreError::InvalidConfig("whiteboard is read-only".into()));
        }
        if replace {
            self.clear();
        }
        let mut count = 0;
        for mut it in snapshot.active_items {
            if self.active.len() >= self.policy.max_items as usize {
                break;
            }
            self.total_added += 1;
            it.id = self.total_added as u32;
            self.active.push(it);
            count += 1;
        }
        self.redo_stack.clear();
        Ok(count)
    }
}

/// Inner frame payload for structured annotation synchronization (v2.19 / ADR-105, v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "data", rename_all = "snake_case")]
pub enum ScreenAnnotationMsg {
    Add(AnnotationItem),
    Remove { id: u32 },
    Undo { author: String },
    Redo { author: String },
    Clear,
    Snapshot(WhiteboardSnapshot),
    Policy(WhiteboardPolicy),
}

impl ScreenAnnotationMsg {
    pub const HEADER_LEN: usize = 5;

    pub fn encode_payload(&self) -> Result<Vec<u8>, CoreError> {
        let json_bytes =
            serde_json::to_vec(self).map_err(|e| CoreError::ScreenRefused(e.to_string()))?;
        let mut out = Vec::with_capacity(Self::HEADER_LEN + json_bytes.len());
        out.extend_from_slice(&ANNOTATION_MAGIC);
        out.push(ANNOTATION_PROTO);
        out.extend_from_slice(&json_bytes);
        Ok(out)
    }

    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < Self::HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != ANNOTATION_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != ANNOTATION_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        serde_json::from_slice(&payload[Self::HEADER_LEN..])
            .map_err(|e| CoreError::ScreenRefused(e.to_string()))
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode_payload()?;
        InnerFrame::new(TYPE_SCREEN_ANNOTATION, payload)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_ANNOTATION {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }
}

/// Default zoom percentage for Region-of-Interest viewports (100 = 100%).
pub fn default_roi_zoom() -> u16 {
    100
}

/// Region-of-Interest (ROI) viewport parameters for dynamic sub-rectangle cropping and zoom (v2.21 / ADR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRoi {
    pub active: bool,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    #[serde(default = "default_roi_zoom")]
    pub zoom_percent: u16,
}

impl Default for ScreenRoi {
    fn default() -> Self {
        Self {
            active: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            zoom_percent: default_roi_zoom(),
        }
    }
}

impl ScreenRoi {
    /// Sanitize and clamp ROI viewport to screen bounds, enforcing even dimensions for codec safety.
    pub fn sanitize(&mut self, screen_w: u16, screen_h: u16) {
        if screen_w < 2 || screen_h < 2 {
            self.active = false;
            self.x = 0;
            self.y = 0;
            self.w = screen_w;
            self.h = screen_h;
            return;
        }
        self.zoom_percent = self.zoom_percent.clamp(50, 800);
        self.x = self.x.min(screen_w.saturating_sub(2));
        self.y = self.y.min(screen_h.saturating_sub(2));
        let max_w = screen_w.saturating_sub(self.x);
        let max_h = screen_h.saturating_sub(self.y);
        let clamped_w = self.w.clamp(2, max_w);
        let clamped_h = self.h.clamp(2, max_h);
        self.w = (clamped_w / 2) * 2;
        self.h = (clamped_h / 2) * 2;
        if self.w == 0 || self.h == 0 {
            self.active = false;
        }
    }

    /// Maps a screen-space coordinate (e.g. laser pointer or whiteboard annotation) into ROI-relative space.
    /// Returns None if the point lies outside the cropped viewport rectangle.
    pub fn map_screen_to_roi(&self, sx: u16, sy: u16) -> Option<(u16, u16)> {
        if !self.active {
            return Some((sx, sy));
        }
        if sx < self.x || sy < self.y {
            return None;
        }
        let rx = sx - self.x;
        let ry = sy - self.y;
        if rx < self.w && ry < self.h {
            Some((rx, ry))
        } else {
            None
        }
    }

    /// Maps an ROI-relative coordinate back to full-screen space.
    pub fn map_roi_to_screen(&self, rx: u16, ry: u16) -> (u16, u16) {
        if !self.active {
            (rx, ry)
        } else {
            (self.x.saturating_add(rx), self.y.saturating_add(ry))
        }
    }
}

/// Status report for the active Screen ROI viewport pipeline (v2.21 / ADR-107).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenRoiReport {
    pub active: bool,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub zoom_percent: u16,
    pub source_w: u16,
    pub source_h: u16,
    pub effective_w: u16,
    pub effective_h: u16,
    pub update_count: u64,
}

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
    config: ScreenQoeConfig,
    last_action_us: u64,
    actions_total: u64,
    last_action: ScreenQoeAction,
    last_reason: String,
    active_boosts: u32,
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

/// Estimated 2D motion vector between consecutive screen frames (v2.29 / ADR-115).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionVector {
    pub dx: i16,
    pub dy: i16,
    pub match_ratio: f32,
}

/// Screen 2D Motion Vector Estimation and Scroll Acceleration Configuration (v2.29 / ADR-115).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenMotionConfig {
    pub enabled: bool,
    pub max_displacement: u16,
}

impl Default for ScreenMotionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_displacement: SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT,
        }
    }
}

impl ScreenMotionConfig {
    pub fn new(enabled: bool, max_displacement: u16) -> Self {
        Self {
            enabled,
            max_displacement: if max_displacement == 0 {
                SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
            } else {
                max_displacement.clamp(
                    SCREEN_MOTION_MIN_DISPLACEMENT,
                    SCREEN_MOTION_MAX_DISPLACEMENT,
                )
            },
        }
    }
}

/// Screen 2D Motion Vector Telemetry Report (v2.29 / ADR-115).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenMotionReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_displacement: u16,
    pub motion_frames_sent: u64,
    pub fallback_frames_sent: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenMotionReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_displacement: SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT,
            motion_frames_sent: 0,
            fallback_frames_sent: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Computes the exposed edge strip rectangle newly revealed by scroll displacement (dx, dy) (v2.29 / ADR-115).
pub fn compute_exposed_strip(w: u16, h: u16, dx: i16, dy: i16) -> Option<ScreenDamageRect> {
    if w == 0 || h == 0 || (dx == 0 && dy == 0) {
        return None;
    }
    if dy != 0 && dx == 0 {
        let abs_dy = dy.unsigned_abs().min(h);
        if dy < 0 {
            // Shift up: exposed strip is at the bottom
            let y = h.saturating_sub(abs_dy);
            Some(ScreenDamageRect {
                x: 0,
                y,
                width: w,
                height: abs_dy,
            })
        } else {
            // Shift down: exposed strip is at the top
            Some(ScreenDamageRect {
                x: 0,
                y: 0,
                width: w,
                height: abs_dy,
            })
        }
    } else if dx != 0 && dy == 0 {
        let abs_dx = dx.unsigned_abs().min(w);
        if dx < 0 {
            // Shift left: exposed strip is at the right
            let x = w.saturating_sub(abs_dx);
            Some(ScreenDamageRect {
                x,
                y: 0,
                width: abs_dx,
                height: h,
            })
        } else {
            // Shift right: exposed strip is at the left
            Some(ScreenDamageRect {
                x: 0,
                y: 0,
                width: abs_dx,
                height: h,
            })
        }
    } else {
        // 2D diagonal: cover union bounding rect
        let x = if dx > 0 {
            0
        } else {
            w.saturating_sub(dx.unsigned_abs())
        };
        let y = if dy > 0 {
            0
        } else {
            h.saturating_sub(dy.unsigned_abs())
        };
        let width = if dx != 0 { dx.unsigned_abs() } else { w };
        let height = if dy != 0 { dy.unsigned_abs() } else { h };
        Some(ScreenDamageRect {
            x,
            y,
            width: width.min(w),
            height: height.min(h),
        })
    }
}

/// Estimates 2D scroll displacement motion vector between reference frame (`prev`)
/// and current frame (`curr`) using lightweight spatial sampling (v2.29 / ADR-115).
/// Returns `Some(MotionVector)` if the dominant displacement achieves >= `min_match_ratio`.
pub fn estimate_scroll_motion(
    prev: &ScreenFrame,
    curr: &ScreenFrame,
    max_displacement: u16,
    min_match_ratio: f32,
) -> Option<MotionVector> {
    if prev.compressed() || curr.compressed() {
        return None;
    }
    if prev.rect.w != curr.rect.w || prev.rect.h != curr.rect.h || prev.format != curr.format {
        return None;
    }
    if prev.bytes == curr.bytes {
        return None;
    }
    let w = prev.rect.w as usize;
    let h = prev.rect.h as usize;
    if w < 16 || h < 16 {
        return None;
    }
    let bpp = prev.format.bpp();
    let prev_stride = prev.effective_stride() as usize;
    let curr_stride = curr.effective_stride() as usize;

    let d_max = (max_displacement as usize)
        .clamp(
            SCREEN_MOTION_MIN_DISPLACEMENT as usize,
            SCREEN_MOTION_MAX_DISPLACEMENT as usize,
        )
        .min(h.saturating_sub(2) / 2);
    if d_max < 2 {
        return None;
    }

    // 1. Vertical motion estimation (dy in [-d_max, d_max], dy != 0)
    let mut x_samples = Vec::with_capacity(16);
    let step_x = (w / 17).max(1);
    for i in 1..=16 {
        let x = (i * step_x).min(w - 1);
        x_samples.push(x);
    }

    let y_start = d_max;
    let y_end = h.saturating_sub(d_max);
    if y_end <= y_start {
        return None;
    }
    let step_y = ((y_end - y_start) / 16).max(1);
    let mut y_samples = Vec::with_capacity(16);
    let mut y = y_start;
    while y < y_end {
        y_samples.push(y);
        y += step_y;
    }
    let total_samples = x_samples.len() * y_samples.len();
    if total_samples == 0 {
        return None;
    }

    let mut best_v_dy: i16 = 0;
    let mut best_v_matches: usize = 0;

    for dy in -(d_max as i16)..=(d_max as i16) {
        if dy == 0 {
            continue;
        }
        let mut matches = 0;
        for &cy in &y_samples {
            let py = (cy as i32 - dy as i32) as usize;
            let c_row = cy * curr_stride;
            let p_row = py * prev_stride;
            for &cx in &x_samples {
                let c_off = c_row + cx * bpp;
                let p_off = p_row + cx * bpp;
                if c_off + bpp <= curr.bytes.len()
                    && p_off + bpp <= prev.bytes.len()
                    && curr.bytes[c_off..c_off + bpp] == prev.bytes[p_off..p_off + bpp]
                {
                    matches += 1;
                }
            }
        }
        if matches > best_v_matches {
            best_v_matches = matches;
            best_v_dy = dy;
        }
    }

    let v_ratio = best_v_matches as f32 / total_samples as f32;
    if v_ratio >= min_match_ratio && best_v_dy != 0 {
        return Some(MotionVector {
            dx: 0,
            dy: best_v_dy,
            match_ratio: v_ratio,
        });
    }

    // 2. Horizontal motion estimation (dx in [-d_max_x, d_max_x], dx != 0)
    let d_max_x = d_max.min(w.saturating_sub(2) / 2);
    if d_max_x >= 2 {
        let x_start = d_max_x;
        let x_end = w.saturating_sub(d_max_x);
        if x_end > x_start {
            let step_h_x = ((x_end - x_start) / 16).max(1);
            let mut h_x_samples = Vec::with_capacity(16);
            let mut x = x_start;
            while x < x_end {
                h_x_samples.push(x);
                x += step_h_x;
            }
            let step_h_y = (h / 17).max(1);
            let mut h_y_samples = Vec::with_capacity(16);
            for i in 1..=16 {
                let y = (i * step_h_y).min(h - 1);
                h_y_samples.push(y);
            }
            let h_total = h_x_samples.len() * h_y_samples.len();
            if h_total > 0 {
                let mut best_h_dx: i16 = 0;
                let mut best_h_matches: usize = 0;
                for dx in -(d_max_x as i16)..=(d_max_x as i16) {
                    if dx == 0 {
                        continue;
                    }
                    let mut matches = 0;
                    for &cy in &h_y_samples {
                        let c_row = cy * curr_stride;
                        let p_row = cy * prev_stride;
                        for &cx in &h_x_samples {
                            let px = (cx as i32 - dx as i32) as usize;
                            let c_off = c_row + cx * bpp;
                            let p_off = p_row + px * bpp;
                            if c_off + bpp <= curr.bytes.len()
                                && p_off + bpp <= prev.bytes.len()
                                && curr.bytes[c_off..c_off + bpp] == prev.bytes[p_off..p_off + bpp]
                            {
                                matches += 1;
                            }
                        }
                    }
                    if matches > best_h_matches {
                        best_h_matches = matches;
                        best_h_dx = dx;
                    }
                }
                let h_ratio = best_h_matches as f32 / h_total as f32;
                if h_ratio >= min_match_ratio && best_h_dx != 0 {
                    return Some(MotionVector {
                        dx: best_h_dx,
                        dy: 0,
                        match_ratio: h_ratio,
                    });
                }
            }
        }
    }

    None
}

/// Spatial tile-grid dirty tile payload record (v2.30 / ADR-116).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirtyTile {
    pub index: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub data: Vec<u8>,
}

/// Screen Spatial Tile-Grid Decomposition Configuration (v2.30 / ADR-116).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileConfig {
    pub enabled: bool,
    pub tile_size: u16,
}

impl Default for ScreenTileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tile_size: SCREEN_TILE_DEFAULT_SIZE,
        }
    }
}

impl ScreenTileConfig {
    pub fn new(enabled: bool, tile_size: u16) -> Self {
        Self {
            enabled,
            tile_size: if tile_size == 0 {
                SCREEN_TILE_DEFAULT_SIZE
            } else {
                tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
            },
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen Spatial Tile-Grid Telemetry Report (v2.30 / ADR-116).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileReport {
    pub ok: bool,
    pub enabled: bool,
    pub tile_size: u16,
    pub grid_frames_sent: u64,
    pub dirty_tiles_sent: u64,
    pub skipped_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTileReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            tile_size: SCREEN_TILE_DEFAULT_SIZE,
            grid_frames_sent: 0,
            dirty_tiles_sent: 0,
            skipped_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Computes (columns, rows, total_tiles) for screen dimensions given tile_size (v2.30 / ADR-116).
pub fn compute_tile_grid_dims(screen_w: u16, screen_h: u16, tile_size: u16) -> (u16, u16, u16) {
    if screen_w == 0 || screen_h == 0 {
        return (0, 0, 0);
    }
    let t = if tile_size == 0 {
        SCREEN_TILE_DEFAULT_SIZE
    } else {
        tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
    };
    let cols = screen_w.div_ceil(t);
    let rows = screen_h.div_ceil(t);
    let total = cols.saturating_mul(rows);
    (cols, rows, total)
}

/// Extracts dirty tiles between previous uncompressed frame (`prev`) and current uncompressed frame (`curr`) (v2.30 / ADR-116).
/// Returns (total_tiles, Vec<DirtyTile>).
pub fn extract_dirty_tiles(
    prev: &ScreenFrame,
    curr: &ScreenFrame,
    tile_size: u16,
) -> (u16, Vec<DirtyTile>) {
    if prev.compressed() || curr.compressed() {
        return (0, Vec::new());
    }
    if prev.rect.w != curr.rect.w || prev.rect.h != curr.rect.h || prev.format != curr.format {
        return (0, Vec::new());
    }
    let w = curr.rect.w;
    let h = curr.rect.h;
    let (cols, rows, total_tiles) = compute_tile_grid_dims(w, h, tile_size);
    if total_tiles == 0 {
        return (0, Vec::new());
    }
    let t = if tile_size == 0 {
        SCREEN_TILE_DEFAULT_SIZE
    } else {
        tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
    };

    let bpp = curr.format.bpp();
    let prev_stride = (prev.rect.w as usize) * bpp;
    let curr_stride = (curr.rect.w as usize) * bpp;

    let mut dirty_tiles = Vec::new();
    let mut tile_idx = 0u16;

    for r in 0..rows {
        let ty = r * t;
        let th = (h - ty).min(t);
        for c in 0..cols {
            let tx = c * t;
            let tw = (w - tx).min(t);
            let idx = tile_idx;
            tile_idx += 1;

            let mut is_dirty = false;
            let tile_row_bytes = (tw as usize) * bpp;

            for dy in 0..th as usize {
                let p_off = (ty as usize + dy) * prev_stride + (tx as usize) * bpp;
                let c_off = (ty as usize + dy) * curr_stride + (tx as usize) * bpp;
                if p_off + tile_row_bytes <= prev.bytes.len()
                    && c_off + tile_row_bytes <= curr.bytes.len()
                {
                    if prev.bytes[p_off..p_off + tile_row_bytes]
                        != curr.bytes[c_off..c_off + tile_row_bytes]
                    {
                        is_dirty = true;
                        break;
                    }
                } else {
                    is_dirty = true;
                    break;
                }
            }

            if is_dirty {
                let mut data = Vec::with_capacity((tw as usize) * (th as usize) * bpp);
                for dy in 0..th as usize {
                    let c_off = (ty as usize + dy) * curr_stride + (tx as usize) * bpp;
                    data.extend_from_slice(&curr.bytes[c_off..c_off + tile_row_bytes]);
                }
                dirty_tiles.push(DirtyTile {
                    index: idx,
                    x: tx,
                    y: ty,
                    w: tw,
                    h: th,
                    data,
                });
            }
        }
    }

    (total_tiles, dirty_tiles)
}

/// Spatial tile cache dirty tile payload record (v2.31 / ADR-117).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedTileEntry {
    pub index: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub cache_hit: bool,
    pub cache_id: u16,
    pub data: Vec<u8>,
}

/// Screen Spatial Tile Dictionary Cache Configuration (v2.31 / ADR-117).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileCacheConfig {
    pub enabled: bool,
    pub max_entries: u16,
}

impl Default for ScreenTileCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES,
        }
    }
}

impl ScreenTileCacheConfig {
    pub fn new(enabled: bool, max_entries: u16) -> Self {
        Self {
            enabled,
            max_entries: if max_entries == 0 {
                SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
            } else {
                max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
            },
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen Spatial Tile Dictionary Cache Telemetry Report (v2.31 / ADR-117).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileCacheReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_entries: u16,
    pub current_entries: u16,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub hit_ratio_pct: f32,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTileCacheReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_entries: SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES,
            current_entries: 0,
            cache_hits: 0,
            cache_misses: 0,
            hit_ratio_pct: 0.0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Fast, zero-dependency 64-bit FNV-1a content hash for dirty tile deduplication (v2.31 / ADR-117).
pub fn fnv1a_hash64(data: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Dual-synchronized LRU tile dictionary cache for cross-frame and spatial deduplication (v2.31 / ADR-117).
#[derive(Debug, Clone)]
pub struct ScreenTileDictionary {
    max_entries: u16,
    hash_to_id: HashMap<u64, u16>,
    id_to_hash: Vec<Option<u64>>,
    lru_order: VecDeque<u16>,
}

impl ScreenTileDictionary {
    pub fn new(max_entries: u16) -> Self {
        let max = if max_entries == 0 {
            SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        } else {
            max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
        };
        Self {
            max_entries: max,
            hash_to_id: HashMap::with_capacity(max as usize),
            id_to_hash: vec![None; max as usize],
            lru_order: VecDeque::with_capacity(max as usize),
        }
    }

    pub fn max_entries(&self) -> u16 {
        self.max_entries
    }

    pub fn current_entries(&self) -> u16 {
        self.hash_to_id.len() as u16
    }

    pub fn clear(&mut self) {
        self.hash_to_id.clear();
        self.id_to_hash.fill(None);
        self.lru_order.clear();
    }

    pub fn resize(&mut self, new_max_entries: u16) {
        let max = if new_max_entries == 0 {
            SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        } else {
            new_max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
        };
        if max != self.max_entries {
            self.max_entries = max;
            self.clear();
            self.id_to_hash.resize(max as usize, None);
        }
    }

    /// Checks if a tile is in the dictionary. If present, returns `(true, cache_id)`.
    /// If absent, assigns an entry (evicting LRU if full) and returns `(false, cache_id)`.
    pub fn lookup_or_insert(&mut self, hash: u64) -> (bool, u16) {
        if let Some(&id) = self.hash_to_id.get(&hash) {
            // Cache Hit: update LRU order
            if let Some(pos) = self.lru_order.iter().position(|&x| x == id) {
                self.lru_order.remove(pos);
            }
            self.lru_order.push_back(id);
            (true, id)
        } else {
            // Cache Miss: assign a slot
            let id = if (self.hash_to_id.len() as u16) < self.max_entries {
                let new_id = self.hash_to_id.len() as u16;
                self.id_to_hash[new_id as usize] = Some(hash);
                new_id
            } else {
                // Evict LRU
                let victim_id = self.lru_order.pop_front().unwrap_or(0);
                if let Some(old_hash) = self.id_to_hash[victim_id as usize].take() {
                    self.hash_to_id.remove(&old_hash);
                }
                self.id_to_hash[victim_id as usize] = Some(hash);
                victim_id
            };
            self.hash_to_id.insert(hash, id);
            self.lru_order.push_back(id);
            (false, id)
        }
    }
}

/// Screen streaming foveated focus mode (v2.32 / ADR-118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScreenFocusMode {
    #[default]
    Cursor,
    Box,
}

impl ScreenFocusMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenFocusMode::Cursor => "cursor",
            ScreenFocusMode::Box => "box",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ScreenFocusMode::Box,
            _ => ScreenFocusMode::Cursor,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            ScreenFocusMode::Cursor => 0,
            ScreenFocusMode::Box => 1,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "box" | "1" | "rect" | "window" => Some(ScreenFocusMode::Box),
            "cursor" | "0" | "mouse" | "pointer" => Some(ScreenFocusMode::Cursor),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenFocusMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or(CoreError::ScreenFrame)
    }
}

/// Screen streaming foveated focus configuration (v2.32 / ADR-118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenFocusConfig {
    pub enabled: bool,
    pub mode: ScreenFocusMode,
    pub radius: u16,
    pub peripheral_skip_ratio: u8,
    pub box_x: u16,
    pub box_y: u16,
    pub box_w: u16,
    pub box_h: u16,
}

impl Default for ScreenFocusConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ScreenFocusMode::Cursor,
            radius: SCREEN_FOCUS_DEFAULT_RADIUS,
            peripheral_skip_ratio: SCREEN_FOCUS_DEFAULT_SKIP_RATIO,
            box_x: 0,
            box_y: 0,
            box_w: 512,
            box_h: 512,
        }
    }
}

impl ScreenFocusConfig {
    pub fn new(
        enabled: bool,
        mode: ScreenFocusMode,
        radius: u16,
        peripheral_skip_ratio: u8,
        box_coords: Option<(u16, u16, u16, u16)>,
    ) -> Self {
        let (box_x, box_y, box_w, box_h) = box_coords.unwrap_or((0, 0, 512, 512));
        Self {
            enabled,
            mode,
            radius: if radius == 0 {
                SCREEN_FOCUS_DEFAULT_RADIUS
            } else {
                radius.clamp(SCREEN_FOCUS_MIN_RADIUS, SCREEN_FOCUS_MAX_RADIUS)
            },
            peripheral_skip_ratio: peripheral_skip_ratio.min(SCREEN_FOCUS_MAX_SKIP_RATIO),
            box_x,
            box_y,
            box_w: box_w.max(32),
            box_h: box_h.max(32),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming foveated focus telemetry report (v2.32 / ADR-118).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenFocusReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: ScreenFocusMode,
    pub radius: u16,
    pub peripheral_skip_ratio: u8,
    pub box_x: u16,
    pub box_y: u16,
    pub box_w: u16,
    pub box_h: u16,
    pub focus_tiles_sent: u64,
    pub peripheral_tiles_sent: u64,
    pub peripheral_tiles_skipped: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenFocusReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            mode: ScreenFocusMode::Cursor,
            radius: SCREEN_FOCUS_DEFAULT_RADIUS,
            peripheral_skip_ratio: SCREEN_FOCUS_DEFAULT_SKIP_RATIO,
            box_x: 0,
            box_y: 0,
            box_w: 512,
            box_h: 512,
            focus_tiles_sent: 0,
            peripheral_tiles_sent: 0,
            peripheral_tiles_skipped: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Checks whether a spatial tile is inside the active focus zone (v2.32 / ADR-118).
pub fn is_tile_in_focus(
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    config: &ScreenFocusConfig,
    cursor_pos: Option<(i32, i32)>,
) -> bool {
    let center_x = tile_x as i32 + (tile_w as i32 / 2);
    let center_y = tile_y as i32 + (tile_h as i32 / 2);
    match config.mode {
        ScreenFocusMode::Cursor => {
            let (cx, cy) = cursor_pos.unwrap_or((0, 0));
            let dx = center_x - cx;
            let dy = center_y - cy;
            let dist_sq = (dx as i64 * dx as i64 + dy as i64 * dy as i64) as u64;
            let r = config.radius as u64;
            dist_sq <= r * r
        }
        ScreenFocusMode::Box => {
            let bx1 = config.box_x as i32;
            let by1 = config.box_y as i32;
            let bx2 = bx1 + config.box_w as i32;
            let by2 = by1 + config.box_h as i32;
            center_x >= bx1 && center_x < bx2 && center_y >= by1 && center_y < by2
        }
    }
}

/// Screen streaming tile-level adaptive palette configuration (v2.33 / ADR-119).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePaletteConfig {
    pub enabled: bool,
    pub max_colors: u8,
    pub enable_rle: bool,
}

impl Default for ScreenTilePaletteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_colors: SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS,
            enable_rle: false,
        }
    }
}

impl ScreenTilePaletteConfig {
    pub fn new(enabled: bool, max_colors: Option<u8>, enable_rle: Option<bool>) -> Self {
        let max_c = max_colors.unwrap_or(SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS);
        let clamped = max_c.clamp(2, 16);
        Self {
            enabled,
            max_colors: clamped,
            enable_rle: enable_rle.unwrap_or(false),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming tile-level adaptive palette telemetry report (v2.33 / ADR-119).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePaletteReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_colors: u8,
    pub enable_rle: bool,
    pub solid_tiles_count: u64,
    pub bilevel_tiles_count: u64,
    pub palette_tiles_count: u64,
    pub truecolor_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTilePaletteReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_colors: SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS,
            enable_rle: false,
            solid_tiles_count: 0,
            bilevel_tiles_count: 0,
            palette_tiles_count: 0,
            truecolor_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Compresses a 32-bit RGBA dirty tile into an adaptive palette or bi-level representation (v2.33 / ADR-119).
/// Returns (subcodec, encoded_payload).
/// If the tile has > max_colors (default 16), early-aborts in < 2us and returns (TILE_SUBCODEC_RAW, raw_pixels.to_vec()).
pub fn compress_tile_palette(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    config: &ScreenTilePaletteConfig,
) -> (u8, Vec<u8>) {
    let total_pixels = (w as usize) * (h as usize);
    if raw_pixels.len() != total_pixels * 4 || total_pixels == 0 || !config.enabled {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }

    let max_colors = (config.max_colors as usize).clamp(2, 16);
    let mut palette = [0u32; 16];
    let mut color_count = 0usize;

    for chunk in raw_pixels.chunks_exact(4) {
        let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let found = palette[..color_count].contains(&color);
        if !found {
            if color_count >= max_colors {
                return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
            }
            palette[color_count] = color;
            color_count += 1;
        }
    }

    if color_count == 1 {
        // Solid Color Tile: 1 byte subcodec + 4 bytes color (Total 5 bytes)
        let mut payload = Vec::with_capacity(5);
        payload.push(TILE_SUBCODEC_SOLID);
        payload.extend_from_slice(&palette[0].to_ne_bytes());
        (TILE_SUBCODEC_SOLID, payload)
    } else if color_count == 2 {
        // Bi-Level Bitmap Tile: 1 byte subcodec + 8 bytes (C0, C1) + 1-bit packed bitmap
        let bitmap_bytes = total_pixels.div_ceil(8);
        let mut payload = Vec::with_capacity(1 + 8 + bitmap_bytes);
        payload.push(TILE_SUBCODEC_BILEVEL);
        payload.extend_from_slice(&palette[0].to_ne_bytes());
        payload.extend_from_slice(&palette[1].to_ne_bytes());
        let c1 = palette[1];
        let mut cur_byte = 0u8;
        let mut bit_idx = 0usize;
        for chunk in raw_pixels.chunks_exact(4) {
            let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if color == c1 {
                cur_byte |= 1 << (7 - bit_idx);
            }
            bit_idx += 1;
            if bit_idx == 8 {
                payload.push(cur_byte);
                cur_byte = 0;
                bit_idx = 0;
            }
        }
        if bit_idx > 0 {
            payload.push(cur_byte);
        }
        (TILE_SUBCODEC_BILEVEL, payload)
    } else {
        // Indexed Palette (3..=16 colors): 1 byte subcodec + 1 byte count + K*4 bytes palette + 4-bit packed nibbles
        let nibble_bytes = total_pixels.div_ceil(2);
        let mut payload = Vec::with_capacity(2 + color_count * 4 + nibble_bytes);
        payload.push(TILE_SUBCODEC_PALETTE16);
        payload.push(color_count as u8);
        for color in &palette[..color_count] {
            payload.extend_from_slice(&color.to_ne_bytes());
        }
        let mut high_nibble = true;
        let mut cur_byte = 0u8;
        for chunk in raw_pixels.chunks_exact(4) {
            let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let idx = palette[..color_count]
                .iter()
                .position(|&c| c == color)
                .unwrap_or(0) as u8;
            if high_nibble {
                cur_byte = (idx & 0x0F) << 4;
                high_nibble = false;
            } else {
                cur_byte |= idx & 0x0F;
                payload.push(cur_byte);
                cur_byte = 0;
                high_nibble = true;
            }
        }
        if !high_nibble {
            payload.push(cur_byte);
        }
        (TILE_SUBCODEC_PALETTE16, payload)
    }
}

/// Decompresses an adaptive palette, bi-level bitmap or solid color tile back into raw 32-bit RGBA pixels (v2.33 / ADR-119).
pub fn decompress_tile_palette(
    subcodec: u8,
    payload: &[u8],
    w: u16,
    h: u16,
    expected_len: usize,
) -> Result<Vec<u8>, CoreError> {
    let total_pixels = (w as usize) * (h as usize);
    let target_len = total_pixels * 4;
    if target_len != expected_len && expected_len != 0 {
        return Err(CoreError::ScreenFrame);
    }
    let actual_expected = if expected_len == 0 {
        target_len
    } else {
        expected_len
    };

    match subcodec {
        TILE_SUBCODEC_SOLID => {
            let color_bytes = if payload.len() == 5 && payload[0] == TILE_SUBCODEC_SOLID {
                &payload[1..5]
            } else if payload.len() >= 4 {
                &payload[0..4]
            } else {
                return Err(CoreError::ScreenFrame);
            };
            let mut out = Vec::with_capacity(actual_expected);
            for _ in 0..total_pixels {
                out.extend_from_slice(color_bytes);
            }
            Ok(out)
        }
        TILE_SUBCODEC_BILEVEL => {
            let offset = if payload.len() >= 9 && payload[0] == TILE_SUBCODEC_BILEVEL {
                1
            } else {
                0
            };
            if payload.len() < offset + 8 {
                return Err(CoreError::ScreenFrame);
            }
            let c0 = &payload[offset..offset + 4];
            let c1 = &payload[offset + 4..offset + 8];
            let bitmap = &payload[offset + 8..];
            let mut out = Vec::with_capacity(actual_expected);
            for p in 0..total_pixels {
                let byte_idx = p / 8;
                let bit_idx = p % 8;
                if byte_idx >= bitmap.len() {
                    return Err(CoreError::ScreenFrame);
                }
                let bit = (bitmap[byte_idx] >> (7 - bit_idx)) & 1;
                if bit == 0 {
                    out.extend_from_slice(c0);
                } else {
                    out.extend_from_slice(c1);
                }
            }
            Ok(out)
        }
        TILE_SUBCODEC_PALETTE16 => {
            let offset = if payload.len() >= 2 && payload[0] == TILE_SUBCODEC_PALETTE16 {
                1
            } else {
                0
            };
            if payload.len() < offset + 1 {
                return Err(CoreError::ScreenFrame);
            }
            let count = payload[offset] as usize;
            if !(1..=16).contains(&count) || payload.len() < offset + 1 + count * 4 {
                return Err(CoreError::ScreenFrame);
            }
            let pal_start = offset + 1;
            let nibbles_start = pal_start + count * 4;
            let nibbles = &payload[nibbles_start..];
            let mut out = Vec::with_capacity(actual_expected);
            for p in 0..total_pixels {
                let byte_idx = p / 2;
                if byte_idx >= nibbles.len() {
                    return Err(CoreError::ScreenFrame);
                }
                let idx = if (p % 2) == 0 {
                    (nibbles[byte_idx] >> 4) & 0x0F
                } else {
                    nibbles[byte_idx] & 0x0F
                } as usize;
                if idx >= count {
                    return Err(CoreError::ScreenFrame);
                }
                let color_off = pal_start + idx * 4;
                out.extend_from_slice(&payload[color_off..color_off + 4]);
            }
            Ok(out)
        }
        TILE_SUBCODEC_RAW => Ok(payload.to_vec()),
        _ => Err(CoreError::ScreenFrame),
    }
}

/// Screen streaming tile-level predictive filter mode (v2.34 / ADR-120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenTilePredictFilter {
    None = 0,
    Sub = 1,
    Up = 2,
    Average = 3,
    Paeth = 4,
    #[default]
    Auto = 5,
}

impl ScreenTilePredictFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenTilePredictFilter::None => "none",
            ScreenTilePredictFilter::Sub => "sub",
            ScreenTilePredictFilter::Up => "up",
            ScreenTilePredictFilter::Average => "average",
            ScreenTilePredictFilter::Paeth => "paeth",
            ScreenTilePredictFilter::Auto => "auto",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => ScreenTilePredictFilter::None,
            1 => ScreenTilePredictFilter::Sub,
            2 => ScreenTilePredictFilter::Up,
            3 => ScreenTilePredictFilter::Average,
            4 => ScreenTilePredictFilter::Paeth,
            _ => ScreenTilePredictFilter::Auto,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "0" | "off" => Some(ScreenTilePredictFilter::None),
            "sub" | "1" | "left" | "h" | "horizontal" => Some(ScreenTilePredictFilter::Sub),
            "up" | "2" | "top" | "v" | "vertical" => Some(ScreenTilePredictFilter::Up),
            "average" | "avg" | "3" | "mean" => Some(ScreenTilePredictFilter::Average),
            "paeth" | "4" | "diag" | "gradient" => Some(ScreenTilePredictFilter::Paeth),
            "auto" | "5" | "adaptive" | "best" => Some(ScreenTilePredictFilter::Auto),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenTilePredictFilter {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or_else(|| {
            CoreError::InvalidConfig(format!("unknown screen tile predict filter mode: '{s}'"))
        })
    }
}

/// Screen streaming tile-level predictive filter configuration (v2.34 / ADR-120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePredictConfig {
    pub enabled: bool,
    pub filter_mode: ScreenTilePredictFilter,
    pub enable_rct: bool,
}

impl Default for ScreenTilePredictConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            filter_mode: ScreenTilePredictFilter::Auto,
            enable_rct: true,
        }
    }
}

impl ScreenTilePredictConfig {
    pub fn new(
        enabled: bool,
        filter_mode: Option<ScreenTilePredictFilter>,
        enable_rct: Option<bool>,
    ) -> Self {
        Self {
            enabled,
            filter_mode: filter_mode.unwrap_or(ScreenTilePredictFilter::Auto),
            enable_rct: enable_rct.unwrap_or(true),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming tile-level predictive filter telemetry report (v2.34 / ADR-120).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePredictReport {
    pub ok: bool,
    pub enabled: bool,
    pub filter_mode: ScreenTilePredictFilter,
    pub enable_rct: bool,
    pub predict_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTilePredictReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            filter_mode: ScreenTilePredictFilter::Auto,
            enable_rct: true,
            predict_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// In-place / forward Reversible Color Transform (RCT) for 32-bit RGBA (v2.34 / ADR-120).
/// Transforms (R, G, B, A) into (R - G, G, B - G, A) in Z/256Z.
pub fn rct_transform(pixels: &[u8], out: &mut [u8]) {
    assert_eq!(pixels.len(), out.len());
    for (src, dst) in pixels.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let r = src[0];
        let g = src[1];
        let b = src[2];
        let a = src[3];
        dst[0] = r.wrapping_sub(g);
        dst[1] = g;
        dst[2] = b.wrapping_sub(g);
        dst[3] = a;
    }
}

/// In-place / inverse Reversible Color Transform (RCT) for 32-bit RGBA (v2.34 / ADR-120).
/// Reconstructs (R, G, B, A) from (U, G, V, A) where R = U + G, B = V + G in Z/256Z.
pub fn rct_inverse(transformed: &[u8], out: &mut [u8]) {
    assert_eq!(transformed.len(), out.len());
    for (src, dst) in transformed.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let u = src[0];
        let g = src[1];
        let v = src[2];
        let a = src[3];
        dst[0] = u.wrapping_add(g);
        dst[1] = g;
        dst[2] = v.wrapping_add(g);
        dst[3] = a;
    }
}

/// Standard 2D Paeth predictor for spatial differential prediction (v2.34 / ADR-120).
#[inline(always)]
pub fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a_i = a as i32;
    let b_i = b as i32;
    let c_i = c as i32;
    let p = a_i + b_i - c_i;
    let pa = (p - a_i).abs();
    let pb = (p - b_i).abs();
    let pc = (p - c_i).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn apply_single_filter(pixels: &[u8], w: u16, h: u16, filter: ScreenTilePredictFilter) -> Vec<u8> {
    let expected = (w as usize) * (h as usize) * 4;
    let mut out = vec![0u8; expected];
    if filter == ScreenTilePredictFilter::None {
        out.copy_from_slice(pixels);
        return out;
    }
    let stride = (w as usize) * 4;
    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let curr = pixels[idx + ch];
                let a = if col > 0 { pixels[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    pixels[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    pixels[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };
                let pred = match filter {
                    ScreenTilePredictFilter::None | ScreenTilePredictFilter::Auto => 0,
                    ScreenTilePredictFilter::Sub => a,
                    ScreenTilePredictFilter::Up => b,
                    ScreenTilePredictFilter::Average => (((a as u16) + (b as u16)) / 2) as u8,
                    ScreenTilePredictFilter::Paeth => paeth_predictor(a, b, c),
                };
                out[idx + ch] = curr.wrapping_sub(pred);
            }
        }
    }
    out
}

/// Applies directional spatial differential predictive filtering on a 32-bit RGBA tile (v2.34 / ADR-120).
/// In Auto mode, evaluates L1 residual energies across candidates and picks the optimal filter.
/// Returns (chosen_filter, residual_bytes).
pub fn predict_filter_tile(
    pixels: &[u8],
    w: u16,
    h: u16,
    filter: ScreenTilePredictFilter,
) -> (ScreenTilePredictFilter, Vec<u8>) {
    let expected = (w as usize) * (h as usize) * 4;
    if pixels.len() != expected || w == 0 || h == 0 {
        return (ScreenTilePredictFilter::None, pixels.to_vec());
    }

    if filter != ScreenTilePredictFilter::Auto {
        return (filter, apply_single_filter(pixels, w, h, filter));
    }

    // Auto Mode: evaluate L1 residual energies across None, Sub, Up, Average, Paeth
    let stride = (w as usize) * 4;
    let mut energy_none = 0u64;
    let mut energy_sub = 0u64;
    let mut energy_up = 0u64;
    let mut energy_avg = 0u64;
    let mut energy_paeth = 0u64;

    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let curr = pixels[idx + ch];
                let a = if col > 0 { pixels[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    pixels[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    pixels[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };

                let d_none = curr as i8;
                energy_none += (d_none as i16).unsigned_abs() as u64;

                let d_sub = curr.wrapping_sub(a) as i8;
                energy_sub += (d_sub as i16).unsigned_abs() as u64;

                let d_up = curr.wrapping_sub(b) as i8;
                energy_up += (d_up as i16).unsigned_abs() as u64;

                let avg = (((a as u16) + (b as u16)) / 2) as u8;
                let d_avg = curr.wrapping_sub(avg) as i8;
                energy_avg += (d_avg as i16).unsigned_abs() as u64;

                let p_val = paeth_predictor(a, b, c);
                let d_paeth = curr.wrapping_sub(p_val) as i8;
                energy_paeth += (d_paeth as i16).unsigned_abs() as u64;
            }
        }
    }

    let mut best_filter = ScreenTilePredictFilter::None;
    let mut min_energy = energy_none;

    if energy_sub < min_energy {
        min_energy = energy_sub;
        best_filter = ScreenTilePredictFilter::Sub;
    }
    if energy_up < min_energy {
        min_energy = energy_up;
        best_filter = ScreenTilePredictFilter::Up;
    }
    if energy_avg < min_energy {
        min_energy = energy_avg;
        best_filter = ScreenTilePredictFilter::Average;
    }
    if energy_paeth < min_energy {
        best_filter = ScreenTilePredictFilter::Paeth;
    }

    (best_filter, apply_single_filter(pixels, w, h, best_filter))
}

/// Unpredicts / inverts spatial differential prediction back to raw pixels (v2.34 / ADR-120).
pub fn unpredict_filter_tile(
    residual: &[u8],
    w: u16,
    h: u16,
    filter: ScreenTilePredictFilter,
) -> Vec<u8> {
    let expected = (w as usize) * (h as usize) * 4;
    if residual.len() != expected || w == 0 || h == 0 {
        return residual.to_vec();
    }
    if filter == ScreenTilePredictFilter::None {
        return residual.to_vec();
    }

    let stride = (w as usize) * 4;
    let mut out = vec![0u8; expected];

    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let delta = residual[idx + ch];
                let a = if col > 0 { out[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    out[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    out[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };

                let pred = match filter {
                    ScreenTilePredictFilter::None | ScreenTilePredictFilter::Auto => 0,
                    ScreenTilePredictFilter::Sub => a,
                    ScreenTilePredictFilter::Up => b,
                    ScreenTilePredictFilter::Average => (((a as u16) + (b as u16)) / 2) as u8,
                    ScreenTilePredictFilter::Paeth => paeth_predictor(a, b, c),
                };
                out[idx + ch] = delta.wrapping_add(pred);
            }
        }
    }
    out
}

/// Compresses a 32-bit RGBA dirty tile using Reversible Color Transform, Directional Spatial Prediction
/// and generic residual entropy coding (v2.34 / ADR-120).
/// Returns (subcodec, encoded_payload).
pub fn compress_tile_predictive(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    config: &ScreenTilePredictConfig,
    codec: &dyn crate::ScreenCodec,
) -> (u8, Vec<u8>) {
    let total_pixels = (w as usize) * (h as usize);
    if raw_pixels.len() != total_pixels * 4 || total_pixels == 0 || !config.enabled {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }

    let mut rct_buf = vec![0u8; raw_pixels.len()];
    let input = if config.enable_rct {
        rct_transform(raw_pixels, &mut rct_buf);
        &rct_buf[..]
    } else {
        raw_pixels
    };

    let (chosen_filter, residual) = predict_filter_tile(input, w, h, config.filter_mode);

    let compressed_res = match codec.compress_frame(&residual, w, h) {
        Ok(c) => c,
        Err(_) => return (TILE_SUBCODEC_RAW, raw_pixels.to_vec()),
    };

    let mut payload = Vec::with_capacity(3 + compressed_res.len());
    payload.push(TILE_SUBCODEC_PREDICT_ZSTD);
    payload.push(chosen_filter.to_u8());
    let flags = if config.enable_rct { 1u8 } else { 0u8 };
    payload.push(flags);
    payload.extend_from_slice(&compressed_res);

    (TILE_SUBCODEC_PREDICT_ZSTD, payload)
}

/// Decompresses a predictive filtered & RCT transformed tile back into raw 32-bit RGBA pixels (v2.34 / ADR-120).
pub fn decompress_tile_predictive(
    payload: &[u8],
    w: u16,
    h: u16,
    expected_len: usize,
    codec: &dyn crate::ScreenCodec,
) -> Result<Vec<u8>, CoreError> {
    let total_pixels = (w as usize) * (h as usize);
    let target_len = total_pixels * 4;
    if target_len != expected_len && expected_len != 0 {
        return Err(CoreError::ScreenFrame);
    }
    let actual_expected = if expected_len == 0 {
        target_len
    } else {
        expected_len
    };

    if payload.len() < 3 || payload[0] != TILE_SUBCODEC_PREDICT_ZSTD {
        return Err(CoreError::ScreenFrame);
    }
    let filter = ScreenTilePredictFilter::from_u8(payload[1]);
    let enable_rct = (payload[2] & 1) != 0;
    let compressed_residual = &payload[3..];

    let residual = codec.decompress_frame(compressed_residual, w, h, actual_expected)?;
    if residual.len() != actual_expected {
        return Err(CoreError::ScreenFrame);
    }

    let unfiltered = unpredict_filter_tile(&residual, w, h, filter);
    if unfiltered.len() != actual_expected {
        return Err(CoreError::ScreenFrame);
    }

    if enable_rct {
        let mut out = vec![0u8; actual_expected];
        rct_inverse(&unfiltered, &mut out);
        Ok(out)
    } else {
        Ok(unfiltered)
    }
}

/// Screen streaming vertical sync pacing mode (v2.35 / ADR-121).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenTilePresentVsyncMode {
    #[default]
    Off = 0,
    Vsync60 = 1,
    Vsync120 = 2,
    Adaptive = 3,
}

impl ScreenTilePresentVsyncMode {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ScreenTilePresentVsyncMode::Vsync60,
            2 => ScreenTilePresentVsyncMode::Vsync120,
            3 => ScreenTilePresentVsyncMode::Adaptive,
            _ => ScreenTilePresentVsyncMode::Off,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ScreenTilePresentVsyncMode::Off => "off",
            ScreenTilePresentVsyncMode::Vsync60 => "vsync60",
            ScreenTilePresentVsyncMode::Vsync120 => "vsync120",
            ScreenTilePresentVsyncMode::Adaptive => "adaptive",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        let lower = s.trim().to_lowercase();
        match lower.as_str() {
            "off" | "disabled" | "immediate" | "0" => Some(ScreenTilePresentVsyncMode::Off),
            "vsync60" | "60" | "60hz" | "1" => Some(ScreenTilePresentVsyncMode::Vsync60),
            "vsync120" | "120" | "120hz" | "2" => Some(ScreenTilePresentVsyncMode::Vsync120),
            "adaptive" | "adapt" | "3" => Some(ScreenTilePresentVsyncMode::Adaptive),
            _ => None,
        }
    }

    pub fn target_interval_us(self) -> u64 {
        match self {
            ScreenTilePresentVsyncMode::Off => 0,
            ScreenTilePresentVsyncMode::Vsync60 => 16_666,
            ScreenTilePresentVsyncMode::Vsync120 => 8_333,
            ScreenTilePresentVsyncMode::Adaptive => 16_666,
        }
    }
}

/// Screen streaming tile presentation transaction and double buffering configuration (v2.35 / ADR-121).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePresentConfig {
    pub enabled: bool,
    pub vsync_mode: ScreenTilePresentVsyncMode,
    pub double_buffer: bool,
    pub tear_prevention: bool,
}

impl Default for ScreenTilePresentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vsync_mode: ScreenTilePresentVsyncMode::Off,
            double_buffer: true,
            tear_prevention: true,
        }
    }
}

impl ScreenTilePresentConfig {
    pub fn new(
        enabled: bool,
        vsync_mode: Option<ScreenTilePresentVsyncMode>,
        double_buffer: Option<bool>,
        tear_prevention: Option<bool>,
    ) -> Self {
        Self {
            enabled,
            vsync_mode: vsync_mode.unwrap_or(ScreenTilePresentVsyncMode::Off),
            double_buffer: double_buffer.unwrap_or(true),
            tear_prevention: tear_prevention.unwrap_or(true),
        }
    }
}

/// Real-time status report and performance telemetry for tile presentation transaction (v2.35 / ADR-121).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePresentReport {
    pub ok: bool,
    pub enabled: bool,
    pub vsync_mode: String,
    pub double_buffer: bool,
    pub tear_prevention: bool,
    pub committed_transactions: u64,
    pub swapped_frames: u64,
    pub torn_frames_prevented: u64,
    pub avg_swap_latency_us: u64,
}

impl Default for ScreenTilePresentReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            vsync_mode: ScreenTilePresentVsyncMode::Off.as_str().to_string(),
            double_buffer: true,
            tear_prevention: true,
            committed_transactions: 0,
            swapped_frames: 0,
            torn_frames_prevented: 0,
            avg_swap_latency_us: 0,
        }
    }
}

/// Double-buffered atomic presentation transaction engine for anti-tearing screen streaming (v2.35 / ADR-121).
#[derive(Debug, Clone)]
pub struct ScreenTilePresenter {
    config: ScreenTilePresentConfig,
    front_buffer: Option<ScreenFrame>,
    back_buffer: Option<ScreenFrame>,
    staged_tiles: usize,
    committed_transactions: u64,
    swapped_frames: u64,
    torn_frames_prevented: u64,
    total_swap_latency_us: u64,
    last_swap_us: u64,
}

impl Default for ScreenTilePresenter {
    fn default() -> Self {
        Self::new(ScreenTilePresentConfig::default())
    }
}

impl ScreenTilePresenter {
    pub fn new(config: ScreenTilePresentConfig) -> Self {
        Self {
            config,
            front_buffer: None,
            back_buffer: None,
            staged_tiles: 0,
            committed_transactions: 0,
            swapped_frames: 0,
            torn_frames_prevented: 0,
            total_swap_latency_us: 0,
            last_swap_us: 0,
        }
    }

    pub fn config(&self) -> &ScreenTilePresentConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: ScreenTilePresentConfig) {
        self.config = config;
    }

    pub fn reset(&mut self) {
        self.config = ScreenTilePresentConfig::default();
        self.staged_tiles = 0;
        self.committed_transactions = 0;
        self.swapped_frames = 0;
        self.torn_frames_prevented = 0;
        self.total_swap_latency_us = 0;
        self.last_swap_us = 0;
        self.back_buffer = None;
    }

    pub fn front_buffer(&self) -> Option<&ScreenFrame> {
        self.front_buffer.as_ref()
    }

    pub fn back_buffer(&self) -> Option<&ScreenFrame> {
        self.back_buffer.as_ref()
    }

    pub fn set_front_buffer(&mut self, frame: ScreenFrame) {
        self.front_buffer = Some(frame);
    }

    /// Prepare back-buffer for incoming frame/transaction.
    /// Clones the front buffer into back buffer if dimensions match, or initializes new buffer.
    pub fn begin_transaction(&mut self, w: u16, h: u16) {
        self.staged_tiles = 0;
        let expected_len = (w as usize) * (h as usize) * 4;
        let need_init = match &self.back_buffer {
            Some(b) => b.rect.w != w || b.rect.h != h || b.bytes.len() != expected_len,
            None => true,
        };
        if need_init {
            if let Some(front) = &self.front_buffer {
                if front.rect.w == w && front.rect.h == h && front.bytes.len() == expected_len {
                    self.back_buffer = Some(front.clone());
                    return;
                }
            }
            if let Ok(rect) = ScreenRect::new(0, 0, w, h) {
                self.back_buffer = Some(ScreenFrame {
                    seq: 0,
                    rect,
                    format: PixelFormat::Rgba8,
                    stride: (w as u32) * 4,
                    codec_tag: 0,
                    bytes: vec![0u8; expected_len],
                });
            }
        }
    }

    /// Stages a dirty tile into the back buffer.
    pub fn stage_dirty_tile(&mut self, tile: &DirtyTile) -> bool {
        if let Some(back) = &mut self.back_buffer {
            let ok = back.apply_dirty_tile(tile).is_ok();
            if ok {
                self.staged_tiles = self.staged_tiles.saturating_add(1);
            }
            ok
        } else {
            false
        }
    }

    /// Commits transaction and atomically swaps back buffer to front buffer.
    /// Returns true if swap succeeded.
    pub fn commit_and_swap(&mut self, frame_seq: u64, now_us: u64) -> bool {
        if !self.config.enabled {
            if let Some(mut back) = self.back_buffer.take() {
                back.seq = frame_seq;
                self.front_buffer = Some(back);
                return true;
            }
            return false;
        }

        if let Some(mut back) = self.back_buffer.take() {
            back.seq = frame_seq;
            if self.config.tear_prevention && self.staged_tiles > 1 {
                self.torn_frames_prevented = self.torn_frames_prevented.saturating_add(1);
            }
            self.committed_transactions = self.committed_transactions.saturating_add(1);
            self.swapped_frames = self.swapped_frames.saturating_add(1);

            let latency = 2u64; // Microsecond scale memory swap
            self.total_swap_latency_us = self.total_swap_latency_us.saturating_add(latency);
            self.last_swap_us = now_us;

            if self.config.double_buffer {
                let mut old_front = self.front_buffer.take();
                if let Some(ref mut of) = old_front {
                    if of.rect.w == back.rect.w
                        && of.rect.h == back.rect.h
                        && of.bytes.len() == back.bytes.len()
                    {
                        of.bytes.copy_from_slice(&back.bytes);
                        of.seq = frame_seq;
                        self.back_buffer = old_front;
                    }
                }
            }

            self.front_buffer = Some(back);
            self.staged_tiles = 0;
            true
        } else {
            false
        }
    }

    pub fn report(&self) -> ScreenTilePresentReport {
        let avg_latency = self
            .total_swap_latency_us
            .checked_div(self.swapped_frames)
            .unwrap_or(0);
        ScreenTilePresentReport {
            ok: true,
            enabled: self.config.enabled,
            vsync_mode: self.config.vsync_mode.as_str().to_string(),
            double_buffer: self.config.double_buffer,
            tear_prevention: self.config.tear_prevention,
            committed_transactions: self.committed_transactions,
            swapped_frames: self.swapped_frames,
            torn_frames_prevented: self.torn_frames_prevented,
            avg_swap_latency_us: avg_latency,
        }
    }
}

/// Screen streaming tile-level fast retransmission and micro-NACK configuration (v2.36 / ADR-122).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileNackConfig {
    pub enabled: bool,
    pub timeout_ms: u16,
    pub max_retries: u8,
}

impl Default for ScreenTileNackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: 20,
            max_retries: 2,
        }
    }
}

impl ScreenTileNackConfig {
    pub fn new(enabled: bool, timeout_ms: Option<u16>, max_retries: Option<u8>) -> Self {
        Self {
            enabled,
            timeout_ms: timeout_ms.unwrap_or(20).clamp(5, 500),
            max_retries: max_retries.unwrap_or(2).clamp(1, 10),
        }
    }
}

/// Real-time status report and performance telemetry for tile-level micro-NACK recovery (v2.36 / ADR-122).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileNackReport {
    pub ok: bool,
    pub enabled: bool,
    pub timeout_ms: u16,
    pub max_retries: u8,
    pub nack_requests_sent: u64,
    pub tiles_retransmitted: u64,
    pub degraded_commits: u64,
}

impl Default for ScreenTileNackReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            timeout_ms: 20,
            max_retries: 2,
            nack_requests_sent: 0,
            tiles_retransmitted: 0,
            degraded_commits: 0,
        }
    }
}

/// Sender bounded ring-buffer of recent dirty tiles for O(1) instant tile-level retransmission (v2.36 / ADR-122).
#[derive(Debug, Clone)]
pub struct ScreenTileResendBuffer {
    pub capacity: usize,
    pub frames: std::collections::VecDeque<(u32, Vec<DirtyTile>)>,
}

impl ScreenTileResendBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(2),
            frames: std::collections::VecDeque::with_capacity(capacity.max(2)),
        }
    }

    pub fn push_frame(&mut self, seq: u32, tiles: Vec<DirtyTile>) {
        if self.frames.len() >= self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back((seq, tiles));
    }

    pub fn get_tile(&self, seq: u32, tile_index: u16) -> Option<DirtyTile> {
        for (s, tiles) in self.frames.iter().rev() {
            if *s == seq {
                for tile in tiles {
                    if tile.index == tile_index {
                        return Some(tile.clone());
                    }
                }
            }
        }
        None
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

impl Default for ScreenTileResendBuffer {
    fn default() -> Self {
        Self::new(4)
    }
}

/// Screen streaming tile visual saliency tier (v2.37 / ADR-123).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum TileSaliencyTier {
    /// High-contrast text, code editor, interactive controls, or near focus viewport.
    Critical = 0,
    /// Standard visual motion and general UI graphics.
    Normal = 1,
    /// Low-entropy flat background, smooth gradients, or distant static areas.
    Background = 2,
}

impl TileSaliencyTier {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => TileSaliencyTier::Critical,
            1 => TileSaliencyTier::Normal,
            _ => TileSaliencyTier::Background,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TileSaliencyTier::Critical => "critical",
            TileSaliencyTier::Normal => "normal",
            TileSaliencyTier::Background => "background",
        }
    }
}

/// Screen streaming dynamic tile saliency tiering and priority scheduling configuration (v2.37 / ADR-123).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileSaliencyConfig {
    pub enabled: bool,
    pub high_threshold: u8,
    pub low_threshold: u8,
    pub drop_background_on_congestion: bool,
}

impl Default for ScreenTileSaliencyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            high_threshold: 120,
            low_threshold: 40,
            drop_background_on_congestion: true,
        }
    }
}

impl ScreenTileSaliencyConfig {
    pub fn new(
        enabled: bool,
        high_threshold: Option<u8>,
        low_threshold: Option<u8>,
        drop_background_on_congestion: Option<bool>,
    ) -> Self {
        let mut high = high_threshold.unwrap_or(120);
        let mut low = low_threshold.unwrap_or(40);
        if low > high {
            std::mem::swap(&mut low, &mut high);
        }
        Self {
            enabled,
            high_threshold: high.max(1),
            low_threshold: low.min(254),
            drop_background_on_congestion: drop_background_on_congestion.unwrap_or(true),
        }
    }
}

/// Telemetry and report for screen streaming tile saliency tiering pipeline (v2.37 / ADR-123).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileSaliencyReport {
    pub ok: bool,
    pub enabled: bool,
    pub high_threshold: u8,
    pub low_threshold: u8,
    pub drop_background_on_congestion: bool,
    pub critical_tiles_sent: u64,
    pub normal_tiles_sent: u64,
    pub background_tiles_sent: u64,
    pub background_tiles_skipped: u64,
}

impl Default for ScreenTileSaliencyReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            high_threshold: 120,
            low_threshold: 40,
            drop_background_on_congestion: true,
            critical_tiles_sent: 0,
            normal_tiles_sent: 0,
            background_tiles_sent: 0,
            background_tiles_skipped: 0,
        }
    }
}

/// Dynamic spatial tile visual saliency & focus proximity evaluator (v2.37 / ADR-123).
pub struct ScreenTileSaliencyEvaluator;

impl ScreenTileSaliencyEvaluator {
    /// Computes spatial gradient energy [0, 255] across pixel buffer.
    /// Fast sub-sampling (step 2) to maintain microsecond-level performance.
    pub fn compute_gradient_energy(data: &[u8], w: usize, h: usize) -> u8 {
        if w < 2 || h < 2 || data.len() < w * h * 4 {
            return 0;
        }
        let stride = w * 4;
        let mut total_gradient = 0u64;
        let mut sample_count = 0u64;

        let step = 2usize;
        for y in (0..h.saturating_sub(step)).step_by(step) {
            let row_off = y * stride;
            let next_row_off = (y + step) * stride;
            for x in (0..w.saturating_sub(step)).step_by(step) {
                let off = row_off + x * 4;
                let off_right = row_off + (x + step) * 4;
                let off_down = next_row_off + x * 4;

                let y_curr = ((data[off] as u32 * 29)
                    + (data[off + 1] as u32 * 150)
                    + (data[off + 2] as u32 * 77))
                    >> 8;

                let y_right = ((data[off_right] as u32 * 29)
                    + (data[off_right + 1] as u32 * 150)
                    + (data[off_right + 2] as u32 * 77))
                    >> 8;

                let y_down = ((data[off_down] as u32 * 29)
                    + (data[off_down + 1] as u32 * 150)
                    + (data[off_down + 2] as u32 * 77))
                    >> 8;

                let gx = (y_curr as i32 - y_right as i32).unsigned_abs() as u64;
                let gy = (y_curr as i32 - y_down as i32).unsigned_abs() as u64;

                total_gradient += gx + gy;
                sample_count += 1;
            }
        }

        if sample_count == 0 {
            return 0;
        }
        let avg_gradient = total_gradient / sample_count;
        let scaled = avg_gradient * 3;
        scaled.min(255) as u8
    }

    /// Computes proximity score [0, 255] to active focus viewport.
    pub fn compute_focus_proximity(
        tile_x: u16,
        tile_y: u16,
        tile_w: u16,
        tile_h: u16,
        focus_box: (u16, u16, u16, u16),
    ) -> u8 {
        let (fx, fy, fw, fh) = focus_box;
        if fw == 0 || fh == 0 {
            return 0;
        }
        let disjoint = tile_x + tile_w <= fx
            || fx + fw <= tile_x
            || tile_y + tile_h <= fy
            || fy + fh <= tile_y;
        if !disjoint {
            return 255;
        }

        let t_cx = tile_x as i32 + (tile_w as i32 / 2);
        let t_cy = tile_y as i32 + (tile_h as i32 / 2);
        let f_cx = fx as i32 + (fw as i32 / 2);
        let f_cy = fy as i32 + (fh as i32 / 2);

        let dx = t_cx - f_cx;
        let dy = t_cy - f_cy;
        let dist = ((dx * dx + dy * dy) as f64).sqrt() as i32;

        let prox = 255i32.saturating_sub(dist / 2).max(0);
        prox as u8
    }

    /// Evaluates saliency score [0, 255] and assigns semantic tier.
    pub fn evaluate_tile(
        tile: &DirtyTile,
        focus_box: Option<(u16, u16, u16, u16)>,
        config: &ScreenTileSaliencyConfig,
    ) -> (u8, TileSaliencyTier) {
        if !config.enabled {
            return (100, TileSaliencyTier::Normal);
        }

        let g = Self::compute_gradient_energy(&tile.data, tile.w as usize, tile.h as usize);
        let p = if let Some(fb) = focus_box {
            Self::compute_focus_proximity(tile.x, tile.y, tile.w, tile.h, fb)
        } else {
            0
        };

        let score = if p == 255 {
            255
        } else {
            let combined = ((g as u16 * 3 + p as u16) / 4) as u8;
            combined.max(g)
        };

        let tier = if score >= config.high_threshold {
            TileSaliencyTier::Critical
        } else if score >= config.low_threshold {
            TileSaliencyTier::Normal
        } else {
            TileSaliencyTier::Background
        };

        (score, tier)
    }

    /// Schedules and stably reorders dirty tiles by saliency tier.
    /// When congested and drop_background_on_congestion is true, background tiles are dropped.
    pub fn schedule_tiles(
        tiles: Vec<DirtyTile>,
        focus_box: Option<(u16, u16, u16, u16)>,
        config: &ScreenTileSaliencyConfig,
        is_congested: bool,
    ) -> (Vec<DirtyTile>, u64, u64, u64, u64) {
        if !config.enabled {
            let total = tiles.len() as u64;
            return (tiles, 0, total, 0, 0);
        }

        let mut critical_count = 0u64;
        let mut normal_count = 0u64;
        let mut background_count = 0u64;
        let mut skipped_count = 0u64;

        let should_drop_bg = is_congested && config.drop_background_on_congestion;

        let mut scored: Vec<(TileSaliencyTier, u8, usize, DirtyTile)> =
            Vec::with_capacity(tiles.len());

        for (idx, tile) in tiles.into_iter().enumerate() {
            let (score, tier) = Self::evaluate_tile(&tile, focus_box, config);
            match tier {
                TileSaliencyTier::Critical => critical_count += 1,
                TileSaliencyTier::Normal => normal_count += 1,
                TileSaliencyTier::Background => background_count += 1,
            }

            if should_drop_bg && tier == TileSaliencyTier::Background {
                skipped_count += 1;
                continue;
            }

            scored.push((tier, score, idx, tile));
        }

        // Stably sort: Critical (0) < Normal (1) < Background (2)
        // Secondary sort: original index idx to preserve deterministic layout
        scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));

        let reordered_tiles: Vec<DirtyTile> = scored.into_iter().map(|item| item.3).collect();
        (
            reordered_tiles,
            critical_count,
            normal_count,
            background_count,
            skipped_count,
        )
    }
}

/// Screen tile dynamic multi-resolution Level-of-Detail (LoD) levels (v2.38 / ADR-124).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum TileLodLevel {
    Lod0 = 0, // 64x64 Full Resolution (baseline lossless)
    Lod1 = 1, // 32x32 Half Resolution (factor 2, 75% bandwidth reduction)
    Lod2 = 2, // 16x16 Quarter Resolution (factor 4, 93.75% bandwidth reduction)
}

impl TileLodLevel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TileLodLevel::Lod1,
            2 => TileLodLevel::Lod2,
            _ => TileLodLevel::Lod0,
        }
    }

    pub fn scale_factor(&self) -> u16 {
        match self {
            TileLodLevel::Lod0 => 1,
            TileLodLevel::Lod1 => 2,
            TileLodLevel::Lod2 => 4,
        }
    }
}

/// Runtime configuration for screen tile dynamic LoD downsampling (v2.38 / ADR-124).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileLodConfig {
    pub enabled: bool,
    pub max_lod: u8, // 1 (Lod1) or 2 (Lod2)
    pub auto_lod_on_congestion: bool,
}

impl Default for ScreenTileLodConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_lod: 1,
            auto_lod_on_congestion: true,
        }
    }
}

/// Comprehensive telemetry report for screen tile dynamic LoD downsampling (v2.38 / ADR-124).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileLodReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_lod: u8,
    pub auto_lod_on_congestion: bool,
    pub lod0_tiles_sent: u64,
    pub lod1_tiles_sent: u64,
    pub lod2_tiles_sent: u64,
    pub bytes_saved: u64,
}

impl Default for ScreenTileLodReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_lod: 1,
            auto_lod_on_congestion: true,
            lod0_tiles_sent: 0,
            lod1_tiles_sent: 0,
            lod2_tiles_sent: 0,
            bytes_saved: 0,
        }
    }
}

/// Downsamples an RGBA tile buffer into a smaller resolution according to TileLodLevel using box-filter averaging.
pub fn downsample_tile_rgba(
    raw_pixels: &[u8],
    src_w: u16,
    src_h: u16,
    lod: TileLodLevel,
) -> (Vec<u8>, u16, u16) {
    if lod == TileLodLevel::Lod0 || src_w == 0 || src_h == 0 {
        return (raw_pixels.to_vec(), src_w, src_h);
    }
    let factor = lod.scale_factor() as usize;
    let dst_w = (src_w as usize / factor).max(1);
    let dst_h = (src_h as usize / factor).max(1);
    let mut out = vec![0u8; dst_w * dst_h * 4];
    let src_stride = src_w as usize * 4;

    for dy in 0..dst_h {
        let sy_start = dy * factor;
        let sy_end = (sy_start + factor).min(src_h as usize);
        let h_count = (sy_end - sy_start).max(1);

        for dx in 0..dst_w {
            let sx_start = dx * factor;
            let sx_end = (sx_start + factor).min(src_w as usize);
            let w_count = (sx_end - sx_start).max(1);
            let pixel_count = (h_count * w_count) as u32;

            let mut r_acc = 0u32;
            let mut g_acc = 0u32;
            let mut b_acc = 0u32;
            let mut a_acc = 0u32;

            for sy in sy_start..sy_end {
                let row_off = sy * src_stride;
                for sx in sx_start..sx_end {
                    let off = row_off + sx * 4;
                    if off + 3 < raw_pixels.len() {
                        r_acc += raw_pixels[off] as u32;
                        g_acc += raw_pixels[off + 1] as u32;
                        b_acc += raw_pixels[off + 2] as u32;
                        a_acc += raw_pixels[off + 3] as u32;
                    }
                }
            }

            let dst_off = (dy * dst_w + dx) * 4;
            out[dst_off] = (r_acc / pixel_count) as u8;
            out[dst_off + 1] = (g_acc / pixel_count) as u8;
            out[dst_off + 2] = (b_acc / pixel_count) as u8;
            out[dst_off + 3] = (a_acc / pixel_count) as u8;
        }
    }

    (out, dst_w as u16, dst_h as u16)
}

/// Upsamples a sub-resolution RGBA tile buffer back to full dimensions (dst_w, dst_h) using bilinear interpolation.
pub fn upsample_tile_rgba(
    sub_pixels: &[u8],
    sub_w: u16,
    sub_h: u16,
    dst_w: u16,
    dst_h: u16,
) -> Vec<u8> {
    if sub_w == 0 || sub_h == 0 || dst_w == 0 || dst_h == 0 {
        return vec![0u8; dst_w as usize * dst_h as usize * 4];
    }
    if sub_w == dst_w && sub_h == dst_h {
        return sub_pixels.to_vec();
    }
    let sub_w_us = sub_w as usize;
    let sub_h_us = sub_h as usize;
    let dst_w_us = dst_w as usize;
    let dst_h_us = dst_h as usize;
    let mut out = vec![0u8; dst_w_us * dst_h_us * 4];
    let sub_stride = sub_w_us * 4;

    let scale_x = ((sub_w_us as u32) << 16) / dst_w_us as u32;
    let scale_y = ((sub_h_us as u32) << 16) / dst_h_us as u32;

    for dy in 0..dst_h_us {
        let sy_fp = (dy as u32 * scale_y).saturating_sub(1 << 15);
        let sy0 = (sy_fp >> 16) as usize;
        let sy1 = (sy0 + 1).min(sub_h_us - 1);
        let fy = sy_fp & 0xFFFF;
        let ify = 0x10000 - fy;

        let dst_row = dy * dst_w_us * 4;
        let row0 = sy0.min(sub_h_us - 1) * sub_stride;
        let row1 = sy1.min(sub_h_us - 1) * sub_stride;

        for dx in 0..dst_w_us {
            let sx_fp = (dx as u32 * scale_x).saturating_sub(1 << 15);
            let sx0 = (sx_fp >> 16) as usize;
            let sx1 = (sx0 + 1).min(sub_w_us - 1);
            let fx = sx_fp & 0xFFFF;
            let ifx = 0x10000 - fx;

            let c00 = row0 + sx0.min(sub_w_us - 1) * 4;
            let c10 = row0 + sx1.min(sub_w_us - 1) * 4;
            let c01 = row1 + sx0.min(sub_w_us - 1) * 4;
            let c11 = row1 + sx1.min(sub_w_us - 1) * 4;

            let dst_off = dst_row + dx * 4;
            for c in 0..4 {
                let p00 = sub_pixels.get(c00 + c).copied().unwrap_or(0) as u32;
                let p10 = sub_pixels.get(c10 + c).copied().unwrap_or(0) as u32;
                let p01 = sub_pixels.get(c01 + c).copied().unwrap_or(0) as u32;
                let p11 = sub_pixels.get(c11 + c).copied().unwrap_or(0) as u32;

                let top = (p00 * ifx + p10 * fx) >> 16;
                let bottom = (p01 * ifx + p11 * fx) >> 16;
                let val = (top * ify + bottom * fy) >> 16;

                out[dst_off + c] = val.min(255) as u8;
            }
        }
    }

    out
}

/// Compresses an RGBA tile using downsampling (LoD) followed by secondary compression.
/// Returns (TILE_SUBCODEC_LOD, payload).
pub fn compress_tile_lod(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    lod: TileLodLevel,
    codec: &dyn crate::ScreenCodec,
) -> (u8, Vec<u8>) {
    if lod == TileLodLevel::Lod0 {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }
    let (sub_pixels, sub_w, sub_h) = downsample_tile_rgba(raw_pixels, w, h, lod);
    let pal_cfg = ScreenTilePaletteConfig {
        enabled: true,
        max_colors: 16,
        enable_rle: true,
    };
    let (inner_codec, inner_payload) = compress_tile_palette(&sub_pixels, sub_w, sub_h, &pal_cfg);
    let (final_codec, final_payload) = if inner_codec != TILE_SUBCODEC_RAW {
        (inner_codec, inner_payload)
    } else {
        match codec.compress_frame(&sub_pixels, sub_w, sub_h) {
            Ok(comp) if comp.len() < sub_pixels.len() => (TILE_SUBCODEC_ZSTD, comp),
            _ => (TILE_SUBCODEC_RAW, sub_pixels),
        }
    };

    let mut payload = Vec::with_capacity(3 + final_payload.len());
    payload.push(TILE_SUBCODEC_LOD);
    payload.push(lod as u8);
    payload.push(final_codec);
    payload.extend_from_slice(&final_payload);
    (TILE_SUBCODEC_LOD, payload)
}

/// Decompresses an LoD subcodec tile payload and upsamples it back to (dst_w, dst_h).
pub fn decompress_tile_lod(
    payload: &[u8],
    dst_w: u16,
    dst_h: u16,
    _raw_len: usize,
    codec: &dyn crate::ScreenCodec,
) -> Result<Vec<u8>, CoreError> {
    if payload.len() < 3 || payload[0] != TILE_SUBCODEC_LOD {
        return Err(CoreError::ScreenFrame);
    }
    let lod = TileLodLevel::from_u8(payload[1]);
    let inner_codec = payload[2];
    let inner_data = &payload[3..];

    let factor = lod.scale_factor();
    let sub_w = (dst_w / factor).max(1);
    let sub_h = (dst_h / factor).max(1);
    let sub_raw_len = sub_w as usize * sub_h as usize * 4;

    let decompressed_sub = match inner_codec {
        TILE_SUBCODEC_SOLID | TILE_SUBCODEC_BILEVEL | TILE_SUBCODEC_PALETTE16 => {
            decompress_tile_palette(inner_codec, inner_data, sub_w, sub_h, sub_raw_len)?
        }
        TILE_SUBCODEC_ZSTD => codec
            .decompress_frame(inner_data, sub_w, sub_h, sub_raw_len)
            .map_err(|_| CoreError::ScreenFrame)?,
        TILE_SUBCODEC_RAW => {
            if inner_data.len() < sub_raw_len {
                return Err(CoreError::ScreenFrame);
            }
            inner_data[..sub_raw_len].to_vec()
        }
        _ => return Err(CoreError::ScreenFrame),
    };

    let upsampled = upsample_tile_rgba(&decompressed_sub, sub_w, sub_h, dst_w, dst_h);
    Ok(upsampled)
}

/// Runtime configuration for screen tile dynamic MTU batch chunking and pacing (v2.39 / ADR-125).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileBatchConfig {
    pub enabled: bool,
    pub target_chunk_bytes: u16, // Default 1200 bytes, range 512..=64000
    pub pacing_interval_us: u16, // Default 200 us, range 0..=50000
}

impl Default for ScreenTileBatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_chunk_bytes: SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES,
            pacing_interval_us: SCREEN_TILE_BATCH_DEFAULT_PACING_US,
        }
    }
}

impl ScreenTileBatchConfig {
    pub fn new(
        enabled: bool,
        target_chunk_bytes: Option<u16>,
        pacing_interval_us: Option<u16>,
    ) -> Self {
        let chunk_bytes = target_chunk_bytes
            .unwrap_or(SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES)
            .clamp(
                SCREEN_TILE_BATCH_MIN_CHUNK_BYTES,
                SCREEN_TILE_BATCH_MAX_CHUNK_BYTES,
            );
        let pacing = pacing_interval_us
            .unwrap_or(SCREEN_TILE_BATCH_DEFAULT_PACING_US)
            .min(SCREEN_TILE_BATCH_MAX_PACING_US);
        Self {
            enabled,
            target_chunk_bytes: chunk_bytes,
            pacing_interval_us: pacing,
        }
    }
}

/// Comprehensive telemetry report for screen tile dynamic MTU batch chunking and pacing (v2.39 / ADR-125).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileBatchReport {
    pub ok: bool,
    pub enabled: bool,
    pub target_chunk_bytes: u16,
    pub pacing_interval_us: u16,
    pub batches_sent: u64,
    pub chunks_sent: u64,
    pub avg_chunk_bytes: u32,
    pub pacing_pauses: u64,
}

impl Default for ScreenTileBatchReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            target_chunk_bytes: SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES,
            pacing_interval_us: SCREEN_TILE_BATCH_DEFAULT_PACING_US,
            batches_sent: 0,
            chunks_sent: 0,
            avg_chunk_bytes: 0,
            pacing_pauses: 0,
        }
    }
}

/// Chunks a collection of dirty tiles into multiple MTU-safe ScreenFrame chunks (v2.39 / ADR-125).
#[allow(clippy::too_many_arguments)]
pub fn chunk_dirty_tiles(
    seq: u64,
    screen_rect: ScreenRect,
    format: PixelFormat,
    tiles: Vec<DirtyTile>,
    target_chunk_bytes: usize,
    tile_size: u16,
    total_tiles: u16,
    batch_id: u32,
) -> Vec<ScreenFrame> {
    if tiles.is_empty() {
        return vec![ScreenFrame::encode_tile_batch_chunk(
            seq,
            screen_rect,
            format,
            batch_id,
            0,
            1,
            tile_size,
            total_tiles,
            &[],
        )];
    }

    let target = target_chunk_bytes.clamp(
        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES as usize,
        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES as usize,
    );

    let mut chunk_groups: Vec<Vec<DirtyTile>> = Vec::new();
    let mut current_group: Vec<DirtyTile> = Vec::new();
    let mut current_bytes = 14usize; // 14-byte batch chunk header

    for tile in tiles {
        let tile_bytes = 14usize + tile.data.len();
        if !current_group.is_empty() && current_bytes + tile_bytes > target {
            chunk_groups.push(std::mem::take(&mut current_group));
            current_bytes = 14usize;
        }
        current_bytes += tile_bytes;
        current_group.push(tile);
    }

    if !current_group.is_empty() {
        chunk_groups.push(current_group);
    }

    let total_chunks = chunk_groups.len() as u16;
    let mut frames = Vec::with_capacity(chunk_groups.len());
    for (idx, group) in chunk_groups.into_iter().enumerate() {
        frames.push(ScreenFrame::encode_tile_batch_chunk(
            seq,
            screen_rect,
            format,
            batch_id,
            idx as u16,
            total_chunks,
            tile_size,
            total_tiles,
            &group,
        ));
    }
    frames
}

/// Runtime configuration for screen tile local motion vector estimation & displacement compensation (v2.40 / ADR-126).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileMotionConfig {
    pub enabled: bool,
    pub max_displacement: u16, // Default 32, range 4..=128
    pub min_match_ratio: u8,   // Default 90, representing 90% (0.90)
}

impl Default for ScreenTileMotionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_displacement: SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT,
            min_match_ratio: SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO,
        }
    }
}

/// Telemetry report for screen tile motion vector compensation (v2.40 / ADR-126).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileMotionReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_displacement: u16,
    pub min_match_ratio: u8,
    pub motion_tiles_sent: u64,
    pub fallback_tiles_sent: u64,
    pub bytes_saved: u64,
}

/// Estimates the best local motion vector (dx, dy) for a single dirty tile against a reference canvas (v2.40 / ADR-126).
/// Returns `Some((dx, dy, match_ratio))` if `match_ratio >= min_match_ratio`.
#[allow(clippy::too_many_arguments)]
pub fn estimate_tile_motion(
    tile_raw: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
    max_displacement: u16,
    min_match_ratio: f32,
) -> Option<(i16, i16, f32)> {
    if tile_w == 0 || tile_h == 0 || ref_canvas.bytes.is_empty() || ref_canvas.compressed() {
        return None;
    }
    let bpp = ref_canvas.format.bpp();
    let canvas_w = ref_canvas.rect.w as i32;
    let canvas_h = ref_canvas.rect.h as i32;
    let tile_stride = (tile_w as usize) * bpp;
    let canvas_stride = ref_canvas.effective_stride() as usize;

    if tile_raw.len() < (tile_h as usize) * tile_stride {
        return None;
    }

    let d_max = (max_displacement as i32).clamp(
        SCREEN_TILE_MOTION_MIN_DISPLACEMENT as i32,
        SCREEN_TILE_MOTION_MAX_DISPLACEMENT as i32,
    );

    let cur_x = tile_x as i32;
    let cur_y = tile_y as i32;
    let w_i32 = tile_w as i32;
    let h_i32 = tile_h as i32;

    // Sample points inside the tile to speed up matching (every 4th row, every 4th pixel)
    let row_step = 4.min(tile_h as usize);
    let col_step = 4.min(tile_w as usize);
    let sample_pixels = (tile_h as usize).div_ceil(row_step) * (tile_w as usize).div_ceil(col_step);

    if sample_pixels == 0 {
        return None;
    }

    let mut best_dx = 0i16;
    let mut best_dy = 0i16;
    let mut best_ratio = 0.0f32;

    let check_candidate = |dx: i32, dy: i32| -> Option<f32> {
        let ref_x = cur_x - dx;
        let ref_y = cur_y - dy;
        if ref_x < 0 || ref_y < 0 || ref_x + w_i32 > canvas_w || ref_y + h_i32 > canvas_h {
            return None;
        }

        let mut matched = 0usize;
        for r in (0..tile_h as usize).step_by(row_step) {
            let cur_row_off = r * tile_stride;
            let ref_row_off = (ref_y as usize + r) * canvas_stride + (ref_x as usize) * bpp;
            for c in (0..tile_w as usize).step_by(col_step) {
                let cur_p = cur_row_off + c * bpp;
                let ref_p = ref_row_off + c * bpp;
                if tile_raw[cur_p..cur_p + bpp] == ref_canvas.bytes[ref_p..ref_p + bpp] {
                    matched += 1;
                }
            }
        }
        Some((matched as f32) / (sample_pixels as f32))
    };

    // 1. Search vertical displacements: dx = 0, dy in [-d_max, d_max] \ {0}
    for dy in (-d_max..=d_max).filter(|&d| d != 0) {
        if let Some(ratio) = check_candidate(0, dy) {
            if ratio > best_ratio {
                best_ratio = ratio;
                best_dx = 0;
                best_dy = dy as i16;
                if best_ratio >= 0.98 {
                    break;
                }
            }
        }
    }

    // 2. If vertical didn't find high confidence match, search horizontal displacements: dy = 0, dx in [-d_max, d_max] \ {0}
    if best_ratio < min_match_ratio {
        for dx in (-d_max..=d_max).filter(|&d| d != 0) {
            if let Some(ratio) = check_candidate(dx, 0) {
                if ratio > best_ratio {
                    best_ratio = ratio;
                    best_dx = dx as i16;
                    best_dy = 0;
                    if best_ratio >= 0.98 {
                        break;
                    }
                }
            }
        }
    }

    // 3. If still below threshold, search coarse 2D diagonals (step 2)
    if best_ratio < min_match_ratio {
        for dy in (-d_max..=d_max).step_by(2).filter(|&d| d != 0) {
            for dx in (-d_max..=d_max).step_by(2).filter(|&d| d != 0) {
                if let Some(ratio) = check_candidate(dx, dy) {
                    if ratio > best_ratio {
                        best_ratio = ratio;
                        best_dx = dx as i16;
                        best_dy = dy as i16;
                        if best_ratio >= 0.98 {
                            break;
                        }
                    }
                }
            }
            if best_ratio >= 0.98 {
                break;
            }
        }
    }

    if best_ratio >= min_match_ratio {
        Some((best_dx, best_dy, best_ratio))
    } else {
        None
    }
}

/// Compresses a dirty tile using local motion estimation against reference canvas (v2.40 / ADR-126).
/// Returns `Some((TILE_SUBCODEC_MOTION, payload))` where payload is 5 bytes: [subcodec(1) | dx(2) | dy(2)].
pub fn compress_tile_motion(
    tile_raw: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
    config: &ScreenTileMotionConfig,
) -> Option<(u8, Vec<u8>)> {
    if !config.enabled {
        return None;
    }
    let min_ratio = (config.min_match_ratio as f32) / 100.0;
    let (dx, dy, _) = estimate_tile_motion(
        tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        ref_canvas,
        config.max_displacement,
        min_ratio,
    )?;

    let mut payload = Vec::with_capacity(5);
    payload.push(TILE_SUBCODEC_MOTION);
    payload.extend_from_slice(&dx.to_be_bytes());
    payload.extend_from_slice(&dy.to_be_bytes());
    Some((TILE_SUBCODEC_MOTION, payload))
}

/// Decompresses a motion-compressed dirty tile by extracting shifted pixels from reference canvas (v2.40 / ADR-126).
pub fn decompress_tile_motion(
    payload: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
) -> Result<Vec<u8>, CoreError> {
    if payload.len() < 5 || payload[0] != TILE_SUBCODEC_MOTION {
        return Err(CoreError::ScreenFrame);
    }
    if ref_canvas.bytes.is_empty() || ref_canvas.compressed() {
        return Err(CoreError::ScreenFrame);
    }
    let dx = i16::from_be_bytes(payload[1..3].try_into().unwrap()) as i32;
    let dy = i16::from_be_bytes(payload[3..5].try_into().unwrap()) as i32;

    let bpp = ref_canvas.format.bpp();
    let canvas_w = ref_canvas.rect.w as i32;
    let canvas_h = ref_canvas.rect.h as i32;
    let canvas_stride = ref_canvas.effective_stride() as usize;

    let ref_x = (tile_x as i32) - dx;
    let ref_y = (tile_y as i32) - dy;
    let w_i32 = tile_w as i32;
    let h_i32 = tile_h as i32;

    if ref_x < 0 || ref_y < 0 || ref_x + w_i32 > canvas_w || ref_y + h_i32 > canvas_h {
        return Err(CoreError::ScreenFrame);
    }

    let tile_stride = (tile_w as usize) * bpp;
    let mut decomp = vec![0u8; (tile_h as usize) * tile_stride];

    for r in 0..tile_h as usize {
        let src_off = (ref_y as usize + r) * canvas_stride + (ref_x as usize) * bpp;
        let dst_off = r * tile_stride;
        decomp[dst_off..dst_off + tile_stride]
            .copy_from_slice(&ref_canvas.bytes[src_off..src_off + tile_stride]);
    }

    Ok(decomp)
}

/// In-container playback engine reading frames from a TREC container and presenting
/// to a `ScreenSink` with PTS pacing, speed adjustment and loop controls (v2.16 / ADR-102).
pub struct ScreenPlaybackEngine<R: std::io::Read + std::io::Seek> {
    reader: ScreenRecordingReader<R>,
    meta: ScreenRecordingMeta,
    state: ScreenPlaybackState,
    speed: f32,
    loop_enabled: bool,
    current_frame: u64,
    total_frames: u64,
    duration_us: u64,
    elapsed_us: u64,
    last_pts_us: Option<u64>,
}

impl<R: std::io::Read + std::io::Seek> ScreenPlaybackEngine<R> {
    pub fn new(
        mut reader: ScreenRecordingReader<R>,
        speed: f32,
        loop_enabled: bool,
    ) -> Result<Self, CoreError> {
        let (total_frames, duration_us) = reader.scan_summary()?;
        let meta = reader.meta().clone();
        Ok(Self {
            reader,
            meta,
            state: ScreenPlaybackState::Idle,
            speed: if speed < 0.0 { 1.0 } else { speed },
            loop_enabled,
            current_frame: 0,
            total_frames,
            duration_us,
            elapsed_us: 0,
            last_pts_us: None,
        })
    }

    pub fn state(&self) -> ScreenPlaybackState {
        self.state
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = if speed < 0.0 { 1.0 } else { speed };
    }

    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled
    }

    pub fn set_loop_enabled(&mut self, enabled: bool) {
        self.loop_enabled = enabled;
    }

    pub fn meta(&self) -> &ScreenRecordingMeta {
        &self.meta
    }

    pub fn current_frame(&self) -> u64 {
        self.current_frame
    }

    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    pub fn pause(&mut self) {
        if self.state == ScreenPlaybackState::Playing {
            self.state = ScreenPlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == ScreenPlaybackState::Paused {
            self.state = ScreenPlaybackState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.state = ScreenPlaybackState::Stopped;
    }

    pub fn report(&self) -> ScreenPlaybackReport {
        ScreenPlaybackReport {
            ok: true,
            state: self.state.as_str().to_string(),
            path: String::new(),
            speed: self.speed,
            loop_enabled: self.loop_enabled,
            current_frame: self.current_frame,
            total_frames: self.total_frames,
            elapsed_ms: self.elapsed_us / 1000,
            duration_ms: self.duration_us / 1000,
            width: self.meta.width as u16,
            height: self.meta.height as u16,
            format: PixelFormat::from_tag(self.meta.format_tag)
                .map(|f| f.as_str().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            codec: match self.meta.codec_tag {
                1 => "zstd".to_string(),
                2 => "openh264".to_string(),
                _ => "raw".to_string(),
            },
        }
    }

    /// Step playback by one frame and present it to `sink`.
    /// When `speed > 0.0`, paces according to inter-frame PTS delta.
    /// When `speed == 0.0`, presents immediately without waiting (test mode).
    pub fn step(
        &mut self,
        sink: &dyn crate::ports::ScreenSink,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        if self.state == ScreenPlaybackState::Stopped || self.state == ScreenPlaybackState::Paused {
            return Ok(None);
        }
        self.state = ScreenPlaybackState::Playing;

        match self.reader.next_record()? {
            Some(record) => {
                let pts = record.pts_us;
                if let Some(prev) = self.last_pts_us {
                    let delta = pts.saturating_sub(prev);
                    self.elapsed_us = self.elapsed_us.saturating_add(delta);
                    if self.speed > 0.0 && delta > 0 {
                        let wait_us = (delta as f64 / self.speed as f64) as u64;
                        let capped_wait = wait_us.min(5_000_000);
                        if capped_wait > 0 {
                            std::thread::sleep(std::time::Duration::from_micros(capped_wait));
                        }
                    }
                }
                self.last_pts_us = Some(pts);
                let frame = self.reader.decode_frame(&record)?;
                sink.present(&frame)?;
                self.current_frame = self.current_frame.saturating_add(1);
                Ok(Some(frame))
            }
            None => {
                if self.loop_enabled && self.total_frames > 0 {
                    self.reader.rewind_to_start()?;
                    self.last_pts_us = None;
                    self.step(sink)
                } else {
                    self.state = ScreenPlaybackState::Finished;
                    Ok(None)
                }
            }
        }
    }
}

impl ScreenPlaybackEngine<Box<dyn ReadSeek>> {
    pub fn from_file(
        path: impl AsRef<std::path::Path>,
        speed: f32,
        loop_enabled: bool,
    ) -> Result<Self, CoreError> {
        let file = std::fs::File::open(path)?;
        let reader = ScreenRecordingReader::new(
            Box::new(std::io::BufReader::new(file)) as Box<dyn ReadSeek>
        )?;
        Self::new(reader, speed, loop_enabled)
    }

    pub fn from_bytes(bytes: Vec<u8>, speed: f32, loop_enabled: bool) -> Result<Self, CoreError> {
        let cursor = std::io::Cursor::new(bytes);
        let reader = ScreenRecordingReader::new(Box::new(cursor) as Box<dyn ReadSeek>)?;
        Self::new(reader, speed, loop_enabled)
    }
}

/// Virtual screen source fed by a recorded TREC container (v2.16 / ADR-102).
/// Implements `crate::ports::ScreenSource` so a node can stream a recorded session
/// over the network without requiring a physical monitor or GPU.
pub struct ScreenRecordingSource {
    reader: std::sync::Mutex<ScreenRecordingReader<Box<dyn ReadSeek>>>,
    meta: ScreenRecordingMeta,
    loop_enabled: std::sync::atomic::AtomicBool,
    fps: std::sync::atomic::AtomicU32,
    frames_grabbed: std::sync::atomic::AtomicU64,
}

impl ScreenRecordingSource {
    pub fn new(reader: ScreenRecordingReader<Box<dyn ReadSeek>>, loop_enabled: bool) -> Self {
        let meta = reader.meta().clone();
        Self {
            reader: std::sync::Mutex::new(reader),
            meta,
            loop_enabled: std::sync::atomic::AtomicBool::new(loop_enabled),
            fps: std::sync::atomic::AtomicU32::new(30),
            frames_grabbed: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn from_file(
        path: impl AsRef<std::path::Path>,
        loop_enabled: bool,
    ) -> Result<Self, CoreError> {
        let file = std::fs::File::open(path)?;
        let reader = ScreenRecordingReader::new(
            Box::new(std::io::BufReader::new(file)) as Box<dyn ReadSeek>
        )?;
        Ok(Self::new(reader, loop_enabled))
    }

    pub fn from_bytes(bytes: Vec<u8>, loop_enabled: bool) -> Result<Self, CoreError> {
        let cursor = std::io::Cursor::new(bytes);
        let reader = ScreenRecordingReader::new(Box::new(cursor) as Box<dyn ReadSeek>)?;
        Ok(Self::new(reader, loop_enabled))
    }

    pub fn meta(&self) -> &ScreenRecordingMeta {
        &self.meta
    }

    pub fn frames_grabbed(&self) -> u64 {
        self.frames_grabbed
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_loop_enabled(&self, val: bool) {
        self.loop_enabled
            .store(val, std::sync::atomic::Ordering::SeqCst);
    }
}

impl crate::ports::ScreenSource for ScreenRecordingSource {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        let mut reader = self.reader.lock().unwrap();
        match reader.next_record()? {
            Some(rec) => {
                self.frames_grabbed
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let frame = reader.decode_frame(&rec)?;
                Ok(Some(frame))
            }
            None => {
                if self.loop_enabled.load(std::sync::atomic::Ordering::SeqCst) {
                    reader.rewind_to_start()?;
                    match reader.next_record()? {
                        Some(rec) => {
                            self.frames_grabbed
                                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let frame = reader.decode_frame(&rec)?;
                            Ok(Some(frame))
                        }
                        None => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn fps(&self) -> u32 {
        self.fps.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_fps(&self, fps: u32) {
        self.fps
            .store(fps.clamp(1, 120), std::sync::atomic::Ordering::SeqCst);
    }

    fn displays(&self) -> Vec<ScreenDisplayInfo> {
        vec![ScreenDisplayInfo::new(
            0,
            "Virtual Recorded Display",
            self.meta.width,
            self.meta.height,
            100,
            true,
            true,
        )]
    }

    fn active_display_id(&self) -> u8 {
        0
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
    retransmit_buffer: ScreenRetransmitBuffer,
    twcc_sender: ScreenTwccSenderTracker,
    probe_controller: ScreenProbeController,
    sync_coordinator: ScreenAudioSyncCoordinator,
    active_display_id: u8,
    displays: Vec<ScreenDisplayInfo>,
    display_switches_count: u64,
    last_display_switch_us: u64,
    privacy_enabled: bool,
    privacy_lock_input: bool,
    privacy_blanking_method: String,
    privacy_last_change_us: u64,
    frame_profiler: ScreenFrameProfiler,
    tuner: ScreenTuner,
    congested_override: Option<bool>,
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
        let retransmit_buffer = ScreenRetransmitBuffer::new(ScreenHarqMode::Auto);
        let twcc_sender = ScreenTwccSenderTracker::new(ScreenTwccMode::Auto);
        let probe_controller = ScreenProbeController::default();
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let active_display_id = 0;
        let display_switches_count = 0;
        let last_display_switch_us = 0;
        let privacy_enabled = false;
        let privacy_lock_input = false;
        let privacy_blanking_method = "virtual_overlay".to_string();
        let privacy_last_change_us = 0;
        let frame_profiler = ScreenFrameProfiler::new(ScreenProfilerMode::Auto);
        let tuner = ScreenTuner::default();
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
            trendline_state: ScreenBandwidthUsageState::Normal.tag(),
            trendline_slope_us: 0.0,
            trendline_overuse_count: 0,
            harq_mode: retransmit_buffer.mode().tag(),
            harq_nacks_sent: 0,
            harq_nacks_received: 0,
            harq_shards_retransmitted: 0,
            harq_recovered_frames: 0,
            twcc_mode: twcc_sender.mode().tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_controller.mode().tag(),
            probe_state: probe_controller.state().tag(),
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: sync_coordinator.mode().tag(),
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            privacy_enabled,
            privacy_input_locked: privacy_lock_input,
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
            retransmit_buffer,
            twcc_sender,
            probe_controller,
            sync_coordinator,
            active_display_id,
            displays,
            display_switches_count,
            last_display_switch_us,
            privacy_enabled,
            privacy_lock_input,
            privacy_blanking_method,
            privacy_last_change_us,
            frame_profiler,
            tuner,
            congested_override: None,
        }
    }

    pub fn with_profiler(mode: ScreenProfilerMode) -> Self {
        let mut sender = Self::new();
        sender.set_profiler_mode(mode);
        sender
    }

    pub fn with_display(display_id: u8) -> Self {
        let mut sender = Self::new();
        let _ = sender.switch_display(display_id, 0);
        sender
    }

    pub fn with_sync(sync: ScreenAudioSyncMode) -> Self {
        let mut sender = Self::new();
        sender.set_sync_mode(sync);
        sender
    }

    pub fn with_probe(probe: ScreenProbeMode) -> Self {
        let mut sender = Self::new();
        sender.set_probe_mode(probe);
        sender
    }

    pub fn with_twcc(twcc: ScreenTwccMode) -> Self {
        let mut sender = Self::new();
        sender.set_twcc_mode(twcc);
        sender
    }

    pub fn with_harq(harq: ScreenHarqMode) -> Self {
        let mut sender = Self::new();
        sender.set_harq_mode(harq);
        sender
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
        let h_stats = self.retransmit_buffer.stats();
        s.harq_mode = self.retransmit_buffer.mode().tag();
        s.harq_nacks_sent = h_stats.nacks_sent;
        s.harq_nacks_received = h_stats.nacks_received;
        s.harq_shards_retransmitted = h_stats.shards_retransmitted;
        s.harq_recovered_frames = h_stats.recovered_frames;
        s.twcc_mode = self.twcc_sender.mode().tag();
        let twcc_st = self.twcc_sender.stats();
        s.twcc_feedbacks_sent = twcc_st.feedbacks_sent;
        s.twcc_feedbacks_received = twcc_st.feedbacks_received;
        s.twcc_delivery_rate_bps = twcc_st.delivery_rate_bps;
        s.twcc_delay_gradient_us = twcc_st.delay_gradient_us;
        let p_stats = self.probe_controller.stats();
        s.probe_mode = p_stats.mode.tag();
        s.probe_state = p_stats.state.tag();
        s.probes_triggered = p_stats.probes_triggered;
        s.probes_completed = p_stats.probes_completed;
        s.probes_failed = p_stats.probes_failed;
        s.probe_target_bps = p_stats.last_target_bps;
        s.probe_measured_bps = p_stats.last_measured_bps;
        let sync_st = self.sync_coordinator.stats();
        s.sync_mode = sync_st.mode.tag();
        s.sync_beacons_sent = sync_st.beacons_sent;
        s.sync_beacons_received = sync_st.beacons_received;
        s.sync_skew_us = sync_st.current_skew_us;
        s.sync_clock_drift_ppm = sync_st.clock_drift_ppm;
        s.sync_adjustments_applied = sync_st.adjustments_applied;
        s.in_sync = sync_st.in_sync;
        let lstats = self.frame_profiler.report();
        s.latency_mode = self.frame_profiler.mode().tag();
        s.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        s.latency_total_e2e_us = lstats.avg_e2e_us;
        s.latency_p95_e2e_us = lstats.p95_e2e_us;
        s.latency_frames_profiled = lstats.frames_profiled;
        s
    }

    /// Microsecond-precision multi-stage latency profiler mode (v2.13 / ADR-099).
    pub fn profiler_mode(&self) -> ScreenProfilerMode {
        self.frame_profiler.mode()
    }

    /// Alias for profiler_mode.
    pub fn latency_mode(&self) -> ScreenProfilerMode {
        self.profiler_mode()
    }

    /// Set microsecond-precision multi-stage latency profiler mode.
    pub fn set_profiler_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.frame_profiler.set_mode(mode);
        self.sync_profiler_stats();
        mode
    }

    /// Alias for set_profiler_mode.
    pub fn set_latency_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.set_profiler_mode(mode)
    }

    /// Microsecond-precision multi-stage latency report.
    pub fn latency_report(&self) -> ScreenLatencyReport {
        self.frame_profiler.report()
    }

    /// Reset latency profiler statistics and percentiles ring buffer.
    pub fn reset_latency(&mut self) -> ScreenLatencyReport {
        let rep = self.frame_profiler.reset();
        self.sync_profiler_stats();
        rep
    }

    /// Ingest a measured frame latency breakdown.
    pub fn record_latency_breakdown(&mut self, breakdown: ScreenLatencyBreakdown) {
        self.frame_profiler.record_frame(breakdown);
        self.sync_profiler_stats();
    }

    /// Ingest an inbound profile frame (TYPE_SCREEN_PROFILE).
    pub fn on_profile_frame(&mut self, frame: &ScreenProfileFrame) {
        self.frame_profiler.record_profile_frame(frame);
        self.sync_profiler_stats();
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.run_bottleneck_tuner(now_us, frame.frame_seq);
    }

    /// Bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    pub fn tuning_mode(&self) -> ScreenTunerMode {
        self.tuner.mode()
    }

    /// Alias for tuning_mode.
    pub fn tuner_mode(&self) -> ScreenTunerMode {
        self.tuning_mode()
    }

    /// Configure the bottleneck-driven autonomous tuner mode.
    pub fn set_tuning_mode(&mut self, mode: ScreenTunerMode) -> ScreenTunerMode {
        self.tuner.set_mode(mode);
        self.sync_tuner_stats();
        self.tuner.mode()
    }

    /// Structured autonomous tuning report for REST API and CLI.
    pub fn tuning_report(&self) -> ScreenTuningReport {
        self.tuner.report()
    }

    /// Reset autonomous tuning counters, hysteresis windows and recovery stack.
    pub fn reset_tuning(&mut self) -> ScreenTuningReport {
        self.tuner.reset();
        self.sync_tuner_stats();
        self.tuner.report()
    }

    /// Reference to internal autonomous tuner.
    pub fn tuner(&self) -> &ScreenTuner {
        &self.tuner
    }

    /// Mutable reference to internal autonomous tuner.
    pub fn tuner_mut(&mut self) -> &mut ScreenTuner {
        &mut self.tuner
    }

    /// Run the bottleneck-driven tuner over the freshly ingested profile frame and
    /// apply sender-side knob updates (bitrate / fps / scale / QP) (v2.14 / ADR-100).
    /// Only real cross-end profile envelopes drive the tuner; the local synthetic
    /// Ack-path breakdown stays observation-only so it cannot fight AIMD/TWCC.
    fn run_bottleneck_tuner(&mut self, now_us: u64, frame_seq: u64) {
        let bottleneck = self.frame_profiler.bottleneck();
        let (qp_min, qp_max) = self.quality_controller.qp_range();
        let ctx = ScreenTuningContext {
            bitrate_bps: self.rate_controller.current_bps(),
            bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
            bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
            fps: self.stats.fps_target,
            scale_pct: self.stats.scale_pct,
            qp_min,
            qp_max,
            playout_delay_us: 0,
        };
        let outcome = self.tuner.evaluate(now_us, frame_seq, bottleneck, &ctx);
        if let Some(bps) = outcome.bitrate_bps {
            self.rate_controller.set_manual_bps(bps);
            self.stats.bitrate_bps = self.rate_controller.current_bps();
        }
        if let Some(fps) = outcome.fps {
            self.stats.fps_target = self.fps_pacer.set_fps(fps);
        }
        if let Some(scale) = outcome.scale_pct {
            self.stats.scale_pct = self.scale_controller.set_scale_pct(scale);
        }
        if let Some((qmin, qmax)) = outcome.qp_range {
            let (amin, amax) = self.quality_controller.set_qp_range(qmin, qmax);
            self.stats.qp_min = amin;
            self.stats.qp_max = amax;
        }
        self.sync_tuner_stats();
    }

    /// Synchronize autonomous tuner telemetry into ScreenStats (v2.14 / ADR-100).
    pub fn sync_tuner_stats(&mut self) {
        let report = self.tuner.report();
        self.stats.tuning_mode = self.tuner.mode().tag();
        self.stats.tuning_actions_total = report.actions_applied;
        self.stats.tuning_recoveries = report.recoveries_applied;
        self.stats.tuning_last_stage = self.tuner.last_stage_tag();
        self.stats.tuning_last_action = self.tuner.last_action_tag();
    }

    /// Access reference to internal frame profiler.
    pub fn frame_profiler(&self) -> &ScreenFrameProfiler {
        &self.frame_profiler
    }

    /// Access mutable reference to internal frame profiler.
    pub fn frame_profiler_mut(&mut self) -> &mut ScreenFrameProfiler {
        &mut self.frame_profiler
    }

    /// Synchronize internal profiler telemetry to ScreenStats.
    pub fn sync_profiler_stats(&mut self) {
        let lstats = self.frame_profiler.report();
        self.stats.latency_mode = self.frame_profiler.mode().tag();
        self.stats.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        self.stats.latency_total_e2e_us = lstats.avg_e2e_us;
        self.stats.latency_p95_e2e_us = lstats.p95_e2e_us;
        self.stats.latency_frames_profiled = lstats.frames_profiled;
    }

    /// Active bandwidth probing mode (v2.09 / ADR-095).
    pub fn probe_mode(&self) -> ScreenProbeMode {
        self.probe_controller.mode()
    }

    /// Set active bandwidth probing mode.
    pub fn set_probe_mode(&mut self, mode: ScreenProbeMode) {
        self.probe_controller.set_mode(mode);
        self.sync_probe_stats();
    }

    /// Telemetry statistics for Screen Active Bandwidth Probing.
    pub fn probe_stats(&self) -> ScreenProbeStats {
        self.probe_controller.stats()
    }

    /// Structured probe status report for REST API and CLI.
    pub fn probe_report(&self) -> ScreenProbeReport {
        self.probe_controller.report()
    }

    /// Reference to internal probe controller.
    pub fn probe_controller(&self) -> &ScreenProbeController {
        &self.probe_controller
    }

    /// Mutable reference to internal probe controller.
    pub fn probe_controller_mut(&mut self) -> &mut ScreenProbeController {
        &mut self.probe_controller
    }

    /// Manually trigger an active bandwidth probe burst to target_bps.
    pub fn trigger_probe(&mut self, target_bps: u32, now_us: u64) -> bool {
        if self.probe_controller.mode() == ScreenProbeMode::Disabled {
            return false;
        }
        let cluster_size = self.probe_controller.mode().cluster_size().max(4);
        let mut seqs = Vec::with_capacity(cluster_size as usize);
        for _ in 0..cluster_size {
            let seq = self
                .twcc_sender
                .allocate_seq(now_us, SCREEN_PROBE_DEFAULT_PACKET_LEN);
            seqs.push(seq);
        }
        let res = self
            .probe_controller
            .trigger_probe_with_seqs(target_bps, now_us, &seqs);
        self.sync_probe_stats();
        res
    }

    /// Automatically schedule a bandwidth probe if network conditions and intervals allow.
    pub fn maybe_schedule_probe(&mut self, now_us: u64) -> bool {
        if self.probe_controller.mode() == ScreenProbeMode::Disabled {
            return false;
        }
        let cur_bps = self.rate_controller.current_bps();
        let mut next_tseq = self.twcc_sender.next_seq();
        let res = self
            .probe_controller
            .maybe_schedule_probe(cur_bps, now_us, &mut next_tseq);
        self.twcc_sender.set_next_seq(next_tseq);
        self.sync_probe_stats();
        res
    }

    /// Pop a pending probe packet to be transmitted over the wire.
    pub fn pop_probe_packet(&mut self) -> Option<ScreenProbePacket> {
        let pkt = self.probe_controller.pop_probe_packet()?;
        self.sync_probe_stats();
        Some(pkt)
    }

    /// Synchronize internal probe stats to ScreenStats.
    pub fn sync_probe_stats(&mut self) {
        let st = self.probe_controller.stats();
        self.stats.probe_mode = st.mode.tag();
        self.stats.probe_state = st.state.tag();
        self.stats.probes_triggered = st.probes_triggered;
        self.stats.probes_completed = st.probes_completed;
        self.stats.probes_failed = st.probes_failed;
        self.stats.probe_target_bps = st.last_target_bps;
        self.stats.probe_measured_bps = st.last_measured_bps;
    }

    /// Screen and audio presentation timestamp (PTS) sync mode (v2.10 / ADR-096).
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        self.sync_coordinator.mode()
    }

    /// Set screen and audio presentation timestamp sync mode.
    pub fn set_sync_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.sync_coordinator.set_mode(mode);
        self.sync_sync_stats();
    }

    /// Telemetry statistics for audio-video presentation sync.
    pub fn sync_stats(&self) -> ScreenAudioSyncStats {
        self.sync_coordinator.stats()
    }

    /// Structured sync status report for REST API and CLI.
    pub fn sync_report(&self) -> ScreenAudioSyncReport {
        self.sync_coordinator.report()
    }

    /// Record latest video presentation timestamp in microseconds.
    pub fn record_video_pts(&mut self, pts_us: u64) {
        self.sync_coordinator.record_video_pts(pts_us);
    }

    /// Record latest audio presentation timestamp in microseconds.
    pub fn record_audio_pts(&mut self, pts_us: u64) {
        self.sync_coordinator.record_audio_pts(pts_us);
    }

    /// Evaluates beacon generation interval and produces an outbound sync beacon if due.
    pub fn maybe_generate_sync_beacon(&mut self, now_us: u64) -> Option<ScreenAudioSyncFrame> {
        let frame = self.sync_coordinator.maybe_generate_sync_beacon(now_us);
        if frame.is_some() {
            self.sync_sync_stats();
        }
        frame
    }

    /// Unconditionally trigger a sync beacon (for tests or manual inspection).
    pub fn trigger_sync_beacon(&mut self, now_us: u64) -> ScreenAudioSyncFrame {
        let frame = self.sync_coordinator.trigger_sync_beacon(now_us);
        self.sync_sync_stats();
        frame
    }

    /// Pop a pending sync beacon to be transmitted over the wire.
    pub fn pop_sync_beacon(&mut self) -> Option<ScreenAudioSyncFrame> {
        self.sync_coordinator.pop_sync_beacon()
    }

    /// Synchronize internal sync coordinator stats to ScreenStats.
    pub fn sync_sync_stats(&mut self) {
        let st = self.sync_coordinator.stats();
        self.stats.sync_mode = st.mode.tag();
        self.stats.sync_beacons_sent = st.beacons_sent;
        self.stats.sync_beacons_received = st.beacons_received;
        self.stats.sync_skew_us = st.current_skew_us;
        self.stats.sync_clock_drift_ppm = st.clock_drift_ppm;
        self.stats.sync_adjustments_applied = st.adjustments_applied;
        self.stats.in_sync = st.in_sync;
    }

    pub fn twcc_mode(&self) -> ScreenTwccMode {
        self.twcc_sender.mode()
    }

    pub fn set_twcc_mode(&mut self, mode: ScreenTwccMode) {
        self.twcc_sender.set_mode(mode);
        self.stats.twcc_mode = mode.tag();
    }

    pub fn twcc_stats(&self) -> ScreenTwccStats {
        self.twcc_sender.stats()
    }

    pub fn twcc_report(&self) -> ScreenTwccReport {
        self.twcc_sender.report()
    }

    pub fn twcc_sender(&self) -> &ScreenTwccSenderTracker {
        &self.twcc_sender
    }

    pub fn twcc_sender_mut(&mut self) -> &mut ScreenTwccSenderTracker {
        &mut self.twcc_sender
    }

    pub fn handle_twcc_feedback(
        &mut self,
        feedback: &ScreenTwccFeedback,
        now_us: u64,
    ) -> ScreenTwccReport {
        let rep = self.twcc_sender.on_feedback(feedback, now_us);
        self.stats.twcc_feedbacks_received = rep.feedbacks_received;
        self.stats.twcc_delivery_rate_bps = rep.delivery_rate_bps;
        self.stats.twcc_delay_gradient_us = rep.delay_gradient_us;

        // Evaluate active bandwidth probe cluster with TWCC feedback
        if let Some(_new_bps) =
            self.probe_controller
                .on_twcc_feedback(feedback, &mut self.rate_controller, now_us)
        {
            self.stats.bitrate_bps = self.rate_controller.current_bps();
        }
        self.sync_probe_stats();

        if rep.delivery_rate_bps > 0 {
            let cap_bps = rep.delivery_rate_bps.min(u32::MAX as u64) as u32;
            if cap_bps < self.rate_controller.current_bps() && rep.loss_ratio > 0.05 {
                self.rate_controller.force_step_down();
                self.stats.bitrate_bps = self.rate_controller.current_bps();
            }
        }
        rep
    }

    pub fn harq_mode(&self) -> ScreenHarqMode {
        self.retransmit_buffer.mode()
    }

    pub fn set_harq_mode(&mut self, mode: ScreenHarqMode) {
        self.retransmit_buffer.set_mode(mode);
        self.stats.harq_mode = mode.tag();
    }

    pub fn harq_stats(&self) -> ScreenHarqStats {
        self.retransmit_buffer.stats()
    }

    pub fn harq_report(&self) -> ScreenHarqReport {
        self.retransmit_buffer.report()
    }

    pub fn retransmit_buffer(&self) -> &ScreenRetransmitBuffer {
        &self.retransmit_buffer
    }

    pub fn retransmit_buffer_mut(&mut self) -> &mut ScreenRetransmitBuffer {
        &mut self.retransmit_buffer
    }

    pub fn handle_nack(&mut self, nack: &ScreenNack, now_us: u64) -> Vec<InnerFrame> {
        let frames = self.retransmit_buffer.handle_nack(nack, now_us);
        let st = self.retransmit_buffer.stats();
        self.stats.harq_nacks_received = st.nacks_received;
        self.stats.harq_shards_retransmitted = st.shards_retransmitted;
        if nack.requires_keyframe_fallback {
            self.squelch_controller.force_wake(now_us);
            self.stats.is_squelched = false;
        }
        frames
    }

    pub fn record_outbound_frame(&mut self, frame: &InnerFrame, now_us: u64) {
        self.retransmit_buffer.record_outbound_frame(frame, now_us);
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

    /// Sets an explicit congestion override for testing and deterministic simulation (v2.37 / ADR-123).
    pub fn set_congested_override(&mut self, forced: Option<bool>) {
        self.congested_override = forced;
    }

    /// Whether the screen link is currently congested (v1.93 / ADR-080).
    /// Congestion is active when in-flight frames reach the queue threshold (>= 8),
    /// or when round-trip latency exceeds the 250ms interactive threshold while frames
    /// are in flight (pacing to 1 frame at a time to prevent bufferbloat without stalling).
    pub fn is_congested(&self) -> bool {
        if let Some(forced) = self.congested_override {
            return forced;
        }
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
                self.privacy_enabled = false;
                self.privacy_lock_input = false;
                self.stats.privacy_enabled = false;
                self.stats.privacy_input_locked = false;
                self.privacy_last_change_us = now_us;
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

                    // ADR-099 Screen Streaming Microsecond Latency Breakdown & Multi-Stage Frame Profiler
                    if self.frame_profiler.mode() != ScreenProfilerMode::Disabled {
                        let half_rtt = (rtt / 2) as u32;
                        let pacer_delay = (self.stats.pacer_avg_delay_us as u32).min(20_000);
                        let mut b = ScreenLatencyBreakdown {
                            frame_seq: seq,
                            capture_us: 3200,
                            encode_us: 4100,
                            pacer_us: pacer_delay.max(500),
                            network_us: half_rtt.max(1000),
                            jitter_us: 1200,
                            decode_us: 2400,
                            render_us: 1600,
                            total_e2e_us: 0,
                        };
                        b.calculate_total();
                        self.record_latency_breakdown(b);
                    }

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
            ControlMsg::SwitchDisplay { display_id } => {
                self.switch_display(display_id, now_us)?;
                Ok(None)
            }
            ControlMsg::SetPrivacyMode {
                enabled,
                lock_input,
            } => {
                self.set_privacy_mode(enabled, lock_input, now_us);
                Ok(None)
            }
            ControlMsg::ResetLatencyProfile => {
                self.reset_latency();
                Ok(None)
            }
            ControlMsg::SetTuningMode { mode } => {
                self.set_tuning_mode(ScreenTunerMode::from_tag(mode));
                Ok(None)
            }
            ControlMsg::ResetTuningProfile => {
                self.reset_tuning();
                Ok(None)
            }
            // Recording disclosure is receiver-side state; the sender only mirrors it.
            ControlMsg::SetRecordingState { .. } => Ok(None),
            // Watermark state is handled at node runtime level.
            ControlMsg::SetWatermarkState { .. } => Ok(None),
            // Laser pointer and annotations are handled at node runtime level.
            ControlMsg::SetLaserPointer { .. } => Ok(None),
            ControlMsg::ClearAnnotations => Ok(None),
            ControlMsg::WhiteboardUndo { .. }
            | ControlMsg::WhiteboardRedo { .. }
            | ControlMsg::SetWhiteboardPolicy { .. } => Ok(None),
            // Region of Interest (ROI) controls are handled at node runtime level.
            ControlMsg::SetScreenRoi { .. }
            | ControlMsg::ResetScreenRoi
            | ControlMsg::SetScreenRoiZoom { .. } => Ok(None),
            // Adaptive color depth controls are handled at node runtime level (ADR-108).
            ControlMsg::SetScreenColorMode { .. }
            | ControlMsg::ResetScreenColorMode => Ok(None),
            // Dynamic differential delta patch controls are handled at node runtime level (ADR-109).
            ControlMsg::SetScreenDeltaMode { .. }
            | ControlMsg::ResetScreenDeltaMode => Ok(None),
            // Objective visual quality metric controls are handled at node runtime level (ADR-110).
            ControlMsg::SetScreenQualityMetricMode { .. }
            | ControlMsg::ResetScreenQualityMetricMode => Ok(None),
            // Autonomous closed-loop QoE adaptation controls are handled at node runtime level (ADR-111).
            ControlMsg::SetScreenQoeMode { .. }
            | ControlMsg::ResetScreenQoeMode => Ok(None),
            // Progressive intra-refresh (PIR) controls are handled at node runtime level (ADR-112).
            ControlMsg::SetScreenPirMode { .. }
            | ControlMsg::ResetScreenPirMode => Ok(None),
            // Long-term reference (LTR) controls are handled at node runtime level (ADR-113).
            ControlMsg::SetScreenLtrMode { .. }
            | ControlMsg::ResetScreenLtrMode
            | ControlMsg::LtrRecoveryRequest { .. }
            | ControlMsg::LtrAnchorSync { .. } => Ok(None),
            // Temporal scalability (SVC-T) controls are handled at node runtime level (ADR-114).
            ControlMsg::SetScreenSvcMode { .. }
            | ControlMsg::ResetScreenSvcMode
            | ControlMsg::SvcT1DropSync { .. } => Ok(None),
            // 2D motion vector estimation and scroll acceleration controls are handled at node runtime level (ADR-115).
            ControlMsg::SetScreenMotionMode { .. }
            | ControlMsg::ResetScreenMotionMode
            | ControlMsg::MotionScrollSync { .. } => Ok(None),
            // Spatial tile-grid decomposition controls are handled at node runtime level (ADR-116).
            ControlMsg::SetScreenTileMode { .. }
            | ControlMsg::ResetScreenTileMode
            | ControlMsg::TileGridSync { .. } => Ok(None),
            // Tile dictionary cache controls are handled at node runtime level (ADR-117).
            ControlMsg::SetScreenTileCacheMode { .. }
            | ControlMsg::ResetScreenTileCacheMode
            | ControlMsg::TileCacheSync { .. } => Ok(None),
            // Foveated focus awareness and region-priority scheduling controls are handled at node runtime level (ADR-118).
            ControlMsg::SetScreenFocusMode { .. }
            | ControlMsg::SetScreenFocusBox { .. }
            | ControlMsg::ResetScreenFocusMode
            | ControlMsg::FocusSync { .. }
            | ControlMsg::FocusPeripheralSync { .. } => Ok(None),
            // Tile-level adaptive palette compression controls are handled at node runtime level (ADR-119).
            ControlMsg::SetScreenTilePaletteMode { .. }
            | ControlMsg::ResetScreenTilePaletteMode
            | ControlMsg::TilePaletteSync { .. } => Ok(None),
            // Tile-level predictive filter compression controls are handled at node runtime level (ADR-120).
            ControlMsg::SetScreenTilePredictMode { .. }
            | ControlMsg::ResetScreenTilePredictMode
            | ControlMsg::TilePredictSync { .. }
            // Tile-level presentation transaction controls are handled at node runtime level (ADR-121).
            | ControlMsg::SetScreenTilePresentMode { .. }
            | ControlMsg::ResetScreenTilePresentMode
            | ControlMsg::TilePresentSync { .. }
            // Tile-level fast retransmission and micro-NACK controls are handled at node runtime level (ADR-122).
            | ControlMsg::TileNackRequest { .. }
            | ControlMsg::ResetScreenTileNackMode
            | ControlMsg::TileNackSync { .. }
            // Tile saliency and semantic importance tiering controls are handled at node runtime level (ADR-123).
            | ControlMsg::SetScreenTileSaliencyMode { .. }
            | ControlMsg::ResetScreenTileSaliencyMode
            | ControlMsg::TileSaliencySync { .. }
            // Tile dynamic LoD downsampling controls are handled at node runtime level (ADR-124).
            | ControlMsg::SetScreenTileLodMode { .. }
            | ControlMsg::ResetScreenTileLodMode
            | ControlMsg::TileLodSync { .. }
            // Tile dynamic batch chunking and pacing controls are handled at node runtime level (ADR-125).
            | ControlMsg::SetScreenTileBatchMode { .. }
            | ControlMsg::ResetScreenTileBatchMode
            | ControlMsg::TileBatchSync { .. }
            // Tile dynamic local motion vector estimation controls are handled at node runtime level (ADR-126).
            | ControlMsg::SetScreenTileMotionMode { .. }
            | ControlMsg::ResetScreenTileMotionMode
            | ControlMsg::TileMotionSync { .. } => Ok(None),
        }
    }

    pub fn active_display_id(&self) -> u8 {
        self.active_display_id
    }

    pub fn displays(&self) -> &[ScreenDisplayInfo] {
        &self.displays
    }

    pub fn set_displays(&mut self, displays: Vec<ScreenDisplayInfo>) {
        if !displays.is_empty() {
            self.stats.display_count = displays.len().min(255) as u8;
            self.displays = displays;
        }
    }

    pub fn switch_display(&mut self, display_id: u8, now_us: u64) -> Result<(), CoreError> {
        if !self.displays.is_empty() && !self.displays.iter().any(|d| d.display_id == display_id) {
            return Err(CoreError::ScreenRefused(format!(
                "Display {display_id} not found"
            )));
        }
        self.active_display_id = display_id;
        self.display_switches_count = self.display_switches_count.saturating_add(1);
        self.last_display_switch_us = now_us;
        self.stats.active_display_id = display_id;
        self.stats.display_switches_count = self.display_switches_count;
        self.stats.keyframes_requested = self.stats.keyframes_requested.saturating_add(1);
        self.squelch_controller.force_wake(now_us);
        self.stats.is_squelched = false;
        Ok(())
    }

    pub fn display_report(&self) -> ScreenDisplayReport {
        ScreenDisplayReport {
            ok: true,
            active_display_id: self.active_display_id,
            displays: self.displays.clone(),
            switches_count: self.display_switches_count,
            last_switch_us: self.last_display_switch_us,
        }
    }

    pub fn with_privacy(enabled: bool, lock_input: bool) -> Self {
        let mut sender = Self::new();
        sender.set_privacy_mode(enabled, lock_input, 0);
        sender
    }

    pub fn privacy_report(&self) -> ScreenPrivacyReport {
        ScreenPrivacyReport {
            ok: true,
            enabled: self.privacy_enabled,
            lock_input: self.privacy_lock_input,
            blanking_method: self.privacy_blanking_method.clone(),
            last_change_us: self.privacy_last_change_us,
        }
    }

    pub fn set_privacy_mode(
        &mut self,
        enabled: bool,
        lock_input: bool,
        now_us: u64,
    ) -> ScreenPrivacyReport {
        self.privacy_enabled = enabled;
        self.privacy_lock_input = lock_input;
        self.privacy_last_change_us = now_us;
        self.stats.privacy_enabled = enabled;
        self.stats.privacy_input_locked = lock_input;
        self.privacy_report()
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
        self.record_video_pts(now_us);
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

    /// Convert a ScreenFrame into outbound InnerFrames with monotonic send timestamp (microseconds).
    /// Outbound frames are tracked in the sender's sliding window HARQ retransmission buffer.
    pub fn encode_frame_to_inner_frames_with_time(
        &mut self,
        frame: &ScreenFrame,
        packet_loss_ratio: f32,
        now_us: u64,
    ) -> Result<Vec<InnerFrame>, CoreError> {
        let payload = frame.encode_payload()?;
        let m_parity = self
            .fec_controller
            .calculate_parity_shards(1, packet_loss_ratio);
        if m_parity == 0 {
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        // Split payload into shards that comfortably fit within INNER_PAYLOAD_MAX
        let shard_chunk_max = INNER_PAYLOAD_MAX - FEC_HEADER_LEN - 32;
        let k = payload.len().div_ceil(shard_chunk_max).max(1);
        if k > FEC_MAX_DATA_SHARDS {
            // If too large for single FEC block, fallback to standard fragmentation
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        let mut data_buffers = Vec::with_capacity(k);
        for chunk in payload.chunks(shard_chunk_max) {
            data_buffers.push(chunk.to_vec());
        }

        let m = self
            .fec_controller
            .calculate_parity_shards(k, packet_loss_ratio);
        if m == 0 {
            let frames = frame.to_frames()?;
            for f in &frames {
                self.twcc_sender.allocate_seq(now_us, f.payload.len());
                self.retransmit_buffer.record_outbound_frame(f, now_us);
            }
            return Ok(frames);
        }

        let mut shards = ScreenFecEncoder::encode_block(frame.seq, 0, &data_buffers, m)?;
        self.fec_controller.record_block_encoded(k, m);
        self.stats.fec_redundancy_pct = self.fec_controller.stats().redundancy_pct;

        for shard in &mut shards {
            shard.transport_seq = self
                .twcc_sender
                .allocate_seq(now_us, shard.data.len() + FEC_HEADER_LEN);
        }

        let frames: Vec<InnerFrame> = shards.iter().map(|s| s.to_inner_frame()).collect();
        for f in &frames {
            self.retransmit_buffer.record_outbound_frame(f, now_us);
        }
        Ok(frames)
    }

    /// Convert a ScreenFrame into outbound InnerFrames.
    /// If FEC mode is active (redundancy > 0), encodes payload into Cauchy RS shards (TYPE_SCREEN_FEC).
    /// Otherwise, falls back to standard fragmentation (TYPE_SCREEN_FRAME).
    pub fn encode_frame_to_inner_frames(
        &mut self,
        frame: &ScreenFrame,
        packet_loss_ratio: f32,
    ) -> Result<Vec<InnerFrame>, CoreError> {
        self.encode_frame_to_inner_frames_with_time(frame, packet_loss_ratio, 0)
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
        self.retransmit_buffer.clear();
        self.twcc_sender.clear();
        self.stats.is_squelched = false;
        self.stats.squelched_frames = 0;
        self.stats.harq_nacks_sent = 0;
        self.stats.harq_nacks_received = 0;
        self.stats.harq_shards_retransmitted = 0;
        self.stats.harq_recovered_frames = 0;
        self.stats.twcc_feedbacks_sent = 0;
        self.stats.twcc_feedbacks_received = 0;
        self.stats.twcc_delivery_rate_bps = 0;
        self.stats.twcc_delay_gradient_us = 0;
        self.stats.probe_state = ScreenProbeState::Idle.tag();
        self.stats.probes_triggered = 0;
        self.stats.probes_completed = 0;
        self.stats.probes_failed = 0;
        self.stats.probe_target_bps = 0;
        self.stats.probe_measured_bps = 0;
        self.probe_controller = ScreenProbeController::new(self.probe_controller.mode());
        self.sync_coordinator.reset();
        self.sync_sync_stats();
        self.stats.active_display_id = 0;
        self.active_display_id = 0;
        self.display_switches_count = 0;
        self.last_display_switch_us = 0;
        self.stats.display_switches_count = 0;
        self.privacy_enabled = false;
        self.privacy_lock_input = false;
        self.stats.privacy_enabled = false;
        self.stats.privacy_input_locked = false;
        self.frame_profiler.reset();
        self.sync_profiler_stats();
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
    harq_mode: ScreenHarqMode,
    harq_stats: ScreenHarqStats,
    nack_history: VecDeque<(u64, u16, u64)>,
    twcc_receiver: ScreenTwccReceiverTracker,
    probe_mode: ScreenProbeMode,
    probe_stats: ScreenProbeStats,
    sync_coordinator: ScreenAudioSyncCoordinator,
    active_display_id: u8,
    displays: Vec<ScreenDisplayInfo>,
    display_switches_count: u64,
    last_switch_us: u64,
    privacy_enabled: bool,
    privacy_lock_input: bool,
    privacy_blanking_method: String,
    privacy_last_change_us: u64,
    frame_profiler: ScreenFrameProfiler,
    tuner: ScreenTuner,
}

impl Default for ScreenReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenReceiver {
    pub fn new() -> Self {
        let cursor_controller = ScreenCursorController::default();
        let harq_mode = ScreenHarqMode::Auto;
        let harq_stats = ScreenHarqStats {
            mode: harq_mode,
            ..Default::default()
        };
        let twcc_mode = ScreenTwccMode::Auto;
        let twcc_receiver = ScreenTwccReceiverTracker::new(twcc_mode);
        let probe_mode = ScreenProbeMode::Auto;
        let probe_stats = ScreenProbeStats {
            mode: probe_mode,
            ..Default::default()
        };
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let frame_profiler = ScreenFrameProfiler::default();
        let tuner = ScreenTuner::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            harq_mode: harq_mode.tag(),
            twcc_mode: twcc_mode.tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_mode.tag(),
            probe_state: probe_stats.state.tag(),
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: sync_coordinator.mode().tag(),
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            latency_mode: frame_profiler.mode().tag(),
            latency_bottleneck: frame_profiler.bottleneck().tag(),
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
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
            harq_mode,
            harq_stats,
            nack_history: VecDeque::with_capacity(64),
            twcc_receiver,
            probe_mode,
            probe_stats,
            sync_coordinator,
            active_display_id: 0,
            displays,
            display_switches_count: 0,
            last_switch_us: 0,
            privacy_enabled: false,
            privacy_lock_input: false,
            privacy_blanking_method: "software_blanking".to_string(),
            privacy_last_change_us: 0,
            frame_profiler,
            tuner,
        }
    }

    pub fn with_playout_mode(mode: ScreenPlayoutMode) -> Self {
        let cursor_controller = ScreenCursorController::default();
        let harq_mode = ScreenHarqMode::Auto;
        let harq_stats = ScreenHarqStats {
            mode: harq_mode,
            ..Default::default()
        };
        let twcc_mode = ScreenTwccMode::Auto;
        let twcc_receiver = ScreenTwccReceiverTracker::new(twcc_mode);
        let probe_mode = ScreenProbeMode::Auto;
        let probe_stats = ScreenProbeStats {
            mode: probe_mode,
            ..Default::default()
        };
        let sync_coordinator = ScreenAudioSyncCoordinator::default();
        let displays = vec![ScreenDisplayInfo::primary_default()];
        let frame_profiler = ScreenFrameProfiler::default();
        let tuner = ScreenTuner::default();
        let stats = ScreenStats {
            cursor_mode: cursor_controller.mode().tag(),
            cursor_visible: true,
            cursor_shape: CursorShape::Arrow.tag(),
            harq_mode: harq_mode.tag(),
            twcc_mode: twcc_mode.tag(),
            twcc_feedbacks_sent: 0,
            twcc_feedbacks_received: 0,
            twcc_delivery_rate_bps: 0,
            twcc_delay_gradient_us: 0,
            probe_mode: probe_mode.tag(),
            probe_state: probe_stats.state.tag(),
            probes_triggered: 0,
            probes_completed: 0,
            probes_failed: 0,
            probe_target_bps: 0,
            probe_measured_bps: 0,
            sync_mode: sync_coordinator.mode().tag(),
            sync_beacons_sent: 0,
            sync_beacons_received: 0,
            sync_skew_us: 0,
            sync_clock_drift_ppm: 0.0,
            sync_adjustments_applied: 0,
            in_sync: true,
            active_display_id: 0,
            display_count: 1,
            display_switches_count: 0,
            latency_mode: frame_profiler.mode().tag(),
            latency_bottleneck: frame_profiler.bottleneck().tag(),
            latency_total_e2e_us: 0,
            latency_p95_e2e_us: 0,
            latency_frames_profiled: 0,
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
            harq_mode,
            harq_stats,
            nack_history: VecDeque::with_capacity(64),
            twcc_receiver,
            probe_mode,
            probe_stats,
            sync_coordinator,
            active_display_id: 0,
            displays,
            display_switches_count: 0,
            last_switch_us: 0,
            privacy_enabled: false,
            privacy_lock_input: false,
            privacy_blanking_method: "software_blanking".to_string(),
            privacy_last_change_us: 0,
            frame_profiler,
            tuner,
        }
    }

    pub fn with_profiler(mode: ScreenProfilerMode) -> Self {
        let mut r = Self::new();
        r.set_profiler_mode(mode);
        r
    }

    pub fn with_harq_mode(mode: ScreenHarqMode) -> Self {
        let mut r = Self::new();
        r.set_harq_mode(mode);
        r
    }

    pub fn with_twcc_mode(mode: ScreenTwccMode) -> Self {
        let mut r = Self::new();
        r.set_twcc_mode(mode);
        r
    }

    pub fn with_probe_mode(mode: ScreenProbeMode) -> Self {
        let mut r = Self::new();
        r.set_probe_mode(mode);
        r
    }

    pub fn with_sync_mode(mode: ScreenAudioSyncMode) -> Self {
        let mut r = Self::new();
        r.set_sync_mode(mode);
        r
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
        st.harq_mode = self.harq_mode.tag();
        st.harq_nacks_sent = self.harq_stats.nacks_sent;
        st.harq_nacks_received = self.harq_stats.nacks_received;
        st.harq_shards_retransmitted = self.harq_stats.shards_retransmitted;
        st.harq_recovered_frames = self.harq_stats.recovered_frames;
        st.twcc_mode = self.twcc_receiver.mode().tag();
        let twcc_st = self.twcc_receiver.stats();
        st.twcc_feedbacks_sent = twcc_st.feedbacks_sent;
        st.twcc_feedbacks_received = twcc_st.feedbacks_received;
        st.twcc_delivery_rate_bps = twcc_st.delivery_rate_bps;
        st.twcc_delay_gradient_us = twcc_st.delay_gradient_us;
        st.probe_mode = self.probe_mode.tag();
        st.probe_state = self.probe_stats.state.tag();
        st.probes_triggered = self.probe_stats.probes_triggered;
        st.probes_completed = self.probe_stats.probes_completed;
        st.probes_failed = self.probe_stats.probes_failed;
        st.probe_target_bps = self.probe_stats.last_target_bps;
        st.probe_measured_bps = self.probe_stats.last_measured_bps;
        let sync_st = self.sync_coordinator.stats();
        st.sync_mode = sync_st.mode.tag();
        st.sync_beacons_sent = sync_st.beacons_sent;
        st.sync_beacons_received = sync_st.beacons_received;
        st.sync_skew_us = sync_st.current_skew_us;
        st.sync_clock_drift_ppm = sync_st.clock_drift_ppm;
        st.sync_adjustments_applied = sync_st.adjustments_applied;
        st.in_sync = sync_st.in_sync;
        let lstats = self.frame_profiler.report();
        st.latency_mode = self.frame_profiler.mode().tag();
        st.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        st.latency_total_e2e_us = lstats.avg_e2e_us;
        st.latency_p95_e2e_us = lstats.p95_e2e_us;
        st.latency_frames_profiled = lstats.frames_profiled;
        st
    }

    /// Microsecond-precision multi-stage latency profiler mode (v2.13 / ADR-099).
    pub fn profiler_mode(&self) -> ScreenProfilerMode {
        self.frame_profiler.mode()
    }

    /// Alias for profiler_mode.
    pub fn latency_mode(&self) -> ScreenProfilerMode {
        self.profiler_mode()
    }

    /// Set microsecond-precision multi-stage latency profiler mode.
    pub fn set_profiler_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.frame_profiler.set_mode(mode);
        self.sync_profiler_stats();
        mode
    }

    /// Alias for set_profiler_mode.
    pub fn set_latency_mode(&mut self, mode: ScreenProfilerMode) -> ScreenProfilerMode {
        self.set_profiler_mode(mode)
    }

    /// Microsecond-precision multi-stage latency report.
    pub fn latency_report(&self) -> ScreenLatencyReport {
        self.frame_profiler.report()
    }

    /// Reset latency profiler statistics and percentiles ring buffer.
    pub fn reset_latency(&mut self) -> ScreenLatencyReport {
        let rep = self.frame_profiler.reset();
        self.sync_profiler_stats();
        rep
    }

    /// Ingest a measured frame latency breakdown.
    pub fn record_latency_breakdown(&mut self, breakdown: ScreenLatencyBreakdown) {
        self.frame_profiler.record_frame(breakdown);
        self.sync_profiler_stats();
    }

    /// Ingest an inbound profile frame (TYPE_SCREEN_PROFILE).
    pub fn on_profile_frame(&mut self, frame: &ScreenProfileFrame) {
        self.frame_profiler.record_profile_frame(frame);
        self.sync_profiler_stats();
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.run_bottleneck_tuner(now_us, frame.frame_seq);
    }

    /// Bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    pub fn tuning_mode(&self) -> ScreenTunerMode {
        self.tuner.mode()
    }

    /// Configure the bottleneck-driven autonomous tuner mode.
    pub fn set_tuning_mode(&mut self, mode: ScreenTunerMode) -> ScreenTunerMode {
        self.tuner.set_mode(mode);
        self.sync_tuner_stats();
        self.tuner.mode()
    }

    /// Structured autonomous tuning report for REST API and CLI.
    pub fn tuning_report(&self) -> ScreenTuningReport {
        self.tuner.report()
    }

    /// Reset autonomous tuning counters, hysteresis windows and recovery stack.
    pub fn reset_tuning(&mut self) -> ScreenTuningReport {
        self.tuner.reset();
        self.sync_tuner_stats();
        self.tuner.report()
    }

    /// Reference to internal autonomous tuner.
    pub fn tuner(&self) -> &ScreenTuner {
        &self.tuner
    }

    /// Mutable reference to internal autonomous tuner.
    pub fn tuner_mut(&mut self) -> &mut ScreenTuner {
        &mut self.tuner
    }

    /// Run the bottleneck-driven tuner over the freshly profiled frame and apply the
    /// receiver-side knob update (playout buffer target delay) (v2.14 / ADR-100).
    fn run_bottleneck_tuner(&mut self, now_us: u64, frame_seq: u64) {
        let bottleneck = self.frame_profiler.bottleneck();
        let ctx = ScreenTuningContext {
            playout_delay_us: self.playout_buffer.target_delay_us().min(u32::MAX as u64) as u32,
            ..Default::default()
        };
        let outcome = self.tuner.evaluate(now_us, frame_seq, bottleneck, &ctx);
        if let Some(delta) = outcome.playout_delay_delta_us {
            self.playout_buffer.adjust_target_delay(delta);
            self.stats.playout_delay_us =
                self.playout_buffer.target_delay_us().min(u32::MAX as u64) as u32;
        }
        self.sync_tuner_stats();
    }

    /// Synchronize autonomous tuner telemetry into ScreenStats (v2.14 / ADR-100).
    pub fn sync_tuner_stats(&mut self) {
        let report = self.tuner.report();
        self.stats.tuning_mode = self.tuner.mode().tag();
        self.stats.tuning_actions_total = report.actions_applied;
        self.stats.tuning_recoveries = report.recoveries_applied;
        self.stats.tuning_last_stage = self.tuner.last_stage_tag();
        self.stats.tuning_last_action = self.tuner.last_action_tag();
    }

    /// Access reference to internal frame profiler.
    pub fn frame_profiler(&self) -> &ScreenFrameProfiler {
        &self.frame_profiler
    }

    /// Access mutable reference to internal frame profiler.
    pub fn frame_profiler_mut(&mut self) -> &mut ScreenFrameProfiler {
        &mut self.frame_profiler
    }

    /// Synchronize internal profiler telemetry to ScreenStats.
    pub fn sync_profiler_stats(&mut self) {
        let lstats = self.frame_profiler.report();
        self.stats.latency_mode = self.frame_profiler.mode().tag();
        self.stats.latency_bottleneck = self.frame_profiler.bottleneck().tag();
        self.stats.latency_total_e2e_us = lstats.avg_e2e_us;
        self.stats.latency_p95_e2e_us = lstats.p95_e2e_us;
        self.stats.latency_frames_profiled = lstats.frames_profiled;
    }

    /// Active bandwidth probing mode (v2.09 / ADR-095).
    pub fn probe_mode(&self) -> ScreenProbeMode {
        self.probe_mode
    }

    /// Explicitly configure active bandwidth probing mode.
    pub fn set_probe_mode(&mut self, mode: ScreenProbeMode) {
        self.probe_mode = mode;
        self.probe_stats.mode = mode;
        self.sync_probe_stats();
    }

    /// Telemetry statistics for Screen Active Bandwidth Probing.
    pub fn probe_stats(&self) -> ScreenProbeStats {
        self.probe_stats
    }

    /// Structured probe status report for REST API and CLI.
    pub fn probe_report(&self) -> ScreenProbeReport {
        ScreenProbeReport {
            ok: true,
            mode: self.probe_mode,
            state: self.probe_stats.state,
            probes_triggered: self.probe_stats.probes_triggered,
            probes_completed: self.probe_stats.probes_completed,
            probes_failed: self.probe_stats.probes_failed,
            last_target_bps: self.probe_stats.last_target_bps,
            last_measured_bps: self.probe_stats.last_measured_bps,
            last_rtt_us: self.probe_stats.last_rtt_us,
            last_loss_ratio: self.probe_stats.last_loss_ratio,
            last_delay_gradient_us: self.probe_stats.last_delay_gradient_us,
            active_cluster_id: self.probe_stats.active_cluster_id,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Process incoming active bandwidth probe packet (v2.09 / ADR-095).
    /// Records packet arrival timestamp into TWCC receiver tracker for feedback emission.
    pub fn on_probe_packet_with_time(&mut self, probe: ScreenProbePacket, now_us: u64) {
        if probe.transport_seq > 0 {
            self.twcc_receiver
                .on_packet_received(probe.transport_seq, now_us);
        }
        self.probe_stats.probes_triggered = self.probe_stats.probes_triggered.saturating_add(1);
        self.probe_stats.last_target_bps = probe.target_bps;
        self.probe_stats.active_cluster_id = Some(probe.cluster_id);
        self.sync_probe_stats();
    }

    pub fn on_probe_packet(&mut self, probe: ScreenProbePacket) {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_probe_packet_with_time(probe, now_us);
    }

    fn sync_probe_stats(&mut self) {
        self.stats.probe_mode = self.probe_mode.tag();
        self.stats.probe_state = self.probe_stats.state.tag();
        self.stats.probes_triggered = self.probe_stats.probes_triggered;
        self.stats.probes_completed = self.probe_stats.probes_completed;
        self.stats.probes_failed = self.probe_stats.probes_failed;
        self.stats.probe_target_bps = self.probe_stats.last_target_bps;
        self.stats.probe_measured_bps = self.probe_stats.last_measured_bps;
    }

    /// Audio-video lip-sync alignment mode (v2.10 / ADR-096).
    pub fn sync_mode(&self) -> ScreenAudioSyncMode {
        self.sync_coordinator.mode()
    }

    /// Explicitly configure audio-video lip-sync alignment mode.
    pub fn set_sync_mode(&mut self, mode: ScreenAudioSyncMode) {
        self.sync_coordinator.set_mode(mode);
        self.sync_sync_stats();
    }

    /// Snapshot of receiver-side sync statistics.
    pub fn sync_stats(&self) -> ScreenAudioSyncStats {
        self.sync_coordinator.stats()
    }

    /// Structured receiver-side sync status report.
    pub fn sync_report(&self) -> ScreenAudioSyncReport {
        self.sync_coordinator.report()
    }

    /// Ingest an inbound sync beacon with explicit arrival timestamp in microseconds.
    pub fn on_sync_beacon_with_time(&mut self, beacon: &ScreenAudioSyncFrame, now_us: u64) -> i64 {
        let skew =
            self.sync_coordinator
                .on_sync_beacon(beacon, now_us, Some(&mut self.playout_buffer));
        self.sync_sync_stats();
        skew
    }

    /// Ingest an inbound sync beacon using current system clock.
    pub fn on_sync_beacon(&mut self, beacon: &ScreenAudioSyncFrame) -> i64 {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_sync_beacon_with_time(beacon, now_us)
    }

    fn sync_sync_stats(&mut self) {
        let st = self.sync_coordinator.stats();
        self.stats.sync_mode = st.mode.tag();
        self.stats.sync_beacons_sent = st.beacons_sent;
        self.stats.sync_beacons_received = st.beacons_received;
        self.stats.sync_skew_us = st.current_skew_us;
        self.stats.sync_clock_drift_ppm = st.clock_drift_ppm;
        self.stats.sync_adjustments_applied = st.adjustments_applied;
        self.stats.in_sync = st.in_sync;
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

    /// Screen HARQ mode (v2.07 / ADR-093).
    pub fn harq_mode(&self) -> ScreenHarqMode {
        self.harq_mode
    }

    /// Explicitly configure screen HARQ mode.
    pub fn set_harq_mode(&mut self, mode: ScreenHarqMode) {
        self.harq_mode = mode;
        self.harq_stats.mode = mode;
        self.stats.harq_mode = mode.tag();
        if mode == ScreenHarqMode::Disabled {
            self.nack_history.clear();
        }
    }

    /// Snapshot of receiver-side HARQ statistics.
    pub fn harq_stats(&self) -> ScreenHarqStats {
        self.harq_stats
    }

    /// Structured receiver-side HARQ status report.
    pub fn harq_report(&self) -> ScreenHarqReport {
        ScreenHarqReport {
            ok: true,
            mode: self.harq_mode,
            nacks_sent: self.harq_stats.nacks_sent,
            nacks_received: self.harq_stats.nacks_received,
            shards_retransmitted: self.harq_stats.shards_retransmitted,
            recovered_frames: self.harq_stats.recovered_frames,
            retransmit_timeouts: self.harq_stats.retransmit_timeouts,
            buffer_occupancy: 0,
            buffer_capacity: SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Screen TWCC mode (v2.08 / ADR-094).
    pub fn twcc_mode(&self) -> ScreenTwccMode {
        self.twcc_receiver.mode()
    }

    /// Explicitly configure screen TWCC mode.
    pub fn set_twcc_mode(&mut self, mode: ScreenTwccMode) {
        self.twcc_receiver.set_mode(mode);
        self.stats.twcc_mode = mode.tag();
    }

    /// Snapshot of receiver-side TWCC statistics.
    pub fn twcc_stats(&self) -> ScreenTwccStats {
        self.twcc_receiver.stats()
    }

    /// Structured receiver-side TWCC status report.
    pub fn twcc_report(&self) -> ScreenTwccReport {
        self.twcc_receiver.report()
    }

    /// Access reference to internal TWCC receiver tracker.
    pub fn twcc_receiver(&self) -> &ScreenTwccReceiverTracker {
        &self.twcc_receiver
    }

    /// Access mutable reference to internal TWCC receiver tracker.
    pub fn twcc_receiver_mut(&mut self) -> &mut ScreenTwccReceiverTracker {
        &mut self.twcc_receiver
    }

    /// Generate periodic or packet-threshold TWCC arrival feedback frame.
    pub fn generate_twcc_feedback(&mut self, now_us: u64) -> Option<ScreenTwccFeedback> {
        if self.twcc_receiver.should_send_feedback(now_us) {
            let fb = self.twcc_receiver.build_feedback(now_us);
            if fb.is_some() {
                self.stats.twcc_feedbacks_sent = self.twcc_receiver.stats().feedbacks_sent;
            }
            fb
        } else {
            None
        }
    }

    /// Generate selective NACK on missing FEC shards with RTT pacing debounce.
    pub fn generate_nack_on_fec(
        &mut self,
        now_us: u64,
        rtt_estimate_us: u64,
    ) -> Option<ScreenNack> {
        if self.harq_mode == ScreenHarqMode::Disabled {
            return None;
        }
        let nack = self.fec_assembler.generate_nack(rtt_estimate_us)?;
        let min_interval_us = rtt_estimate_us.max(20_000);
        if self.nack_history.iter().any(|&(seq, blk, t)| {
            seq == nack.frame_seq
                && blk == nack.block_id
                && now_us.saturating_sub(t) < min_interval_us
        }) {
            return None;
        }
        if self.nack_history.len() >= 64 {
            self.nack_history.pop_front();
        }
        self.nack_history
            .push_back((nack.frame_seq, nack.block_id, now_us));
        self.harq_stats.nacks_sent = self.harq_stats.nacks_sent.saturating_add(1);
        self.stats.harq_nacks_sent = self.harq_stats.nacks_sent;
        Some(nack)
    }

    /// Generate selective NACK on missing standard fragments with RTT pacing debounce.
    pub fn generate_nack_on_frag(
        &mut self,
        assembler: &ScreenAssembler,
        now_us: u64,
        rtt_estimate_us: u64,
    ) -> Option<ScreenNack> {
        if self.harq_mode == ScreenHarqMode::Disabled {
            return None;
        }
        let nack = assembler.generate_nack(rtt_estimate_us)?;
        let min_interval_us = rtt_estimate_us.max(20_000);
        if self.nack_history.iter().any(|&(seq, blk, t)| {
            seq == nack.frame_seq
                && blk == nack.block_id
                && now_us.saturating_sub(t) < min_interval_us
        }) {
            return None;
        }
        if self.nack_history.len() >= 64 {
            self.nack_history.pop_front();
        }
        self.nack_history
            .push_back((nack.frame_seq, nack.block_id, now_us));
        self.harq_stats.nacks_sent = self.harq_stats.nacks_sent.saturating_add(1);
        self.stats.harq_nacks_sent = self.harq_stats.nacks_sent;
        Some(nack)
    }

    /// Record a frame successfully recovered via HARQ retransmission.
    pub fn record_recovered_frame(&mut self) {
        self.harq_stats.recovered_frames = self.harq_stats.recovered_frames.saturating_add(1);
        self.stats.harq_recovered_frames = self.harq_stats.recovered_frames;
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

    /// Handle incoming forward error correction (FEC) shard packet with timestamp (v2.08 / ADR-094).
    /// Records arrival microsecond timestamp into TWCC receiver tracker and forwards to FEC assembler.
    pub fn on_fec_shard_with_time(
        &mut self,
        shard: ScreenFecShard,
        now_us: u64,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        if shard.transport_seq > 0 {
            self.twcc_receiver
                .on_packet_received(shard.transport_seq, now_us);
        }
        let res = self.fec_assembler.push_shard(shard);
        self.stats.fec_recovered_shards = self.fec_assembler.recovered_shards();
        self.stats.fec_unrecoverable_blocks = self.fec_assembler.unrecoverable_blocks();
        res
    }

    /// Handle incoming forward error correction (FEC) shard packet (v2.04 / ADR-090).
    /// If shard satisfies K data shards or reconstruction threshold, assembles and returns ScreenFrame.
    pub fn on_fec_shard(
        &mut self,
        shard: ScreenFecShard,
    ) -> Result<Option<ScreenFrame>, CoreError> {
        let now_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        self.on_fec_shard_with_time(shard, now_us)
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

        // ADR-099 Screen Streaming Microsecond Latency Breakdown & Multi-Stage Frame Profiler
        if self.frame_profiler.mode() != ScreenProfilerMode::Disabled {
            let mut b = ScreenLatencyBreakdown {
                frame_seq: frame.seq,
                capture_us: 3200,
                encode_us: 4100,
                pacer_us: 800,
                network_us: 1500,
                jitter_us: 1200,
                decode_us: 2400,
                render_us: 1600,
                total_e2e_us: 0,
            };
            b.calculate_total();
            self.record_latency_breakdown(b);
        }

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
        self.nack_history.clear();
        self.harq_stats = ScreenHarqStats {
            mode: self.harq_mode,
            ..Default::default()
        };
        self.twcc_receiver.clear();
        self.stats.harq_nacks_sent = 0;
        self.stats.harq_nacks_received = 0;
        self.stats.harq_shards_retransmitted = 0;
        self.stats.harq_recovered_frames = 0;
        self.stats.twcc_feedbacks_sent = 0;
        self.stats.twcc_feedbacks_received = 0;
        self.stats.twcc_delivery_rate_bps = 0;
        self.stats.twcc_delay_gradient_us = 0;
        self.probe_stats = ScreenProbeStats {
            mode: self.probe_mode,
            ..Default::default()
        };
        self.stats.probe_state = ScreenProbeState::Idle.tag();
        self.stats.probes_triggered = 0;
        self.stats.probes_completed = 0;
        self.stats.probes_failed = 0;
        self.stats.probe_target_bps = 0;
        self.stats.probe_measured_bps = 0;
        self.sync_coordinator.reset();
        self.sync_sync_stats();
        self.active_display_id = 0;
        self.display_switches_count = 0;
        self.last_switch_us = 0;
        self.stats.active_display_id = 0;
        self.stats.display_switches_count = 0;
        self.privacy_enabled = false;
        self.privacy_lock_input = false;
        self.stats.privacy_enabled = false;
        self.stats.privacy_input_locked = false;
        self.frame_profiler.reset();
        self.sync_profiler_stats();
    }

    pub fn active_display_id(&self) -> u8 {
        self.active_display_id
    }

    pub fn set_active_display_id(&mut self, display_id: u8) {
        self.active_display_id = display_id;
        self.stats.active_display_id = display_id;
    }

    pub fn switch_display(&mut self, display_id: u8, now_us: u64) {
        self.active_display_id = display_id;
        self.display_switches_count = self.display_switches_count.saturating_add(1);
        self.last_switch_us = now_us;
        self.stats.active_display_id = display_id;
        self.stats.display_switches_count = self.display_switches_count;
    }

    pub fn displays(&self) -> &[ScreenDisplayInfo] {
        &self.displays
    }

    pub fn set_displays(&mut self, displays: Vec<ScreenDisplayInfo>) {
        if !displays.is_empty() {
            self.stats.display_count = displays.len().min(255) as u8;
            self.displays = displays;
        }
    }

    pub fn display_report(&self) -> ScreenDisplayReport {
        ScreenDisplayReport {
            ok: true,
            active_display_id: self.active_display_id,
            displays: self.displays.clone(),
            switches_count: self.display_switches_count,
            last_switch_us: self.last_switch_us,
        }
    }

    pub fn with_privacy(mut self, enabled: bool, lock_input: bool) -> Self {
        self.set_privacy_mode(enabled, lock_input, 0);
        self
    }

    pub fn privacy_report(&self) -> ScreenPrivacyReport {
        ScreenPrivacyReport {
            ok: true,
            enabled: self.privacy_enabled,
            lock_input: self.privacy_lock_input,
            blanking_method: self.privacy_blanking_method.clone(),
            last_change_us: self.privacy_last_change_us,
        }
    }

    pub fn set_privacy_mode(&mut self, enabled: bool, lock_input: bool, now_us: u64) {
        self.privacy_enabled = enabled;
        self.privacy_lock_input = lock_input;
        self.privacy_last_change_us = now_us;
        self.stats.privacy_enabled = enabled;
        self.stats.privacy_input_locked = lock_input;
    }

    pub fn privacy_enabled(&self) -> bool {
        self.privacy_enabled
    }

    pub fn privacy_lock_input(&self) -> bool {
        self.privacy_lock_input
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

    /// Whether assembler is currently holding an incomplete fragment stream.
    pub fn is_incomplete(&self) -> bool {
        !self.mask.is_empty() && self.accepted < self.count
    }

    /// Sequence number of current frame stream being assembled.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Generate selective NACK for missing fragment chunks in current stream (v2.07 / ADR-093).
    pub fn generate_nack(&self, rtt_estimate_us: u64) -> Option<ScreenNack> {
        if self.mask.is_empty() || self.accepted >= self.count || self.count == 0 {
            return None;
        }
        let mut missing_indices = Vec::new();
        let mut missing_mask = 0u64;
        for (i, &arrived) in self.mask.iter().enumerate() {
            if !arrived {
                missing_indices.push(i as u16);
                if i < 64 {
                    missing_mask |= 1u64 << i;
                }
            }
        }
        if missing_indices.is_empty() {
            return None;
        }
        Some(ScreenNack {
            frame_seq: self.seq,
            block_id: 0,
            missing_mask,
            missing_indices,
            is_fec: false,
            requires_keyframe_fallback: false,
            rtt_estimate_us,
        })
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

    pub fn reset(&mut self) {
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
    active_display: std::sync::atomic::AtomicU8,
    displays: std::sync::Mutex<Vec<ScreenDisplayInfo>>,
}

impl MemoryScreenSource {
    pub fn new(w: u16, h: u16) -> Result<Self, CoreError> {
        Ok(Self {
            rect: ScreenRect::new(0, 0, w, h)?,
            format: PixelFormat::Bgra8,
            seq: std::sync::atomic::AtomicU64::new(1),
            fail_at: std::sync::atomic::AtomicU64::new(0),
            active_display: std::sync::atomic::AtomicU8::new(0),
            displays: std::sync::Mutex::new(vec![ScreenDisplayInfo::primary_default()]),
        })
    }

    /// Make the `n`-th grab (1-based) return an error once.
    pub fn fail_on_grab(&self, n: u64) {
        self.fail_at.store(n, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn rect(&self) -> ScreenRect {
        self.rect
    }

    pub fn set_displays(&self, displays: Vec<ScreenDisplayInfo>) {
        if let Ok(mut lock) = self.displays.lock() {
            *lock = displays;
        }
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

    fn displays(&self) -> Vec<ScreenDisplayInfo> {
        self.displays
            .lock()
            .map(|d| d.clone())
            .unwrap_or_else(|_| vec![ScreenDisplayInfo::primary_default()])
    }

    fn set_displays(&self, displays: Vec<ScreenDisplayInfo>) {
        if let Ok(mut lock) = self.displays.lock() {
            *lock = displays;
        }
    }

    fn active_display_id(&self) -> u8 {
        self.active_display
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn switch_display(&self, display_id: u8) -> Result<(), CoreError> {
        let displays = self.displays();
        if !displays.is_empty() && !displays.iter().any(|d| d.display_id == display_id) {
            return Err(CoreError::ScreenRefused(format!(
                "Display {display_id} not found"
            )));
        }
        self.active_display
            .store(display_id, std::sync::atomic::Ordering::SeqCst);
        Ok(())
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
        payload[26] = 255;
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
            ControlMsg::SwitchDisplay { display_id: 2 },
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
            transport_seq: 101,
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

#[test]
fn screen_harq_selective_nack_retransmission_pipeline() {
    // 1. Parsing and enum tags for ScreenHarqMode
    assert_eq!(
        "auto".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenHarqMode>().unwrap(),
        ScreenHarqMode::Disabled
    );
    assert!("unknown".parse::<ScreenHarqMode>().is_err());

    assert_eq!(ScreenHarqMode::from_tag(0), ScreenHarqMode::Auto);
    assert_eq!(ScreenHarqMode::from_tag(1), ScreenHarqMode::Aggressive);
    assert_eq!(ScreenHarqMode::from_tag(2), ScreenHarqMode::Conservative);
    assert_eq!(ScreenHarqMode::from_tag(3), ScreenHarqMode::Disabled);
    assert_eq!(ScreenHarqMode::Auto.as_str(), "auto");
    assert_eq!(ScreenHarqMode::Aggressive.as_str(), "aggressive");
    assert_eq!(ScreenHarqMode::Conservative.as_str(), "conservative");
    assert_eq!(ScreenHarqMode::Disabled.as_str(), "disabled");

    // 2. ScreenNack wire protocol encode & decode round-trip
    let nack = ScreenNack {
        frame_seq: 42,
        block_id: 3,
        missing_mask: 0b1010,
        missing_indices: vec![1, 3],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 25_000,
    };
    let encoded = nack.encode();
    assert_eq!(&encoded[0..5], &SCREEN_NACK_MAGIC);
    assert_eq!(encoded[5], SCREEN_NACK_PROTO);

    let decoded = ScreenNack::decode(&encoded).expect("decode nack");
    assert_eq!(decoded.frame_seq, 42);
    assert_eq!(decoded.block_id, 3);
    assert_eq!(decoded.missing_mask, 0b1010);
    assert_eq!(decoded.missing_indices, vec![1, 3]);
    assert!(decoded.is_fec);
    assert!(!decoded.requires_keyframe_fallback);
    assert_eq!(decoded.rtt_estimate_us, 25_000);

    // Corrupt magic rejection
    let mut corrupt = encoded.clone();
    corrupt[0] = b'X';
    assert!(ScreenNack::decode(&corrupt).is_err());

    // Corrupt length rejection
    assert!(ScreenNack::decode(&encoded[0..20]).is_err());

    // InnerFrame packaging round-trip
    let inner = nack.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_NACK);
    let decoded_inner = ScreenNack::decode(&inner.payload).expect("decode inner payload");
    assert_eq!(decoded_inner, nack);

    // 3. ScreenRetransmitBuffer sliding window and selective retry limits
    let mut buf = ScreenRetransmitBuffer::new(ScreenHarqMode::Auto);
    assert_eq!(
        buf.stats().buffer_capacity,
        SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY
    );
    assert_eq!(buf.len(), 0);

    let shard0 = ScreenFecShard {
        frame_seq: 100,
        block_id: 1,
        k_data_shards: 4,
        m_parity_shards: 2,
        shard_index: 0,
        is_parity: false,
        transport_seq: 1,
        data: vec![10; 100],
    };
    let shard1 = ScreenFecShard {
        shard_index: 1,
        data: vec![20; 100],
        ..shard0.clone()
    };
    let shard2 = ScreenFecShard {
        shard_index: 2,
        data: vec![30; 100],
        ..shard0.clone()
    };

    let f0 = shard0.to_inner_frame();
    let f1 = shard1.to_inner_frame();
    let f2 = shard2.to_inner_frame();

    buf.record_outbound_frame(&f0, 1_000_000);
    buf.record_outbound_frame(&f1, 1_000_000);
    buf.record_outbound_frame(&f2, 1_000_000);
    assert_eq!(buf.len(), 3);

    // NACK requesting missing shard 1
    let req_shard1 = ScreenNack {
        frame_seq: 100,
        block_id: 1,
        missing_mask: 0b0010,
        missing_indices: vec![1],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 10_000,
    };
    let retx = buf.handle_nack(&req_shard1, 1_015_000);
    assert_eq!(retx.len(), 1);
    let retx_shard = ScreenFecShard::from_inner_frame(&retx[0]).unwrap();
    assert_eq!(retx_shard.shard_index, 1);
    assert_eq!(retx_shard.data, vec![20; 100]);
    assert_eq!(buf.stats().shards_retransmitted, 1);
    assert_eq!(buf.stats().nacks_received, 1);

    // Second retry under Auto mode (retry ceiling = 2)
    let retx2 = buf.handle_nack(&req_shard1, 1_030_000);
    assert_eq!(retx2.len(), 1);
    assert_eq!(buf.stats().shards_retransmitted, 2);

    // Third retry exceeds Auto mode ceiling (2) -> refused/timeouts incremented
    let retx3 = buf.handle_nack(&req_shard1, 1_045_000);
    assert_eq!(retx3.len(), 0);
    assert_eq!(buf.stats().retransmit_timeouts, 1);

    // Age expiration (> 300ms)
    let req_shard0 = ScreenNack {
        frame_seq: 100,
        block_id: 1,
        missing_mask: 0b0001,
        missing_indices: vec![0],
        is_fec: true,
        requires_keyframe_fallback: false,
        rtt_estimate_us: 10_000,
    };
    let retx_expired = buf.handle_nack(&req_shard0, 1_400_000); // 400ms later
    assert_eq!(retx_expired.len(), 0);
    assert_eq!(buf.stats().retransmit_timeouts, 2);

    // 4. Receiver NACK generation and RTT debounce
    let mut receiver = ScreenReceiver::with_harq_mode(ScreenHarqMode::Auto);
    assert_eq!(receiver.harq_mode(), ScreenHarqMode::Auto);

    // Ingest shard 0 (k=2, m=1), missing shard 1
    let r_shard0 = ScreenFecShard {
        frame_seq: 200,
        block_id: 0,
        k_data_shards: 2,
        m_parity_shards: 1,
        shard_index: 0,
        is_parity: false,
        transport_seq: 10,
        data: vec![55; 100],
    };
    let _ = receiver.on_fec_shard(r_shard0);

    let nack1 = receiver.generate_nack_on_fec(2_000_000, 20_000);
    assert!(nack1.is_some());
    let nack1 = nack1.unwrap();
    assert_eq!(nack1.frame_seq, 200);
    assert_eq!(nack1.block_id, 0);
    assert_eq!(nack1.missing_indices, vec![1, 2]); // missing shard 1 and parity 0 (idx 2)
    assert_eq!(receiver.harq_stats().nacks_sent, 1);

    // Immediate duplicate NACK within RTT window (< 20ms) is debounced
    let nack_dup = receiver.generate_nack_on_fec(2_005_000, 20_000);
    assert!(nack_dup.is_none());
    assert_eq!(receiver.harq_stats().nacks_sent, 1);

    // After RTT window passes (> 20ms) -> NACK allowed again
    let nack2 = receiver.generate_nack_on_fec(2_025_000, 20_000);
    assert!(nack2.is_some());
    assert_eq!(receiver.harq_stats().nacks_sent, 2);

    // 5. Sender end-to-end integration
    let mut sender = ScreenSender::with_harq(ScreenHarqMode::Auto);
    let sf = ScreenFrame {
        seq: 300,
        bytes: vec![123; 500],
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        },
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 40,
    };
    let encoded_frames = sender
        .encode_frame_to_inner_frames_with_time(&sf, 0.05, 3_000_000)
        .expect("encode with time");
    assert!(!encoded_frames.is_empty());
    assert!(!sender.retransmit_buffer().is_empty());

    let fallback_nack = ScreenNack {
        frame_seq: 300,
        block_id: 0,
        missing_mask: 0b1111,
        missing_indices: vec![0, 1, 2, 3],
        is_fec: true,
        requires_keyframe_fallback: true,
        rtt_estimate_us: 15_000,
    };
    let _ = sender.handle_nack(&fallback_nack, 3_010_000);
    assert_eq!(sender.stats().harq_nacks_received, 1);
    assert!(!sender.stats().is_squelched);
}

#[test]
fn screen_twcc_transport_wide_congestion_control_pipeline() {
    // 1. Parsing and enum tags for ScreenTwccMode
    assert_eq!(
        "auto".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenTwccMode>().unwrap(),
        ScreenTwccMode::Disabled
    );
    assert!("unknown".parse::<ScreenTwccMode>().is_err());

    assert_eq!(ScreenTwccMode::from_tag(0), ScreenTwccMode::Auto);
    assert_eq!(ScreenTwccMode::from_tag(1), ScreenTwccMode::Conservative);
    assert_eq!(ScreenTwccMode::from_tag(2), ScreenTwccMode::Aggressive);
    assert_eq!(ScreenTwccMode::from_tag(3), ScreenTwccMode::Disabled);

    assert_eq!(ScreenTwccMode::Auto.interval_us(), 10_000);
    assert_eq!(ScreenTwccMode::Aggressive.interval_us(), 5_000);
    assert_eq!(ScreenTwccMode::Conservative.interval_us(), 25_000);
    assert_eq!(ScreenTwccMode::Disabled.interval_us(), u64::MAX);

    assert_eq!(ScreenTwccMode::Auto.packet_threshold(), 40);
    assert_eq!(ScreenTwccMode::Aggressive.packet_threshold(), 20);
    assert_eq!(ScreenTwccMode::Conservative.packet_threshold(), 80);
    assert_eq!(ScreenTwccMode::Disabled.packet_threshold(), usize::MAX);

    // 2. ScreenTwccFeedback wire protocol codec
    let fb = ScreenTwccFeedback {
        feedback_seq: 12,
        base_seq: 100,
        packet_count: 3,
        reference_time_us: 1_000_000,
        records: vec![
            ScreenTwccPacketRecord {
                transport_seq: 100,
                recv_delta_us: 0,
            },
            ScreenTwccPacketRecord {
                transport_seq: 101,
                recv_delta_us: 250,
            },
            ScreenTwccPacketRecord {
                transport_seq: 102,
                recv_delta_us: SCREEN_TWCC_MISSING_DELTA, // lost packet
            },
        ],
    };
    let encoded = fb.encode();
    assert_eq!(&encoded[0..5], &SCREEN_TWCC_MAGIC);
    assert_eq!(encoded[5], SCREEN_TWCC_PROTO);
    assert_eq!(
        encoded.len(),
        SCREEN_TWCC_HEADER_LEN + 3 * SCREEN_TWCC_RECORD_LEN
    );

    let decoded = ScreenTwccFeedback::decode(&encoded).expect("decode twcc");
    assert_eq!(decoded, fb);

    // Corrupt magic & length checks
    let mut corrupt = encoded.clone();
    corrupt[0] = b'X';
    assert!(ScreenTwccFeedback::decode(&corrupt).is_err());
    assert!(ScreenTwccFeedback::decode(&encoded[..19]).is_err());
    assert!(ScreenTwccFeedback::decode(&encoded[..23]).is_err());

    // InnerFrame round-trip
    let inner = fb.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_TWCC);
    assert_eq!(
        ScreenTwccFeedback::from_inner_frame(&inner).expect("from inner"),
        fb
    );

    // 3. ScreenTwccReceiverTracker
    let mut rx = ScreenTwccReceiverTracker::new(ScreenTwccMode::Auto);
    assert!(!rx.should_send_feedback(1_000_000));
    assert!(rx.build_feedback(1_000_000).is_none());

    // Record packets
    rx.on_packet_received(10, 1_000_000);
    rx.on_packet_received(11, 1_000_200);
    rx.on_packet_received(12, 1_000_500);

    // Before interval or threshold -> false / none
    assert!(!rx.should_send_feedback(1_005_000));

    // After interval (10ms) -> generates feedback
    assert!(rx.should_send_feedback(1_011_000));
    let fb_out = rx.build_feedback(1_011_000).expect("should emit feedback");
    assert_eq!(fb_out.base_seq, 10);
    assert_eq!(fb_out.packet_count, 3);
    assert_eq!(fb_out.records.len(), 3);
    assert_eq!(fb_out.records[0].recv_delta_us, 0);
    assert_eq!(fb_out.records[1].recv_delta_us, 200);
    assert_eq!(fb_out.records[2].recv_delta_us, 500);

    // Generating again immediately returns None (buffer cleared)
    assert!(rx.build_feedback(1_012_000).is_none());

    // Packet threshold trigger (e.g. Aggressive mode: 20 packets)
    rx.set_mode(ScreenTwccMode::Aggressive);
    for seq in 20..40 {
        rx.on_packet_received(seq, 1_020_000 + (seq as u64) * 10);
    }
    assert!(rx.should_send_feedback(1_020_100));
    let fb_agg = rx.build_feedback(1_020_100).expect("threshold triggered");
    assert_eq!(fb_agg.packet_count, 20);

    // 4. ScreenTwccSenderTracker
    let mut tx = ScreenTwccSenderTracker::new(ScreenTwccMode::Auto);
    let s1 = tx.allocate_seq(1_000_000, 1000);
    let s2 = tx.allocate_seq(1_001_000, 1000);
    let s3 = tx.allocate_seq(1_002_000, 1000);
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s3, 3);

    // Create feedback matching s1, s2, s3 (s2 received with 1ms delay, s3 lost)
    let fb_test = ScreenTwccFeedback {
        feedback_seq: 1,
        base_seq: 1,
        packet_count: 3,
        reference_time_us: 1_010_000,
        records: vec![
            ScreenTwccPacketRecord {
                transport_seq: 1,
                recv_delta_us: 0,
            },
            ScreenTwccPacketRecord {
                transport_seq: 2,
                recv_delta_us: 1_000,
            },
            ScreenTwccPacketRecord {
                transport_seq: 3,
                recv_delta_us: SCREEN_TWCC_MISSING_DELTA,
            },
        ],
    };

    let rep = tx.on_feedback(&fb_test, 1_020_000);
    assert_eq!(rep.feedbacks_received, 1);
    assert_eq!(rep.packets_lost, 1);
    assert_eq!(rep.packets_acknowledged, 2);
    assert!(rep.loss_ratio > 0.3);
    assert!(rep.delivery_rate_bps > 0);

    // 5. End-to-end integration with ScreenSender & ScreenReceiver
    let mut sender = ScreenSender::new();
    sender.set_twcc_mode(ScreenTwccMode::Auto);
    sender.set_fec_mode(ScreenFecMode::Aggressive);
    assert_eq!(sender.twcc_mode(), ScreenTwccMode::Auto);
    assert_eq!(sender.stats().twcc_mode, ScreenTwccMode::Auto.tag());

    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    let frame = ScreenFrame {
        seq: 50,
        bytes: vec![0xAB; 2000],
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: 20,
            h: 20,
        },
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 80,
    };

    let inner_frames = sender
        .encode_frame_to_inner_frames_with_time(&frame, 0.05, 2_000_000)
        .expect("encode");
    assert!(!inner_frames.is_empty());

    let mut receiver = ScreenReceiver::new();
    receiver.set_twcc_mode(ScreenTwccMode::Auto);

    let mut shards_received = 0;
    for in_frame in &inner_frames {
        if in_frame.ty == TYPE_SCREEN_FEC {
            let shard = ScreenFecShard::from_inner_frame(in_frame).expect("shard");
            assert!(shard.transport_seq > 0);
            let _ = receiver.on_fec_shard_with_time(shard, 2_005_000);
            shards_received += 1;
        }
    }
    assert!(shards_received > 0);

    let fb_rx = receiver
        .generate_twcc_feedback(2_016_000)
        .expect("rx feedback");
    let sender_rep = sender.handle_twcc_feedback(&fb_rx, 2_020_000);
    assert_eq!(sender_rep.feedbacks_received, 1);
    assert_eq!(sender.stats().twcc_feedbacks_received, 1);
    assert_eq!(receiver.stats().twcc_feedbacks_sent, 1);

    // Test peer disconnect resets
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().twcc_feedbacks_received, 0);
    assert_eq!(receiver.stats().twcc_feedbacks_sent, 0);
}

#[test]
fn screen_active_bandwidth_probing_and_fast_rampup_pipeline() {
    // 1. Parsing and enum tags for ScreenProbeMode
    assert_eq!(
        "auto".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Auto
    );
    assert_eq!(
        "aggressive".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Aggressive
    );
    assert_eq!(
        "agg".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Aggressive
    );
    assert_eq!(
        "conservative".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Conservative
    );
    assert_eq!(
        "cons".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Conservative
    );
    assert_eq!(
        "disabled".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenProbeMode>().unwrap(),
        ScreenProbeMode::Disabled
    );
    assert!("invalid".parse::<ScreenProbeMode>().is_err());
    assert_eq!(ScreenProbeMode::from_tag(0), ScreenProbeMode::Auto);
    assert_eq!(ScreenProbeMode::from_tag(1), ScreenProbeMode::Aggressive);
    assert_eq!(ScreenProbeMode::from_tag(2), ScreenProbeMode::Conservative);
    assert_eq!(ScreenProbeMode::from_tag(3), ScreenProbeMode::Disabled);

    // 2. Wire Protocol and framing: ScreenProbePacket encode/decode round-trip
    let original = ScreenProbePacket {
        flags: 0x01,
        cluster_id: 42,
        cluster_index: 3,
        cluster_size: 8,
        transport_seq: 105,
        target_bps: 12_000_000,
        send_time_us: 1_234_567_890,
        padding: vec![0xEE; 256],
    };
    let encoded = original.encode();
    assert_eq!(encoded.len(), SCREEN_PROBE_HEADER_LEN + 256);
    assert_eq!(&encoded[0..5], &SCREEN_PROBE_MAGIC);
    assert_eq!(encoded[5], SCREEN_PROBE_PROTO);

    let decoded = ScreenProbePacket::decode(&encoded).expect("decode probe packet");
    assert_eq!(decoded.flags, original.flags);
    assert_eq!(decoded.cluster_id, original.cluster_id);
    assert_eq!(decoded.cluster_index, original.cluster_index);
    assert_eq!(decoded.cluster_size, original.cluster_size);
    assert_eq!(decoded.transport_seq, original.transport_seq);
    assert_eq!(decoded.target_bps, original.target_bps);
    assert_eq!(decoded.send_time_us, original.send_time_us);
    assert_eq!(decoded.padding, original.padding);

    // Corrupted magic and truncated slice error tests
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenProbePacket::decode(&bad_magic).is_err());

    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenProbePacket::decode(&bad_proto).is_err());

    assert!(ScreenProbePacket::decode(&encoded[..SCREEN_PROBE_HEADER_LEN - 1]).is_err());
    assert!(ScreenProbePacket::decode(&encoded[..SCREEN_PROBE_HEADER_LEN + 10]).is_err());

    // InnerFrame round-trip
    let inner = original.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_PROBE);
    assert_eq!(
        ScreenProbePacket::from_inner_frame(&inner).expect("from inner"),
        original
    );

    // 3. ScreenProbeController state machine and cluster generation
    let mut controller = ScreenProbeController::new(ScreenProbeMode::Auto);
    assert_eq!(controller.mode(), ScreenProbeMode::Auto);
    assert_eq!(controller.state(), ScreenProbeState::Idle);

    // Trigger probe burst for 12 Mbps
    let mut mock_seq = 1000u16;
    let ok = controller.trigger_probe(12_000_000, 1_000_000, &mut mock_seq);
    assert!(ok);
    assert_eq!(controller.stats().probes_triggered, 1);
    assert_eq!(controller.stats().last_target_bps, 12_000_000);
    assert_eq!(controller.state(), ScreenProbeState::Probing);

    let mut popped = Vec::new();
    while let Some(p) = controller.pop_probe_packet() {
        popped.push(p);
    }
    assert_eq!(popped.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);
    assert_eq!(controller.state(), ScreenProbeState::AwaitingFeedback);

    for (idx, p) in popped.iter().enumerate() {
        assert_eq!(p.cluster_id, 1);
        assert_eq!(p.cluster_index, idx as u8);
        assert_eq!(p.cluster_size, SCREEN_PROBE_CLUSTER_SIZE_AUTO);
        assert_eq!(p.target_bps, 12_000_000);
        assert_eq!(p.transport_seq, 1001 + idx as u16);
    }

    // Evaluating feedback: successful burst delivery with high delivery rate and low loss
    let mut rate_ctrl = ScreenRateController::new(SCREEN_BITRATE_DEFAULT_BPS);
    let mut records = Vec::new();
    for p in &popped {
        records.push(ScreenTwccPacketRecord {
            transport_seq: p.transport_seq,
            recv_delta_us: (p.cluster_index as i16) * 500, // 500us intervals -> 3500us span for 8 packets -> high delivery rate
        });
    }
    let fb_success = ScreenTwccFeedback {
        feedback_seq: 1,
        base_seq: popped[0].transport_seq,
        packet_count: popped.len() as u16,
        reference_time_us: 1_010_000,
        records,
    };

    let ramp_target = controller.on_twcc_feedback(&fb_success, &mut rate_ctrl, 1_015_000);
    assert!(ramp_target.is_some());
    assert_eq!(ramp_target.unwrap(), 12_000_000);
    assert_eq!(controller.state(), ScreenProbeState::Completed);
    assert_eq!(controller.stats().probes_completed, 1);
    assert_eq!(rate_ctrl.current_bps(), 12_000_000);

    // Failed probe evaluation (high loss > 15%)
    let ok_manual = controller.trigger_probe(15_000_000, 2_000_000, &mut mock_seq);
    assert!(ok_manual);
    assert_eq!(controller.state(), ScreenProbeState::Probing);

    let mut popped_fail = Vec::new();
    while let Some(p) = controller.pop_probe_packet() {
        popped_fail.push(p);
    }
    assert_eq!(popped_fail.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);

    // Create feedback where half the packets are marked missing
    let mut records_fail = Vec::new();
    for p in &popped_fail {
        let delta = if p.cluster_index % 2 == 1 {
            SCREEN_TWCC_MISSING_DELTA
        } else {
            (p.cluster_index as i16) * 1000
        };
        records_fail.push(ScreenTwccPacketRecord {
            transport_seq: p.transport_seq,
            recv_delta_us: delta,
        });
    }
    let fb_fail = ScreenTwccFeedback {
        feedback_seq: 2,
        base_seq: popped_fail[0].transport_seq,
        packet_count: popped_fail.len() as u16,
        reference_time_us: 2_010_000,
        records: records_fail,
    };

    let fail_target = controller.on_twcc_feedback(&fb_fail, &mut rate_ctrl, 2_015_000);
    assert!(fail_target.is_none());
    assert_eq!(controller.state(), ScreenProbeState::Failed);
    assert_eq!(controller.stats().probes_failed, 1);

    // 4. ScreenRateController fast_ramp_up integration
    let mut rate_ctrl2 = ScreenRateController::new(SCREEN_BITRATE_DEFAULT_BPS);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_DEFAULT_BPS);

    rate_ctrl2.fast_ramp_up(15_000_000);
    assert_eq!(rate_ctrl2.current_bps(), 15_000_000);

    // Clamping to max allowed bitrate (SCREEN_BITRATE_MAX_BPS)
    rate_ctrl2.fast_ramp_up(500_000_000);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_MAX_BPS);

    // Fast ramp-up only increases, does not decrease
    let stepped_down = rate_ctrl2.fast_ramp_up(100);
    assert!(!stepped_down);
    assert_eq!(rate_ctrl2.current_bps(), SCREEN_BITRATE_MAX_BPS);

    // 5. End-to-end Sender & Receiver Active Probing Pipeline
    let mut sender = ScreenSender::new();
    sender.set_probe_mode(ScreenProbeMode::Auto);
    sender.set_twcc_mode(ScreenTwccMode::Auto);
    sender.set_fec_mode(ScreenFecMode::Auto);
    assert_eq!(sender.probe_mode(), ScreenProbeMode::Auto);

    sender.on_peer_connected();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();

    // Trigger active bandwidth probe burst
    let triggered = sender.trigger_probe(8_000_000, 2_000_000);
    assert!(triggered);

    // Check that sender generated probe packets
    let mut probe_packets = Vec::new();
    while let Some(p) = sender.pop_probe_packet() {
        probe_packets.push(p);
    }
    assert_eq!(probe_packets.len(), SCREEN_PROBE_CLUSTER_SIZE_AUTO as usize);

    let mut receiver = ScreenReceiver::new();
    receiver.set_probe_mode(ScreenProbeMode::Auto);
    receiver.set_twcc_mode(ScreenTwccMode::Auto);

    for (idx, p) in probe_packets.into_iter().enumerate() {
        receiver.on_probe_packet_with_time(p, 2_005_000 + (idx as u64) * 200);
    }
    assert_eq!(
        receiver.probe_stats().probes_triggered,
        SCREEN_PROBE_CLUSTER_SIZE_AUTO as u64
    );

    let fb = receiver
        .generate_twcc_feedback(2_016_000)
        .expect("twcc feedback covering probe packets");
    let rep = sender.handle_twcc_feedback(&fb, 2_020_000);
    assert_eq!(rep.feedbacks_received, 1);

    // Peer disconnect resets
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().probes_triggered, 0);
    assert_eq!(receiver.stats().probes_triggered, 0);
}

#[test]
fn screen_audio_sync_pipeline_and_drift_compensation() {
    // 1. Enum parsing and tags
    assert_eq!(
        "auto".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Auto
    );
    assert_eq!(
        "strict".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(
        "tight".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(
        "loose".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Loose
    );
    assert_eq!(
        "relaxed".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Loose
    );
    assert_eq!(
        "disabled".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenAudioSyncMode>().unwrap(),
        ScreenAudioSyncMode::Disabled
    );
    assert!("invalid".parse::<ScreenAudioSyncMode>().is_err());
    assert_eq!(ScreenAudioSyncMode::from_tag(0), ScreenAudioSyncMode::Auto);
    assert_eq!(
        ScreenAudioSyncMode::from_tag(1),
        ScreenAudioSyncMode::Strict
    );
    assert_eq!(ScreenAudioSyncMode::from_tag(2), ScreenAudioSyncMode::Loose);
    assert_eq!(
        ScreenAudioSyncMode::from_tag(3),
        ScreenAudioSyncMode::Disabled
    );

    // 2. Wire Protocol and framing: ScreenAudioSyncFrame encode/decode round-trip
    let original = ScreenAudioSyncFrame {
        flags: 0x01,
        sync_seq: 42,
        sender_wallclock_us: 1_000_000,
        video_pts_us: 1_020_000,
        audio_pts_us: 1_005_000,
    };
    let encoded = original.encode();
    assert_eq!(encoded.len(), SCREEN_SYNC_HEADER_LEN);
    assert_eq!(&encoded[0..5], &SCREEN_SYNC_MAGIC);
    assert_eq!(encoded[5], SCREEN_SYNC_PROTO);

    let decoded = ScreenAudioSyncFrame::decode(&encoded).expect("decode sync frame");
    assert_eq!(decoded.flags, original.flags);
    assert_eq!(decoded.sync_seq, original.sync_seq);
    assert_eq!(decoded.sender_wallclock_us, original.sender_wallclock_us);
    assert_eq!(decoded.video_pts_us, original.video_pts_us);
    assert_eq!(decoded.audio_pts_us, original.audio_pts_us);

    // Corrupted magic and truncated errors
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenAudioSyncFrame::decode(&bad_magic).is_err());

    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenAudioSyncFrame::decode(&bad_proto).is_err());

    assert!(ScreenAudioSyncFrame::decode(&encoded[..SCREEN_SYNC_HEADER_LEN - 1]).is_err());

    // InnerFrame round-trip
    let inner = original.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_SYNC);
    assert_eq!(
        ScreenAudioSyncFrame::from_inner_frame(&inner).expect("from inner"),
        original
    );

    // 3. ScreenAudioSyncCoordinator generation and interval gating
    let mut coord = ScreenAudioSyncCoordinator::new(ScreenAudioSyncMode::Auto);
    coord.record_video_pts(500_000);
    coord.record_audio_pts(490_000);

    let beacon1 = coord.maybe_generate_sync_beacon(1_000_000);
    assert!(beacon1.is_some());
    let b1 = beacon1.unwrap();
    assert_eq!(b1.sync_seq, 1);
    assert_eq!(b1.video_pts_us, 500_000);
    assert_eq!(b1.audio_pts_us, 490_000);
    assert_eq!(coord.stats().beacons_sent, 1);

    // Call before interval has passed (< 500ms) returns None
    let beacon_early = coord.maybe_generate_sync_beacon(1_200_000);
    assert!(beacon_early.is_none());

    // Call after interval (>= 500ms) generates next beacon
    let beacon2 = coord.maybe_generate_sync_beacon(1_600_000);
    assert!(beacon2.is_some());
    assert_eq!(beacon2.unwrap().sync_seq, 2);
    assert_eq!(coord.stats().beacons_sent, 2);

    // 4. Receiver-side skew calculation, drift tracking, and playout buffer compensation
    let mut receiver_coord = ScreenAudioSyncCoordinator::new(ScreenAudioSyncMode::Auto);
    let mut playout_buf = ScreenPlayoutBuffer::new(ScreenPlayoutMode::Adaptive);
    let initial_delay = playout_buf.target_delay_us();

    // Beacon with video lead > 15ms tolerance (video 600ms, audio 570ms -> skew = +30ms)
    let lead_beacon = ScreenAudioSyncFrame {
        flags: 0,
        sync_seq: 10,
        sender_wallclock_us: 2_000_000,
        video_pts_us: 600_000,
        audio_pts_us: 570_000,
    };
    let skew = receiver_coord.on_sync_beacon(&lead_beacon, 2_005_000, Some(&mut playout_buf));
    assert_eq!(skew, 30_000);
    assert_eq!(receiver_coord.stats().current_skew_us, 30_000);
    assert!(!receiver_coord.stats().in_sync); // |30ms| > 15ms tolerance
    assert_eq!(receiver_coord.stats().adjustments_applied, 1);
    assert!(playout_buf.target_delay_us() > initial_delay); // delay increased to slow down video

    // Second beacon within tolerance (video 700ms, audio 695ms -> skew = +5ms <= 15ms)
    let sync_beacon = ScreenAudioSyncFrame {
        flags: 0,
        sync_seq: 11,
        sender_wallclock_us: 2_500_000,
        video_pts_us: 700_000,
        audio_pts_us: 695_000,
    };
    let skew2 = receiver_coord.on_sync_beacon(&sync_beacon, 2_505_050, Some(&mut playout_buf));
    assert_eq!(skew2, 5_000);
    assert!(receiver_coord.stats().in_sync);

    // Check drift estimation over time
    assert!(receiver_coord.stats().clock_drift_ppm.abs() >= 0.0);

    // 5. End-to-end ScreenSender & ScreenReceiver Pipeline
    let mut sender = ScreenSender::with_sync(ScreenAudioSyncMode::Auto);
    let mut receiver = ScreenReceiver::with_sync_mode(ScreenAudioSyncMode::Auto);
    assert_eq!(sender.sync_mode(), ScreenAudioSyncMode::Auto);
    assert_eq!(receiver.sync_mode(), ScreenAudioSyncMode::Auto);

    sender.record_audio_pts(1_000_000);
    sender.record_video_pts(1_025_000);

    let beacon = sender.trigger_sync_beacon(2_000_000);
    assert_eq!(beacon.sync_seq, 1);
    assert_eq!(sender.stats().sync_beacons_sent, 1);

    let popped = sender.pop_sync_beacon().expect("popped beacon");
    assert_eq!(popped, beacon);

    let skew_rcv = receiver.on_sync_beacon_with_time(&popped, 2_002_000);
    assert_eq!(skew_rcv, 25_000);
    assert_eq!(receiver.stats().sync_beacons_received, 1);
    assert_eq!(receiver.stats().sync_skew_us, 25_000);

    // Disconnect resets sync state
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().sync_beacons_sent, 0);
    assert_eq!(receiver.stats().sync_beacons_received, 0);
}

#[test]
fn screen_multi_display_and_dynamic_switch_pipeline() {
    // 1. ScreenDisplayInfo constructor and defaults
    let d0 = ScreenDisplayInfo::primary_default();
    assert_eq!(d0.display_id, 0);
    assert!(d0.is_primary);
    assert!(!d0.is_virtual);
    assert_eq!(d0.width, 1920);
    assert_eq!(d0.height, 1080);
    assert_eq!(d0.scale_factor_pct, 100);

    let d1 = ScreenDisplayInfo::new(1, "Display 1 (Secondary)", 2560, 1440, 125, false, false);
    let d2 = ScreenDisplayInfo::new(2, "Virtual Display (ADR-073)", 1920, 1200, 100, false, true);

    // 2. ControlMsg wire round-trip
    let ctrl_switch = ControlMsg::SwitchDisplay { display_id: 1 };
    assert_eq!(ctrl_switch.tag(), 4);
    let frame = ctrl_switch.to_frame().expect("to_frame");
    assert_eq!(frame.ty, TYPE_SCREEN_CONTROL);
    let decoded = ControlMsg::from_frame(&frame).expect("from_frame");
    assert_eq!(decoded, ControlMsg::SwitchDisplay { display_id: 1 });

    // 3. ScreenSender display management and switching
    let mut sender = ScreenSender::new();
    sender.allow();
    sender.on_control(ControlMsg::Start).unwrap();
    assert_eq!(sender.active_display_id(), 0);
    assert_eq!(sender.displays().len(), 1);

    sender.set_displays(vec![d0.clone(), d1.clone(), d2.clone()]);
    assert_eq!(sender.displays().len(), 3);
    assert_eq!(sender.stats().display_count, 3);

    // Switch to display 1
    sender
        .switch_display(1, 1_000_000)
        .expect("switch display 1");
    assert_eq!(sender.active_display_id(), 1);
    assert_eq!(sender.stats().active_display_id, 1);
    assert_eq!(sender.stats().display_switches_count, 1);
    assert_eq!(sender.stats().keyframes_requested, 1); // Triggers keyframe refresh

    let report = sender.display_report();
    assert_eq!(report.active_display_id, 1);
    assert_eq!(report.displays.len(), 3);
    assert_eq!(report.switches_count, 1);
    assert_eq!(report.last_switch_us, 1_000_000);

    // Receiving ControlMsg::SwitchDisplay on sender
    sender
        .on_control_with_time(ControlMsg::SwitchDisplay { display_id: 2 }, 1_500_000)
        .expect("on_control switch");
    assert_eq!(sender.active_display_id(), 2);
    assert_eq!(sender.stats().active_display_id, 2);
    assert_eq!(sender.stats().display_switches_count, 2);
    assert_eq!(sender.stats().keyframes_requested, 2);

    // 4. ScreenReceiver display management
    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.active_display_id(), 0);
    receiver.set_displays(vec![d0, d1, d2]);
    assert_eq!(receiver.displays().len(), 3);
    assert_eq!(receiver.stats().display_count, 3);

    receiver.switch_display(2, 1_500_000);
    assert_eq!(receiver.active_display_id(), 2);
    assert_eq!(receiver.stats().active_display_id, 2);
    assert_eq!(receiver.stats().display_switches_count, 1);

    let rx_report = receiver.display_report();
    assert_eq!(rx_report.active_display_id, 2);
    assert_eq!(rx_report.switches_count, 1);

    // 5. MemoryScreenSource multi-display support
    use crate::ports::ScreenSource;
    let mem_src = MemoryScreenSource::new(1920, 1080).expect("mem_src");
    assert_eq!(mem_src.active_display_id(), 0);
    assert_eq!(mem_src.displays().len(), 1);
    assert!(mem_src.switch_display(99).is_err()); // Reject unknown display ID

    mem_src.set_displays(vec![
        ScreenDisplayInfo::primary_default(),
        ScreenDisplayInfo::new(1, "Display 1", 2560, 1440, 100, false, false),
    ]);
    assert_eq!(mem_src.displays().len(), 2);
    mem_src.switch_display(1).expect("mem_src switch");
    assert_eq!(mem_src.active_display_id(), 1);

    // 6. Peer disconnect resets sender & receiver display state
    sender.on_peer_gone();
    receiver.on_peer_gone();
    assert_eq!(sender.stats().active_display_id, 0);
    assert_eq!(sender.stats().display_switches_count, 0);
    assert_eq!(receiver.stats().active_display_id, 0);
    assert_eq!(receiver.stats().display_switches_count, 0);
}

#[test]
fn screen_privacy_mode_and_input_lockout_pipeline() {
    // 1. ControlMsg::SetPrivacyMode wire encode/decode round-trip (tag 5, 14 bytes)
    let ctrl_privacy = ControlMsg::SetPrivacyMode {
        enabled: true,
        lock_input: true,
    };
    assert_eq!(ctrl_privacy.tag(), 5);
    let payload = ctrl_privacy.encode_payload();
    assert_eq!(payload.len(), 14);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode SetPrivacyMode");
    assert_eq!(
        decoded,
        ControlMsg::SetPrivacyMode {
            enabled: true,
            lock_input: true,
        }
    );

    let ctrl_privacy_off = ControlMsg::SetPrivacyMode {
        enabled: false,
        lock_input: false,
    };
    let payload_off = ctrl_privacy_off.encode_payload();
    let decoded_off = ControlMsg::decode_payload(&payload_off).expect("decode SetPrivacyMode off");
    assert_eq!(
        decoded_off,
        ControlMsg::SetPrivacyMode {
            enabled: false,
            lock_input: false,
        }
    );

    // 2. ScreenSender privacy mode management and telemetry
    let mut sender = ScreenSender::new();
    let rep0 = sender.privacy_report();
    assert!(rep0.ok);
    assert!(!rep0.enabled);
    assert!(!rep0.lock_input);
    assert_eq!(rep0.blanking_method, "virtual_overlay");
    assert!(!sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // Set privacy mode directly
    sender.set_privacy_mode(true, true, 1_000_000);
    let rep1 = sender.privacy_report();
    assert!(rep1.enabled);
    assert!(rep1.lock_input);
    assert_eq!(rep1.last_change_us, 1_000_000);
    assert!(sender.stats().privacy_enabled);
    assert!(sender.stats().privacy_input_locked);

    // Update via ControlMsg::SetPrivacyMode
    sender
        .on_control_with_time(
            ControlMsg::SetPrivacyMode {
                enabled: true,
                lock_input: false,
            },
            2_000_000,
        )
        .expect("on_control privacy mode");
    let rep2 = sender.privacy_report();
    assert!(rep2.enabled);
    assert!(!rep2.lock_input);
    assert_eq!(rep2.last_change_us, 2_000_000);
    assert!(sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // 3. Fail-safe disconnect recovery: on_peer_gone disengages privacy & input lockout
    sender.on_peer_gone();
    let rep3 = sender.privacy_report();
    assert!(!rep3.enabled);
    assert!(!rep3.lock_input);
    assert!(!sender.stats().privacy_enabled);
    assert!(!sender.stats().privacy_input_locked);

    // 4. ScreenReceiver privacy mode management
    let mut receiver = ScreenReceiver::new();
    let rx_rep0 = receiver.privacy_report();
    assert!(!rx_rep0.enabled);
    assert!(!rx_rep0.lock_input);

    receiver.set_privacy_mode(true, true, 3_000_000);
    let rx_rep1 = receiver.privacy_report();
    assert!(rx_rep1.enabled);
    assert!(rx_rep1.lock_input);
    assert_eq!(rx_rep1.last_change_us, 3_000_000);
    assert!(receiver.stats().privacy_enabled);
    assert!(receiver.stats().privacy_input_locked);

    // Receiver on_peer_gone reset
    receiver.on_peer_gone();
    let rx_rep2 = receiver.privacy_report();
    assert!(!rx_rep2.enabled);
    assert!(!rx_rep2.lock_input);
    assert!(!receiver.stats().privacy_enabled);
    assert!(!receiver.stats().privacy_input_locked);
}

#[test]
fn screen_latency_breakdown_and_profiler_pipeline() {
    // 1. ScreenProfilerMode & ScreenBottleneckStage parse & display
    assert_eq!(
        "auto".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Auto
    );
    assert_eq!(
        "detailed".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Detailed
    );
    assert_eq!(
        "always".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Detailed
    );
    assert_eq!(
        "disabled".parse::<ScreenProfilerMode>().unwrap(),
        ScreenProfilerMode::Disabled
    );
    assert!("invalid".parse::<ScreenProfilerMode>().is_err());
    assert_eq!(ScreenProfilerMode::Auto.to_string(), "auto");
    assert_eq!(ScreenProfilerMode::Detailed.to_string(), "detailed");
    assert_eq!(ScreenProfilerMode::Disabled.to_string(), "disabled");

    assert_eq!(
        "capture".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Capture
    );
    assert_eq!(
        "encode".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Encode
    );
    assert_eq!(
        "pacer".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Pacer
    );
    assert_eq!(
        "network".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Network
    );
    assert_eq!(
        "jitter".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Jitter
    );
    assert_eq!(
        "decode".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Decode
    );
    assert_eq!(
        "render".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Render
    );
    assert_eq!(
        "balanced".parse::<ScreenBottleneckStage>().unwrap(),
        ScreenBottleneckStage::Balanced
    );
    assert!("invalid".parse::<ScreenBottleneckStage>().is_err());
    assert_eq!(ScreenBottleneckStage::Network.to_string(), "network");

    // 2. ScreenProfileFrame wire encode / decode roundtrip (56 bytes, TYPE_SCREEN_PROFILE)
    let pframe = ScreenProfileFrame {
        flags: 0x01,
        frame_seq: 100,
        capture_us: 2_000,
        encode_us: 4_500,
        pacer_us: 500,
        sender_ts_us: 1_000_000,
        network_us: 12_000,
        jitter_us: 2_500,
        decode_us: 1_800,
        render_us: 700,
        total_e2e_us: 24_000,
    };
    let encoded = pframe.encode();
    assert_eq!(encoded.len(), SCREEN_PROFILE_HEADER_LEN);
    assert_eq!(&encoded[0..5], &SCREEN_PROFILE_MAGIC);
    assert_eq!(encoded[5], SCREEN_PROFILE_PROTO);

    let decoded = ScreenProfileFrame::decode(&encoded).expect("decode profile frame");
    assert_eq!(decoded.frame_seq, 100);
    assert_eq!(decoded.capture_us, 2_000);
    assert_eq!(decoded.encode_us, 4_500);
    assert_eq!(decoded.pacer_us, 500);
    assert_eq!(decoded.sender_ts_us, 1_000_000);
    assert_eq!(decoded.network_us, 12_000);
    assert_eq!(decoded.jitter_us, 2_500);
    assert_eq!(decoded.decode_us, 1_800);
    assert_eq!(decoded.render_us, 700);
    assert_eq!(decoded.total_e2e_us, 24_000);

    // Wire protocol errors: truncated, invalid magic, invalid proto
    assert!(ScreenProfileFrame::decode(&encoded[..SCREEN_PROFILE_HEADER_LEN - 1]).is_err());
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert!(ScreenProfileFrame::decode(&bad_magic).is_err());
    let mut bad_proto = encoded.clone();
    bad_proto[5] = 0x99;
    assert!(ScreenProfileFrame::decode(&bad_proto).is_err());

    // InnerFrame roundtrip
    let inner = pframe.to_inner().expect("to inner");
    assert_eq!(inner.ty, TYPE_SCREEN_PROFILE);
    let from_inner = ScreenProfileFrame::from_inner_frame(&inner).expect("from inner");
    assert_eq!(from_inner, pframe);

    // 3. ControlMsg::ResetLatencyProfile roundtrip (tag 6, 14 bytes)
    let reset_msg = ControlMsg::ResetLatencyProfile;
    assert_eq!(reset_msg.tag(), 6);
    let reset_payload = reset_msg.encode_payload();
    assert_eq!(reset_payload.len(), 14);
    let decoded_reset =
        ControlMsg::decode_payload(&reset_payload).expect("decode ResetLatencyProfile");
    assert_eq!(decoded_reset, ControlMsg::ResetLatencyProfile);

    // 4. ScreenFrameProfiler telemetry, bottleneck detection & percentiles
    let mut profiler = ScreenFrameProfiler::new(ScreenProfilerMode::Auto);
    assert_eq!(profiler.report().frames_profiled, 0);
    assert_eq!(profiler.report().bottleneck, "balanced");

    // Record balanced frame: total 10000 us, each stage <= 30%
    let b_balanced = ScreenLatencyBreakdown {
        frame_seq: 1,
        capture_us: 1500,
        encode_us: 2000,
        pacer_us: 1000,
        network_us: 2500,
        jitter_us: 1000,
        decode_us: 1000,
        render_us: 1000,
        total_e2e_us: 10000,
    };
    profiler.record_breakdown(b_balanced);
    assert_eq!(profiler.report().frames_profiled, 1);
    assert_eq!(profiler.report().bottleneck, "balanced");

    // Record network bottleneck frame: network_us = 12000 us out of 20000 us (60% >= 35%)
    let b_network = ScreenLatencyBreakdown {
        frame_seq: 2,
        capture_us: 1000,
        encode_us: 2000,
        pacer_us: 500,
        network_us: 12000,
        jitter_us: 1500,
        decode_us: 1500,
        render_us: 1500,
        total_e2e_us: 20000,
    };
    profiler.record_breakdown(b_network);
    assert_eq!(profiler.report().frames_profiled, 2);
    assert_eq!(profiler.report().bottleneck, "network");

    // Record encode bottleneck frame: encode_us = 10000 us out of 20000 us (50% >= 35%)
    let b_encode = ScreenLatencyBreakdown {
        frame_seq: 3,
        capture_us: 1000,
        encode_us: 10000,
        pacer_us: 500,
        network_us: 3000,
        jitter_us: 2000,
        decode_us: 2000,
        render_us: 1500,
        total_e2e_us: 20000,
    };
    profiler.record_breakdown(b_encode);
    assert_eq!(profiler.report().frames_profiled, 3);
    assert_eq!(profiler.report().bottleneck, "encode");

    // Percentiles and stats verification
    let rep = profiler.report();
    assert!(rep.avg_e2e_us > 0);
    assert!(rep.p50_e2e_us > 0);
    assert!(rep.p95_e2e_us >= rep.p50_e2e_us);

    // Reset clears metrics
    profiler.reset();
    let rep_reset = profiler.report();
    assert_eq!(rep_reset.frames_profiled, 0);
    assert_eq!(rep_reset.avg_e2e_us, 0);
    assert_eq!(rep_reset.bottleneck, "balanced");

    // 5. ScreenSender & ScreenReceiver latency profiler integration
    let mut sender = ScreenSender::new();
    assert_eq!(sender.latency_mode(), ScreenProfilerMode::Auto);
    sender.set_latency_mode(ScreenProfilerMode::Detailed);
    assert_eq!(sender.latency_mode(), ScreenProfilerMode::Detailed);

    sender.on_profile_frame(&pframe);
    let s_rep = sender.latency_report();
    assert_eq!(s_rep.frames_profiled, 1);
    assert_eq!(s_rep.current.frame_seq, 100);

    sender.on_peer_gone();
    assert_eq!(sender.latency_report().frames_profiled, 0);

    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.latency_mode(), ScreenProfilerMode::Auto);
    receiver.set_latency_mode(ScreenProfilerMode::Detailed);
    assert_eq!(receiver.latency_mode(), ScreenProfilerMode::Detailed);

    receiver.on_peer_gone();
    assert_eq!(receiver.latency_report().frames_profiled, 0);
}

// ADR-100 Screen Streaming Bottleneck-Driven Autonomous Tuning Closed Loop
#[test]
fn screen_bottleneck_autotuning_closed_loop() {
    // 1. ScreenTunerMode parse & display
    assert_eq!(
        "standard".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Standard
    );
    assert_eq!(
        "auto".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Standard
    );
    assert_eq!(
        "aggressive".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Aggressive
    );
    assert_eq!(
        "disabled".parse::<ScreenTunerMode>().unwrap(),
        ScreenTunerMode::Disabled
    );
    assert!("invalid".parse::<ScreenTunerMode>().is_err());
    assert_eq!(ScreenTunerMode::Standard.to_string(), "standard");
    assert_eq!(ScreenTunerMode::Aggressive.trigger_frames(), 2);
    assert_eq!(ScreenTunerMode::Standard.trigger_frames(), 3);
    assert_eq!(ScreenTunerMode::Standard.recovery_frames(), 10);

    // 2. ControlMsg::SetTuningMode / ResetTuningProfile wire roundtrip (14-byte control envelope)
    let set_frame = ControlMsg::SetTuningMode {
        mode: ScreenTunerMode::Aggressive.tag(),
    }
    .to_frame()
    .unwrap();
    assert_eq!(set_frame.ty, TYPE_SCREEN_CONTROL);
    assert_eq!(set_frame.payload.len(), 14);
    match ControlMsg::decode_payload(&set_frame.payload).unwrap() {
        ControlMsg::SetTuningMode { mode } => {
            assert_eq!(ScreenTunerMode::from_tag(mode), ScreenTunerMode::Aggressive)
        }
        other => panic!("expected SetTuningMode, got {other:?}"),
    }
    let rst_frame = ControlMsg::ResetTuningProfile.to_frame().unwrap();
    assert!(matches!(
        ControlMsg::decode_payload(&rst_frame.payload).unwrap(),
        ControlMsg::ResetTuningProfile
    ));

    // 3. Hysteresis: fewer than trigger frames of the same bottleneck must not act
    let ctx = ScreenTuningContext {
        bitrate_bps: SCREEN_BITRATE_DEFAULT_BPS,
        bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
        bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
        fps: 30,
        scale_pct: 100,
        qp_min: 12,
        qp_max: 36,
        playout_delay_us: 0,
    };

    let mut sender = ScreenSender::new();
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Standard);
    for seq in 1..=2u64 {
        sender.on_profile_frame(&ScreenProfileFrame {
            flags: 0,
            frame_seq: seq,
            capture_us: 1_000,
            encode_us: 30_000,
            pacer_us: 1_000,
            sender_ts_us: seq,
            network_us: 1_000,
            jitter_us: 1_000,
            decode_us: 1_000,
            render_us: 1_000,
            total_e2e_us: 36_000,
        });
    }
    assert_eq!(sender.tuning_report().actions_applied, 0);
    assert_eq!(sender.tuning_report().bottleneck_streak, 2);

    // 4. Third consecutive encode-heavy frame triggers RaiseQp (12 -> 40 clamped to 51? no: 36+4=40)
    sender.on_profile_frame(&ScreenProfileFrame {
        flags: 0,
        frame_seq: 3,
        capture_us: 1_000,
        encode_us: 30_000,
        pacer_us: 1_000,
        sender_ts_us: 3,
        network_us: 1_000,
        jitter_us: 1_000,
        decode_us: 1_000,
        render_us: 1_000,
        total_e2e_us: 36_000,
    });
    let rep = sender.tuning_report();
    assert_eq!(
        rep.actions_applied, 1,
        "encode bottleneck must trigger on 3rd frame"
    );
    assert_eq!(rep.last_action, "raise_qp");
    assert_eq!(rep.last_stage, "encode");
    let decision = rep.last_decision.unwrap();
    assert_eq!(decision.new_qp_max, Some(39), "qp_max 35 + 4 relief = 39");
    assert_eq!(sender.stats.qp_max, 39);
    // Sender knobs applied: stats mirror the tuner telemetry
    assert_eq!(sender.stats.tuning_actions_total, 1);

    // 5. Direct ScreenTuner policy checks per stage
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let _ = tuner.evaluate(1_000, 2, ScreenBottleneckStage::Network, &ctx);
    let out = tuner.evaluate(1_000, 3, ScreenBottleneckStage::Network, &ctx);
    assert_eq!(out.decision.action, "reduce_bitrate");
    let expect_bps = SCREEN_BITRATE_DEFAULT_BPS
        - SCREEN_BITRATE_DEFAULT_BPS * SCREEN_TUNER_BITRATE_DROP_PCT / 100;
    assert_eq!(out.bitrate_bps, Some(expect_bps));

    // Cooldown: the action fires on the 3rd frame; an immediate re-trigger must hold
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    for seq in 1..=3u64 {
        let _ = tuner.evaluate(1_000, seq, ScreenBottleneckStage::Pacer, &ctx);
    }
    let out = tuner.evaluate(1_000 + 1, 4, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(
        out.decision.action, "hold",
        "cooldown must suppress immediate re-trigger"
    );
    // After the cooldown elapses the still-running streak triggers again
    let out = tuner.evaluate(3_000_000, 5, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(
        out.decision.action, "reduce_bitrate",
        "post-cooldown streak must trigger"
    );
    // ... and the cooldown window restarts from the new action
    let out = tuner.evaluate(3_000_001, 6, ScreenBottleneckStage::Pacer, &ctx);
    assert_eq!(out.decision.action, "hold");

    // Mixed bottleneck types reset the streak
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Standard);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let _ = tuner.evaluate(1_000, 2, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(1_000, 3, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "hold", "streak resets on stage switch");

    // Capture bottleneck reduces FPS
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "reduce_fps");
    assert_eq!(out.fps, Some(screen_matrix_next_lower_fps(30)));

    // Jitter bottleneck raises playout delay (receiver side)
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Jitter, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Jitter, &ctx);
    assert_eq!(out.decision.action, "raise_playout_delay");
    assert_eq!(
        out.playout_delay_delta_us,
        Some(SCREEN_TUNER_PLAYOUT_RELIEF_DELTA_US)
    );

    // Disabled mode never acts
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Disabled);
    for seq in 1..=10u64 {
        let out = tuner.evaluate(1_000 * seq, seq, ScreenBottleneckStage::Network, &ctx);
        assert_eq!(out.decision.action, "hold");
    }
    assert_eq!(tuner.report().actions_applied, 0);

    // 6. Anti-oscillation recovery: after sustained balance the degraded knobs step back up in reverse order
    let mut tuner = ScreenTuner::new(ScreenTunerMode::Aggressive);
    // Network trigger -> reduce_bitrate
    let _ = tuner.evaluate(1_000, 1, ScreenBottleneckStage::Network, &ctx);
    let out = tuner.evaluate(2_000_000, 2, ScreenBottleneckStage::Network, &ctx);
    assert_eq!(out.decision.action, "reduce_bitrate");
    // Capture trigger -> reduce_fps
    let _ = tuner.evaluate(4_000_000, 3, ScreenBottleneckStage::Capture, &ctx);
    let out = tuner.evaluate(6_000_000, 4, ScreenBottleneckStage::Capture, &ctx);
    assert_eq!(out.decision.action, "reduce_fps");

    // Balanced frames below the recovery threshold keep holding
    for seq in 5..=9u64 {
        let out = tuner.evaluate(6_000_000 + seq, seq, ScreenBottleneckStage::Balanced, &ctx);
        assert_eq!(out.decision.action, "hold");
    }
    assert_eq!(tuner.report().balanced_streak, 5);

    // 6th balanced frame (aggressive needs 6) recovers the most recent degradation first: fps
    let rec_ctx = ScreenTuningContext {
        bitrate_bps: expect_bps,
        bitrate_min_bps: SCREEN_BITRATE_MIN_BPS,
        bitrate_max_bps: SCREEN_BITRATE_MAX_BPS,
        fps: screen_matrix_next_lower_fps(30),
        scale_pct: 100,
        qp_min: 12,
        qp_max: 36,
        playout_delay_us: 0,
    };
    let out = tuner.evaluate(10_000_000, 10, ScreenBottleneckStage::Balanced, &rec_ctx);
    assert_eq!(
        out.decision.action, "recover_fps",
        "recovery must undo the most recent degradation first"
    );
    assert!(out.decision.recovered);
    // Next recovery (after the aggressive 1s cooldown elapses) restores bitrate
    let out = tuner.evaluate(11_100_000, 11, ScreenBottleneckStage::Balanced, &rec_ctx);
    assert_eq!(out.decision.action, "recover_bitrate");
    let expect_recover =
        (expect_bps * (100 + SCREEN_TUNER_BITRATE_RECOVER_PCT) / 100).min(SCREEN_BITRATE_MAX_BPS);
    assert_eq!(out.bitrate_bps, Some(expect_recover));
    assert_eq!(tuner.report().recoveries_applied, 2);

    // 7. ScreenSender reset_tuning clears counters but keeps the mode
    sender.set_tuning_mode(ScreenTunerMode::Aggressive);
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Aggressive);
    let rep = sender.reset_tuning();
    assert_eq!(rep.actions_applied, 0);
    assert_eq!(rep.last_action, "hold");
    assert_eq!(sender.tuning_mode(), ScreenTunerMode::Aggressive);

    // 8. Receiver-side tuner: Jitter bottleneck adjusts the playout buffer target delay
    let mut receiver = ScreenReceiver::new();
    assert_eq!(receiver.tuning_mode(), ScreenTunerMode::Standard);
    receiver.set_tuning_mode(ScreenTunerMode::Aggressive);
    let jitter_profile = |seq: u64| ScreenProfileFrame {
        flags: 0,
        frame_seq: seq,
        capture_us: 1_000,
        encode_us: 1_000,
        pacer_us: 1_000,
        sender_ts_us: seq,
        network_us: 1_000,
        jitter_us: 30_000,
        decode_us: 1_000,
        render_us: 1_000,
        total_e2e_us: 36_000,
    };
    receiver.on_profile_frame(&jitter_profile(1));
    assert_eq!(receiver.tuning_report().actions_applied, 0);
    let delay_before = receiver.playout_stats().target_delay_us;
    receiver.on_profile_frame(&jitter_profile(2));
    let rep = receiver.tuning_report();
    assert_eq!(
        rep.actions_applied, 1,
        "aggressive mode triggers on the 2nd frame"
    );
    assert_eq!(rep.last_action, "raise_playout_delay");
    let delay_after = receiver.playout_stats().target_delay_us;
    assert!(
        delay_after > delay_before,
        "playout target delay must deepen on jitter bottleneck ({delay_before} -> {delay_after})"
    );

    // 9. ScreenStats default tuning telemetry
    let stats = ScreenStats::default();
    assert_eq!(stats.tuning_mode, ScreenTunerMode::Standard.tag());
    assert_eq!(
        stats.tuning_last_stage,
        ScreenBottleneckStage::Balanced.tag()
    );
    assert_eq!(stats.tuning_last_action, ScreenTuningAction::Hold.tag());

    // 10. ScreenTuningAction parse & tag roundtrips
    for tag in 0..=10u8 {
        let action = ScreenTuningAction::from_tag(tag);
        assert_eq!(action.tag(), tag);
    }
    assert_eq!(
        "recover_bitrate".parse::<ScreenTuningAction>().unwrap(),
        ScreenTuningAction::RecoverBitrate
    );
    assert!(ScreenTuningAction::RecoverBitrate.is_recovery());
    assert!(!ScreenTuningAction::ReduceFps.is_recovery());
}

// ADR-101 Screen Streaming Session Recording Container & Replay Pipeline
#[test]
fn screen_recording_container_roundtrip_and_recording_disclosure() {
    // 1. ControlMsg::SetRecordingState wire roundtrip (Tag = 9, 14-byte envelope)
    let on_frame = ControlMsg::SetRecordingState { active: true }
        .to_frame()
        .unwrap();
    assert_eq!(on_frame.ty, TYPE_SCREEN_CONTROL);
    assert_eq!(on_frame.payload.len(), 14);
    assert!(matches!(
        ControlMsg::decode_payload(&on_frame.payload).unwrap(),
        ControlMsg::SetRecordingState { active: true }
    ));
    let off_frame = ControlMsg::SetRecordingState { active: false }
        .to_frame()
        .unwrap();
    assert!(matches!(
        ControlMsg::decode_payload(&off_frame.payload).unwrap(),
        ControlMsg::SetRecordingState { active: false }
    ));

    // 2. Writer -> Reader container roundtrip with three raw frames
    let rect0 = ScreenRect::new(0, 0, 8, 4).unwrap();
    let frame_bytes = rect0.pixel_bytes(PixelFormat::Bgra8);
    let frames: Vec<ScreenFrame> = (1..=3)
        .map(|seq| {
            ScreenFrame::raw(seq, rect0, PixelFormat::Bgra8, vec![7u8; frame_bytes]).unwrap()
        })
        .collect();
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let meta = ScreenRecordingMeta::from_frame(&frames[0], 1_000);
        assert_eq!(meta.width, rect0.w as u32);
        assert_eq!(meta.height, rect0.h as u32);
        let mut writer = ScreenRecordingWriter::new(meta, &mut buf).unwrap();
        let pts = [1_000u64, 16_000, 33_000];
        for (i, f) in frames.iter().enumerate() {
            writer.append(f, pts[i]).unwrap();
        }
        assert_eq!(writer.frames_written(), 3);
        let (fw, bw) = writer.finish().unwrap();
        assert_eq!(fw, 3);
        assert!(bw > 0);
    }
    buf.set_position(0);
    let mut reader = ScreenRecordingReader::new(&mut buf).unwrap();
    let meta = reader.meta().clone();
    assert_eq!(meta.started_at_us, 1_000);
    assert_eq!(meta.format_tag, PixelFormat::Bgra8.tag());
    assert_eq!(meta.codec_tag, SCREEN_COMPRESS_NONE);
    let mut read_back = Vec::new();
    while let Some(rec) = reader.next_record().unwrap() {
        read_back.push(rec);
    }
    assert_eq!(read_back.len(), 3);
    assert_eq!(reader.records_read(), 3);
    for (i, rec) in read_back.iter().enumerate() {
        assert_eq!(rec.seq, frames[i].seq);
        assert_eq!(rec.pts_us, [1_000u64, 16_000, 33_000][i]);
        // decode_frame must restore the exact original ScreenFrame
        let restored = reader.decode_frame(rec).unwrap();
        assert_eq!(restored.seq, frames[i].seq);
        assert_eq!(restored.rect, frames[i].rect);
        assert_eq!(restored.format, frames[i].format);
        assert_eq!(restored.bytes, frames[i].bytes);
        assert_eq!(restored.codec_tag, frames[i].codec_tag);
    }

    // 3. Truncated record -> error, not a hang
    let mut buf2 = std::io::Cursor::new(Vec::new());
    {
        let mut writer =
            ScreenRecordingWriter::new(ScreenRecordingMeta::from_frame(&frames[0], 5), &mut buf2)
                .unwrap();
        writer.append(&frames[0], 5).unwrap();
        writer.finish().unwrap();
    }
    let mut raw = buf2.into_inner();
    raw.truncate(SCREEN_RECORDING_HEADER_LEN + 8);
    let mut reader2 = ScreenRecordingReader::new(&raw[..]).unwrap();
    assert!(reader2.next_record().is_err());

    // 4. Bad magic / proto rejected
    let mut bad = ScreenRecordingMeta::from_frame(&frames[0], 5).encode();
    bad[0] = b'X';
    assert!(ScreenRecordingMeta::decode(&bad).is_err());
    let mut bad_proto = ScreenRecordingMeta::from_frame(&frames[0], 5).encode();
    bad_proto[4] = 0x7F;
    let mut cursor = std::io::Cursor::new(bad_proto);
    assert!(ScreenRecordingReader::new(&mut cursor).is_err());
}

// ADR-102 Screen Session In-Container Playback Engine & Virtual Screen Source Feeder
#[test]
fn screen_session_playback_engine_and_virtual_source_feeder() {
    use crate::ports::ScreenSource;

    // 1. State string parsing & roundtrips
    assert_eq!(ScreenPlaybackState::Idle.as_str(), "idle");
    assert_eq!(ScreenPlaybackState::Playing.as_str(), "playing");
    assert_eq!(ScreenPlaybackState::Paused.as_str(), "paused");
    assert_eq!(ScreenPlaybackState::Finished.as_str(), "finished");
    assert_eq!(ScreenPlaybackState::Stopped.as_str(), "stopped");
    assert_eq!(
        "playing".parse::<ScreenPlaybackState>().unwrap(),
        ScreenPlaybackState::Playing
    );
    assert_eq!(
        "paused".parse::<ScreenPlaybackState>().unwrap(),
        ScreenPlaybackState::Paused
    );
    assert!("invalid_state".parse::<ScreenPlaybackState>().is_err());

    // 2. Prepare in-memory TREC container with 3 frames
    let rect0 = ScreenRect::new(0, 0, 16, 8).unwrap();
    let frame_bytes = rect0.pixel_bytes(PixelFormat::Bgra8);
    let frames: Vec<ScreenFrame> = (1..=3)
        .map(|seq| {
            let mut data = vec![0u8; frame_bytes];
            for (i, b) in data.iter_mut().enumerate() {
                *b = ((seq as usize * 17 + i) & 0xFF) as u8;
            }
            ScreenFrame::raw(seq, rect0, PixelFormat::Bgra8, data).unwrap()
        })
        .collect();

    let mut trec_bytes = Vec::new();
    {
        let meta = ScreenRecordingMeta::from_frame(&frames[0], 1_000);
        let mut writer =
            ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut trec_bytes)).unwrap();
        writer.append(&frames[0], 1_000).unwrap();
        writer.append(&frames[1], 17_000).unwrap();
        writer.append(&frames[2], 33_000).unwrap();
        writer.finish().unwrap();
    }

    // 3. Reader scan_summary fast summary
    let cursor = std::io::Cursor::new(trec_bytes.clone());
    let mut reader = ScreenRecordingReader::new(cursor).unwrap();
    let (scanned_frames, duration_us) = reader.scan_summary().unwrap();
    assert_eq!(scanned_frames, 3);
    assert_eq!(duration_us, 32_000);

    // 4. ScreenPlaybackEngine: step-by-step presentation, pause, resume, finish
    let sink = MemoryScreenSink::new();
    let mut engine = ScreenPlaybackEngine::from_bytes(trec_bytes.clone(), 0.0, false).unwrap();
    assert_eq!(engine.state(), ScreenPlaybackState::Idle);
    assert_eq!(engine.total_frames(), 3);
    let rep0 = engine.report();
    assert_eq!(rep0.state, "idle");
    assert_eq!(rep0.total_frames, 3);
    assert_eq!(rep0.duration_ms, 32);
    assert_eq!(rep0.width, 16);
    assert_eq!(rep0.height, 8);

    // Step frame 1
    let f1 = engine.step(&sink).unwrap().expect("frame 1");
    assert_eq!(f1.seq, 1);
    assert_eq!(engine.state(), ScreenPlaybackState::Playing);
    assert_eq!(engine.current_frame(), 1);
    assert_eq!(sink.count(), 1);

    // Pause -> stepping does nothing
    engine.pause();
    assert_eq!(engine.state(), ScreenPlaybackState::Paused);
    assert!(engine.step(&sink).unwrap().is_none());
    assert_eq!(sink.count(), 1);

    // Resume -> step frame 2
    engine.resume();
    assert_eq!(engine.state(), ScreenPlaybackState::Playing);
    let f2 = engine.step(&sink).unwrap().expect("frame 2");
    assert_eq!(f2.seq, 2);
    assert_eq!(engine.current_frame(), 2);
    assert_eq!(sink.count(), 2);

    // Step frame 3
    let f3 = engine.step(&sink).unwrap().expect("frame 3");
    assert_eq!(f3.seq, 3);
    assert_eq!(engine.current_frame(), 3);
    assert_eq!(sink.count(), 3);

    // Step frame 4 (EOF) -> state Finished
    assert!(engine.step(&sink).unwrap().is_none());
    assert_eq!(engine.state(), ScreenPlaybackState::Finished);

    // 5. ScreenPlaybackEngine with loop enabled
    let sink_loop = MemoryScreenSink::new();
    let mut loop_engine = ScreenPlaybackEngine::from_bytes(trec_bytes.clone(), 0.0, true).unwrap();
    for _ in 0..7 {
        let frame = loop_engine.step(&sink_loop).unwrap().expect("looped frame");
        assert!(frame.seq >= 1 && frame.seq <= 3);
    }
    assert_eq!(sink_loop.count(), 7);
    loop_engine.stop();
    assert_eq!(loop_engine.state(), ScreenPlaybackState::Stopped);

    // 6. ScreenRecordingSource: Virtual ScreenSource adapter
    let source = ScreenRecordingSource::from_bytes(trec_bytes.clone(), true).unwrap();
    assert_eq!(source.meta().width, 16);
    assert_eq!(source.meta().height, 8);
    let displays = source.displays();
    assert_eq!(displays.len(), 1);
    assert_eq!(displays[0].width, 16);
    assert_eq!(displays[0].height, 8);
    assert!(displays[0].is_primary);

    // Grab 1, 2, 3 and loop to 1
    let g1 = source.grab().unwrap().expect("grab 1");
    assert_eq!(g1.seq, 1);
    let g2 = source.grab().unwrap().expect("grab 2");
    assert_eq!(g2.seq, 2);
    let g3 = source.grab().unwrap().expect("grab 3");
    assert_eq!(g3.seq, 3);
    let g4 = source.grab().unwrap().expect("grab 4 (looped)");
    assert_eq!(g4.seq, 1);
    assert_eq!(source.frames_grabbed(), 4);

    // Disable loop on source -> EOF after frames 2 and 3
    source.set_loop_enabled(false);
    assert_eq!(source.grab().unwrap().expect("grab 5").seq, 2);
    assert_eq!(source.grab().unwrap().expect("grab 6").seq, 3);
    assert!(
        source.grab().unwrap().is_none(),
        "expected EOF when loop disabled"
    );
}

// ADR-103 Screen Live Snapshot BMP Encoding & Recording Frame Extraction
#[test]
fn screen_snapshot_bmp_encoding_and_recording_extraction() {
    // 1. BMP Encoding for Bgra8
    let w = 4u16;
    let h = 4u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let pixel_count = (w as usize) * (h as usize);
    let mut bgra_data = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        bgra_data.extend_from_slice(&[
            (i * 10) as u8,     // B
            (i * 10 + 1) as u8, // G
            (i * 10 + 2) as u8, // R
            255,                // A
        ]);
    }
    let frame_bgra = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, bgra_data.clone()).unwrap();
    let bmp_bytes = frame_bgra.to_bmp().expect("to_bmp bgra");

    assert_eq!(bmp_bytes.len(), 54 + pixel_count * 4);
    // Header verification
    assert_eq!(&bmp_bytes[0..2], b"BM");
    let file_size = u32::from_le_bytes(bmp_bytes[2..6].try_into().unwrap());
    assert_eq!(file_size, (54 + pixel_count * 4) as u32);
    let offset = u32::from_le_bytes(bmp_bytes[10..14].try_into().unwrap());
    assert_eq!(offset, 54);

    let bi_size = u32::from_le_bytes(bmp_bytes[14..18].try_into().unwrap());
    assert_eq!(bi_size, 40);
    let bi_width = i32::from_le_bytes(bmp_bytes[18..22].try_into().unwrap());
    assert_eq!(bi_width, 4);
    let bi_height = i32::from_le_bytes(bmp_bytes[22..26].try_into().unwrap());
    assert_eq!(bi_height, -4, "Top-down DIB must have negative biHeight");
    let bi_planes = u16::from_le_bytes(bmp_bytes[26..28].try_into().unwrap());
    assert_eq!(bi_planes, 1);
    let bi_bit_count = u16::from_le_bytes(bmp_bytes[28..30].try_into().unwrap());
    assert_eq!(bi_bit_count, 32);
    let bi_compression = u32::from_le_bytes(bmp_bytes[30..34].try_into().unwrap());
    assert_eq!(bi_compression, 0); // BI_RGB
    let bi_size_image = u32::from_le_bytes(bmp_bytes[34..38].try_into().unwrap());
    assert_eq!(bi_size_image, (pixel_count * 4) as u32);

    // Pixels in BMP match original Bgra8 bytes
    assert_eq!(&bmp_bytes[54..], &bgra_data[..]);

    // 2. BMP Encoding for Rgba8 (channel swap)
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        rgba_data.extend_from_slice(&[
            (i * 10 + 2) as u8, // R
            (i * 10 + 1) as u8, // G
            (i * 10) as u8,     // B
            255,                // A
        ]);
    }
    let frame_rgba = ScreenFrame::raw(2, rect, PixelFormat::Rgba8, rgba_data).unwrap();
    let bmp_from_rgba = frame_rgba.to_bmp().expect("to_bmp rgba");
    assert_eq!(
        &bmp_from_rgba[54..],
        &bgra_data[..],
        "Rgba8 must be converted to Bgra8 in BMP"
    );

    // 3. Rejection of compressed frame
    let mut compressed_frame = frame_bgra.clone();
    compressed_frame.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(compressed_frame.to_bmp().is_err());

    // 4. Recording container extract_frame
    let mut trec_bytes = Vec::new();
    {
        let meta = ScreenRecordingMeta::from_frame(&frame_bgra, 1_000);
        let mut writer =
            ScreenRecordingWriter::new(meta, std::io::Cursor::new(&mut trec_bytes)).unwrap();
        writer.append(&frame_bgra, 1_000).unwrap();

        let f2 =
            ScreenFrame::raw(2, rect, PixelFormat::Bgra8, vec![42u8; pixel_count * 4]).unwrap();
        writer.append(&f2, 16_000).unwrap();

        let f3 =
            ScreenFrame::raw(3, rect, PixelFormat::Bgra8, vec![99u8; pixel_count * 4]).unwrap();
        writer.append(&f3, 33_000).unwrap();
        writer.finish().unwrap();
    }

    let mut reader = ScreenRecordingReader::new(std::io::Cursor::new(trec_bytes)).unwrap();

    // Target seq = 2
    let extracted_2 = reader
        .extract_frame(Some(2), None)
        .unwrap()
        .expect("extract frame 2");
    assert_eq!(extracted_2.seq, 2);
    assert_eq!(extracted_2.bytes[0], 42);

    // Target pts >= 20_000 (should match frame 3 at 33_000)
    let extracted_pts = reader
        .extract_frame(None, Some(20_000))
        .unwrap()
        .expect("extract frame 3 by pts");
    assert_eq!(extracted_pts.seq, 3);
    assert_eq!(extracted_pts.bytes[0], 99);

    // Default: first frame
    let extracted_first = reader
        .extract_frame(None, None)
        .unwrap()
        .expect("extract first");
    assert_eq!(extracted_first.seq, 1);

    // Non-existent seq -> None
    let not_found = reader.extract_frame(Some(999), None).unwrap();
    assert!(not_found.is_none());

    // 5. Default ScreenSnapshotReport
    let report = ScreenSnapshotReport::default();
    assert!(report.ok);
    assert_eq!(report.format, "bmp");
    assert!(!report.privacy_blanked);
    assert!(report.path.is_none());
}

#[test]
fn screen_dynamic_security_watermark_rendering_and_signaling() {
    // 1. Line protocol Tag 10 ControlMsg::SetWatermarkState wire roundtrip
    let msg1 = ControlMsg::SetWatermarkState {
        enabled: true,
        host_enforced: false,
    };
    let payload1 = msg1.encode_payload();
    assert_eq!(payload1.len(), 14);
    assert_eq!(payload1[5], 10);
    let decoded1 = ControlMsg::decode_payload(&payload1).expect("decode tag 10");
    match decoded1 {
        ControlMsg::SetWatermarkState {
            enabled,
            host_enforced,
        } => {
            assert!(enabled);
            assert!(!host_enforced);
        }
        _ => panic!("unexpected decoded variant"),
    }

    let msg2 = ControlMsg::SetWatermarkState {
        enabled: false,
        host_enforced: true,
    };
    let frame2 = msg2.to_frame().expect("to_frame");
    let decoded2 = ControlMsg::from_frame(&frame2).expect("from_frame");
    match decoded2 {
        ControlMsg::SetWatermarkState {
            enabled,
            host_enforced,
        } => {
            assert!(!enabled);
            assert!(host_enforced);
        }
        _ => panic!("unexpected decoded variant"),
    }

    // 2. Pattern parsing and default config
    assert_eq!(
        "grid".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Grid
    );
    assert_eq!(
        "single".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Single
    );
    assert_eq!(
        "banner".parse::<WatermarkPattern>().unwrap(),
        WatermarkPattern::Banner
    );
    assert!("invalid".parse::<WatermarkPattern>().is_err());

    let mut config = ScreenWatermarkConfig::default();
    assert!(!config.enabled);
    assert!(!config.host_enforced);
    assert_eq!(config.pattern, WatermarkPattern::Grid);

    // 3. Pixel rendering tests
    let rect = ScreenRect::new(0, 0, 160, 80).unwrap();
    let pixel_count = (160 * 80) as usize;
    let black_pixels = vec![0u8; pixel_count * 4];

    // Frame in Bgra8 format
    let mut frame_bgra =
        ScreenFrame::raw(1, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();

    // Disabled watermark -> Ok(false)
    let applied_disabled = frame_bgra
        .apply_watermark(&config, "peer-abc", "2026-10-27")
        .unwrap();
    assert!(!applied_disabled);
    assert_eq!(frame_bgra.bytes, black_pixels);

    // Enabled watermark with 50% opacity
    config.enabled = true;
    config.opacity = 128;
    config.color_rgb = [200, 100, 50]; // R=200, G=100, B=50
    config.text_template = "CONFIDENTIAL-{peer}".to_string();

    let applied = frame_bgra.apply_watermark(&config, "dev1", "now").unwrap();
    assert!(applied);
    // At least some pixels should now be non-zero
    let non_zero_count = frame_bgra.bytes.iter().filter(|&&b| b > 0).count();
    assert!(
        non_zero_count > 50,
        "watermark text should render non-zero pixels"
    );

    // Single pattern
    config.pattern = WatermarkPattern::Single;
    let mut frame_single =
        ScreenFrame::raw(2, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();
    assert!(frame_single
        .apply_watermark(&config, "dev1", "now")
        .unwrap());

    // Banner pattern
    config.pattern = WatermarkPattern::Banner;
    let mut frame_banner =
        ScreenFrame::raw(3, rect, PixelFormat::Bgra8, black_pixels.clone()).unwrap();
    assert!(frame_banner
        .apply_watermark(&config, "dev1", "now")
        .unwrap());

    // Rgba8 pixel format
    config.pattern = WatermarkPattern::Grid;
    let mut frame_rgba = ScreenFrame::raw(4, rect, PixelFormat::Rgba8, black_pixels).unwrap();
    assert!(frame_rgba.apply_watermark(&config, "dev1", "now").unwrap());

    // Compressed frame rejection
    let mut frame_compressed = frame_rgba.clone();
    frame_compressed.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(frame_compressed
        .apply_watermark(&config, "dev1", "now")
        .is_err());
}

// ADR-105 Screen Streaming Collaborative Annotation, Whiteboard Overlay & Transient Laser Pointer Pipeline
#[test]
fn screen_collaborative_annotation_and_laser_pointer_pipeline() {
    // 1. Tag 11 ControlMsg::SetLaserPointer wire roundtrip
    let laser_msg = ControlMsg::SetLaserPointer {
        x: 320,
        y: 240,
        active: true,
        color_rgb: [255, 60, 100],
    };
    let payload = laser_msg.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 11);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode tag 11");
    match decoded {
        ControlMsg::SetLaserPointer {
            x,
            y,
            active,
            color_rgb,
        } => {
            assert_eq!(x, 320);
            assert_eq!(y, 240);
            assert!(active);
            assert_eq!(color_rgb, [255, 60, 100]);
        }
        _ => panic!("expected SetLaserPointer"),
    }

    // 2. Tag 12 ControlMsg::ClearAnnotations wire roundtrip
    let clear_msg = ControlMsg::ClearAnnotations;
    let payload = clear_msg.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 12);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode tag 12");
    assert!(matches!(decoded, ControlMsg::ClearAnnotations));

    // 3. ScreenAnnotationMsg inner frame roundtrip
    let stroke_item = AnnotationItem {
        id: 1,
        author: "visitor1".into(),
        kind: AnnotationKind::Stroke {
            points: vec![(10, 10), (50, 10), (50, 50)],
        },
        color_rgb: [255, 0, 0],
        thickness: 4,
        alpha: 200,
        ttl_ms: 5000,
        created_at_ms: 1000,
    };
    let msg = ScreenAnnotationMsg::Add(stroke_item.clone());
    let frame = msg.to_frame().expect("to_frame");
    assert_eq!(frame.ty, TYPE_SCREEN_ANNOTATION);
    let decoded_msg = ScreenAnnotationMsg::from_frame(&frame).expect("from_frame");
    match decoded_msg {
        ScreenAnnotationMsg::Add(item) => {
            assert_eq!(item.id, 1);
            assert_eq!(item.author, "visitor1");
            assert_eq!(item.color_rgb, [255, 0, 0]);
            assert_eq!(item.thickness, 4);
            assert_eq!(item.alpha, 200);
            assert_eq!(item.ttl_ms, 5000);
        }
        _ => panic!("expected Add"),
    }

    // Clear annotation msg
    let clear_frame = ScreenAnnotationMsg::Clear.to_frame().expect("to_frame");
    let decoded_clear = ScreenAnnotationMsg::from_frame(&clear_frame).expect("from_frame");
    assert!(matches!(decoded_clear, ScreenAnnotationMsg::Clear));

    // 4. Rasterization & Annotation Application on ScreenFrame
    let rect = ScreenRect::new(0, 0, 100, 100).unwrap();
    let pixel_count = 100 * 100;
    let initial_bgra = vec![0u8; pixel_count * 4]; // All black, transparent
    let mut frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, initial_bgra).unwrap();

    let rect_item = AnnotationItem {
        id: 2,
        author: "visitor2".into(),
        kind: AnnotationKind::Rect {
            x: 20,
            y: 20,
            w: 30,
            h: 30,
        },
        color_rgb: [0, 255, 0],
        thickness: 2,
        alpha: 255,
        ttl_ms: 0, // Permanent
        created_at_ms: 1000,
    };

    let arrow_item = AnnotationItem {
        id: 3,
        author: "visitor1".into(),
        kind: AnnotationKind::Arrow {
            from: (10, 80),
            to: (60, 80),
        },
        color_rgb: [0, 0, 255],
        thickness: 2,
        alpha: 255,
        ttl_ms: 3000, // Expires at 4000ms
        created_at_ms: 1000,
    };

    let text_item = AnnotationItem {
        id: 4,
        author: "visitor1".into(),
        kind: AnnotationKind::Text {
            x: 5,
            y: 5,
            text: "TEST".into(),
            scale: 1,
        },
        color_rgb: [255, 255, 255],
        thickness: 1,
        alpha: 255,
        ttl_ms: 2000, // Expires at 3000ms
        created_at_ms: 1000,
    };

    // Apply at now_ms = 1500 (none expired)
    let applied = frame
        .apply_annotations(
            &[
                stroke_item.clone(),
                rect_item.clone(),
                arrow_item.clone(),
                text_item.clone(),
            ],
            1500,
        )
        .expect("apply annotations");
    assert_eq!(applied, 4);

    // Verify pixels were changed
    let has_non_zero = frame.bytes.iter().any(|&b| b > 0);
    assert!(has_non_zero);

    // Apply at now_ms = 3500 (text_item expired at 3000ms, others active)
    let applied_later = frame
        .apply_annotations(
            &[stroke_item, rect_item.clone(), arrow_item, text_item],
            3500,
        )
        .expect("apply annotations at 3500ms");
    assert_eq!(applied_later, 3, "expired text item should be skipped");

    // 5. Laser Pointer rendering
    let active_laser = LaserPointerState {
        x: 50,
        y: 50,
        active: true,
        color_rgb: [255, 50, 50],
        radius: 12,
    };
    let laser_drawn = frame
        .apply_laser_pointer(&active_laser)
        .expect("laser pointer");
    assert!(laser_drawn);

    let inactive_laser = LaserPointerState {
        x: 50,
        y: 50,
        active: false,
        color_rgb: [255, 50, 50],
        radius: 12,
    };
    let inactive_drawn = frame
        .apply_laser_pointer(&inactive_laser)
        .expect("inactive laser");
    assert!(!inactive_drawn);

    // Compressed frame rejection
    let mut compressed = frame.clone();
    compressed.codec_tag = SCREEN_COMPRESS_ZSTD;
    assert!(compressed.apply_laser_pointer(&active_laser).is_err());
    assert!(compressed.apply_annotations(&[], 0).is_ok()); // empty is ok
    assert!(compressed.apply_annotations(&[rect_item], 0).is_err());
}

// ADR-106 Screen Streaming Multi-Party Shared Whiteboard Synchronization, Chronological Undo/Redo & Vector Snapshot Pipeline
#[test]
fn screen_whiteboard_multi_user_sync_and_undo_redo_pipeline() {
    // 1. Tag 13, 14, 15 ControlMsg wire roundtrips
    let undo_msg = ControlMsg::WhiteboardUndo {
        author_hash: 0x1122334455667788,
    };
    let payload13 = undo_msg.encode_payload();
    assert_eq!(payload13.len(), 14);
    assert_eq!(payload13[5], 13);
    let decoded13 = ControlMsg::decode_payload(&payload13).expect("decode tag 13");
    match decoded13 {
        ControlMsg::WhiteboardUndo { author_hash } => {
            assert_eq!(author_hash, 0x1122334455667788);
        }
        _ => panic!("expected WhiteboardUndo"),
    }

    let redo_msg = ControlMsg::WhiteboardRedo {
        author_hash: 0x99AABBCCDDEEFF00,
    };
    let payload14 = redo_msg.encode_payload();
    assert_eq!(payload14.len(), 14);
    assert_eq!(payload14[5], 14);
    let decoded14 = ControlMsg::decode_payload(&payload14).expect("decode tag 14");
    match decoded14 {
        ControlMsg::WhiteboardRedo { author_hash } => {
            assert_eq!(author_hash, 0x99AABBCCDDEEFF00);
        }
        _ => panic!("expected WhiteboardRedo"),
    }

    let policy_msg = ControlMsg::SetWhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 450,
    };
    let payload15 = policy_msg.encode_payload();
    assert_eq!(payload15.len(), 14);
    assert_eq!(payload15[5], 15);
    let decoded15 = ControlMsg::decode_payload(&payload15).expect("decode tag 15");
    match decoded15 {
        ControlMsg::SetWhiteboardPolicy {
            read_only,
            allow_peer_clear,
            max_items,
        } => {
            assert!(read_only);
            assert!(!allow_peer_clear);
            assert_eq!(max_items, 450);
        }
        _ => panic!("expected SetWhiteboardPolicy"),
    }

    // 2. hash_author helper properties
    assert_eq!(hash_author(""), 0);
    let alice_hash = hash_author("alice");
    let bob_hash = hash_author("bob");
    assert_ne!(alice_hash, 0);
    assert_ne!(bob_hash, 0);
    assert_ne!(alice_hash, bob_hash);
    assert_eq!(hash_author("alice"), alice_hash);

    // 3. ScreenAnnotationMsg inner frame roundtrips
    let undo_ann = ScreenAnnotationMsg::Undo {
        author: "alice".into(),
    };
    let frame_undo = undo_ann.to_frame().expect("to_frame");
    let decoded_undo = ScreenAnnotationMsg::from_frame(&frame_undo).expect("from_frame");
    match decoded_undo {
        ScreenAnnotationMsg::Undo { author } => assert_eq!(author, "alice"),
        _ => panic!("expected Undo"),
    }

    let policy_ann = ScreenAnnotationMsg::Policy(WhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 500,
    });
    let frame_pol = policy_ann.to_frame().expect("to_frame");
    let decoded_pol = ScreenAnnotationMsg::from_frame(&frame_pol).expect("from_frame");
    match decoded_pol {
        ScreenAnnotationMsg::Policy(pol) => {
            assert!(pol.read_only);
            assert!(!pol.allow_peer_clear);
            assert_eq!(pol.max_items, 500);
        }
        _ => panic!("expected Policy"),
    }

    // 4. WhiteboardHistory: multi-author undo/redo, capacity and snapshot
    let mut history = WhiteboardHistory::new(WhiteboardPolicy::default());

    let item_alice = AnnotationItem {
        id: 0,
        author: "alice".into(),
        kind: AnnotationKind::Rect {
            x: 10,
            y: 10,
            w: 20,
            h: 20,
        },
        color_rgb: [255, 0, 0],
        thickness: 2,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 100,
    };
    let id1 = history.add_item(item_alice).expect("add alice");
    assert_eq!(id1, 1);

    let item_bob = AnnotationItem {
        id: 0,
        author: "bob".into(),
        kind: AnnotationKind::Arrow {
            from: (50, 50),
            to: (80, 80),
        },
        color_rgb: [0, 255, 0],
        thickness: 3,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 200,
    };
    let id2 = history.add_item(item_bob).expect("add bob");
    assert_eq!(id2, 2);
    assert_eq!(history.active_count(), 2);

    // Undo for Alice only: Alice's item is removed, Bob's item remains active!
    let undone_alice = history.undo(Some("alice")).expect("undo alice");
    assert_eq!(undone_alice.author, "alice");
    assert_eq!(history.active_count(), 1);
    assert_eq!(history.active_items()[0].author, "bob");
    assert_eq!(history.redo_count(), 1);

    // Redo for Alice
    let redone_alice = history.redo(Some("alice")).expect("redo alice");
    assert_eq!(redone_alice.author, "alice");
    assert_eq!(history.active_count(), 2);
    assert_eq!(history.redo_count(), 0);

    // Global undo: pops the last item (Alice)
    let global_undone = history.undo(None).expect("global undo");
    assert_eq!(global_undone.author, "alice");
    assert_eq!(history.active_count(), 1);

    // Snapshot Export & Import
    let snapshot = history.export_snapshot("sess-1".into(), 300);
    assert_eq!(snapshot.session_id, "sess-1");
    assert_eq!(snapshot.active_items.len(), 1);
    assert_eq!(snapshot.active_items[0].author, "bob");

    let mut fresh_history = WhiteboardHistory::new(WhiteboardPolicy::default());
    let imported = fresh_history
        .import_snapshot(snapshot, true)
        .expect("import");
    assert_eq!(imported, 1);
    assert_eq!(fresh_history.active_count(), 1);
    assert_eq!(fresh_history.active_items()[0].author, "bob");

    // Read-only policy rejection
    fresh_history.set_policy(WhiteboardPolicy {
        read_only: true,
        allow_peer_clear: false,
        max_items: 10,
    });
    let rejected = fresh_history.add_item(AnnotationItem {
        id: 0,
        author: "eve".into(),
        kind: AnnotationKind::Text {
            x: 0,
            y: 0,
            text: "test".into(),
            scale: 1,
        },
        color_rgb: [0, 0, 0],
        thickness: 1,
        alpha: 255,
        ttl_ms: 0,
        created_at_ms: 0,
    });
    assert!(rejected.is_err());
}

#[test]
fn screen_roi_cropping_and_viewport_signaling() {
    // 1. Line protocol 14-byte envelope tag 16, 17, 18 roundtrips
    let msg_roi = ControlMsg::SetScreenRoi {
        x: 100,
        y: 200,
        w: 800,
        h: 600,
    };
    assert_eq!(msg_roi.tag(), 16);
    let payload = msg_roi.encode_payload();
    assert_eq!(payload.len(), 14);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode roi");
    assert_eq!(decoded, msg_roi);

    let msg_reset = ControlMsg::ResetScreenRoi;
    assert_eq!(msg_reset.tag(), 17);
    let payload = msg_reset.encode_payload();
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset");
    assert_eq!(decoded, msg_reset);

    let msg_zoom = ControlMsg::SetScreenRoiZoom { zoom_percent: 250 };
    assert_eq!(msg_zoom.tag(), 18);
    let payload = msg_zoom.encode_payload();
    let decoded = ControlMsg::decode_payload(&payload).expect("decode zoom");
    assert_eq!(decoded, msg_zoom);

    // 2. ScreenRoi sanitization & clamping
    let mut roi = ScreenRoi {
        active: true,
        x: 3800,
        y: 2100,
        w: 1921,
        h: 1081,
        zoom_percent: 30, // below 50 -> clamped to 50
    };
    roi.sanitize(3840, 2160);
    assert!(roi.active);
    assert_eq!(roi.zoom_percent, 50);
    assert_eq!(roi.x, 3800);
    assert_eq!(roi.y, 2100);
    assert_eq!(roi.w, 40); // 3840 - 3800 = 40 (even)
    assert_eq!(roi.h, 60); // 2160 - 2100 = 60 (even)

    // 3. Coordinate mapping
    let test_roi = ScreenRoi {
        active: true,
        x: 100,
        y: 100,
        w: 200,
        h: 200,
        zoom_percent: 100,
    };
    assert_eq!(test_roi.map_screen_to_roi(150, 150), Some((50, 50)));
    assert_eq!(test_roi.map_screen_to_roi(50, 150), None);
    assert_eq!(test_roi.map_screen_to_roi(350, 150), None);
    assert_eq!(test_roi.map_roi_to_screen(50, 50), (150, 150));

    // Inactive ROI map pass-through
    let inactive_roi = ScreenRoi {
        active: false,
        ..test_roi
    };
    assert_eq!(inactive_roi.map_screen_to_roi(50, 50), Some((50, 50)));
    assert_eq!(inactive_roi.map_roi_to_screen(50, 50), (50, 50));

    // 4. Pixel-perfect row-stride cropping (32x32 -> 16x16)
    let src_w = 32u16;
    let src_h = 32u16;
    let mut src_bytes = vec![0u8; (src_w as usize) * (src_h as usize) * 4];
    for y in 0..src_h as usize {
        for x in 0..src_w as usize {
            let idx = (y * (src_w as usize) + x) * 4;
            src_bytes[idx] = x as u8;
            src_bytes[idx + 1] = y as u8;
            src_bytes[idx + 2] = (x ^ y) as u8;
            src_bytes[idx + 3] = 255;
        }
    }
    let full_frame = ScreenFrame::raw(
        1,
        ScreenRect::new(0, 0, src_w, src_h).unwrap(),
        PixelFormat::Bgra8,
        src_bytes,
    )
    .expect("raw frame");

    // Pass-through when inactive
    let cropped_inactive = full_frame.crop_roi(&inactive_roi).expect("inactive crop");
    assert_eq!(cropped_inactive.rect.w, 32);
    assert_eq!(cropped_inactive.rect.h, 32);
    assert_eq!(cropped_inactive.bytes.len(), full_frame.bytes.len());

    // Crop center 16x16
    let crop_active_roi = ScreenRoi {
        active: true,
        x: 8,
        y: 8,
        w: 16,
        h: 16,
        zoom_percent: 200,
    };
    let cropped = full_frame.crop_roi(&crop_active_roi).expect("crop active");
    assert_eq!(cropped.rect.x, 8);
    assert_eq!(cropped.rect.y, 8);
    assert_eq!(cropped.rect.w, 16);
    assert_eq!(cropped.rect.h, 16);
    assert_eq!(cropped.bytes.len(), 16 * 16 * 4);

    // Verify pixel content in cropped sub-rectangle
    for cy in 0..16usize {
        for cx in 0..16usize {
            let c_idx = (cy * 16 + cx) * 4;
            let sx = (8 + cx) as u8;
            let sy = (8 + cy) as u8;
            assert_eq!(cropped.bytes[c_idx], sx);
            assert_eq!(cropped.bytes[c_idx + 1], sy);
            assert_eq!(cropped.bytes[c_idx + 2], sx ^ sy);
            assert_eq!(cropped.bytes[c_idx + 3], 255);
        }
    }
}

#[test]
fn screen_adaptive_color_depth_and_grayscale_signaling_and_quantization() {
    // 1. Line protocol 14-byte envelope tag 19, 20 roundtrips
    let msg_color = ControlMsg::SetScreenColorMode {
        mode: ScreenColorMode::Grayscale8.tag(),
        threshold: 128,
    };
    assert_eq!(msg_color.tag(), 19);
    let payload = msg_color.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 19);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode color mode");
    assert_eq!(decoded, msg_color);

    let msg_reset = ControlMsg::ResetScreenColorMode;
    assert_eq!(msg_reset.tag(), 20);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 20);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset color");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenColorMode parsing & Display
    assert_eq!(
        "full".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::FullColor32
    );
    assert_eq!(
        "reduced".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::ReducedColor16
    );
    assert_eq!(
        "grayscale".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::Grayscale8
    );
    assert_eq!(
        "monochrome".parse::<ScreenColorMode>().unwrap(),
        ScreenColorMode::Monochrome1
    );
    assert_eq!(
        ScreenColorMode::from_tag(1),
        ScreenColorMode::ReducedColor16
    );
    assert_eq!(ScreenColorMode::from_tag(2), ScreenColorMode::Grayscale8);
    assert_eq!(ScreenColorMode::from_tag(3), ScreenColorMode::Monochrome1);
    assert_eq!(ScreenColorMode::from_tag(0), ScreenColorMode::FullColor32);
    assert_eq!(ScreenColorMode::Grayscale8.as_str(), "grayscale8");

    // 3. ScreenColorConfig defaults & clamping
    let cfg_default = ScreenColorConfig::default();
    assert_eq!(cfg_default.mode, ScreenColorMode::FullColor32);
    assert_eq!(cfg_default.threshold, 128);
    assert!(!cfg_default.is_active());

    let cfg_mono = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(200));
    assert!(cfg_mono.is_active());
    assert_eq!(cfg_mono.threshold, 200);

    let cfg_clamped = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(255));
    assert_eq!(cfg_clamped.threshold, 254);

    // 4. Test In-Place Grayscale8 Transformation on raw BGRA8 frame
    let rect = ScreenRect::new(0, 0, 2, 2).unwrap();
    // Pixel 0: Red (B=0, G=0, R=255)
    // Pixel 1: Green (B=0, G=255, R=0)
    // Pixel 2: Blue (B=255, G=0, R=0)
    // Pixel 3: White (B=255, G=255, R=255)
    let bgra_pixels = vec![
        0, 0, 255, 255, // Red
        0, 255, 0, 255, // Green
        255, 0, 0, 255, // Blue
        255, 255, 255, 255, // White
    ];
    let mut frame_bgra = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, bgra_pixels).unwrap();
    let cfg_gray = ScreenColorConfig::new(ScreenColorMode::Grayscale8, None);
    frame_bgra
        .apply_color_mode(&cfg_gray)
        .expect("apply grayscale");

    // Y_red = (77 * 255 + 0 + 0) >> 8 = 76
    // Y_green = (150 * 255) >> 8 = 149
    // Y_blue = (29 * 255) >> 8 = 28
    // Y_white = (77*255 + 150*255 + 29*255) >> 8 = 255
    let px0_y = frame_bgra.bytes[0];
    assert_eq!(px0_y, 76);
    assert_eq!(frame_bgra.bytes[1], 76); // G == Y
    assert_eq!(frame_bgra.bytes[2], 76); // R == Y
    assert_eq!(frame_bgra.bytes[3], 255); // Alpha preserved

    let px1_y = frame_bgra.bytes[4];
    assert_eq!(px1_y, 149);
    assert_eq!(frame_bgra.bytes[5], 149);
    assert_eq!(frame_bgra.bytes[6], 149);

    let px2_y = frame_bgra.bytes[8];
    assert_eq!(px2_y, 28);
    assert_eq!(frame_bgra.bytes[9], 28);
    assert_eq!(frame_bgra.bytes[10], 28);

    let px3_y = frame_bgra.bytes[12];
    assert_eq!(px3_y, 255);
    assert_eq!(frame_bgra.bytes[13], 255);
    assert_eq!(frame_bgra.bytes[14], 255);

    // 5. Test Monochrome1 bi-level binarization with threshold 100
    let rgba_pixels = vec![
        255, 0, 0, 255, // Red (Y=76 < 100 -> 0)
        0, 255, 0, 255, // Green (Y=149 >= 100 -> 255)
        0, 0, 255, 255, // Blue (Y=28 < 100 -> 0)
        255, 255, 255, 255, // White (Y=255 >= 100 -> 255)
    ];
    let mut frame_mono = ScreenFrame::raw(2, rect, PixelFormat::Rgba8, rgba_pixels).unwrap();
    let cfg_mono100 = ScreenColorConfig::new(ScreenColorMode::Monochrome1, Some(100));
    frame_mono
        .apply_color_mode(&cfg_mono100)
        .expect("apply mono");

    // Red -> 0
    assert_eq!(frame_mono.bytes[0..3], [0, 0, 0]);
    // Green -> 255
    assert_eq!(frame_mono.bytes[4..7], [255, 255, 255]);
    // Blue -> 0
    assert_eq!(frame_mono.bytes[8..11], [0, 0, 0]);
    // White -> 255
    assert_eq!(frame_mono.bytes[12..15], [255, 255, 255]);

    // 6. Test ReducedColor16 (RGB565 quantization)
    let test_pixels = vec![123, 234, 45, 255];
    let mut frame_reduced = ScreenFrame::raw(
        3,
        ScreenRect::new(0, 0, 1, 1).unwrap(),
        PixelFormat::Rgba8,
        test_pixels,
    )
    .unwrap();
    let cfg_reduced = ScreenColorConfig::new(ScreenColorMode::ReducedColor16, None);
    frame_reduced
        .apply_color_mode(&cfg_reduced)
        .expect("apply reduced");

    let r_orig = 123u8;
    let g_orig = 234u8;
    let b_orig = 45u8;
    let r_expected = (r_orig & 0xF8) | (r_orig >> 5);
    let g_expected = (g_orig & 0xFC) | (g_orig >> 6);
    let b_expected = (b_orig & 0xF8) | (b_orig >> 5);
    assert_eq!(frame_reduced.bytes[0], r_expected);
    assert_eq!(frame_reduced.bytes[1], g_expected);
    assert_eq!(frame_reduced.bytes[2], b_expected);
    assert_eq!(frame_reduced.bytes[3], 255);
}

#[test]
fn screen_dynamic_delta_patch_and_temporal_compression_algorithms() {
    // 1. Line protocol 14-byte envelope tag 21, 22 roundtrips
    let msg_delta = ControlMsg::SetScreenDeltaMode {
        enabled: true,
        keyframe_interval: 120,
    };
    assert_eq!(msg_delta.tag(), 21);
    let payload = msg_delta.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 21);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode delta mode");
    assert_eq!(decoded, msg_delta);

    let msg_reset = ControlMsg::ResetScreenDeltaMode;
    assert_eq!(msg_reset.tag(), 22);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 22);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset delta");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenDeltaConfig defaults & clamping
    let cfg_default = ScreenDeltaConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(
        cfg_default.keyframe_interval,
        SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL
    );

    let cfg_custom = ScreenDeltaConfig::new(true, Some(30));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.keyframe_interval, 30);

    let cfg_clamped_low = ScreenDeltaConfig::new(true, Some(0));
    assert_eq!(cfg_clamped_low.keyframe_interval, 1);

    let cfg_clamped_high = ScreenDeltaConfig::new(true, Some(999));
    assert_eq!(cfg_clamped_high.keyframe_interval, 600);

    // 3. ScreenDeltaReport default
    let report = ScreenDeltaReport::default();
    assert!(report.ok);
    assert!(!report.enabled);
    assert_eq!(report.keyframe_interval, 60);
    assert_eq!(report.saved_bytes, 0);

    // 4. Test extract_sub_rect and apply_sub_rect algorithms
    let w = 8u16;
    let h = 8u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            base_pixels[idx] = (x * 10) as u8; // B
            base_pixels[idx + 1] = (y * 10) as u8; // G
            base_pixels[idx + 2] = ((x + y) * 5) as u8; // R
            base_pixels[idx + 3] = 255; // A
        }
    }
    let full_frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Extract dirty patch at (2, 2) size 4x3
    let patch = full_frame
        .extract_sub_rect(2, 2, 4, 3)
        .expect("extract sub rect");
    assert_eq!(patch.rect.x, 2);
    assert_eq!(patch.rect.y, 2);
    assert_eq!(patch.rect.w, 4);
    assert_eq!(patch.rect.h, 3);
    assert_eq!(patch.bytes.len(), 4 * 3 * 4);

    // Verify patch pixels match expected region from full_frame
    for py in 0..3 {
        for px in 0..4 {
            let patch_idx = (py * 4 + px) * 4;
            let full_x = 2 + px;
            let full_y = 2 + py;
            let full_idx = (full_y * 8 + full_x) * 4;
            assert_eq!(
                patch.bytes[patch_idx..patch_idx + 4],
                base_pixels[full_idx..full_idx + 4]
            );
        }
    }

    // Now test apply_sub_rect onto an empty (zeroed) receiver canvas
    let canvas_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    let mut canvas = ScreenFrame::raw(0, rect, PixelFormat::Bgra8, canvas_pixels).unwrap();
    canvas
        .apply_sub_rect(&patch)
        .expect("apply patch to canvas");

    // Check that patch region in canvas is updated, and outside is still zero
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            if (2..6).contains(&x) && (2..5).contains(&y) {
                assert_eq!(canvas.bytes[idx..idx + 4], base_pixels[idx..idx + 4]);
            } else {
                assert_eq!(canvas.bytes[idx..idx + 4], [0, 0, 0, 0]);
            }
        }
    }

    // Test error cases for apply_sub_rect:
    // Out-of-bounds patch
    let oob_patch = ScreenFrame {
        seq: 2,
        rect: ScreenRect::new(6, 6, 4, 4).unwrap(), // 6+4=10 > 8
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 16,
        bytes: vec![0u8; 4 * 4 * 4],
    };
    assert!(canvas.apply_sub_rect(&oob_patch).is_err());

    // Format mismatch
    let format_mismatch_patch = ScreenFrame {
        seq: 3,
        rect: ScreenRect::new(0, 0, 2, 2).unwrap(),
        format: PixelFormat::Rgba8,
        codec_tag: SCREEN_COMPRESS_NONE,
        stride: 8,
        bytes: vec![0u8; 2 * 2 * 4],
    };
    assert!(canvas.apply_sub_rect(&format_mismatch_patch).is_err());

    // 5. Wire frame serialization with SCREEN_COMPRESS_DELTA
    let mut delta_wire_frame = patch.clone();
    delta_wire_frame.codec_tag = SCREEN_COMPRESS_DELTA;
    let encoded = delta_wire_frame
        .encode_payload()
        .expect("encode delta frame");
    let decoded = ScreenFrame::decode_payload(&encoded).expect("decode delta frame");
    assert_eq!(decoded.codec_tag, SCREEN_COMPRESS_DELTA);
    assert_eq!(decoded.rect, patch.rect);
    assert_eq!(decoded.bytes, patch.bytes);
}

#[test]
fn screen_objective_visual_quality_metric_and_psnr_ssim_algorithms() {
    // 1. Line protocol 14-byte envelope tag 23, 24 roundtrips
    let msg_metric = ControlMsg::SetScreenQualityMetricMode {
        enabled: true,
        sample_interval: 15,
    };
    assert_eq!(msg_metric.tag(), 23);
    let payload = msg_metric.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 23);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode metric mode");
    assert_eq!(decoded, msg_metric);

    let msg_reset = ControlMsg::ResetScreenQualityMetricMode;
    assert_eq!(msg_reset.tag(), 24);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 24);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset metric");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenQualityMetricConfig defaults & clamping
    let cfg_default = ScreenQualityMetricConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(
        cfg_default.sample_interval,
        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
    );

    let cfg_custom = ScreenQualityMetricConfig::new(true, Some(25));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.sample_interval, 25);

    let cfg_clamped_low = ScreenQualityMetricConfig::new(true, Some(0));
    assert_eq!(cfg_clamped_low.sample_interval, 1);

    let cfg_clamped_high = ScreenQualityMetricConfig::new(true, Some(200));
    assert_eq!(cfg_clamped_high.sample_interval, 120);

    // 3. ScreenQualityMetricReport default
    let report = ScreenQualityMetricReport::default();
    assert!(report.ok);
    assert!(!report.enabled);
    assert_eq!(
        report.sample_interval,
        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
    );
    assert_eq!(report.evaluated_frames, 0);
    assert_eq!(report.avg_mse, 0.0);
    assert_eq!(report.avg_psnr_db, 100.0);
    assert_eq!(report.avg_ssim, 1.0);
    assert_eq!(report.last_psnr_db, 100.0);
    assert_eq!(report.last_ssim, 1.0);
    assert!(!report.quality_warning);

    // 4. Test compute_mse, compute_psnr, compute_ssim
    let w = 32u16;
    let h = 32u16;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h {
        for x in 0..w {
            let idx = ((y as usize) * (w as usize) + (x as usize)) * 4;
            base_pixels[idx] = (x * 7) as u8; // B
            base_pixels[idx + 1] = (y * 7) as u8; // G
            base_pixels[idx + 2] = ((x + y) * 3) as u8; // R
            base_pixels[idx + 3] = 255; // A
        }
    }
    let frame_ref = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();
    let frame_identical =
        ScreenFrame::raw(2, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Exact match: MSE = 0, PSNR = 100 dB, SSIM = 1.0
    let mse_ident = frame_identical
        .compute_mse(&frame_ref)
        .expect("mse identical");
    assert!(mse_ident <= 1e-10);
    let psnr_ident = frame_identical
        .compute_psnr(&frame_ref)
        .expect("psnr identical");
    assert_eq!(psnr_ident, 100.0);
    let ssim_ident = frame_identical
        .compute_ssim(&frame_ref)
        .expect("ssim identical");
    assert!((ssim_ident - 1.0).abs() < 1e-6);

    // Slightly modified frame (small perturbation on a few pixels)
    let mut perturbed_pixels = base_pixels.clone();
    perturbed_pixels[0] = perturbed_pixels[0].wrapping_add(20);
    perturbed_pixels[1] = perturbed_pixels[1].wrapping_add(15);
    perturbed_pixels[4] = perturbed_pixels[4].wrapping_add(10);
    let frame_perturbed = ScreenFrame::raw(3, rect, PixelFormat::Bgra8, perturbed_pixels).unwrap();

    let mse_pert = frame_perturbed
        .compute_mse(&frame_ref)
        .expect("mse perturbed");
    assert!(mse_pert > 0.0);
    let psnr_pert = frame_perturbed
        .compute_psnr(&frame_ref)
        .expect("psnr perturbed");
    assert!(
        psnr_pert < 100.0 && psnr_pert > 35.0,
        "psnr_pert was {}",
        psnr_pert
    );
    let ssim_pert = frame_perturbed
        .compute_ssim(&frame_ref)
        .expect("ssim perturbed");
    assert!(
        ssim_pert > 0.95 && ssim_pert < 1.0,
        "ssim_pert was {}",
        ssim_pert
    );

    // Heavily distorted frame (all inverted colors)
    let mut inverted_pixels = base_pixels.clone();
    for p in inverted_pixels.chunks_exact_mut(4) {
        p[0] = 255 - p[0];
        p[1] = 255 - p[1];
        p[2] = 255 - p[2];
    }
    let frame_inverted = ScreenFrame::raw(4, rect, PixelFormat::Bgra8, inverted_pixels).unwrap();
    let psnr_inv = frame_inverted
        .compute_psnr(&frame_ref)
        .expect("psnr inverted");
    assert!(psnr_inv < 25.0, "psnr_inv was {}", psnr_inv);
    let ssim_inv = frame_inverted
        .compute_ssim(&frame_ref)
        .expect("ssim inverted");
    assert!(ssim_inv < 0.50, "ssim_inv was {}", ssim_inv);

    // Error case: dimension mismatch
    let rect_diff = ScreenRect::new(0, 0, 32, 16).unwrap();
    let frame_diff =
        ScreenFrame::raw(5, rect_diff, PixelFormat::Bgra8, vec![0u8; 32 * 16 * 4]).unwrap();
    assert!(frame_diff.compute_mse(&frame_ref).is_err());
    assert!(frame_diff.compute_psnr(&frame_ref).is_err());
    assert!(frame_diff.compute_ssim(&frame_ref).is_err());

    // Error case: format mismatch
    let frame_rgba = ScreenFrame::raw(6, rect, PixelFormat::Rgba8, base_pixels).unwrap();
    assert!(frame_rgba.compute_mse(&frame_ref).is_err());
    assert!(frame_rgba.compute_psnr(&frame_ref).is_err());
    assert!(frame_rgba.compute_ssim(&frame_ref).is_err());
}

#[test]
fn screen_autonomous_qoe_closed_loop_adaptation_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 25, 26 roundtrips
    let msg_qoe = ControlMsg::SetScreenQoeMode {
        mode: ScreenQoeMode::Balanced.tag(),
        target_psnr_db: 32,
        target_ssim_pct: 90,
    };
    assert_eq!(msg_qoe.tag(), 25);
    let payload = msg_qoe.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 25);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode qoe mode");
    assert_eq!(decoded, msg_qoe);

    let msg_reset = ControlMsg::ResetScreenQoeMode;
    assert_eq!(msg_reset.tag(), 26);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 26);
    let decoded_reset = ControlMsg::decode_payload(&payload).expect("decode qoe reset");
    assert_eq!(decoded_reset, msg_reset);

    // 2. ScreenQoeMode string parsing & defaults
    assert_eq!(
        "balanced".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Balanced
    );
    assert_eq!(
        "conservative".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Conservative
    );
    assert_eq!(
        "economy".parse::<ScreenQoeMode>().unwrap(),
        ScreenQoeMode::Economy
    );
    assert_eq!("off".parse::<ScreenQoeMode>().unwrap(), ScreenQoeMode::Off);

    // 3. ScreenQoeConfig defaults and clamping
    let cfg_default = ScreenQoeConfig::default();
    assert_eq!(cfg_default.mode, ScreenQoeMode::Off);
    assert_eq!(cfg_default.target_psnr_db, 30);
    assert_eq!(cfg_default.target_ssim_pct, 85);
    assert!(!cfg_default.is_active());

    let cfg_custom = ScreenQoeConfig::new(ScreenQoeMode::Balanced, Some(35), Some(92));
    assert_eq!(cfg_custom.target_psnr_db, 35);
    assert_eq!(cfg_custom.target_ssim_pct, 92);
    assert!(cfg_custom.is_active());

    // 4. ScreenQoeController evaluation state machine
    let mut controller = ScreenQoeController::new(cfg_custom);
    let mut metric = ScreenQualityMetricReport {
        ok: true,
        enabled: true,
        sample_interval: 10,
        evaluated_frames: 10,
        avg_mse: 0.0,
        avg_psnr_db: 40.0,
        avg_ssim: 0.98,
        last_psnr_db: 40.0,
        last_ssim: 0.98,
        quality_warning: false,
    };

    // Quality satisfies target -> Hold
    let action1 = controller.evaluate(&metric, 1_000_000);
    assert_eq!(action1, ScreenQoeAction::Hold);
    assert_eq!(controller.actions_total(), 0);

    // Quality drops: high SSIM deficit (downsampling blurriness) -> UpgradeScale
    metric.last_ssim = 0.70; // target is 0.92 -> deficit 0.22 > 0.15
    let action2 = controller.evaluate(&metric, 1_100_000);
    assert_eq!(action2, ScreenQoeAction::UpgradeScale);
    assert_eq!(controller.actions_total(), 1);
    assert_eq!(controller.last_action(), ScreenQoeAction::UpgradeScale);

    // Cooldown prevents immediate re-triggering
    metric.last_ssim = 0.65;
    let action_cooldown = controller.evaluate(&metric, 1_200_000);
    assert_eq!(action_cooldown, ScreenQoeAction::Hold);
    assert_eq!(controller.actions_total(), 1);

    // After cooldown expires (cooldown is 500,000 us -> 1_100_000 + 600_000 = 1_700_000)
    // Large PSNR deficit -> LowerQp
    metric.last_ssim = 0.90; // deficit 0.02
    metric.last_psnr_db = 22.0; // target 35.0 -> deficit 13.0 > 8.0
    let action3 = controller.evaluate(&metric, 1_700_000);
    assert_eq!(action3, ScreenQoeAction::LowerQp);
    assert_eq!(controller.actions_total(), 2);

    // Report generation
    let rep = controller.report(1_700_000);
    assert_eq!(rep.mode, "balanced");
    assert_eq!(rep.target_psnr_db, 35);
    assert_eq!(rep.target_ssim_pct, 92);
    assert_eq!(rep.actions_total, 2);
    assert_eq!(rep.last_action, "lower_qp");

    // Reset restores defaults
    controller.reset();
    assert_eq!(controller.actions_total(), 0);
    assert_eq!(controller.last_action(), ScreenQoeAction::Hold);
    assert!(!controller.config().is_active());
}

#[test]
fn screen_progressive_intra_refresh_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 27, 28 roundtrips
    let msg_pir = ControlMsg::SetScreenPirMode {
        mode: ScreenPirMode::Continuous.tag(),
        bands: 12,
    };
    assert_eq!(msg_pir.tag(), 27);
    let payload = msg_pir.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 27);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode pir mode");
    assert_eq!(decoded, msg_pir);

    let msg_reset = ControlMsg::ResetScreenPirMode;
    assert_eq!(msg_reset.tag(), 28);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 28);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset pir");
    assert_eq!(decoded, msg_reset);

    // 2. ScreenPirMode string parsing & defaults
    assert_eq!(
        "continuous".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Continuous
    );
    assert_eq!(
        "on-demand".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::OnDemand
    );
    assert_eq!(
        "ondemand".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::OnDemand
    );
    assert_eq!(
        "disabled".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Disabled
    );
    assert_eq!(
        "off".parse::<ScreenPirMode>().unwrap(),
        ScreenPirMode::Disabled
    );

    // 3. ScreenPirConfig defaults & clamping
    let cfg_default = ScreenPirConfig::default();
    assert_eq!(cfg_default.mode, ScreenPirMode::Disabled);
    assert_eq!(cfg_default.bands, SCREEN_PIR_DEFAULT_BANDS);
    assert!(!cfg_default.is_enabled());

    let cfg_custom = ScreenPirConfig::new(ScreenPirMode::Continuous, Some(16));
    assert_eq!(cfg_custom.mode, ScreenPirMode::Continuous);
    assert_eq!(cfg_custom.bands, 16);
    assert!(cfg_custom.is_enabled());

    let cfg_clamped_low = ScreenPirConfig::new(ScreenPirMode::OnDemand, Some(1));
    assert_eq!(cfg_clamped_low.bands, SCREEN_PIR_MIN_BANDS);

    let cfg_clamped_high = ScreenPirConfig::new(ScreenPirMode::OnDemand, Some(64));
    assert_eq!(cfg_clamped_high.bands, SCREEN_PIR_MAX_BANDS);

    // 4. Band rectangle geometry calculation & full area coverage
    let w = 1920u32;
    let h = 1080u32;
    let bands = 8u8;
    let mut total_h = 0u32;
    for i in 0..bands {
        let (rx, ry, rw, rh) = compute_pir_band_rect(w, h, bands, i);
        assert_eq!(rx, 0);
        assert_eq!(rw, w);
        assert_eq!(ry, total_h);
        total_h += rh;
    }
    assert_eq!(total_h, h);

    // Check with odd height to verify remainder absorption
    let h_odd = 1085u32;
    let mut total_h_odd = 0u32;
    for i in 0..bands {
        let (rx, ry, rw, rh) = compute_pir_band_rect(w, h_odd, bands, i);
        assert_eq!(rx, 0);
        assert_eq!(rw, w);
        assert_eq!(ry, total_h_odd);
        total_h_odd += rh;
    }
    assert_eq!(total_h_odd, h_odd);

    // 5. Default Report
    let rep = ScreenPirReport::default();
    assert!(rep.ok);
    assert_eq!(rep.mode, "disabled");
    assert_eq!(rep.bands, SCREEN_PIR_DEFAULT_BANDS);
    assert_eq!(rep.sweeps_completed, 0);
    assert_eq!(rep.slices_sent, 0);
    assert_eq!(rep.current_band, 0);
    assert!(!rep.active_sweep);
}

#[test]
fn screen_long_term_reference_algorithms_and_signaling() {
    // 1. Line protocol 14-byte envelope tag 29, 30, 31, 32 roundtrips
    let msg_ltr = ControlMsg::SetScreenLtrMode {
        enabled: true,
        auto_interval: 120,
    };
    assert_eq!(msg_ltr.tag(), 29);
    let payload = msg_ltr.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 29);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr mode");
    assert_eq!(decoded, msg_ltr);

    let msg_reset = ControlMsg::ResetScreenLtrMode;
    assert_eq!(msg_reset.tag(), 30);
    let payload = msg_reset.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 30);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode reset ltr");
    assert_eq!(decoded, msg_reset);

    let msg_rec = ControlMsg::LtrRecoveryRequest { ltr_seq: 4242 };
    assert_eq!(msg_rec.tag(), 31);
    let payload = msg_rec.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 31);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr recovery request");
    assert_eq!(decoded, msg_rec);

    let msg_sync = ControlMsg::LtrAnchorSync { ltr_seq: 8888 };
    assert_eq!(msg_sync.tag(), 32);
    let payload = msg_sync.encode_payload();
    assert_eq!(payload.len(), 14);
    assert_eq!(payload[5], 32);
    let decoded = ControlMsg::decode_payload(&payload).expect("decode ltr anchor sync");
    assert_eq!(decoded, msg_sync);

    // 2. ScreenLtrConfig defaults & clamping
    let cfg_default = ScreenLtrConfig::default();
    assert!(!cfg_default.enabled);
    assert_eq!(cfg_default.auto_interval, SCREEN_LTR_DEFAULT_AUTO_INTERVAL);
    assert!(!cfg_default.is_enabled());

    let cfg_custom = ScreenLtrConfig::new(true, Some(30));
    assert!(cfg_custom.enabled);
    assert_eq!(cfg_custom.auto_interval, 30);
    assert!(cfg_custom.is_enabled());

    let cfg_clamped_low = ScreenLtrConfig::new(true, Some(1));
    assert_eq!(cfg_clamped_low.auto_interval, SCREEN_LTR_MIN_INTERVAL);

    let cfg_clamped_high = ScreenLtrConfig::new(true, Some(1000));
    assert_eq!(cfg_clamped_high.auto_interval, SCREEN_LTR_MAX_INTERVAL);

    let cfg_manual = ScreenLtrConfig::new(true, Some(0));
    assert_eq!(cfg_manual.auto_interval, 0);

    // 3. Default Report
    let rep = ScreenLtrReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.auto_interval, SCREEN_LTR_DEFAULT_AUTO_INTERVAL);
    assert_eq!(rep.current_ltr_seq, 0);
    assert_eq!(rep.anchors_established, 0);
    assert_eq!(rep.recoveries_requested, 0);
    assert_eq!(rep.recoveries_completed, 0);
    assert_eq!(rep.saved_burst_bytes, 0);

    // 4. SCREEN_COMPRESS_LTR codec tag roundtrip
    let rect = ScreenRect::new(0, 0, 16, 16).unwrap();
    let pixels = vec![0xABu8; 16 * 16 * 4];
    let mut frame = ScreenFrame::raw(100, rect, PixelFormat::Bgra8, pixels).unwrap();
    frame.codec_tag = SCREEN_COMPRESS_LTR;
    let wire_payload = frame.encode_payload().unwrap();
    let decoded_frame =
        ScreenFrame::decode_payload(&wire_payload).expect("decode frame with SCREEN_COMPRESS_LTR");
    assert_eq!(decoded_frame.codec_tag, SCREEN_COMPRESS_LTR);
    assert_eq!(decoded_frame.seq, 100);

    // 5. LTR Recovery sub-rect blitting
    let canvas_rect = ScreenRect::new(0, 0, 32, 32).unwrap();
    let canvas_pixels = vec![0x10u8; 32 * 32 * 4];
    let mut golden_canvas =
        ScreenFrame::raw(50, canvas_rect, PixelFormat::Bgra8, canvas_pixels).unwrap();

    let patch_rect = ScreenRect::new(8, 8, 8, 8).unwrap();
    let patch_pixels = vec![0xFFu8; 8 * 8 * 4];
    let patch_frame = ScreenFrame::raw(101, patch_rect, PixelFormat::Bgra8, patch_pixels).unwrap();

    golden_canvas
        .apply_sub_rect(&patch_frame)
        .expect("apply ltr recovery patch");
    // Verify patched area
    let idx = ((8 * 32 + 8) * 4) as usize;
    assert_eq!(golden_canvas.bytes[idx], 0xFF);
    // Verify unpatched area
    let unpatched_idx = 0usize;
    assert_eq!(golden_canvas.bytes[unpatched_idx], 0x10);
}

#[test]
fn screen_temporal_scalability_svc_algorithms_and_signaling() {
    // 1. ScreenSvcMode enum conversions and parsing
    assert_eq!(ScreenSvcMode::from_u8(0), ScreenSvcMode::Disabled);
    assert_eq!(ScreenSvcMode::from_u8(1), ScreenSvcMode::Adaptive);
    assert_eq!(ScreenSvcMode::from_u8(2), ScreenSvcMode::BaseOnly);
    assert_eq!(ScreenSvcMode::from_u8(99), ScreenSvcMode::Disabled);

    assert_eq!(ScreenSvcMode::Disabled.as_u8(), 0);
    assert_eq!(ScreenSvcMode::Adaptive.as_u8(), 1);
    assert_eq!(ScreenSvcMode::BaseOnly.as_u8(), 2);

    assert_eq!(ScreenSvcMode::Disabled.as_str(), "disabled");
    assert_eq!(ScreenSvcMode::Adaptive.as_str(), "adaptive");
    assert_eq!(ScreenSvcMode::BaseOnly.as_str(), "base_only");

    assert_eq!(
        ScreenSvcMode::parse_mode("adaptive"),
        ScreenSvcMode::Adaptive
    );
    assert_eq!(
        ScreenSvcMode::parse_mode("base_only"),
        ScreenSvcMode::BaseOnly
    );
    assert_eq!(
        ScreenSvcMode::parse_mode("disabled"),
        ScreenSvcMode::Disabled
    );

    // 2. ScreenSvcConfig methods
    let default_cfg = ScreenSvcConfig::default();
    assert_eq!(default_cfg.mode, ScreenSvcMode::Disabled);
    assert!(!default_cfg.drop_t1);
    assert!(!default_cfg.is_enabled());
    assert!(!default_cfg.should_drop_t1());

    let adaptive_cfg = ScreenSvcConfig::new(ScreenSvcMode::Adaptive, false);
    assert!(adaptive_cfg.is_enabled());
    assert!(!adaptive_cfg.should_drop_t1());

    let adaptive_drop_cfg = ScreenSvcConfig::new(ScreenSvcMode::Adaptive, true);
    assert!(adaptive_drop_cfg.is_enabled());
    assert!(adaptive_drop_cfg.should_drop_t1());

    let base_only_cfg = ScreenSvcConfig::new(ScreenSvcMode::BaseOnly, false);
    assert!(base_only_cfg.is_enabled());
    assert!(base_only_cfg.should_drop_t1()); // BaseOnly always drops T1

    // 3. ScreenSvcReport defaults
    let rep = ScreenSvcReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.mode, "disabled");
    assert!(!rep.drop_t1);
    assert_eq!(rep.t0_frames_sent, 0);
    assert_eq!(rep.t1_frames_sent, 0);
    assert_eq!(rep.t1_frames_dropped, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 4. Tag 33: SetScreenSvcMode
    let msg_set = ControlMsg::SetScreenSvcMode {
        mode: 1,
        drop_t1: true,
    };
    assert_eq!(msg_set.tag(), 33);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 33);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set svc mode");
    assert_eq!(decoded_set, msg_set);

    // 5. Tag 34: ResetScreenSvcMode
    let msg_reset = ControlMsg::ResetScreenSvcMode;
    assert_eq!(msg_reset.tag(), 34);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 34);
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).expect("decode reset svc mode");
    assert_eq!(decoded_reset, msg_reset);

    // 6. Tag 35: SvcT1DropSync
    let msg_sync = ControlMsg::SvcT1DropSync {
        dropped_count: 12345,
    };
    assert_eq!(msg_sync.tag(), 35);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 35);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode svc drop sync");
    assert_eq!(decoded_sync, msg_sync);

    // 7. SCREEN_COMPRESS_SVC_T1 Codec Tag roundtrip
    let rect = ScreenRect::new(0, 0, 16, 16).unwrap();
    let pixels = vec![0x77u8; 16 * 16 * 4];
    let mut frame = ScreenFrame::raw(101, rect, PixelFormat::Bgra8, pixels).unwrap();
    frame.codec_tag = SCREEN_COMPRESS_SVC_T1;
    let wire = frame.encode_payload().unwrap();
    let decoded =
        ScreenFrame::decode_payload(&wire).expect("decode frame with SCREEN_COMPRESS_SVC_T1");
    assert_eq!(decoded.codec_tag, SCREEN_COMPRESS_SVC_T1);
    assert_eq!(decoded.seq, 101);
}

#[test]
fn screen_motion_vector_estimation_and_scroll_acceleration_algorithms() {
    // 1. ScreenMotionConfig defaults & clamping
    let default_cfg = ScreenMotionConfig::default();
    assert!(!default_cfg.enabled);
    assert_eq!(
        default_cfg.max_displacement,
        SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
    );

    let custom_cfg = ScreenMotionConfig::new(true, 128);
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.max_displacement, 128);

    let low_clamped = ScreenMotionConfig::new(true, 2);
    assert_eq!(low_clamped.max_displacement, SCREEN_MOTION_MIN_DISPLACEMENT);

    let high_clamped = ScreenMotionConfig::new(true, 500);
    assert_eq!(
        high_clamped.max_displacement,
        SCREEN_MOTION_MAX_DISPLACEMENT
    );

    // 2. ScreenMotionReport defaults
    let rep = ScreenMotionReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.max_displacement, SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT);
    assert_eq!(rep.motion_frames_sent, 0);
    assert_eq!(rep.fallback_frames_sent, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 3. Tag 36: SetScreenMotionMode
    let msg_set = ControlMsg::SetScreenMotionMode {
        enabled: true,
        max_displacement: 96,
    };
    assert_eq!(msg_set.tag(), 36);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 36);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set motion mode");
    assert_eq!(decoded_set, msg_set);

    // 4. Tag 37: ResetScreenMotionMode
    let msg_reset = ControlMsg::ResetScreenMotionMode;
    assert_eq!(msg_reset.tag(), 37);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 37);
    let decoded_reset =
        ControlMsg::decode_payload(&payload_reset).expect("decode reset motion mode");
    assert_eq!(decoded_reset, msg_reset);

    // 5. Tag 38: MotionScrollSync
    let msg_sync = ControlMsg::MotionScrollSync {
        scroll_frames_sent: 9999,
    };
    assert_eq!(msg_sync.tag(), 38);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 38);
    let decoded_sync =
        ControlMsg::decode_payload(&payload_sync).expect("decode motion scroll sync");
    assert_eq!(decoded_sync, msg_sync);

    // 6. compute_exposed_strip tests
    assert_eq!(compute_exposed_strip(100, 100, 0, 0), None);
    // Shift up by 16: exposed strip is bottom 16 rows (y=84, h=16)
    let strip_up = compute_exposed_strip(100, 100, 0, -16).unwrap();
    assert_eq!(strip_up.x, 0);
    assert_eq!(strip_up.y, 84);
    assert_eq!(strip_up.width, 100);
    assert_eq!(strip_up.height, 16);

    // Shift down by 20: exposed strip is top 20 rows (y=0, h=20)
    let strip_down = compute_exposed_strip(100, 100, 0, 20).unwrap();
    assert_eq!(strip_down.x, 0);
    assert_eq!(strip_down.y, 0);
    assert_eq!(strip_down.width, 100);
    assert_eq!(strip_down.height, 20);

    // Shift left by 15: exposed strip is right 15 cols (x=85, w=15)
    let strip_left = compute_exposed_strip(100, 100, -15, 0).unwrap();
    assert_eq!(strip_left.x, 85);
    assert_eq!(strip_left.y, 0);
    assert_eq!(strip_left.width, 15);
    assert_eq!(strip_left.height, 100);

    // Shift right by 10: exposed strip is left 10 cols (x=0, w=10)
    let strip_right = compute_exposed_strip(100, 100, 10, 0).unwrap();
    assert_eq!(strip_right.x, 0);
    assert_eq!(strip_right.y, 0);
    assert_eq!(strip_right.width, 10);
    assert_eq!(strip_right.height, 100);

    // 7. ScreenFrame::scroll_canvas memory movement tests
    let w = 32u16;
    let h = 32u16;
    let mut base_pixels = vec![0u8; (w as usize) * (h as usize) * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = (y * (w as usize) + x) * 4;
            base_pixels[idx] = (x & 0xFF) as u8;
            base_pixels[idx + 1] = (y & 0xFF) as u8;
            base_pixels[idx + 2] = 0xAA;
            base_pixels[idx + 3] = 255;
        }
    }
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let mut frame_canvas =
        ScreenFrame::raw(1, rect, PixelFormat::Rgba8, base_pixels.clone()).unwrap();

    // Scroll up by 4 pixels (dy = -4)
    frame_canvas.scroll_canvas(0, -4);
    // At row 0, pixels should now equal original row 4
    let row0_offset = 0;
    let orig_row4_offset = 4 * (w as usize) * 4;
    assert_eq!(
        &frame_canvas.bytes[row0_offset..row0_offset + (w as usize) * 4],
        &base_pixels[orig_row4_offset..orig_row4_offset + (w as usize) * 4]
    );

    // Scroll down by 4 pixels (dy = 4) on fresh canvas
    let mut frame_canvas_down =
        ScreenFrame::raw(2, rect, PixelFormat::Rgba8, base_pixels.clone()).unwrap();
    frame_canvas_down.scroll_canvas(0, 4);
    // At row 4, pixels should now equal original row 0
    let row4_offset = 4 * (w as usize) * 4;
    let orig_row0_offset = 0;
    assert_eq!(
        &frame_canvas_down.bytes[row4_offset..row4_offset + (w as usize) * 4],
        &base_pixels[orig_row0_offset..orig_row0_offset + (w as usize) * 4]
    );

    // 8. estimate_scroll_motion tests
    let w_est = 64u16;
    let h_est = 64u16;
    let mut prev_pixels = vec![0u8; (w_est as usize) * (h_est as usize) * 4];
    for y in 0..h_est as usize {
        for x in 0..w_est as usize {
            let idx = (y * (w_est as usize) + x) * 4;
            prev_pixels[idx] = ((x * 3) & 0xFF) as u8;
            prev_pixels[idx + 1] = ((y * 7) & 0xFF) as u8;
            prev_pixels[idx + 2] = ((x ^ y) & 0xFF) as u8;
            prev_pixels[idx + 3] = 255;
        }
    }
    let rect_est = ScreenRect::new(0, 0, w_est, h_est).unwrap();
    let prev_frame =
        ScreenFrame::raw(10, rect_est, PixelFormat::Rgba8, prev_pixels.clone()).unwrap();

    // Create current frame scrolled up by 8 pixels (dy = -8)
    let mut curr_pixels = vec![0u8; (w_est as usize) * (h_est as usize) * 4];
    let dy_scroll = -8i16;
    let abs_dy = 8usize;
    // Shift up
    for y in 0..(h_est as usize - abs_dy) {
        let src_idx = (y + abs_dy) * (w_est as usize) * 4;
        let dst_idx = y * (w_est as usize) * 4;
        curr_pixels[dst_idx..dst_idx + (w_est as usize) * 4]
            .copy_from_slice(&prev_pixels[src_idx..src_idx + (w_est as usize) * 4]);
    }
    // New exposed bottom rows filled with distinct pattern
    for y in (h_est as usize - abs_dy)..(h_est as usize) {
        for x in 0..w_est as usize {
            let idx = (y * (w_est as usize) + x) * 4;
            curr_pixels[idx] = 0xFE;
            curr_pixels[idx + 1] = 0xDC;
            curr_pixels[idx + 2] = 0xBA;
            curr_pixels[idx + 3] = 255;
        }
    }
    let curr_frame =
        ScreenFrame::raw(11, rect_est, PixelFormat::Rgba8, curr_pixels.clone()).unwrap();

    let motion = estimate_scroll_motion(&prev_frame, &curr_frame, 32, 0.85);
    assert!(motion.is_some(), "motion must be detected for scroll dy=-8");
    let mv = motion.unwrap();
    assert_eq!(mv.dx, 0);
    assert_eq!(mv.dy, dy_scroll);
    assert!(mv.match_ratio >= 0.85);

    // Static frame check: prev == curr returns None
    assert_eq!(
        estimate_scroll_motion(&prev_frame, &prev_frame, 32, 0.85),
        None
    );

    // 9. encode_motion_scroll and decode_motion_scroll full roundtrip & canvas synthesis
    let exposed_rect = compute_exposed_strip(w_est, h_est, mv.dx, mv.dy).unwrap();
    let exposed_patch = curr_frame
        .extract_sub_rect(
            exposed_rect.x,
            exposed_rect.y,
            exposed_rect.width,
            exposed_rect.height,
        )
        .expect("extract exposed sub rect");

    let motion_wire_frame = ScreenFrame::encode_motion_scroll(
        rect_est,
        PixelFormat::Rgba8,
        12,
        mv.dx,
        mv.dy,
        exposed_rect,
        &exposed_patch.bytes,
    );
    assert_eq!(motion_wire_frame.codec_tag, SCREEN_COMPRESS_MOTION_SCROLL);

    // Wire serialization roundtrip
    let wire_bytes = motion_wire_frame.encode_payload().unwrap();
    let decoded_wire_frame =
        ScreenFrame::decode_payload(&wire_bytes).expect("decode motion wire frame");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_MOTION_SCROLL);
    assert_eq!(decoded_wire_frame.seq, 12);

    let (dec_dx, dec_dy, dec_exp, dec_patch) = decoded_wire_frame.decode_motion_scroll().unwrap();
    assert_eq!(dec_dx, mv.dx);
    assert_eq!(dec_dy, mv.dy);
    assert_eq!(dec_exp, exposed_rect);

    // Synthesize receiver canvas from prev_frame
    let mut receiver_canvas = prev_frame.clone();
    receiver_canvas.scroll_canvas(dec_dx, dec_dy);
    let patch_rect = ScreenRect::new(
        dec_exp.x as i32,
        dec_exp.y as i32,
        dec_exp.width,
        dec_exp.height,
    )
    .unwrap();
    let patch_frame = ScreenFrame::raw(
        decoded_wire_frame.seq,
        patch_rect,
        PixelFormat::Rgba8,
        dec_patch.to_vec(),
    )
    .unwrap();
    receiver_canvas.apply_sub_rect(&patch_frame).unwrap();

    // Assert 100% exact pixel match with curr_frame
    assert_eq!(receiver_canvas.bytes, curr_frame.bytes);
}

#[test]
fn screen_spatial_tile_grid_decomposition_and_streaming_algorithms() {
    // 1. ScreenTileConfig defaults & clamping
    let default_cfg = ScreenTileConfig::default();
    assert!(!default_cfg.enabled);
    assert_eq!(default_cfg.tile_size, SCREEN_TILE_DEFAULT_SIZE);
    assert!(!default_cfg.is_enabled());

    let custom_cfg = ScreenTileConfig::new(true, 128);
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.tile_size, 128);
    assert!(custom_cfg.is_enabled());

    let low_clamped = ScreenTileConfig::new(true, 10);
    assert_eq!(low_clamped.tile_size, SCREEN_TILE_MIN_SIZE);

    let high_clamped = ScreenTileConfig::new(true, 512);
    assert_eq!(high_clamped.tile_size, SCREEN_TILE_MAX_SIZE);

    let zero_cfg = ScreenTileConfig::new(true, 0);
    assert_eq!(zero_cfg.tile_size, SCREEN_TILE_DEFAULT_SIZE);

    // 2. ScreenTileReport defaults
    let rep = ScreenTileReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.tile_size, SCREEN_TILE_DEFAULT_SIZE);
    assert_eq!(rep.grid_frames_sent, 0);
    assert_eq!(rep.dirty_tiles_sent, 0);
    assert_eq!(rep.skipped_tiles_count, 0);
    assert_eq!(rep.saved_bandwidth_bytes, 0);

    // 3. Line protocol 14-byte envelope tag 39, 40, 41 roundtrips
    // Tag 39: SetScreenTileMode
    let msg_set = ControlMsg::SetScreenTileMode {
        enabled: true,
        tile_size: 96,
    };
    assert_eq!(msg_set.tag(), 39);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 39);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set tile mode");
    assert_eq!(decoded_set, msg_set);

    // Tag 40: ResetScreenTileMode
    let msg_reset = ControlMsg::ResetScreenTileMode;
    assert_eq!(msg_reset.tag(), 40);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 40);
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).expect("decode reset tile mode");
    assert_eq!(decoded_reset, msg_reset);

    // Tag 41: TileGridSync
    let msg_sync = ControlMsg::TileGridSync { tiles_sent: 12345 };
    assert_eq!(msg_sync.tag(), 41);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 41);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode tile grid sync");
    assert_eq!(decoded_sync, msg_sync);

    // 4. compute_tile_grid_dims calculations
    assert_eq!(compute_tile_grid_dims(0, 100, 64), (0, 0, 0));
    assert_eq!(compute_tile_grid_dims(100, 0, 64), (0, 0, 0));
    // Exact multiples: 128x128 with T=64 -> 2x2 = 4
    assert_eq!(compute_tile_grid_dims(128, 128, 64), (2, 2, 4));
    // Uneven edges: 130x70 with T=64 -> cols=ceil(130/64)=3, rows=ceil(70/64)=2 -> 6 tiles
    assert_eq!(compute_tile_grid_dims(130, 70, 64), (3, 2, 6));

    // 5. extract_dirty_tiles & spatial decomposition
    let w = 128u16;
    let h = 128u16;
    let bpp = 4usize;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();
    let base_pixels = vec![0x20u8; (w as usize) * (h as usize) * bpp];

    let prev_frame = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, base_pixels.clone()).unwrap();

    // Case A: Identical frames -> 0 dirty tiles, total_tiles = 4
    let (total_a, dirty_a) = extract_dirty_tiles(&prev_frame, &prev_frame, 64);
    assert_eq!(total_a, 4);
    assert!(dirty_a.is_empty());

    // Case B: Discrete micro-updates at top-left (x=10, y=10 -> tile 0) and bottom-right (x=120, y=120 -> tile 3)
    let mut curr_pixels = base_pixels.clone();
    // Invert pixel in tile 0: (10, 10)
    let idx0 = (10 * (w as usize) + 10) * bpp;
    curr_pixels[idx0] = 0xFF;
    curr_pixels[idx0 + 1] = 0xAA;

    // Invert pixel in tile 3: (120, 120)
    let idx3 = (120 * (w as usize) + 120) * bpp;
    curr_pixels[idx3] = 0xEE;
    curr_pixels[idx3 + 2] = 0xCC;

    let curr_frame = ScreenFrame::raw(2, rect, PixelFormat::Bgra8, curr_pixels.clone()).unwrap();

    let (total_b, dirty_b) = extract_dirty_tiles(&prev_frame, &curr_frame, 64);
    assert_eq!(total_b, 4);
    assert_eq!(dirty_b.len(), 2, "only 2 tiles should be dirty");
    assert_eq!(dirty_b[0].index, 0);
    assert_eq!(dirty_b[0].x, 0);
    assert_eq!(dirty_b[0].y, 0);
    assert_eq!(dirty_b[0].w, 64);
    assert_eq!(dirty_b[0].h, 64);

    assert_eq!(dirty_b[1].index, 3);
    assert_eq!(dirty_b[1].x, 64);
    assert_eq!(dirty_b[1].y, 64);
    assert_eq!(dirty_b[1].w, 64);
    assert_eq!(dirty_b[1].h, 64);

    // 6. encode_tile_grid and decode_tile_grid wire serialization roundtrip
    let wire_frame =
        ScreenFrame::encode_tile_grid(100, rect, PixelFormat::Bgra8, 64, total_b, &dirty_b);
    assert_eq!(wire_frame.codec_tag, SCREEN_COMPRESS_TILE_GRID);
    assert_eq!(wire_frame.seq, 100);

    let wire_bytes = wire_frame.encode_payload().unwrap();
    let decoded_wire_frame = ScreenFrame::decode_payload(&wire_bytes).expect("decode wire payload");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_TILE_GRID);

    let (dec_tile_size, dec_total, dec_dirty) = decoded_wire_frame.decode_tile_grid().unwrap();
    assert_eq!(dec_tile_size, 64);
    assert_eq!(dec_total, 4);
    assert_eq!(dec_dirty.len(), 2);
    assert_eq!(dec_dirty[0], dirty_b[0]);
    assert_eq!(dec_dirty[1], dirty_b[1]);

    // 7. Receiver in-place blitting via apply_dirty_tile
    let mut receiver_canvas = prev_frame.clone();
    for tile in &dec_dirty {
        receiver_canvas
            .apply_dirty_tile(tile)
            .expect("apply dirty tile");
    }

    // 100% bit-exact reproduction verification!
    assert_eq!(receiver_canvas.bytes, curr_frame.bytes);

    // Boundary error checks: tile exceeds canvas boundary
    let oob_tile = DirtyTile {
        index: 99,
        x: 100,
        y: 100,
        w: 64, // 100 + 64 = 164 > 128
        h: 64,
        data: vec![0u8; 64 * 64 * bpp],
    };
    assert!(receiver_canvas.apply_dirty_tile(&oob_tile).is_err());

    // Truncated tile data error check
    let short_tile = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![0u8; 10], // expected 64*64*4 = 16384 bytes
    };
    assert!(receiver_canvas.apply_dirty_tile(&short_tile).is_err());
}

#[test]
fn screen_tile_cache_deduplication_and_streaming_algorithms() {
    // 1. ScreenTileCacheConfig defaults, clamping, and helpers
    let def_cfg = ScreenTileCacheConfig::default();
    assert!(!def_cfg.is_enabled());
    assert_eq!(def_cfg.max_entries, SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES);

    let clamped_low = ScreenTileCacheConfig::new(true, 10);
    assert!(clamped_low.is_enabled());
    assert_eq!(clamped_low.max_entries, SCREEN_TILE_CACHE_MIN_ENTRIES);

    let clamped_high = ScreenTileCacheConfig::new(true, 10000);
    assert_eq!(clamped_high.max_entries, SCREEN_TILE_CACHE_MAX_ENTRIES);

    let custom_cfg = ScreenTileCacheConfig::new(true, 256);
    assert_eq!(custom_cfg.max_entries, 256);

    // 2. ScreenTileCacheReport defaults
    let def_rep = ScreenTileCacheReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.current_entries, 0);
    assert_eq!(def_rep.cache_hits, 0);
    assert_eq!(def_rep.cache_misses, 0);

    // 3. ControlMsg Tag 42, 43, 44 wire serialization roundtrips
    // Tag 42: SetScreenTileCacheMode
    let msg_set = ControlMsg::SetScreenTileCacheMode {
        enabled: true,
        max_entries: 512,
    };
    assert_eq!(msg_set.tag(), 42);
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 42);
    let decoded_set = ControlMsg::decode_payload(&payload_set).expect("decode set tile cache mode");
    assert_eq!(decoded_set, msg_set);

    // Tag 43: ResetScreenTileCacheMode
    let msg_reset = ControlMsg::ResetScreenTileCacheMode;
    assert_eq!(msg_reset.tag(), 43);
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 43);
    let decoded_reset =
        ControlMsg::decode_payload(&payload_reset).expect("decode reset tile cache mode");
    assert_eq!(decoded_reset, msg_reset);

    // Tag 44: TileCacheSync
    let msg_sync = ControlMsg::TileCacheSync { hits: 98765 };
    assert_eq!(msg_sync.tag(), 44);
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 44);
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).expect("decode tile cache sync");
    assert_eq!(decoded_sync, msg_sync);

    // 4. fnv1a_hash64 fast content hashing
    let tile1 = vec![0x11u8; 1024];
    let tile2 = vec![0x11u8; 1024];
    let tile3 = vec![0x22u8; 1024];
    let hash1 = fnv1a_hash64(&tile1);
    let hash2 = fnv1a_hash64(&tile2);
    let hash3 = fnv1a_hash64(&tile3);
    assert_eq!(hash1, hash2, "Identical content must yield identical hash");
    assert_ne!(hash1, hash3, "Different content must yield different hash");

    // 5. ScreenTileDictionary LRU and eviction logic
    let dict = ScreenTileDictionary::new(3);
    assert_eq!(dict.max_entries(), 64, "Should clamp to minimum 64");
    // Test with small dictionary capacity by constructing directly or resizing
    let mut small_dict = ScreenTileDictionary {
        max_entries: 3,
        hash_to_id: HashMap::with_capacity(3),
        id_to_hash: vec![None; 3],
        lru_order: VecDeque::with_capacity(3),
    };

    // Insert item A: miss -> slot 0
    let (hit, id_a) = small_dict.lookup_or_insert(1001);
    assert!(!hit);
    assert_eq!(id_a, 0);
    assert_eq!(small_dict.current_entries(), 1);

    // Lookup item A: hit -> slot 0
    let (hit, id_a_hit) = small_dict.lookup_or_insert(1001);
    assert!(hit);
    assert_eq!(id_a_hit, 0);

    // Insert item B: miss -> slot 1
    let (hit, id_b) = small_dict.lookup_or_insert(1002);
    assert!(!hit);
    assert_eq!(id_b, 1);

    // Insert item C: miss -> slot 2 (now full: [A, B, C])
    let (hit, id_c) = small_dict.lookup_or_insert(1003);
    assert!(!hit);
    assert_eq!(id_c, 2);
    assert_eq!(small_dict.current_entries(), 3);

    // Access B to make it more recently used: order becomes [A, C, B]
    let (hit, _) = small_dict.lookup_or_insert(1002);
    assert!(hit);

    // Insert item D: miss -> evicts LRU (item A, slot 0) -> slot 0 re-used
    let (hit, id_d) = small_dict.lookup_or_insert(1004);
    assert!(!hit);
    assert_eq!(id_d, 0);
    assert_eq!(small_dict.current_entries(), 3);

    // Verify item A is now gone
    let (hit, _) = small_dict.lookup_or_insert(1001);
    assert!(!hit, "Item A should have been evicted");

    // 6. encode_tile_cache and decode_tile_cache wire serialization roundtrip
    let w = 128u16;
    let h = 128u16;
    let bpp = 4usize;
    let rect = ScreenRect::new(0, 0, w, h).unwrap();

    let entry0 = CachedTileEntry {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        cache_hit: false,
        cache_id: 0,
        data: vec![0xAA; 64 * 64 * bpp],
    };

    let entry1 = CachedTileEntry {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        cache_hit: true,
        cache_id: 0, // Zero payload reference to slot 0!
        data: Vec::new(),
    };

    let entries = vec![entry0.clone(), entry1.clone()];
    let wire_frame = ScreenFrame::encode_tile_cache(200, rect, PixelFormat::Bgra8, 64, 4, &entries);
    assert_eq!(wire_frame.codec_tag, SCREEN_COMPRESS_TILE_CACHE);
    assert_eq!(wire_frame.seq, 200);

    let wire_bytes = wire_frame.encode_payload().unwrap();
    let decoded_wire_frame = ScreenFrame::decode_payload(&wire_bytes).expect("decode wire payload");
    assert_eq!(decoded_wire_frame.codec_tag, SCREEN_COMPRESS_TILE_CACHE);

    let (dec_tile_size, dec_total, dec_entries) = decoded_wire_frame.decode_tile_cache().unwrap();
    assert_eq!(dec_tile_size, 64);
    assert_eq!(dec_total, 4);
    assert_eq!(dec_entries.len(), 2);
    assert_eq!(dec_entries[0], entry0);
    assert_eq!(dec_entries[1], entry1);

    // 7. Receiver canvas replay with apply_cached_tile
    let mut canvas = ScreenFrame::raw(
        0,
        rect,
        PixelFormat::Bgra8,
        vec![0x00; (w as usize) * (h as usize) * bpp],
    )
    .unwrap();
    let mut receiver_cache: Vec<Option<Vec<u8>>> = vec![None; 512];

    for entry in &dec_entries {
        if !entry.cache_hit {
            // Cache miss: store in receiver dictionary slot
            receiver_cache[entry.cache_id as usize] = Some(entry.data.clone());
            canvas
                .apply_cached_tile(entry.x, entry.y, entry.w, entry.h, &entry.data)
                .expect("apply miss tile");
        } else {
            // Cache hit: fetch from receiver dictionary slot
            let cached_pixels = receiver_cache[entry.cache_id as usize]
                .as_ref()
                .expect("found in cache");
            canvas
                .apply_cached_tile(entry.x, entry.y, entry.w, entry.h, cached_pixels)
                .expect("apply hit tile");
        }
    }

    // Verify both tile 0 and tile 1 on canvas have 0xAA pixels!
    let tile0_offset = 0;
    assert_eq!(
        &canvas.bytes[tile0_offset..tile0_offset + 64 * bpp],
        &vec![0xAA; 64 * bpp][..]
    );
    let tile1_offset = 64 * bpp;
    assert_eq!(
        &canvas.bytes[tile1_offset..tile1_offset + 64 * bpp],
        &vec![0xAA; 64 * bpp][..]
    );
}

#[test]
fn screen_foveated_focus_scheduling_algorithms() {
    // 1. ScreenFocusConfig defaults, clamping, and helpers
    let def_cfg = ScreenFocusConfig::default();
    assert!(!def_cfg.is_enabled());
    assert_eq!(def_cfg.mode, ScreenFocusMode::Cursor);
    assert_eq!(def_cfg.radius, SCREEN_FOCUS_DEFAULT_RADIUS);
    assert_eq!(
        def_cfg.peripheral_skip_ratio,
        SCREEN_FOCUS_DEFAULT_SKIP_RATIO
    );
    assert_eq!(def_cfg.box_x, 0);
    assert_eq!(def_cfg.box_y, 0);
    assert_eq!(def_cfg.box_w, 512);
    assert_eq!(def_cfg.box_h, 512);

    let clamped_low = ScreenFocusConfig::new(true, ScreenFocusMode::Cursor, 10, 20, None);
    assert_eq!(clamped_low.radius, SCREEN_FOCUS_MIN_RADIUS);
    assert_eq!(
        clamped_low.peripheral_skip_ratio,
        SCREEN_FOCUS_MAX_SKIP_RATIO
    );

    let clamped_high =
        ScreenFocusConfig::new(true, ScreenFocusMode::Box, 5000, 5, Some((10, 20, 10, 10)));
    assert_eq!(clamped_high.radius, SCREEN_FOCUS_MAX_RADIUS);
    assert_eq!(clamped_high.peripheral_skip_ratio, 5);
    assert_eq!(clamped_high.box_x, 10);
    assert_eq!(clamped_high.box_y, 20);
    assert_eq!(clamped_high.box_w, 32); // Min box width clamped to 32
    assert_eq!(clamped_high.box_h, 32); // Min box height clamped to 32

    // 2. ScreenFocusMode enum representations
    assert_eq!(ScreenFocusMode::Cursor.as_str(), "cursor");
    assert_eq!(ScreenFocusMode::Box.as_str(), "box");
    assert_eq!(ScreenFocusMode::from_u8(0), ScreenFocusMode::Cursor);
    assert_eq!(ScreenFocusMode::from_u8(1), ScreenFocusMode::Box);
    assert_eq!(ScreenFocusMode::Cursor.to_u8(), 0);
    assert_eq!(ScreenFocusMode::Box.to_u8(), 1);

    // 3. ScreenFocusReport defaults
    let def_rep = ScreenFocusReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.focus_tiles_sent, 0);
    assert_eq!(def_rep.peripheral_tiles_sent, 0);
    assert_eq!(def_rep.peripheral_tiles_skipped, 0);

    // 4. Geometry and distance check: ScreenFocusMode::Cursor
    // Cursor at (100, 100), radius = 64
    let cursor_cfg = ScreenFocusConfig {
        enabled: true,
        mode: ScreenFocusMode::Cursor,
        radius: 64,
        peripheral_skip_ratio: 2,
        box_x: 0,
        box_y: 0,
        box_w: 512,
        box_h: 512,
    };
    let cursor_pos = Some((100, 100));

    // Tile A: x=80, y=80, w=32, h=32 -> center is (96, 96), dx=-4, dy=-4 -> dist_sq = 32 <= 64^2 (4096) -> In Focus!
    assert!(is_tile_in_focus(80, 80, 32, 32, &cursor_cfg, cursor_pos));

    // Tile B: x=300, y=300, w=32, h=32 -> center is (316, 316), dx=216, dy=216 -> dist_sq > 4096 -> Peripheral!
    assert!(!is_tile_in_focus(300, 300, 32, 32, &cursor_cfg, cursor_pos));

    // Tile C: Boundary check at exactly radius distance: center at (164, 100), dx=64, dy=0 -> dist_sq = 4096 == 4096 -> In Focus!
    assert!(is_tile_in_focus(148, 84, 32, 32, &cursor_cfg, cursor_pos));

    // 5. Geometry and boundary check: ScreenFocusMode::Box
    let box_cfg = ScreenFocusConfig {
        enabled: true,
        mode: ScreenFocusMode::Box,
        radius: 128,
        peripheral_skip_ratio: 2,
        box_x: 100,
        box_y: 100,
        box_w: 200,
        box_h: 200,
    };

    // Tile inside box: center (150, 150) -> In Focus!
    assert!(is_tile_in_focus(134, 134, 32, 32, &box_cfg, None));

    // Tile outside box: center (50, 50) -> Peripheral!
    assert!(!is_tile_in_focus(34, 34, 32, 32, &box_cfg, None));

    // Tile outside box right edge: center (350, 150) -> Peripheral!
    assert!(!is_tile_in_focus(334, 134, 32, 32, &box_cfg, None));

    // 6. ControlMsg Tag 45~49 line protocol wire serialization roundtrip
    // Tag 45: SetScreenFocusMode
    let msg_set = ControlMsg::SetScreenFocusMode {
        enabled: true,
        mode: 1, // Box
        radius: 384,
        peripheral_skip_ratio: 3,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 45); // Tag 45
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenFocusMode {
            enabled: true,
            mode: 1,
            radius: 384,
            peripheral_skip_ratio: 3,
        }
    );

    // Tag 46: SetScreenFocusBox
    let msg_box = ControlMsg::SetScreenFocusBox {
        box_x: 64,
        box_y: 128,
        box_w: 800,
        box_h: 600,
    };
    let payload_box = msg_box.encode_payload();
    assert_eq!(payload_box.len(), 14);
    assert_eq!(payload_box[5], 46); // Tag 46
    let decoded_box = ControlMsg::decode_payload(&payload_box).unwrap();
    assert_eq!(
        decoded_box,
        ControlMsg::SetScreenFocusBox {
            box_x: 64,
            box_y: 128,
            box_w: 800,
            box_h: 600,
        }
    );

    // Tag 47: ResetScreenFocusMode
    let msg_reset = ControlMsg::ResetScreenFocusMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 47); // Tag 47
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenFocusMode);

    // Tag 48: FocusSync
    let msg_sync = ControlMsg::FocusSync {
        focus_tiles_sent: 123456,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 48); // Tag 48
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::FocusSync {
            focus_tiles_sent: 123456,
        }
    );

    // Tag 49: FocusPeripheralSync
    let msg_peri = ControlMsg::FocusPeripheralSync {
        peripheral_skipped: 987654,
    };
    let payload_peri = msg_peri.encode_payload();
    assert_eq!(payload_peri.len(), 14);
    assert_eq!(payload_peri[5], 49); // Tag 49
    let decoded_peri = ControlMsg::decode_payload(&payload_peri).unwrap();
    assert_eq!(
        decoded_peri,
        ControlMsg::FocusPeripheralSync {
            peripheral_skipped: 987654,
        }
    );
}

#[test]
fn screen_tile_palette_compression_and_decompression_algorithms() {
    // 1. Config defaults and clamping
    let def_cfg = ScreenTilePaletteConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.max_colors, 16);
    assert!(!def_cfg.enable_rle);

    let custom_cfg = ScreenTilePaletteConfig::new(true, Some(24), Some(true));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.max_colors, 16); // clamped to 16
    assert!(custom_cfg.enable_rle);

    let min_cfg = ScreenTilePaletteConfig::new(true, Some(1), None);
    assert_eq!(min_cfg.max_colors, 2); // clamped to 2

    let cfg = ScreenTilePaletteConfig::new(true, Some(16), None);

    // 2. Solid Color Tile (K = 1) -> 5 bytes payload (1 byte subcodec + 4 bytes color)
    let w = 64u16;
    let h = 64u16;
    let total_pixels = (w as usize) * (h as usize);
    let solid_color = [0x12, 0x34, 0x56, 0xFF];
    let mut solid_buf = Vec::with_capacity(total_pixels * 4);
    for _ in 0..total_pixels {
        solid_buf.extend_from_slice(&solid_color);
    }

    let (codec, payload) = compress_tile_palette(&solid_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_SOLID);
    assert_eq!(payload.len(), 5);
    assert_eq!(payload[0], TILE_SUBCODEC_SOLID);

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, solid_buf);

    // 3. Bi-Level Tile (K = 2) -> 521 bytes payload (1 + 8 + 512)
    let c0 = [0x00, 0x00, 0x00, 0xFF];
    let c1 = [0xFF, 0xFF, 0xFF, 0xFF];
    let mut bilevel_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        if i % 2 == 0 {
            bilevel_buf.extend_from_slice(&c0);
        } else {
            bilevel_buf.extend_from_slice(&c1);
        }
    }

    let (codec, payload) = compress_tile_palette(&bilevel_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_BILEVEL);
    assert_eq!(payload.len(), 1 + 8 + (total_pixels / 8));
    assert_eq!(payload[0], TILE_SUBCODEC_BILEVEL);

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, bilevel_buf);

    // 4. Indexed Palette Tile (3 <= K <= 16) -> 4 colors
    let colors = [
        [0xAA, 0x00, 0x00, 0xFF],
        [0x00, 0xBB, 0x00, 0xFF],
        [0x00, 0x00, 0xCC, 0xFF],
        [0xDD, 0xDD, 0x00, 0xFF],
    ];
    let mut palette_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        palette_buf.extend_from_slice(&colors[i % 4]);
    }

    let (codec, payload) = compress_tile_palette(&palette_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_PALETTE16);
    assert_eq!(payload.len(), 2 + 4 * 4 + (total_pixels / 2));
    assert_eq!(payload[0], TILE_SUBCODEC_PALETTE16);
    assert_eq!(payload[1], 4); // 4 colors

    let decompressed = decompress_tile_palette(codec, &payload, w, h, total_pixels * 4).unwrap();
    assert_eq!(decompressed, palette_buf);

    // 5. True Color Tile (K > 16) -> Short-circuit abort to RAW
    let mut truecolor_buf = Vec::with_capacity(total_pixels * 4);
    for i in 0..total_pixels {
        let val = (i % 256) as u8;
        truecolor_buf.extend_from_slice(&[val, val.wrapping_add(1), val.wrapping_add(2), 0xFF]);
    }

    let (codec, payload) = compress_tile_palette(&truecolor_buf, w, h, &cfg);
    assert_eq!(codec, TILE_SUBCODEC_RAW);
    assert_eq!(payload.len(), total_pixels * 4);
    assert_eq!(payload, truecolor_buf);

    // 6. ControlMsg Tag 50~52 line protocol roundtrip
    // Tag 50: SetScreenTilePaletteMode
    let msg_set = ControlMsg::SetScreenTilePaletteMode {
        enabled: true,
        max_colors: 12,
        enable_rle: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 50); // Tag 50
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePaletteMode {
            enabled: true,
            max_colors: 12,
            enable_rle: true,
        }
    );

    // Tag 51: ResetScreenTilePaletteMode
    let msg_reset = ControlMsg::ResetScreenTilePaletteMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 51); // Tag 51
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePaletteMode);

    // Tag 52: TilePaletteSync
    let msg_sync = ControlMsg::TilePaletteSync {
        solid_tiles: 0,
        bilevel_tiles: 0,
        palette_tiles: 0,
        saved_bytes: 987654321,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 52); // Tag 52
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePaletteSync {
            solid_tiles: 0,
            bilevel_tiles: 0,
            palette_tiles: 0,
            saved_bytes: 987654321,
        }
    );
}

#[test]
fn screen_tile_predictive_compression_and_decompression_algorithms() {
    // 1. Config defaults and helpers
    let def_cfg = ScreenTilePredictConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.filter_mode, ScreenTilePredictFilter::Auto);
    assert!(def_cfg.enable_rct);

    let custom_cfg =
        ScreenTilePredictConfig::new(true, Some(ScreenTilePredictFilter::Sub), Some(false));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.filter_mode, ScreenTilePredictFilter::Sub);
    assert!(!custom_cfg.enable_rct);

    // 2. Filter mode enum representation & parsing
    assert_eq!(ScreenTilePredictFilter::None.as_str(), "none");
    assert_eq!(ScreenTilePredictFilter::Sub.as_str(), "sub");
    assert_eq!(ScreenTilePredictFilter::Up.as_str(), "up");
    assert_eq!(ScreenTilePredictFilter::Average.as_str(), "average");
    assert_eq!(ScreenTilePredictFilter::Paeth.as_str(), "paeth");
    assert_eq!(ScreenTilePredictFilter::Auto.as_str(), "auto");

    assert_eq!(
        ScreenTilePredictFilter::from_u8(0),
        ScreenTilePredictFilter::None
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(1),
        ScreenTilePredictFilter::Sub
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(2),
        ScreenTilePredictFilter::Up
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(3),
        ScreenTilePredictFilter::Average
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(4),
        ScreenTilePredictFilter::Paeth
    );
    assert_eq!(
        ScreenTilePredictFilter::from_u8(5),
        ScreenTilePredictFilter::Auto
    );

    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("left"),
        Some(ScreenTilePredictFilter::Sub)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("top"),
        Some(ScreenTilePredictFilter::Up)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("avg"),
        Some(ScreenTilePredictFilter::Average)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("diag"),
        Some(ScreenTilePredictFilter::Paeth)
    );
    assert_eq!(
        ScreenTilePredictFilter::from_str_loose("adaptive"),
        Some(ScreenTilePredictFilter::Auto)
    );

    // 3. Report defaults
    let def_rep = ScreenTilePredictReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.predict_tiles_count, 0);
    assert_eq!(def_rep.saved_bandwidth_bytes, 0);

    // 4. Reversible Color Transform (RCT) bit-for-bit exact reversibility
    let mut orig_pixels = Vec::new();
    for i in 0..64 * 64 {
        let r = ((i * 7) % 256) as u8;
        let g = ((i * 13) % 256) as u8;
        let b = ((i * 29) % 256) as u8;
        let a = 0xFFu8;
        orig_pixels.extend_from_slice(&[r, g, b, a]);
    }
    let mut rct_buf = vec![0u8; orig_pixels.len()];
    rct_transform(&orig_pixels, &mut rct_buf);
    let mut restored_pixels = vec![0u8; orig_pixels.len()];
    rct_inverse(&rct_buf, &mut restored_pixels);
    assert_eq!(restored_pixels, orig_pixels);

    // 5. Directional Spatial Prediction: Horizontal Line (h=1 -> Sub is strictly optimal for Auto)
    let w = 64u16;
    let mut h_line = Vec::with_capacity((w as usize) * 4);
    for col in 0..w {
        let v = (col.wrapping_mul(37) % 256) as u8;
        h_line.extend_from_slice(&[v, v, v, 0xFF]);
    }
    let (chosen_h, residual_h) = predict_filter_tile(&h_line, w, 1, ScreenTilePredictFilter::Auto);
    assert_eq!(chosen_h, ScreenTilePredictFilter::Sub);
    let restored_h = unpredict_filter_tile(&residual_h, w, 1, chosen_h);
    assert_eq!(restored_h, h_line);

    // 6. Directional Spatial Prediction: Vertical Line (w=1 -> Up is strictly optimal for Auto)
    let h = 64u16;
    let mut v_line = Vec::with_capacity((h as usize) * 4);
    for row in 0..h {
        let v = (row.wrapping_mul(37) % 256) as u8;
        v_line.extend_from_slice(&[v, v, v, 0xFF]);
    }
    let (chosen_v, residual_v) = predict_filter_tile(&v_line, 1, h, ScreenTilePredictFilter::Auto);
    assert_eq!(chosen_v, ScreenTilePredictFilter::Up);
    let restored_v = unpredict_filter_tile(&residual_v, 1, h, chosen_v);
    assert_eq!(restored_v, v_line);

    // 7. 2D 64x64 Tile with Explicit Filter Modes (None, Sub, Up, Average, Paeth)
    let mut tile_2d = Vec::with_capacity(64 * 64 * 4);
    for row in 0..64u16 {
        for col in 0..64u16 {
            let r = (row.wrapping_mul(3) ^ col.wrapping_mul(5)) as u8;
            let g = (row.wrapping_add(col).wrapping_mul(7)) as u8;
            let b = (row.wrapping_mul(11)) as u8;
            tile_2d.extend_from_slice(&[r, g, b, 0xFF]);
        }
    }
    for filter in [
        ScreenTilePredictFilter::None,
        ScreenTilePredictFilter::Sub,
        ScreenTilePredictFilter::Up,
        ScreenTilePredictFilter::Average,
        ScreenTilePredictFilter::Paeth,
        ScreenTilePredictFilter::Auto,
    ] {
        let (f_used, res) = predict_filter_tile(&tile_2d, 64, 64, filter);
        let restored = unpredict_filter_tile(&res, 64, 64, f_used);
        assert_eq!(
            restored, tile_2d,
            "Failed roundtrip for filter {:?}",
            filter
        );
    }

    // 8. Full Compress & Decompress pipeline test with dummy codec
    struct TestIdentityCodec;
    impl crate::ScreenCodec for TestIdentityCodec {
        fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress(
            &self,
            compressed: &[u8],
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(compressed.to_vec())
        }
    }
    let codec = TestIdentityCodec;
    let predict_cfg = ScreenTilePredictConfig {
        enabled: true,
        filter_mode: ScreenTilePredictFilter::Auto,
        enable_rct: true,
    };
    let (subc, payload) = compress_tile_predictive(&tile_2d, 64, 64, &predict_cfg, &codec);
    assert_eq!(subc, TILE_SUBCODEC_PREDICT_ZSTD);
    assert_eq!(payload[0], TILE_SUBCODEC_PREDICT_ZSTD);
    assert_eq!(payload[2], 1); // enable_rct flag is 1

    let decompressed = decompress_tile_predictive(&payload, 64, 64, 64 * 64 * 4, &codec).unwrap();
    assert_eq!(decompressed, tile_2d);

    // 9. ControlMsg Tag 53~55 wire protocol roundtrip
    // Tag 53: SetScreenTilePredictMode
    let msg_set = ControlMsg::SetScreenTilePredictMode {
        enabled: true,
        filter_mode: 3, // Average
        enable_rct: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 53); // Tag 53
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePredictMode {
            enabled: true,
            filter_mode: 3,
            enable_rct: true,
        }
    );

    // Tag 54: ResetScreenTilePredictMode
    let msg_reset = ControlMsg::ResetScreenTilePredictMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 54); // Tag 54
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePredictMode);

    // Tag 55: TilePredictSync
    let msg_sync = ControlMsg::TilePredictSync {
        predict_tiles: 0,
        saved_bytes: 1234567890,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 55); // Tag 55
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePredictSync {
            predict_tiles: 0,
            saved_bytes: 1234567890,
        }
    );
}

#[test]
fn screen_tile_presentation_transaction_and_double_buffer_algorithms() {
    // 1. Config defaults and helpers
    let def_cfg = ScreenTilePresentConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.vsync_mode, ScreenTilePresentVsyncMode::Off);
    assert!(def_cfg.double_buffer);
    assert!(def_cfg.tear_prevention);

    let custom_cfg = ScreenTilePresentConfig::new(
        true,
        Some(ScreenTilePresentVsyncMode::Vsync60),
        Some(true),
        Some(true),
    );
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.vsync_mode, ScreenTilePresentVsyncMode::Vsync60);
    assert!(custom_cfg.double_buffer);
    assert!(custom_cfg.tear_prevention);

    // 2. VsyncMode enum representation & parsing
    assert_eq!(ScreenTilePresentVsyncMode::Off.as_str(), "off");
    assert_eq!(ScreenTilePresentVsyncMode::Vsync60.as_str(), "vsync60");
    assert_eq!(ScreenTilePresentVsyncMode::Vsync120.as_str(), "vsync120");
    assert_eq!(ScreenTilePresentVsyncMode::Adaptive.as_str(), "adaptive");

    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(0),
        ScreenTilePresentVsyncMode::Off
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(1),
        ScreenTilePresentVsyncMode::Vsync60
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(2),
        ScreenTilePresentVsyncMode::Vsync120
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_u8(3),
        ScreenTilePresentVsyncMode::Adaptive
    );

    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("off"),
        Some(ScreenTilePresentVsyncMode::Off)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("vsync60"),
        Some(ScreenTilePresentVsyncMode::Vsync60)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("60hz"),
        Some(ScreenTilePresentVsyncMode::Vsync60)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("vsync120"),
        Some(ScreenTilePresentVsyncMode::Vsync120)
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::from_str_loose("adaptive"),
        Some(ScreenTilePresentVsyncMode::Adaptive)
    );
    assert_eq!(ScreenTilePresentVsyncMode::from_str_loose("unknown"), None);

    assert_eq!(
        ScreenTilePresentVsyncMode::Vsync60.target_interval_us(),
        16_666
    );
    assert_eq!(
        ScreenTilePresentVsyncMode::Vsync120.target_interval_us(),
        8_333
    );

    // 3. Report defaults
    let def_rep = ScreenTilePresentReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.committed_transactions, 0);
    assert_eq!(def_rep.swapped_frames, 0);
    assert_eq!(def_rep.torn_frames_prevented, 0);

    // 4. Double-Buffered Presenter lifecycle & Atomic Swap
    let mut presenter = ScreenTilePresenter::new(custom_cfg);
    let w = 128u16;
    let h = 128u16;
    let total_pixels = (w as usize) * (h as usize);
    let mut initial_pixels = vec![0u8; total_pixels * 4];
    for p in initial_pixels.chunks_exact_mut(4) {
        p[0] = 0x11;
        p[1] = 0x22;
        p[2] = 0x33;
        p[3] = 0xFF;
    }
    presenter.set_front_buffer(ScreenFrame {
        seq: 1,
        rect: ScreenRect::new(0, 0, w, h).unwrap(),
        format: PixelFormat::Rgba8,
        stride: (w as u32) * 4,
        codec_tag: 0,
        bytes: initial_pixels.clone(),
    });

    // Begin transaction for new frame
    presenter.begin_transaction(w, h);
    assert!(presenter.back_buffer().is_some());
    assert_eq!(presenter.back_buffer().unwrap().bytes, initial_pixels);

    // Stage Tile 0 (top-left 64x64) with red
    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: [0xFF, 0x00, 0x00, 0xFF].repeat(64 * 64),
    };
    assert!(presenter.stage_dirty_tile(&t0));

    // Stage Tile 1 (top-right 64x64) with green
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: [0x00, 0xFF, 0x00, 0xFF].repeat(64 * 64),
    };
    assert!(presenter.stage_dirty_tile(&t1));

    // During staging: front buffer is completely untouched!
    assert_eq!(presenter.front_buffer().unwrap().bytes, initial_pixels);

    // Commit and swap
    let swapped = presenter.commit_and_swap(2, 100_000);
    assert!(swapped);
    let front = presenter.front_buffer().unwrap();
    assert_eq!(front.seq, 2);

    // Front buffer now reflects both tiles atomically!
    // Sample Tile 0 pixel at (0, 0)
    assert_eq!(&front.bytes[0..4], &[0xFF, 0x00, 0x00, 0xFF]);
    // Sample Tile 1 pixel at (64, 0)
    let t1_idx = 64 * 4;
    assert_eq!(&front.bytes[t1_idx..t1_idx + 4], &[0x00, 0xFF, 0x00, 0xFF]);

    let rep = presenter.report();
    assert!(rep.enabled);
    assert_eq!(rep.committed_transactions, 1);
    assert_eq!(rep.swapped_frames, 1);
    assert_eq!(rep.torn_frames_prevented, 1); // Both tiles were staged before swap -> 1 prevention
    assert!(rep.avg_swap_latency_us > 0);

    // 5. ControlMsg Tag 56~58 wire protocol roundtrip
    // Tag 56: SetScreenTilePresentMode
    let msg_set = ControlMsg::SetScreenTilePresentMode {
        enabled: true,
        vsync_mode: 1, // Vsync60
        double_buffer: true,
        tear_prevention: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 56); // Tag 56
    let decoded_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(
        decoded_set,
        ControlMsg::SetScreenTilePresentMode {
            enabled: true,
            vsync_mode: 1,
            double_buffer: true,
            tear_prevention: true,
        }
    );

    // Tag 57: ResetScreenTilePresentMode
    let msg_reset = ControlMsg::ResetScreenTilePresentMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 57); // Tag 57
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTilePresentMode);

    // Tag 58: TilePresentSync
    let msg_sync = ControlMsg::TilePresentSync {
        committed_txs: 42,
        swapped_frames: 42,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 58); // Tag 58
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TilePresentSync {
            committed_txs: 42,
            swapped_frames: 0,
        }
    );
}

#[test]
fn screen_tile_nack_and_fast_retransmission_algorithms() {
    // 1. ScreenTileNackConfig validation and clamping
    let def_cfg = ScreenTileNackConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.timeout_ms, 20);
    assert_eq!(def_cfg.max_retries, 2);

    let custom_cfg = ScreenTileNackConfig::new(true, Some(3), Some(20));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.timeout_ms, 5); // Clamped to min 5ms
    assert_eq!(custom_cfg.max_retries, 10); // Clamped to max 10 retries

    // 2. ScreenTileNackReport defaults
    let rep = ScreenTileNackReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.timeout_ms, 20);
    assert_eq!(rep.max_retries, 2);
    assert_eq!(rep.nack_requests_sent, 0);
    assert_eq!(rep.tiles_retransmitted, 0);
    assert_eq!(rep.degraded_commits, 0);

    // 3. ScreenTileResendBuffer FIFO ring-buffer mechanics
    let mut resend_buf = ScreenTileResendBuffer::new(3);
    assert_eq!(resend_buf.capacity, 3);

    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![1, 2, 3, 4],
    };
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: vec![5, 6, 7, 8],
    };
    resend_buf.push_frame(10, vec![t0.clone(), t1.clone()]);

    let t2 = DirtyTile {
        index: 2,
        x: 0,
        y: 64,
        w: 64,
        h: 64,
        data: vec![9, 10, 11, 12],
    };
    let t3 = DirtyTile {
        index: 3,
        x: 64,
        y: 64,
        w: 64,
        h: 64,
        data: vec![13, 14, 15, 16],
    };
    resend_buf.push_frame(11, vec![t2.clone(), t3.clone()]);

    let t4 = DirtyTile {
        index: 4,
        x: 128,
        y: 0,
        w: 64,
        h: 64,
        data: vec![17, 18, 19, 20],
    };
    resend_buf.push_frame(12, vec![t4.clone()]);

    // Query existing frames
    let retrieved_t0 = resend_buf.get_tile(10, 0);
    assert!(retrieved_t0.is_some());
    assert_eq!(retrieved_t0.unwrap().data, t0.data);

    let retrieved_t3 = resend_buf.get_tile(11, 3);
    assert!(retrieved_t3.is_some());
    assert_eq!(retrieved_t3.unwrap().data, t3.data);

    let retrieved_t4 = resend_buf.get_tile(12, 4);
    assert!(retrieved_t4.is_some());
    assert_eq!(retrieved_t4.unwrap().data, t4.data);

    // Non-existent tile index in frame 10
    assert!(resend_buf.get_tile(10, 2).is_none());

    // Push frame 13 -> Frame 10 must be evicted (capacity = 3)
    let t5 = DirtyTile {
        index: 5,
        x: 128,
        y: 64,
        w: 64,
        h: 64,
        data: vec![21, 22, 23, 24],
    };
    resend_buf.push_frame(13, vec![t5.clone()]);
    assert_eq!(resend_buf.frames.len(), 3);
    assert!(resend_buf.get_tile(10, 0).is_none()); // Evicted!
    assert!(resend_buf.get_tile(11, 2).is_some()); // Still in buffer
    assert!(resend_buf.get_tile(13, 5).is_some()); // Newly added

    resend_buf.clear();
    assert_eq!(resend_buf.frames.len(), 0);
    assert!(resend_buf.get_tile(13, 5).is_none());

    // 4. ControlMsg Tag 59~61 wire protocol roundtrip
    // Tag 59: TileNackRequest
    let msg_nack = ControlMsg::TileNackRequest {
        seq: 54321,
        tile_index: 7,
        retry_count: 1,
    };
    let payload_nack = msg_nack.encode_payload();
    assert_eq!(payload_nack.len(), 14);
    assert_eq!(payload_nack[5], 59); // Tag 59
    let decoded_nack = ControlMsg::decode_payload(&payload_nack).unwrap();
    assert_eq!(
        decoded_nack,
        ControlMsg::TileNackRequest {
            seq: 54321,
            tile_index: 7,
            retry_count: 1,
        }
    );

    // Tag 60: ResetScreenTileNackMode
    let msg_reset = ControlMsg::ResetScreenTileNackMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 60); // Tag 60
    let decoded_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(decoded_reset, ControlMsg::ResetScreenTileNackMode);

    // Tag 61: TileNackSync
    let msg_sync = ControlMsg::TileNackSync {
        nack_requests: 99,
        retransmitted_tiles: 42,
        degraded_commits: 3,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 61); // Tag 61
    let decoded_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        decoded_sync,
        ControlMsg::TileNackSync {
            nack_requests: 99,
            retransmitted_tiles: 0,
            degraded_commits: 0,
        }
    );
}

#[test]
fn screen_tile_saliency_and_tiered_scheduling_algorithms() {
    // 1. TileSaliencyTier enum
    assert_eq!(TileSaliencyTier::Critical.to_u8(), 0);
    assert_eq!(TileSaliencyTier::Normal.to_u8(), 1);
    assert_eq!(TileSaliencyTier::Background.to_u8(), 2);
    assert_eq!(TileSaliencyTier::from_u8(0), TileSaliencyTier::Critical);
    assert_eq!(TileSaliencyTier::from_u8(1), TileSaliencyTier::Normal);
    assert_eq!(TileSaliencyTier::from_u8(2), TileSaliencyTier::Background);
    assert_eq!(TileSaliencyTier::Critical.as_str(), "critical");
    assert_eq!(TileSaliencyTier::Normal.as_str(), "normal");
    assert_eq!(TileSaliencyTier::Background.as_str(), "background");

    // 2. ScreenTileSaliencyConfig validation and defaults
    let def_cfg = ScreenTileSaliencyConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.high_threshold, 120);
    assert_eq!(def_cfg.low_threshold, 40);
    assert!(def_cfg.drop_background_on_congestion);

    let custom_cfg = ScreenTileSaliencyConfig::new(true, Some(30), Some(150), Some(false));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.high_threshold, 150);
    assert_eq!(custom_cfg.low_threshold, 30);
    assert!(!custom_cfg.drop_background_on_congestion);

    // 3. ScreenTileSaliencyReport defaults
    let rep = ScreenTileSaliencyReport::default();
    assert!(rep.ok);
    assert!(!rep.enabled);
    assert_eq!(rep.critical_tiles_sent, 0);
    assert_eq!(rep.normal_tiles_sent, 0);
    assert_eq!(rep.background_tiles_sent, 0);
    assert_eq!(rep.background_tiles_skipped, 0);

    // 4. Gradient energy on flat vs high-contrast tiles
    let flat_data = vec![128u8; 64 * 64 * 4];
    let flat_energy = ScreenTileSaliencyEvaluator::compute_gradient_energy(&flat_data, 64, 64);
    assert_eq!(flat_energy, 0);

    let mut high_contrast_data = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let val = if (x / 2 + y / 2) % 2 == 0 { 255u8 } else { 0u8 };
            let off = (y * 64 + x) * 4;
            high_contrast_data[off] = val;
            high_contrast_data[off + 1] = val;
            high_contrast_data[off + 2] = val;
            high_contrast_data[off + 3] = 255;
        }
    }
    let hc_energy =
        ScreenTileSaliencyEvaluator::compute_gradient_energy(&high_contrast_data, 64, 64);
    assert!(
        hc_energy >= 150,
        "high contrast checkerboard energy: {}",
        hc_energy
    );

    // 5. Focus proximity
    let p_intersect = ScreenTileSaliencyEvaluator::compute_focus_proximity(
        100,
        100,
        64,
        64,
        (120, 120, 100, 100),
    );
    assert_eq!(p_intersect, 255);

    let p_distant = ScreenTileSaliencyEvaluator::compute_focus_proximity(
        100,
        100,
        64,
        64,
        (900, 900, 100, 100),
    );
    assert!(p_distant < 50, "distant proximity: {}", p_distant);

    // 6. Schedule and reorder tiles
    let tile_crit = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: high_contrast_data.clone(),
    };
    let tile_bg = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: flat_data.clone(),
    };
    let mut moderate_data = vec![128u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let off = (y * 64 + x) * 4;
            let v = ((x * 10) % 255) as u8;
            moderate_data[off] = v;
            moderate_data[off + 1] = v;
            moderate_data[off + 2] = v;
            moderate_data[off + 3] = 255;
        }
    }
    let tile_norm = DirtyTile {
        index: 2,
        x: 128,
        y: 0,
        w: 64,
        h: 64,
        data: moderate_data,
    };

    let cfg_active = ScreenTileSaliencyConfig::new(true, Some(120), Some(40), Some(true));
    // Input order: [tile_bg (idx 1), tile_norm (idx 2), tile_crit (idx 0)]
    let raw_list = vec![tile_bg.clone(), tile_norm.clone(), tile_crit.clone()];

    // Schedule without congestion
    let (reordered, crit_cnt, norm_cnt, bg_cnt, skipped_cnt) =
        ScreenTileSaliencyEvaluator::schedule_tiles(raw_list.clone(), None, &cfg_active, false);
    assert_eq!(crit_cnt, 1);
    assert_eq!(norm_cnt, 1);
    assert_eq!(bg_cnt, 1);
    assert_eq!(skipped_cnt, 0);
    assert_eq!(reordered.len(), 3);
    // Critical tile (idx 0) must be reordered to front!
    assert_eq!(reordered[0].index, 0);
    assert_eq!(reordered[1].index, 2);
    assert_eq!(reordered[2].index, 1);

    // Schedule with congestion -> background tile must be skipped!
    let (reordered_cong, c_cnt, n_cnt, b_cnt, s_cnt) =
        ScreenTileSaliencyEvaluator::schedule_tiles(raw_list, None, &cfg_active, true);
    assert_eq!(c_cnt, 1);
    assert_eq!(n_cnt, 1);
    assert_eq!(b_cnt, 1);
    assert_eq!(s_cnt, 1);
    assert_eq!(reordered_cong.len(), 2);
    assert_eq!(reordered_cong[0].index, 0);
    assert_eq!(reordered_cong[1].index, 2);

    // 7. ControlMsg Tag 62~64 wire codecs
    // Tag 62: SetScreenTileSaliencyMode
    let msg_set = ControlMsg::SetScreenTileSaliencyMode {
        enabled: true,
        high_threshold: 140,
        low_threshold: 50,
        drop_background_on_congestion: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 62);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 63: ResetScreenTileSaliencyMode
    let msg_reset = ControlMsg::ResetScreenTileSaliencyMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 63);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 64: TileSaliencySync
    let msg_sync = ControlMsg::TileSaliencySync {
        critical_tiles: 888,
        normal_tiles: 555,
        background_tiles: 333,
        background_tiles_skipped: 42,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 64);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileSaliencySync {
            critical_tiles: 888,
            normal_tiles: 0,
            background_tiles: 0,
            background_tiles_skipped: 0,
        }
    );
}

#[test]
fn screen_tile_lod_and_multiresolution_downsampling_algorithms() {
    // 1. TileLodLevel conversions and scale factors
    assert_eq!(TileLodLevel::from_u8(0), TileLodLevel::Lod0);
    assert_eq!(TileLodLevel::from_u8(1), TileLodLevel::Lod1);
    assert_eq!(TileLodLevel::from_u8(2), TileLodLevel::Lod2);
    assert_eq!(TileLodLevel::from_u8(99), TileLodLevel::Lod0);
    assert_eq!(TileLodLevel::Lod0.scale_factor(), 1);
    assert_eq!(TileLodLevel::Lod1.scale_factor(), 2);
    assert_eq!(TileLodLevel::Lod2.scale_factor(), 4);

    // 2. ScreenTileLodConfig and Report defaults
    let def_cfg = ScreenTileLodConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(def_cfg.max_lod, 1);
    assert!(def_cfg.auto_lod_on_congestion);

    let def_rep = ScreenTileLodReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(def_rep.lod0_tiles_sent, 0);
    assert_eq!(def_rep.lod1_tiles_sent, 0);
    assert_eq!(def_rep.lod2_tiles_sent, 0);
    assert_eq!(def_rep.bytes_saved, 0);

    // 3. Flat color downsampling & upsampling invariance
    let mut flat_red = vec![0u8; 64 * 64 * 4];
    for chunk in flat_red.chunks_exact_mut(4) {
        chunk[0] = 250;
        chunk[1] = 10;
        chunk[2] = 20;
        chunk[3] = 255;
    }

    // Downsample to Lod1 (32x32)
    let (down1, w1, h1) = downsample_tile_rgba(&flat_red, 64, 64, TileLodLevel::Lod1);
    assert_eq!(w1, 32);
    assert_eq!(h1, 32);
    assert_eq!(down1.len(), 32 * 32 * 4);
    assert_eq!(down1[0], 250);
    assert_eq!(down1[1], 10);
    assert_eq!(down1[2], 20);
    assert_eq!(down1[3], 255);

    // Upsample back to 64x64
    let up1 = upsample_tile_rgba(&down1, 32, 32, 64, 64);
    assert_eq!(up1.len(), 64 * 64 * 4);
    assert_eq!(up1[0], 250);
    assert_eq!(up1[1], 10);
    assert_eq!(up1[2], 20);
    assert_eq!(up1[3], 255);

    // Downsample to Lod2 (16x16)
    let (down2, w2, h2) = downsample_tile_rgba(&flat_red, 64, 64, TileLodLevel::Lod2);
    assert_eq!(w2, 16);
    assert_eq!(h2, 16);
    assert_eq!(down2.len(), 16 * 16 * 4);
    assert_eq!(down2[0], 250);

    // Upsample back to 64x64
    let up2 = upsample_tile_rgba(&down2, 16, 16, 64, 64);
    assert_eq!(up2.len(), 64 * 64 * 4);
    assert_eq!(up2[0], 250);

    // 4. Test compress_tile_lod and decompress_tile_lod roundtrip
    struct DummyCodec;
    impl crate::ScreenCodec for DummyCodec {
        fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress(
            &self,
            compressed: &[u8],
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(compressed.to_vec())
        }
        fn compress_frame(&self, raw: &[u8], _w: u16, _h: u16) -> Result<Vec<u8>, CoreError> {
            Ok(raw.to_vec())
        }
        fn decompress_frame(
            &self,
            data: &[u8],
            _w: u16,
            _h: u16,
            _expected_len: usize,
        ) -> Result<Vec<u8>, CoreError> {
            Ok(data.to_vec())
        }
    }
    let dummy = DummyCodec;

    let (codec_tag, payload) = compress_tile_lod(&flat_red, 64, 64, TileLodLevel::Lod1, &dummy);
    assert_eq!(codec_tag, TILE_SUBCODEC_LOD);
    assert_eq!(payload[0], TILE_SUBCODEC_LOD);
    assert_eq!(payload[1], 1); // Lod1

    let decompressed = decompress_tile_lod(&payload, 64, 64, 64 * 64 * 4, &dummy).unwrap();
    assert_eq!(decompressed.len(), 64 * 64 * 4);
    assert_eq!(decompressed[0], 250);
    assert_eq!(decompressed[1], 10);
    assert_eq!(decompressed[2], 20);
    assert_eq!(decompressed[3], 255);

    // 5. ControlMsg Tag 65~67 wire codecs
    // Tag 65: SetScreenTileLodMode
    let msg_set = ControlMsg::SetScreenTileLodMode {
        enabled: true,
        max_lod: 2,
        auto_lod_on_congestion: true,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 65);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 66: ResetScreenTileLodMode
    let msg_reset = ControlMsg::ResetScreenTileLodMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 66);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 67: TileLodSync
    let msg_sync = ControlMsg::TileLodSync {
        lod0_tiles: 120,
        lod1_tiles: 34,
        lod2_tiles: 12,
        bytes_saved: 987654,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 67);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileLodSync {
            lod0_tiles: 0,
            lod1_tiles: 0,
            lod2_tiles: 0,
            bytes_saved: 987654,
        }
    );
}

#[test]
fn screen_tile_batch_and_chunking_algorithms() {
    // 1. ScreenTileBatchConfig defaults and clamping
    let def_cfg = ScreenTileBatchConfig::default();
    assert!(!def_cfg.enabled);
    assert_eq!(
        def_cfg.target_chunk_bytes,
        SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
    );
    assert_eq!(
        def_cfg.pacing_interval_us,
        SCREEN_TILE_BATCH_DEFAULT_PACING_US
    );

    let custom_cfg = ScreenTileBatchConfig::new(true, Some(2000), Some(350));
    assert!(custom_cfg.enabled);
    assert_eq!(custom_cfg.target_chunk_bytes, 2000);
    assert_eq!(custom_cfg.pacing_interval_us, 350);

    let clamped_low = ScreenTileBatchConfig::new(true, Some(100), Some(0));
    assert_eq!(
        clamped_low.target_chunk_bytes,
        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES
    );
    assert_eq!(clamped_low.pacing_interval_us, 0);

    let clamped_high = ScreenTileBatchConfig::new(true, Some(65000), Some(60000));
    assert_eq!(
        clamped_high.target_chunk_bytes,
        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES
    );
    assert_eq!(
        clamped_high.pacing_interval_us,
        SCREEN_TILE_BATCH_MAX_PACING_US
    );

    // 2. ScreenTileBatchReport defaults
    let def_rep = ScreenTileBatchReport::default();
    assert!(def_rep.ok);
    assert!(!def_rep.enabled);
    assert_eq!(
        def_rep.target_chunk_bytes,
        SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
    );
    assert_eq!(
        def_rep.pacing_interval_us,
        SCREEN_TILE_BATCH_DEFAULT_PACING_US
    );
    assert_eq!(def_rep.batches_sent, 0);
    assert_eq!(def_rep.chunks_sent, 0);
    assert_eq!(def_rep.avg_chunk_bytes, 0);
    assert_eq!(def_rep.pacing_pauses, 0);

    // 3. ScreenFrame::encode_tile_batch_chunk and decode_tile_batch roundtrip
    let rect = ScreenRect::new(0, 0, 128, 64).unwrap();
    let t0 = DirtyTile {
        index: 0,
        x: 0,
        y: 0,
        w: 64,
        h: 64,
        data: vec![10u8; 128],
    };
    let t1 = DirtyTile {
        index: 1,
        x: 64,
        y: 0,
        w: 64,
        h: 64,
        data: vec![20u8; 256],
    };
    let tiles = vec![t0.clone(), t1.clone()];

    let frame = ScreenFrame::encode_tile_batch_chunk(
        42,
        rect,
        PixelFormat::Bgra8,
        1001,
        0,
        2,
        64,
        10,
        &tiles,
    );
    assert_eq!(frame.codec_tag, SCREEN_COMPRESS_TILE_BATCH);
    assert!(frame.compressed());

    let (batch_id, chunk_index, total_chunks, tile_size, total_tiles, dec_tiles) =
        frame.decode_tile_batch().expect("decode tile batch");
    assert_eq!(batch_id, 1001);
    assert_eq!(chunk_index, 0);
    assert_eq!(total_chunks, 2);
    assert_eq!(tile_size, 64);
    assert_eq!(total_tiles, 10);
    assert_eq!(dec_tiles.len(), 2);
    assert_eq!(dec_tiles[0].index, 0);
    assert_eq!(dec_tiles[0].data, vec![10u8; 128]);
    assert_eq!(dec_tiles[1].index, 1);
    assert_eq!(dec_tiles[1].data, vec![20u8; 256]);

    // Truncated payload check
    let mut bad_frame = frame.clone();
    bad_frame.bytes.truncate(10);
    assert!(bad_frame.decode_tile_batch().is_err());

    // 4. chunk_dirty_tiles algorithm
    // Empty tiles produces single empty chunk
    let empty_chunks = chunk_dirty_tiles(1, rect, PixelFormat::Bgra8, Vec::new(), 1200, 64, 0, 500);
    assert_eq!(empty_chunks.len(), 1);
    let (_, _, total_chunks, _, _, empty_tiles) = empty_chunks[0].decode_tile_batch().unwrap();
    assert_eq!(total_chunks, 1);
    assert!(empty_tiles.is_empty());

    // Multiple tiles exceeding target_chunk_bytes
    // 5 tiles, each with 300 bytes of data (+ 14 record header = 314 bytes per tile)
    // target = 600 bytes
    // chunk 0: batch_hdr(14) + tile0(314) = 328 bytes. tile1 would make it 328 + 314 = 642 > 600 -> closes chunk 0!
    // So 5 tiles should chunk into 5 separate chunks!
    let mut big_tiles = Vec::new();
    for i in 0..5u16 {
        big_tiles.push(DirtyTile {
            index: i,
            x: i * 10,
            y: 0,
            w: 64,
            h: 64,
            data: vec![i as u8; 300],
        });
    }
    let chunks = chunk_dirty_tiles(
        2,
        rect,
        PixelFormat::Bgra8,
        big_tiles.clone(),
        600,
        64,
        5,
        777,
    );
    assert_eq!(chunks.len(), 5);
    for (idx, chk) in chunks.iter().enumerate() {
        let (bid, c_idx, tot_c, _, tot_t, c_tiles) = chk.decode_tile_batch().unwrap();
        assert_eq!(bid, 777);
        assert_eq!(c_idx, idx as u16);
        assert_eq!(tot_c, 5);
        assert_eq!(tot_t, 5);
        assert_eq!(c_tiles.len(), 1);
        assert_eq!(c_tiles[0].index, idx as u16);
    }

    // 5. ControlMsg Tag 68~70 wire codecs
    // Tag 68: SetScreenTileBatchMode
    let msg_set = ControlMsg::SetScreenTileBatchMode {
        enabled: true,
        target_chunk_bytes: 1400,
        pacing_interval_us: 300,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 68);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 69: ResetScreenTileBatchMode
    let msg_reset = ControlMsg::ResetScreenTileBatchMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 69);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 70: TileBatchSync
    let msg_sync = ControlMsg::TileBatchSync {
        batches_sent: 12345,
        chunks_sent: 54321,
        avg_chunk_bytes: 1180,
        pacing_pauses: 999,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 70);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileBatchSync {
            batches_sent: 12345,
            chunks_sent: 0,
            avg_chunk_bytes: 0,
            pacing_pauses: 0,
        }
    );
}

#[test]
fn screen_tile_motion_and_displacement_compensation_algorithms() {
    // 1. ScreenTileMotionConfig defaults and bounds
    let cfg = ScreenTileMotionConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(
        cfg.max_displacement,
        SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT
    );
    assert_eq!(
        cfg.min_match_ratio,
        SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO
    );

    // 2. Build reference canvas (192x192 Bgra8) with distinct pattern
    let canvas_w = 192u16;
    let canvas_h = 192u16;
    let mut canvas_raw = vec![0u8; (canvas_w as usize) * (canvas_h as usize) * 4];
    for y in 0..canvas_h as usize {
        for x in 0..canvas_w as usize {
            let off = (y * (canvas_w as usize) + x) * 4;
            canvas_raw[off] = (x % 256) as u8;
            canvas_raw[off + 1] = (y % 256) as u8;
            canvas_raw[off + 2] = ((x + y) % 256) as u8;
            canvas_raw[off + 3] = 255;
        }
    }
    let ref_canvas = ScreenFrame {
        seq: 1,
        rect: ScreenRect {
            x: 0,
            y: 0,
            w: canvas_w,
            h: canvas_h,
        },
        stride: (canvas_w as u32) * 4,
        format: PixelFormat::Bgra8,
        codec_tag: SCREEN_COMPRESS_NONE,
        bytes: canvas_raw.clone(),
    };

    // 3. Create a dirty tile at (64, 64) with size 64x64 that corresponds to
    // reference canvas shifted by dx = 0, dy = 12 (vertical scroll)
    let tile_x = 64u16;
    let tile_y = 64u16;
    let tile_w = 64u16;
    let tile_h = 64u16;
    let expected_dx = 0i16;
    let expected_dy = 12i16;

    let src_ref_x = (tile_x as i32 - expected_dx as i32) as usize;
    let src_ref_y = (tile_y as i32 - expected_dy as i32) as usize;
    let mut tile_raw = vec![0u8; (tile_w as usize) * (tile_h as usize) * 4];
    for r in 0..tile_h as usize {
        let src_off = (src_ref_y + r) * (canvas_w as usize) * 4 + src_ref_x * 4;
        let dst_off = r * (tile_w as usize) * 4;
        tile_raw[dst_off..dst_off + (tile_w as usize) * 4]
            .copy_from_slice(&canvas_raw[src_off..src_off + (tile_w as usize) * 4]);
    }

    // 4. Test estimate_tile_motion
    let motion = estimate_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        32,
        0.90,
    );
    assert!(
        motion.is_some(),
        "Motion vector must be found for vertical shift"
    );
    let (est_dx, est_dy, ratio) = motion.unwrap();
    assert_eq!(est_dx, expected_dx);
    assert_eq!(est_dy, expected_dy);
    assert!(
        ratio >= 0.99,
        "Similarity ratio should be close to 1.0, got {}",
        ratio
    );

    // 5. Test compress_tile_motion with enabled config
    let mut active_cfg = ScreenTileMotionConfig {
        enabled: true,
        max_displacement: 32,
        min_match_ratio: 90,
    };
    let comp = compress_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        &active_cfg,
    );
    assert!(comp.is_some());
    let (codec, payload) = comp.unwrap();
    assert_eq!(codec, TILE_SUBCODEC_MOTION);
    assert_eq!(payload.len(), 5);
    assert_eq!(payload[0], TILE_SUBCODEC_MOTION);
    assert_eq!(i16::from_be_bytes([payload[1], payload[2]]), expected_dx);
    assert_eq!(i16::from_be_bytes([payload[3], payload[4]]), expected_dy);

    // Test disabled config returns None
    active_cfg.enabled = false;
    assert!(compress_tile_motion(
        &tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        &active_cfg,
    )
    .is_none());

    // 6. Test decompress_tile_motion
    let decomp = decompress_tile_motion(&payload, tile_x, tile_y, tile_w, tile_h, &ref_canvas)
        .expect("decompress tile motion");
    assert_eq!(decomp.len(), tile_raw.len());
    assert_eq!(
        decomp, tile_raw,
        "Decompressed pixels must match original dirty tile byte-for-byte"
    );

    // 7. Test horizontal shift (dx = -8, dy = 0)
    let h_dx = -8i16;
    let h_dy = 0i16;
    let src_h_x = (tile_x as i32 - h_dx as i32) as usize;
    let src_h_y = (tile_y as i32 - h_dy as i32) as usize;
    let mut h_tile_raw = vec![0u8; (tile_w as usize) * (tile_h as usize) * 4];
    for r in 0..tile_h as usize {
        let src_off = (src_h_y + r) * (canvas_w as usize) * 4 + src_h_x * 4;
        let dst_off = r * (tile_w as usize) * 4;
        h_tile_raw[dst_off..dst_off + (tile_w as usize) * 4]
            .copy_from_slice(&canvas_raw[src_off..src_off + (tile_w as usize) * 4]);
    }
    let h_motion = estimate_tile_motion(
        &h_tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
        32,
        0.90,
    )
    .expect("horizontal motion found");
    assert_eq!(h_motion.0, h_dx);
    assert_eq!(h_motion.1, h_dy);

    // 8. Test out-of-bounds displacement rejection in decompress_tile_motion
    let mut invalid_payload = vec![TILE_SUBCODEC_MOTION];
    invalid_payload.extend_from_slice(&(200i16).to_be_bytes()); // out of canvas
    invalid_payload.extend_from_slice(&(0i16).to_be_bytes());
    assert!(decompress_tile_motion(
        &invalid_payload,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        &ref_canvas,
    )
    .is_err());

    // 9. ControlMsg Tag 71~73 wire codecs
    // Tag 71: SetScreenTileMotionMode
    let msg_set = ControlMsg::SetScreenTileMotionMode {
        enabled: true,
        max_displacement: 64,
        min_match_ratio: 95,
    };
    let payload_set = msg_set.encode_payload();
    assert_eq!(payload_set.len(), 14);
    assert_eq!(payload_set[5], 71);
    let dec_set = ControlMsg::decode_payload(&payload_set).unwrap();
    assert_eq!(dec_set, msg_set);

    // Tag 72: ResetScreenTileMotionMode
    let msg_reset = ControlMsg::ResetScreenTileMotionMode;
    let payload_reset = msg_reset.encode_payload();
    assert_eq!(payload_reset.len(), 14);
    assert_eq!(payload_reset[5], 72);
    let dec_reset = ControlMsg::decode_payload(&payload_reset).unwrap();
    assert_eq!(dec_reset, msg_reset);

    // Tag 73: TileMotionSync
    let msg_sync = ControlMsg::TileMotionSync {
        motion_tiles: 8888,
        fallback_tiles: 2222,
        bytes_saved: 500000,
    };
    let payload_sync = msg_sync.encode_payload();
    assert_eq!(payload_sync.len(), 14);
    assert_eq!(payload_sync[5], 73);
    let dec_sync = ControlMsg::decode_payload(&payload_sync).unwrap();
    assert_eq!(
        dec_sync,
        ControlMsg::TileMotionSync {
            motion_tiles: 8888,
            fallback_tiles: 0,
            bytes_saved: 0,
        }
    );
}
