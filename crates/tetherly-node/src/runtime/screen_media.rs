// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen audio sync, recording and playback.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tetherly_core::{
    ControlMsg, CoreError, ScreenFrame, ScreenPlaybackEngine, ScreenPlaybackReport,
    ScreenPlaybackState, ScreenRecordingReader, ScreenSnapshotReport, ScreenWatermarkReport,
    SCREEN_RECORDING_REPLAY_LIMIT_MAX,
};

impl Node {
    /// Report the screen session recorder state and statistics (v2.15 / ADR-101).
    pub fn screen_recording_report(&self) -> ScreenRecordingReport {
        let peer = self
            .shared
            .screen_peer_recording
            .load(std::sync::atomic::Ordering::Relaxed);
        let guard = self.shared.screen_recording.lock().expect("recording");
        match guard.as_ref() {
            Some(s) => {
                let now_us = self.shared.clock.unix_us();
                let (width, height) = s
                    .writer
                    .as_ref()
                    .map(|w| (w.meta().width, w.meta().height))
                    .unwrap_or((0, 0));
                ScreenRecordingReport {
                    ok: true,
                    active: true,
                    path: s.path.display().to_string(),
                    frames_recorded: s.frames,
                    bytes_recorded: s.bytes,
                    started_at_us: s.started_at_us,
                    duration_us: now_us.saturating_sub(s.started_at_us),
                    last_frame_seq: s.last_seq,
                    peer_recording: peer,
                    width,
                    height,
                }
            }
            None => ScreenRecordingReport {
                ok: true,
                active: false,
                path: String::new(),
                frames_recorded: 0,
                bytes_recorded: 0,
                started_at_us: 0,
                duration_us: 0,
                last_frame_seq: 0,
                peer_recording: peer,
                width: 0,
                height: 0,
            },
        }
    }

    /// Start recording the locally presented screen stream into a TREC container
    /// (v2.15 / ADR-101). If a screen session is live, discloses the recording to
    /// the peer via ControlMsg::SetRecordingState.
    pub async fn screen_start_recording(
        &self,
        path: Option<String>,
    ) -> Result<ScreenRecordingReport, CoreError> {
        {
            let guard = self.shared.screen_recording.lock().expect("recording");
            if guard.is_some() {
                return Err(CoreError::InvalidConfig(
                    "screen recording already active; stop it first".into(),
                ));
            }
        }
        let resolved: std::path::PathBuf = match path.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => {
                let raw = std::path::PathBuf::from(p);
                if raw.file_name().is_none()
                    || raw
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(CoreError::UnsafeFileName);
                }
                if raw.is_absolute() {
                    raw
                } else {
                    self.shared.cfg.data_dir.join(raw)
                }
            }
            _ => {
                let ts = self.shared.clock.unix_us() / 1000;
                self.shared
                    .cfg
                    .data_dir
                    .join("recordings")
                    .join(format!("rec-{ts}.trec"))
            }
        };
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::OpenRefused(format!("create recordings dir: {e}")))?;
        }
        let session = ScreenRecordingSession {
            path: resolved,
            started_at_us: self.shared.clock.unix_us(),
            writer: None,
            frames: 0,
            bytes: 0,
            last_seq: 0,
        };
        *self.shared.screen_recording.lock().expect("recording") = Some(session);

        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let _ = ctl
                .send(ControlMsg::SetRecordingState { active: true })
                .await;
        }
        Ok(self.screen_recording_report())
    }

    /// Stop the active screen session recording, flush the TREC container to disk
    /// and disclose the stop to a live peer (v2.15 / ADR-101).
    pub async fn screen_stop_recording(&self) -> Result<ScreenRecordingReport, CoreError> {
        let session = {
            let mut guard = self.shared.screen_recording.lock().expect("recording");
            guard.take()
        };
        if let Some(sess) = session {
            if let Some(w) = sess.writer {
                if let Err(e) = w.finish() {
                    tracing::warn!(%e, "screen recording flush failed");
                }
            }
            tracing::info!(
                path = %sess.path.display(),
                frames = sess.frames,
                "screen recording stopped"
            );
        }

        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let _ = ctl
                .send(ControlMsg::SetRecordingState { active: false })
                .await;
        }
        Ok(self.screen_recording_report())
    }

    /// Inspect a recorded TREC container: validate the header and return up to
    /// `limit` frame record summaries (v2.15 / ADR-101).
    pub fn screen_recording_replay(
        &self,
        path: String,
        limit: usize,
    ) -> Result<ScreenRecordingReplayReport, CoreError> {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidConfig(
                "recording path is required".into(),
            ));
        }
        let p = std::path::PathBuf::from(trimmed);
        let file = std::fs::File::open(&p)
            .map_err(|e| CoreError::OpenRefused(format!("open recording: {e}")))?;
        let mut reader = ScreenRecordingReader::new(std::io::BufReader::new(file))?;
        let eff_limit = limit.clamp(1, SCREEN_RECORDING_REPLAY_LIMIT_MAX);
        let mut records = Vec::new();
        let mut truncated = false;
        while let Some(r) = reader.next_record()? {
            if records.len() < eff_limit {
                records.push(ScreenRecordingReplayRecord {
                    seq: r.seq,
                    pts_us: r.pts_us,
                    len: r.payload.len(),
                });
            } else {
                truncated = true;
            }
        }
        let meta = reader.meta().clone();
        Ok(ScreenRecordingReplayReport {
            ok: true,
            path: p.display().to_string(),
            width: meta.width,
            height: meta.height,
            format_tag: meta.format_tag,
            codec_tag: meta.codec_tag,
            started_at_us: meta.started_at_us,
            records_returned: records.len(),
            records,
            truncated,
        })
    }

    /// Report current screen playback state and metrics (v2.16 / ADR-102).
    pub fn screen_playback_report(&self) -> ScreenPlaybackReport {
        let guard = self.shared.screen_playback.lock().expect("playback");
        match guard.as_ref() {
            Some(s) => s.current_report(),
            None => ScreenPlaybackReport::default(),
        }
    }

    /// Start in-container screen session playback from a recorded TREC container (v2.16 / ADR-102).
    pub async fn screen_playback_start(
        &self,
        path: String,
        speed: Option<f32>,
        loop_enabled: Option<bool>,
    ) -> Result<ScreenPlaybackReport, CoreError> {
        {
            let guard = self.shared.screen_playback.lock().expect("playback");
            if let Some(s) = guard.as_ref() {
                let state = s.state();
                if state == ScreenPlaybackState::Playing || state == ScreenPlaybackState::Paused {
                    return Err(CoreError::InvalidConfig(
                        "screen playback already active; stop it first".into(),
                    ));
                }
            }
        }

        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidConfig(
                "playback recording path is required".into(),
            ));
        }
        let raw = std::path::PathBuf::from(trimmed);
        if raw.file_name().is_none()
            || raw
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(CoreError::UnsafeFileName);
        }
        let resolved = if raw.is_absolute() {
            raw
        } else {
            self.shared.cfg.data_dir.join(raw)
        };

        if !resolved.exists() {
            return Err(CoreError::OpenRefused(format!(
                "recording file not found: {}",
                resolved.display()
            )));
        }

        let sp = speed.unwrap_or(1.0).max(0.0);
        let lp = loop_enabled.unwrap_or(false);

        let mut engine = ScreenPlaybackEngine::from_file(&resolved, sp, lp)?;
        let mut rep = engine.report();
        rep.path = resolved.display().to_string();

        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let paused = Arc::new(AtomicBool::new(false));
        let speed_atomic = Arc::new(AtomicU32::new(sp.to_bits()));
        let loop_atomic = Arc::new(AtomicBool::new(lp));
        let report_arc = Arc::new(Mutex::new(rep.clone()));

        let session = ScreenPlaybackSession {
            path: resolved.clone(),
            stop_tx,
            paused: paused.clone(),
            speed: speed_atomic.clone(),
            loop_enabled: loop_atomic.clone(),
            report: report_arc.clone(),
        };
        *self.shared.screen_playback.lock().expect("playback") = Some(session);

        let shared = self.shared.clone();
        let rep_clone = report_arc.clone();
        let path_str = resolved.display().to_string();

        tokio::task::spawn_blocking(move || {
            while !*stop_rx.borrow() {
                if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
                    break;
                }
                if paused.load(std::sync::atomic::Ordering::Relaxed) {
                    engine.pause();
                    let mut r = engine.report();
                    r.path = path_str.clone();
                    *rep_clone.lock().unwrap() = r;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                } else if engine.state() == ScreenPlaybackState::Paused {
                    engine.resume();
                }

                let curr_sp =
                    f32::from_bits(speed_atomic.load(std::sync::atomic::Ordering::Relaxed));
                if (curr_sp - engine.speed()).abs() > f32::EPSILON {
                    engine.set_speed(curr_sp);
                }
                let curr_lp = loop_atomic.load(std::sync::atomic::Ordering::Relaxed);
                if curr_lp != engine.loop_enabled() {
                    engine.set_loop_enabled(curr_lp);
                }

                let res = engine.step(&*shared.screen_sink);
                let mut r = engine.report();
                r.path = path_str.clone();

                match res {
                    Ok(Some(mut frame)) => {
                        {
                            let wm_cfg = shared.screen_watermark.read().unwrap();
                            if wm_cfg.enabled {
                                let peer_id_str = shared
                                    .screen_peer
                                    .lock()
                                    .ok()
                                    .and_then(|g| g.clone())
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| "playback".to_string());
                                let time_str = format!("{}", shared.clock.unix_us() / 1_000_000);
                                if let Ok(applied) =
                                    frame.apply_watermark(&wm_cfg, &peer_id_str, &time_str)
                                {
                                    if applied {
                                        shared
                                            .screen_watermark_frames
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }
                                }
                            }
                        }
                        {
                            let now_ms = shared.clock.unix_us() / 1_000;
                            let annotations = shared.screen_annotations.read().unwrap();
                            if !annotations.is_empty() {
                                let _ = frame.apply_annotations(&annotations, now_ms);
                            }
                            if let Some(laser) =
                                shared.screen_laser_pointer.read().unwrap().as_ref()
                            {
                                let _ = frame.apply_laser_pointer(laser);
                            }
                        }
                        let _ = shared.screen_observer.lock().map(|o| o.present(&frame));
                        *shared.screen_last_frame.lock().expect("last_frame") = Some(frame.clone());
                        if !*stop_rx.borrow() {
                            *rep_clone.lock().unwrap() = r;
                        }
                    }
                    Ok(None) => {
                        if !*stop_rx.borrow() {
                            *rep_clone.lock().unwrap() = r;
                        }
                        if engine.state() == ScreenPlaybackState::Finished
                            || engine.state() == ScreenPlaybackState::Stopped
                        {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => {
                        tracing::warn!(%e, "screen playback step error");
                        r.state = ScreenPlaybackState::Stopped.as_str().to_string();
                        *rep_clone.lock().unwrap() = r;
                        break;
                    }
                }
            }
            if *stop_rx.borrow() {
                let mut guard = rep_clone.lock().unwrap();
                guard.state = ScreenPlaybackState::Stopped.as_str().to_string();
            }
        });

        Ok(rep)
    }

    /// Pause the active screen playback session (v2.16 / ADR-102).
    pub fn screen_playback_pause(&self) -> Result<ScreenPlaybackReport, CoreError> {
        let guard = self.shared.screen_playback.lock().expect("playback");
        if let Some(s) = guard.as_ref() {
            if s.state() == ScreenPlaybackState::Finished
                || s.state() == ScreenPlaybackState::Stopped
            {
                return Err(CoreError::InvalidConfig("playback is not active".into()));
            }
            s.paused.store(true, std::sync::atomic::Ordering::Relaxed);
            let mut r = s.report.lock().unwrap();
            r.state = ScreenPlaybackState::Paused.as_str().to_string();
            Ok(r.clone())
        } else {
            Err(CoreError::InvalidConfig(
                "no screen playback session active".into(),
            ))
        }
    }

    /// Resume the paused screen playback session (v2.16 / ADR-102).
    pub fn screen_playback_resume(&self) -> Result<ScreenPlaybackReport, CoreError> {
        let guard = self.shared.screen_playback.lock().expect("playback");
        if let Some(s) = guard.as_ref() {
            if s.state() == ScreenPlaybackState::Finished
                || s.state() == ScreenPlaybackState::Stopped
            {
                return Err(CoreError::InvalidConfig("playback is not active".into()));
            }
            s.paused.store(false, std::sync::atomic::Ordering::Relaxed);
            let mut r = s.report.lock().unwrap();
            r.state = ScreenPlaybackState::Playing.as_str().to_string();
            Ok(r.clone())
        } else {
            Err(CoreError::InvalidConfig(
                "no screen playback session active".into(),
            ))
        }
    }

    /// Stop the active screen playback session (v2.16 / ADR-102).
    pub fn screen_playback_stop(&self) -> Result<ScreenPlaybackReport, CoreError> {
        let guard = self.shared.screen_playback.lock().expect("playback");
        if let Some(s) = guard.as_ref() {
            let _ = s.stop_tx.send(true);
            let mut r = s.report.lock().unwrap();
            r.state = ScreenPlaybackState::Stopped.as_str().to_string();
            Ok(r.clone())
        } else {
            Ok(ScreenPlaybackReport::default())
        }
    }

    /// Capture a live screen snapshot as raw or standard BMP bytes along with a structured telemetry report (v2.17 / ADR-103).
    ///
    /// When privacy mode is enabled (`screen_privacy_enabled`), screen contents are strictly blanked (all zero pixels)
    /// to prevent surveillance while returning `privacy_blanked: true`.
    pub fn screen_snapshot_data(
        &self,
        format: Option<String>,
        out: Option<String>,
    ) -> Result<(Vec<u8>, ScreenSnapshotReport), CoreError> {
        let fmt = format
            .map(|f| f.trim().to_ascii_lowercase())
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "bmp".to_string());
        if fmt != "bmp" && fmt != "raw" && fmt != "json" {
            return Err(CoreError::InvalidConfig(format!(
                "unsupported snapshot format: {fmt} (supported: bmp, raw, json)"
            )));
        }

        // 1. Fetch latest live frame: cached last frame -> observer sink -> screen source grab
        let mut frame = self
            .shared
            .screen_last_frame
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .or_else(|| {
                self.shared
                    .screen_observer
                    .lock()
                    .ok()
                    .and_then(|o| o.last())
            })
            .or_else(|| self.shared.screen_source.grab().ok().flatten())
            .ok_or_else(|| {
                CoreError::ScreenRefused("no screen frame available for snapshot".into())
            })?;

        // 2. If frame is compressed, decode it to raw pixel buffer using negotiated codec
        if frame.compressed() {
            let codec = if frame.codec_tag == tetherly_core::SCREEN_COMPRESS_H264 {
                self.shared.screen_codec_h264.clone()
            } else {
                self.shared.screen_codec.clone()
            };
            let raw = codec.decompress_frame(
                &frame.bytes,
                frame.rect.w,
                frame.rect.h,
                frame.raw_len(),
            )?;
            frame = ScreenFrame {
                bytes: raw,
                codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
                ..frame
            };
        }

        // 3. Privacy defense check (ADR-098): blank screen if privacy mode is active
        let privacy_active = self
            .shared
            .screen_privacy_enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        if privacy_active {
            frame.bytes.fill(0);
        }

        // 4. Dynamic security watermark overlay (ADR-104)
        let mut watermarked = false;
        if !privacy_active {
            let wm_cfg = self.shared.screen_watermark.read().unwrap();
            if wm_cfg.enabled {
                let peer_id_str = self
                    .shared
                    .screen_peer
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "local".to_string());
                let time_str = format!("{}", self.shared.clock.unix_us() / 1_000_000);
                if let Ok(applied) = frame.apply_watermark(&wm_cfg, &peer_id_str, &time_str) {
                    watermarked = applied;
                    if applied {
                        self.shared
                            .screen_watermark_frames
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            // Collaborative whiteboard annotations & transient virtual laser pointer (ADR-105)
            let now_ms = self.shared.clock.unix_us() / 1_000;
            let annotations = self.shared.screen_annotations.read().unwrap();
            if !annotations.is_empty() {
                let _ = frame.apply_annotations(&annotations, now_ms);
            }
            if let Some(laser) = self.shared.screen_laser_pointer.read().unwrap().as_ref() {
                let _ = frame.apply_laser_pointer(laser);
            }
        }

        // 5. Encode image bytes
        let (bytes, final_fmt) = if fmt == "raw" {
            (frame.bytes.clone(), "raw".to_string())
        } else {
            let bmp = frame.to_bmp()?;
            (bmp, "bmp".to_string())
        };

        // 4. Save to disk if out path is specified
        let mut saved_path = None;
        if let Some(out_p) = out.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            let out_path = std::path::PathBuf::from(&out_p);
            if out_path.file_name().is_none()
                || out_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(CoreError::UnsafeFileName);
            }
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            std::fs::write(&out_path, &bytes)?;
            saved_path = Some(out_path.display().to_string());
        }

        self.shared
            .screen_snapshot_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let data_b64 = if saved_path.is_none() {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            Some(STANDARD.encode(&bytes))
        } else {
            None
        };

        let report = ScreenSnapshotReport {
            ok: true,
            source: if self.screen_playback_report().state == "playing" {
                "playback".to_string()
            } else {
                "live".to_string()
            },
            frame_seq: frame.seq,
            pts_us: self.shared.clock.unix_us(),
            width: frame.rect.w as u32,
            height: frame.rect.h as u32,
            format: final_fmt,
            bytes_len: bytes.len(),
            privacy_blanked: privacy_active,
            watermarked,
            path: saved_path,
            data_base64: data_b64,
        };

        Ok((bytes, report))
    }

    /// Capture a live screen snapshot and return its structured telemetry report (v2.17 / ADR-103).
    pub fn screen_snapshot(
        &self,
        format: Option<String>,
        out: Option<String>,
    ) -> Result<ScreenSnapshotReport, CoreError> {
        self.screen_snapshot_data(format, out).map(|(_, rep)| rep)
    }

    /// Extract a single frame from a recording container (.trec) and export as BMP (v2.17 / ADR-103).
    pub fn screen_recording_snapshot_data(
        &self,
        path: &str,
        frame_seq: Option<u64>,
        pts_us: Option<u64>,
        out: Option<String>,
    ) -> Result<(Vec<u8>, ScreenSnapshotReport), CoreError> {
        let trimmed_path = path.trim();
        if trimmed_path.is_empty() {
            return Err(CoreError::InvalidConfig(
                "recording path cannot be empty".into(),
            ));
        }
        let raw_path = std::path::PathBuf::from(trimmed_path);
        if raw_path.file_name().is_none()
            || raw_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(CoreError::UnsafeFileName);
        }

        let file = std::fs::File::open(&raw_path).map_err(|e| {
            CoreError::InvalidConfig(format!("failed to open recording file {trimmed_path}: {e}"))
        })?;
        let mut reader = ScreenRecordingReader::new(std::io::BufReader::new(file))?;
        let (frame, frame_pts) = reader
            .extract_frame_with_pts(frame_seq, pts_us)?
            .ok_or_else(|| {
                CoreError::ScreenRefused("no matching frame found in recording".into())
            })?;

        let bmp_bytes = frame.to_bmp()?;

        let mut saved_path = None;
        if let Some(out_p) = out.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            let out_path = std::path::PathBuf::from(&out_p);
            if out_path.file_name().is_none()
                || out_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(CoreError::UnsafeFileName);
            }
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
            std::fs::write(&out_path, &bmp_bytes)?;
            saved_path = Some(out_path.display().to_string());
        }

        self.shared
            .screen_snapshot_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let data_b64 = if saved_path.is_none() {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            Some(STANDARD.encode(&bmp_bytes))
        } else {
            None
        };

        let report = ScreenSnapshotReport {
            ok: true,
            source: "recording".to_string(),
            frame_seq: frame.seq,
            pts_us: frame_pts,
            width: frame.rect.w as u32,
            height: frame.rect.h as u32,
            format: "bmp".to_string(),
            bytes_len: bmp_bytes.len(),
            privacy_blanked: false,
            watermarked: false,
            path: saved_path,
            data_base64: data_b64,
        };

        Ok((bmp_bytes, report))
    }

    /// Extract a single frame from a recording container (.trec) and return report (v2.17 / ADR-103).
    pub fn screen_recording_snapshot(
        &self,
        path: &str,
        frame_seq: Option<u64>,
        pts_us: Option<u64>,
        out: Option<String>,
    ) -> Result<ScreenSnapshotReport, CoreError> {
        self.screen_recording_snapshot_data(path, frame_seq, pts_us, out)
            .map(|(_, rep)| rep)
    }

    /// Report dynamic security watermark status and configuration (v2.18 / ADR-104).
    pub fn screen_watermark_report(&self) -> ScreenWatermarkReport {
        let cfg = self.shared.screen_watermark.read().unwrap().clone();
        let peer_id = self
            .shared
            .screen_peer
            .lock()
            .ok()
            .and_then(|p| p.clone())
            .map(|p| p.to_string())
            .unwrap_or_else(|| "local".to_string());
        let timestamp = format!("{}", self.shared.clock.unix_us() / 1_000_000);
        let frames = self.shared.screen_watermark_frames.load(Ordering::Relaxed);
        let mut rep = cfg.report(&peer_id, &timestamp, frames);
        if self
            .shared
            .screen_peer_watermark_enforced
            .load(Ordering::Relaxed)
        {
            rep.host_enforced = true;
        }
        rep
    }

    /// Set dynamic security watermark configuration with host enforcement check (v2.18 / ADR-104).
    pub async fn screen_set_watermark(
        &self,
        update: ScreenWatermarkUpdate,
    ) -> Result<ScreenWatermarkReport, CoreError> {
        // If peer host enforced watermark, visitor cannot disable it
        if let Some(false) = update.enabled {
            if self
                .shared
                .screen_peer_watermark_enforced
                .load(Ordering::Relaxed)
            {
                return Err(CoreError::InvalidConfig(
                    "watermark is enforced by host and cannot be disabled".into(),
                ));
            }
        }

        let (enabled, host_enforced) = {
            let mut cfg = self.shared.screen_watermark.write().unwrap();
            if let Some(e) = update.enabled {
                cfg.enabled = e;
            }
            if let Some(he) = update.host_enforced {
                cfg.host_enforced = he;
            }
            if let Some(t) = update.text_template {
                let trimmed = t.trim();
                if trimmed.is_empty() {
                    return Err(CoreError::InvalidConfig(
                        "watermark text cannot be empty".into(),
                    ));
                }
                cfg.text_template = trimmed.to_string();
            }
            if let Some(op) = update.opacity {
                cfg.opacity = op;
            }
            if let Some(c) = update.color_rgb {
                cfg.color_rgb = c;
            }
            if let Some(s) = update.scale {
                if s == 0 || s > 4 {
                    return Err(CoreError::InvalidConfig(
                        "watermark scale must be between 1 and 4".into(),
                    ));
                }
                cfg.scale = s;
            }
            if let Some(p) = update.pattern {
                cfg.pattern = p.parse()?;
            }
            if let Some(sx) = update.spacing_x {
                cfg.spacing_x = sx.max(40);
            }
            if let Some(sy) = update.spacing_y {
                cfg.spacing_y = sy.max(30);
            }
            (cfg.enabled, cfg.host_enforced)
        };

        // Synchronize to peer via screen control channel
        let msg = ControlMsg::SetWatermarkState {
            enabled,
            host_enforced,
        };
        let _ = self.send_screen_control(msg).await;

        Ok(self.screen_watermark_report())
    }

    /// Enable dynamic security watermark with optional host enforcement (v2.18 / ADR-104).
    pub async fn screen_enable_watermark(
        &self,
        host_enforced: bool,
    ) -> Result<ScreenWatermarkReport, CoreError> {
        self.screen_set_watermark(ScreenWatermarkUpdate {
            enabled: Some(true),
            host_enforced: Some(host_enforced),
            ..Default::default()
        })
        .await
    }

    /// Disable dynamic security watermark, blocked if host enforced (v2.18 / ADR-104).
    pub async fn screen_disable_watermark(&self) -> Result<ScreenWatermarkReport, CoreError> {
        self.screen_set_watermark(ScreenWatermarkUpdate {
            enabled: Some(false),
            host_enforced: Some(false),
            ..Default::default()
        })
        .await
    }

    /// Add a collaborative screen annotation item (v2.19 / ADR-105, v2.20 / ADR-106).
    pub async fn screen_add_annotation(
        &self,
        mut item: tetherly_core::AnnotationItem,
    ) -> Result<u32, CoreError> {
        if item.created_at_ms == 0 {
            item.created_at_ms = self.shared.clock.unix_ms();
        }
        let item_id = {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            let id = wb.add_item(item.clone())?;
            item.id = id;
            *self.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
            id
        };
        self.shared
            .screen_annotation_count
            .fetch_add(1, Ordering::Relaxed);

        // Broadcast/synchronize to peer via dedicated annotation frame
        let msg = tetherly_core::ScreenAnnotationMsg::Add(item.clone());
        if let Ok(frame) = msg.to_frame() {
            let _ = self.send_screen_frame_raw(frame).await;
        }

        Ok(item_id)
    }

    /// Clear all collaborative screen whiteboard annotations (v2.19 / ADR-105, v2.20 / ADR-106).
    pub async fn screen_clear_annotations(&self) -> Result<(), CoreError> {
        {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            wb.clear();
            self.shared.screen_annotations.write().unwrap().clear();
        }

        // Broadcast clear control to peer
        let _ = self.send_screen_control(ControlMsg::ClearAnnotations).await;
        // Also send dedicated clear frame
        let msg = tetherly_core::ScreenAnnotationMsg::Clear;
        if let Ok(frame) = msg.to_frame() {
            let _ = self.send_screen_frame_raw(frame).await;
        }
        Ok(())
    }

    /// Undo last annotation for a specific author or global (v2.20 / ADR-106).
    pub async fn screen_whiteboard_undo(
        &self,
        author: Option<&str>,
    ) -> Result<Option<tetherly_core::AnnotationItem>, CoreError> {
        let undone = {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            let res = wb.undo(author);
            *self.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
            res
        };
        if undone.is_some() {
            self.shared
                .screen_whiteboard_undo_count
                .fetch_add(1, Ordering::Relaxed);
            let author_str = author.unwrap_or("").to_string();
            let author_hash = tetherly_core::hash_author(&author_str);
            let _ = self
                .send_screen_control(ControlMsg::WhiteboardUndo { author_hash })
                .await;
            let msg = tetherly_core::ScreenAnnotationMsg::Undo { author: author_str };
            if let Ok(frame) = msg.to_frame() {
                let _ = self.send_screen_frame_raw(frame).await;
            }
        }
        Ok(undone)
    }

    /// Redo last undone annotation for a specific author or global (v2.20 / ADR-106).
    pub async fn screen_whiteboard_redo(
        &self,
        author: Option<&str>,
    ) -> Result<Option<tetherly_core::AnnotationItem>, CoreError> {
        let redone = {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            let res = wb.redo(author);
            *self.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
            res
        };
        if let Some(ref it) = redone {
            self.shared
                .screen_whiteboard_redo_count
                .fetch_add(1, Ordering::Relaxed);
            let author_str = author.unwrap_or("").to_string();
            let author_hash = tetherly_core::hash_author(&author_str);
            let _ = self
                .send_screen_control(ControlMsg::WhiteboardRedo { author_hash })
                .await;
            let msg = tetherly_core::ScreenAnnotationMsg::Redo { author: author_str };
            if let Ok(frame) = msg.to_frame() {
                let _ = self.send_screen_frame_raw(frame).await;
            }
            let add_msg = tetherly_core::ScreenAnnotationMsg::Add(it.clone());
            if let Ok(frame) = add_msg.to_frame() {
                let _ = self.send_screen_frame_raw(frame).await;
            }
        }
        Ok(redone)
    }

    /// Export whiteboard vector snapshot (v2.20 / ADR-106).
    pub fn screen_whiteboard_export(
        &self,
        out: Option<&str>,
    ) -> Result<tetherly_core::WhiteboardSnapshot, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let session_id = self.shared.identity.device_id().to_string();
        let snapshot = {
            let wb = self.shared.screen_whiteboard.read().unwrap();
            wb.export_snapshot(session_id, now_ms)
        };
        if let Some(path_str) = out {
            let path = std::path::Path::new(path_str);
            if path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(CoreError::InvalidConfig("path traversal rejected".into()));
            }
            let json_bytes = serde_json::to_vec_pretty(&snapshot)
                .map_err(|e| CoreError::InvalidConfig(e.to_string()))?;
            std::fs::write(path, json_bytes)
                .map_err(|e| CoreError::InvalidConfig(e.to_string()))?;
        }
        Ok(snapshot)
    }

    /// Import whiteboard vector snapshot (v2.20 / ADR-106).
    pub async fn screen_whiteboard_import(
        &self,
        snapshot: tetherly_core::WhiteboardSnapshot,
        replace: bool,
    ) -> Result<usize, CoreError> {
        let count = {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            let c = wb.import_snapshot(snapshot.clone(), replace)?;
            *self.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
            c
        };
        let msg = tetherly_core::ScreenAnnotationMsg::Snapshot(snapshot);
        if let Ok(frame) = msg.to_frame() {
            let _ = self.send_screen_frame_raw(frame).await;
        }
        Ok(count)
    }

    /// Update whiteboard policy (v2.20 / ADR-106).
    pub async fn screen_whiteboard_set_policy(
        &self,
        policy: tetherly_core::WhiteboardPolicy,
    ) -> Result<(), CoreError> {
        {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            wb.set_policy(policy);
        }
        let msg = ControlMsg::SetWhiteboardPolicy {
            read_only: policy.read_only,
            allow_peer_clear: policy.allow_peer_clear,
            max_items: policy.max_items,
        };
        let _ = self.send_screen_control(msg).await;
        let frame_msg = tetherly_core::ScreenAnnotationMsg::Policy(policy);
        if let Ok(frame) = frame_msg.to_frame() {
            let _ = self.send_screen_frame_raw(frame).await;
        }
        Ok(())
    }

    /// Get current whiteboard policy (v2.20 / ADR-106).
    pub fn screen_whiteboard_get_policy(&self) -> tetherly_core::WhiteboardPolicy {
        self.shared.screen_whiteboard.read().unwrap().policy()
    }

    /// Update transient virtual laser pointer state (v2.19 / ADR-105).
    pub async fn screen_set_laser_pointer(
        &self,
        state: tetherly_core::LaserPointerState,
    ) -> Result<(), CoreError> {
        let active = state.active;
        let x = state.x;
        let y = state.y;
        let color_rgb = state.color_rgb;

        *self.shared.screen_laser_pointer.write().unwrap() =
            if active { Some(state) } else { None };
        self.shared
            .screen_laser_updates
            .fetch_add(1, Ordering::Relaxed);

        // Send 14-byte ControlMsg Tag 11
        let msg = ControlMsg::SetLaserPointer {
            x,
            y,
            active,
            color_rgb,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(())
    }

    /// Report current collaborative annotations and laser pointer state (v2.19 / ADR-105, v2.20 / ADR-106).
    pub fn screen_annotation_report(&self) -> tetherly_core::ScreenAnnotationReport {
        let now_ms = self.shared.clock.unix_ms();
        let (items, policy, redo_cnt) = {
            let mut wb = self.shared.screen_whiteboard.write().unwrap();
            wb.prune_expired(now_ms);
            *self.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
            (wb.active_items().to_vec(), wb.policy(), wb.redo_count())
        };
        let laser = *self.shared.screen_laser_pointer.read().unwrap();
        tetherly_core::ScreenAnnotationReport {
            ok: true,
            annotations_count: items.len(),
            laser_active: laser.as_ref().map(|l| l.active).unwrap_or(false),
            laser,
            annotations: items,
            total_annotations_added: self.shared.screen_annotation_count.load(Ordering::Relaxed),
            laser_updates_total: self.shared.screen_laser_updates.load(Ordering::Relaxed),
            timestamp_ms: now_ms,
            whiteboard_policy: policy,
            redo_count: redo_cnt,
        }
    }
}
