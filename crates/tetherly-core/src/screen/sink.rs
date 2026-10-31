// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Frame assembler and the memory-backed screen source/sink for tests.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use crate::frame::INNER_PAYLOAD_MAX;
use std::collections::VecDeque;

/// Reassembles `TMF1` fragment streams back into one `ScreenFrame`. A whole
/// `TMV1` payload (small frame) passes through unchanged. Fragments that
/// disagree with the current stream (a different seq, index, or shape) reset
/// the buffer — the wire is a live stream and a stale fragment never pollutes
/// a newer frame. Bounded: the assembled body is capped at
/// `SCREEN_PAYLOAD_MAX`, so a hostile peer cannot grow memory without limit.
#[derive(Debug, Default)]
pub struct ScreenAssembler {
    pub(super) seq: u64,
    pub(super) total: usize,
    pub(super) count: usize,
    /// Reassembly body, allocated once per stream and filled in place at each
    /// fragment's offset — one buffer instead of per-chunk Vecs plus a final
    /// concatenation copy.
    pub(super) body: Vec<u8>,
    pub(super) mask: Vec<bool>,
    pub(super) accepted: usize,
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
    pub(super) rect: ScreenRect,
    pub(super) format: PixelFormat,
    pub(super) seq: std::sync::atomic::AtomicU64,
    /// Optional per-grab failure injection (index of the grab call, 1-based).
    pub(super) fail_at: std::sync::atomic::AtomicU64,
    pub(super) active_display: std::sync::atomic::AtomicU8,
    pub(super) displays: std::sync::Mutex<Vec<ScreenDisplayInfo>>,
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
    pub(super) inner: std::sync::Arc<std::sync::Mutex<MemoryScreenSinkInner>>,
}

#[derive(Debug, Default)]
pub(super) struct MemoryScreenSinkInner {
    pub(super) presented: VecDeque<u64>,
    pub(super) last: Option<ScreenFrame>,
    pub(super) fail: bool,
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
