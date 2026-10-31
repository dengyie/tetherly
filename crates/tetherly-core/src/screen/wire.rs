// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Wire constants, screen frame codec and pixel geometry.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use crate::frame::FLAG_MUST_UNDERSTAND;
use crate::frame::INNER_PAYLOAD_MAX;

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

    pub(super) fn tag(self) -> u8 {
        match self {
            PixelFormat::Bgra8 => 0,
            PixelFormat::Rgba8 => 1,
        }
    }

    pub(super) fn from_tag(tag: u8) -> Result<Self, CoreError> {
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

    pub(super) fn encode_header(&self) -> Vec<u8> {
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
