// SPDX-License-Identifier: Apache-2.0 OR MIT
//! TREC session recording container writer and reader.

use super::*;
use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;

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
    pub(super) meta: ScreenRecordingMeta,
    pub(super) inner: W,
    pub(super) frames_written: u64,
    pub(super) bytes_written: u64,
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
    pub(super) meta: ScreenRecordingMeta,
    pub(super) inner: R,
    pub(super) records_read: u64,
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
