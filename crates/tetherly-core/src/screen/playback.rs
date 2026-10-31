// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Recording playback engine and virtual screen source feeder.

use super::*;
use crate::error::CoreError;
use serde::Deserialize;
use serde::Serialize;

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

/// In-container playback engine reading frames from a TREC container and presenting
/// to a `ScreenSink` with PTS pacing, speed adjustment and loop controls (v2.16 / ADR-102).
pub struct ScreenPlaybackEngine<R: std::io::Read + std::io::Seek> {
    pub(super) reader: ScreenRecordingReader<R>,
    pub(super) meta: ScreenRecordingMeta,
    pub(super) state: ScreenPlaybackState,
    pub(super) speed: f32,
    pub(super) loop_enabled: bool,
    pub(super) current_frame: u64,
    pub(super) total_frames: u64,
    pub(super) duration_us: u64,
    pub(super) elapsed_us: u64,
    pub(super) last_pts_us: Option<u64>,
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
    pub(super) reader: std::sync::Mutex<ScreenRecordingReader<Box<dyn ReadSeek>>>,
    pub(super) meta: ScreenRecordingMeta,
    pub(super) loop_enabled: std::sync::atomic::AtomicBool,
    pub(super) fps: std::sync::atomic::AtomicU32,
    pub(super) frames_grabbed: std::sync::atomic::AtomicU64,
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
