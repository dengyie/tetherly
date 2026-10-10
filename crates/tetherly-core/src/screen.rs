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
/// 64 KiB, so a full-resolution raw frame must be split by the caller; this is
/// the hard ceiling the codec enforces on the concatenated pixel body.
pub const SCREEN_PAYLOAD_MAX: usize = INNER_PAYLOAD_MAX;

/// Most recent presented frames the memory sink retains, so a long-lived
/// receiver cannot grow its trace without bound.
pub const SCREEN_TRACE_MAX: usize = 64;

/// Compression tag for the pixel body. `None` is raw; `Zstd` is the default
/// lossless codec. A decoder that does not know the tag refuses the frame
/// rather than guessing.
pub const SCREEN_COMPRESS_NONE: u8 = 0;
pub const SCREEN_COMPRESS_ZSTD: u8 = 1;

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
        self.w as usize * self.h as usize * format.bpp()
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
    pub compressed: bool,
    /// Row stride in bytes as produced by the source. `0` means tightly packed.
    pub stride: u32,
    pub bytes: Vec<u8>,
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
            compressed: false,
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
        out.push(if self.compressed {
            SCREEN_COMPRESS_ZSTD
        } else {
            SCREEN_COMPRESS_NONE
        });
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
        let compressed = match payload[26] {
            SCREEN_COMPRESS_NONE => false,
            SCREEN_COMPRESS_ZSTD => true,
            _ => return Err(CoreError::ScreenFrame),
        };
        let stride = u32::from_be_bytes(payload[27..31].try_into().unwrap());
        let rect = ScreenRect::new(x, y, w, h)?;
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
            compressed,
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
}

/// Sender side. Owns the monotonic sequence so a reconnect cannot replay old
/// frames, and so the controller can detect a gap.
pub struct ScreenSender {
    next_seq: u64,
    state: ScreenState,
    stats: ScreenStats,
    pending: VecDeque<ScreenFrame>,
}

impl Default for ScreenSender {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenSender {
    pub fn new() -> Self {
        Self {
            next_seq: 1,
            state: ScreenState::Idle,
            stats: ScreenStats::default(),
            pending: VecDeque::new(),
        }
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
    }

    /// Apply a controller instruction. Returns the reply to send back, if any.
    /// A `Start` before `allow` is counted as refused and does not stream.
    pub fn on_control(&mut self, msg: ControlMsg) -> Result<Option<ControlMsg>, CoreError> {
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
                Ok(None)
            }
            ControlMsg::Ack { .. } => Ok(None),
        }
    }

    /// Queue a frame while streaming. Returns the assigned sequence number.
    pub fn push(&mut self, frame: ScreenFrame) -> Option<u64> {
        if self.state != ScreenState::Streaming {
            return None;
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.stats.sent = self.stats.sent.saturating_add(1);
        self.stats.bytes = self.stats.bytes.saturating_add(frame.bytes.len() as u64);
        self.pending.push_back(ScreenFrame { seq, ..frame });
        Some(seq)
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
        // A raw frame must match its rect; the codec already enforced this on
        // the wire, but a locally built frame could still lie.
        if !frame.compressed && frame.bytes.len() != frame.raw_len() {
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

    pub fn with_format(mut self, format: PixelFormat) -> Self {
        self.format = format;
        self
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
    fn grab(&self) -> Result<ScreenFrame, CoreError> {
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
        ScreenFrame::raw(seq, self.rect, self.format, bytes)
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
        assert_eq!(a.grab().unwrap().seq, 2);

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
}
