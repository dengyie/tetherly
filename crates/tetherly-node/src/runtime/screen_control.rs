// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen consent, start/stop, input and control envelopes.

use super::*;
use crate::events::UiEvent;
use std::sync::atomic::Ordering;
use tetherly_core::{
    ControlMsg, CoreError, ScreenAudioSyncReport, ScreenBottleneckStage, ScreenCursorUpdate,
    ScreenFecReport, ScreenHarqReport, ScreenLatencyBreakdown, ScreenLatencyReport,
    ScreenPacerReport, ScreenProbeReport, ScreenProbeState, ScreenState, ScreenTrendlineReport,
    ScreenTunerMode, ScreenTuningReport, ScreenTwccReport,
};

impl Node {
    /// Local consent for the currently connected visitor. This is the ONLY path
    /// into `Allowed`; nothing a peer sends can reach it.
    pub fn screen_allow(&self) {
        self.shared.screen_engine.lock().expect("screen").allow();
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        self.emit(UiEvent::screen_state_changed(state.as_str(), allowed, peer));
    }

    /// Local consent with a strongly typed report return.
    pub fn screen_allow_reported(&self) -> ScreenControlReport {
        self.screen_allow();
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        ScreenControlReport {
            ok: true,
            action: "allow".to_string(),
            state: state.as_str().to_string(),
            allowed,
            peer,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Revoke local consent; any queued frames are dropped.
    pub fn screen_revoke(&self) {
        self.shared.screen_engine.lock().expect("screen").revoke();
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        self.emit(UiEvent::screen_state_changed(state.as_str(), allowed, peer));
    }

    /// Revoke local consent with a strongly typed report return.
    pub fn screen_revoke_reported(&self) -> ScreenControlReport {
        self.screen_revoke();
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        ScreenControlReport {
            ok: true,
            action: "revoke".to_string(),
            state: state.as_str().to_string(),
            allowed,
            peer,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Visitor: ask the host to begin sending. Refused by the host until it has
    /// allowed locally. A refusal comes back as the host's `Stop` reply, which
    /// the control pump latches in `screen_start_refused`; the next call
    /// surfaces it as an error so callers (e.g. the ADR-138 session start) can
    /// roll back deterministically.
    pub async fn screen_start(&self) -> Result<(), CoreError> {
        self.shared
            .screen_start_refused
            .store(false, std::sync::atomic::Ordering::SeqCst);
        // Open the refusal window (ticket → odd) BEFORE sending Start and
        // close it (ticket → even) on every exit path. The wire pump only
        // latches a refusal Stop-reply while the window is open, so a leftover
        // refusal from the previous screen channel — its teardown racing this
        // start — cannot poison this attempt's verdict (m6_7 CI race).
        self.shared
            .screen_start_ticket
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        debug_assert!(
            self.shared
                .screen_start_ticket
                .load(std::sync::atomic::Ordering::SeqCst)
                % 2
                == 1,
            "screen_start window must open odd"
        );
        self.send_screen_control(ControlMsg::Start).await?;
        // Give the host's reply a beat to round-trip before the caller reads
        // the latch (poll, not sleep-guess: exit early on refusal).
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
        let outcome = loop {
            if self
                .shared
                .screen_start_refused
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                break Err(CoreError::Json(
                    "screen start refused by host (consent not granted)".into(),
                ));
            }
            if self.screen_state() == ScreenState::Streaming {
                break Ok(());
            }
            if std::time::Instant::now() > deadline {
                break Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        // Close the refusal window (ticket → even) — after this, any Stop
        // reply from this or a stale channel is ignored by the pump latch.
        self.shared
            .screen_start_ticket
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        self.emit(UiEvent::screen_state_changed(state.as_str(), allowed, peer));
        outcome
    }

    /// Ask the host to begin sending with channel auto-dial and strongly typed report return.
    pub async fn screen_start_reported(
        &self,
        peer_target: Option<&str>,
    ) -> Result<ScreenControlReport, CoreError> {
        let target_id = match peer_target.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => {
                let id = self.resolve_peer(raw)?;
                let peers = self.live_peers();
                if !peers.iter().any(|p| p.device_id == id) {
                    return Err(CoreError::PeerNotFound(format!(
                        "peer {} is not connected",
                        id
                    )));
                }
                id
            }
            None => {
                if let Some(existing) = self.shared.screen_peer.lock().expect("screen_peer").clone()
                {
                    existing
                } else {
                    let mut peers = self.live_peers();
                    peers.sort_by_key(|p| p.device_id.to_string());
                    match peers.len() {
                        0 => return Err(CoreError::PeerNotFound("no connected peer".to_string())),
                        1 => peers[0].device_id.clone(),
                        _ => {
                            return Err(CoreError::Json(
                                "multiple peers connected; specify target peer".to_string(),
                            ))
                        }
                    }
                }
            }
        };

        if !self.screen_channel_open().await {
            self.open_screen(&target_id)
                .await
                .map_err(|e| CoreError::Json(format!("screen open: {e}")))?;
        }

        self.screen_start().await?;
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        Ok(ScreenControlReport {
            ok: true,
            action: "start".to_string(),
            state: state.as_str().to_string(),
            allowed,
            peer: Some(target_id.to_string()),
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    /// True while a screen control channel to a host is attached. The UI uses
    /// this to dial the screen channel on demand instead of tracking sessions.
    pub async fn screen_channel_open(&self) -> bool {
        self.shared.screen_ctl.lock().await.is_some()
    }

    /// Visitor: ask the host to stop sending.
    pub async fn screen_stop(&self) -> Result<(), CoreError> {
        self.send_screen_control(ControlMsg::Stop).await?;
        self.shared
            .screen_privacy_enabled
            .store(false, Ordering::Relaxed);
        self.shared
            .screen_privacy_lock_input
            .store(false, Ordering::Relaxed);
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        self.emit(UiEvent::screen_state_changed(state.as_str(), allowed, peer));
        Ok(())
    }

    /// Stop remote screen streaming with strongly typed report return.
    pub async fn screen_stop_reported(&self) -> Result<ScreenControlReport, CoreError> {
        self.screen_stop().await?;
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        Ok(ScreenControlReport {
            ok: true,
            action: "stop".to_string(),
            state: state.as_str().to_string(),
            allowed,
            peer,
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    /// Visitor: ask the host to emit an independent keyframe (IDR for H.264, v1.95 / ADR-081).
    pub async fn screen_keyframe_request(&self) -> Result<(), CoreError> {
        self.send_screen_control(ControlMsg::KeyframeRequest).await
    }

    /// Request a keyframe from the host with strongly typed report return.
    pub async fn screen_keyframe_request_reported(&self) -> Result<ScreenControlReport, CoreError> {
        self.screen_keyframe_request().await?;
        let peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        Ok(ScreenControlReport {
            ok: true,
            action: "keyframe".to_string(),
            state: state.as_str().to_string(),
            allowed,
            peer,
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    /// Override screen streaming target bitrate in bps (ADR-082). Clamped to [MIN, MAX].
    pub fn screen_set_bitrate_bps(&self, bps: u32) -> u32 {
        let actual = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_bitrate_bps(bps);
            engine.bitrate_bps()
        };
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.set_bitrate_bps(actual);
        }
        self.shared.screen_source.set_bitrate_bps(actual);
        actual
    }

    /// Current target screen streaming bitrate in bps.
    pub fn screen_bitrate_bps(&self) -> u32 {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .bitrate_bps()
    }

    /// Report current screen streaming bitrate (ADR-082).
    pub fn screen_bitrate_report(&self) -> ScreenBitrateReport {
        let bps = self.screen_bitrate_bps();
        ScreenBitrateReport {
            ok: true,
            bitrate_bps: bps,
            bitrate_kbps: bps / 1000,
            min_bps: tetherly_core::SCREEN_BITRATE_MIN_BPS,
            max_bps: tetherly_core::SCREEN_BITRATE_MAX_BPS,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Set and report screen streaming target bitrate in kbps (ADR-082).
    pub fn screen_set_bitrate_reported(&self, kbps: u32) -> ScreenBitrateReport {
        let bps = kbps.saturating_mul(1000);
        let actual_bps = self.screen_set_bitrate_bps(bps);
        ScreenBitrateReport {
            ok: true,
            bitrate_bps: actual_bps,
            bitrate_kbps: actual_bps / 1000,
            min_bps: tetherly_core::SCREEN_BITRATE_MIN_BPS,
            max_bps: tetherly_core::SCREEN_BITRATE_MAX_BPS,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Override screen streaming target frame rate in FPS (ADR-083). Clamped to [SCREEN_FPS_MIN, SCREEN_FPS_MAX].
    pub fn screen_set_fps(&self, fps: u32) -> u32 {
        let actual = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_fps(fps)
        };
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.set_fps(actual);
        }
        self.shared.screen_source.set_fps(actual);
        actual
    }

    /// Current target screen streaming frame rate in FPS.
    pub fn screen_fps(&self) -> u32 {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .fps_target()
    }

    /// Report current screen streaming frame rate (ADR-083).
    pub fn screen_fps_report(&self) -> ScreenFpsReport {
        let fps = self.screen_fps();
        let interval_us = 1_000_000u64 / (fps.max(1) as u64);
        ScreenFpsReport {
            ok: true,
            fps,
            min_fps: tetherly_core::SCREEN_FPS_MIN,
            max_fps: tetherly_core::SCREEN_FPS_MAX,
            interval_us,
            auto_profile: self.shared.screen_fps_auto,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Set and report screen streaming target frame rate in FPS (ADR-083).
    pub fn screen_set_fps_reported(&self, fps: u32) -> ScreenFpsReport {
        let actual = self.screen_set_fps(fps);
        let interval_us = 1_000_000u64 / (actual.max(1) as u64);
        ScreenFpsReport {
            ok: true,
            fps: actual,
            min_fps: tetherly_core::SCREEN_FPS_MIN,
            max_fps: tetherly_core::SCREEN_FPS_MAX,
            interval_us,
            auto_profile: self.shared.screen_fps_auto,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Override screen streaming resolution scale percentage (25..=100) (ADR-084).
    pub fn screen_set_scale_pct(&self, scale_pct: u32) -> u32 {
        let actual = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_scale_pct(scale_pct)
        };
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.set_scale_pct(actual);
        }
        self.shared.screen_source.set_scale_pct(actual);
        actual
    }

    /// Current target screen streaming resolution scale percentage.
    pub fn screen_scale_pct(&self) -> u32 {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .scale_pct()
    }

    /// Report current screen streaming resolution scale (ADR-084).
    pub fn screen_scale_report(&self) -> ScreenScaleReport {
        let scale_pct = self.screen_scale_pct();
        ScreenScaleReport {
            ok: true,
            scale_pct,
            min_scale_pct: tetherly_core::SCREEN_SCALE_MIN_PCT,
            max_scale_pct: tetherly_core::SCREEN_SCALE_MAX_PCT,
            auto_profile: self.shared.screen_scale_auto,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Set and report screen streaming resolution scale percentage (ADR-084).
    pub fn screen_set_scale_reported(&self, scale_pct: u32) -> ScreenScaleReport {
        let actual = self.screen_set_scale_pct(scale_pct);
        ScreenScaleReport {
            ok: true,
            scale_pct: actual,
            min_scale_pct: tetherly_core::SCREEN_SCALE_MIN_PCT,
            max_scale_pct: tetherly_core::SCREEN_SCALE_MAX_PCT,
            auto_profile: self.shared.screen_scale_auto,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Override screen streaming content profile (Text, Motion, Auto) (ADR-085).
    pub fn screen_set_profile(
        &self,
        profile: tetherly_core::ScreenContentProfile,
    ) -> tetherly_core::ScreenContentProfile {
        let actual = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_content_profile(profile)
        };
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.set_content_profile(actual);
        }
        self.shared.screen_source.set_content_profile(actual);
        actual
    }

    /// Override screen streaming quantization parameter (QP) range bounds (ADR-085).
    pub fn screen_set_qp_range(&self, min_qp: u8, max_qp: u8) -> (u8, u8) {
        let (min, max) = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_qp_range(min_qp, max_qp)
        };
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.set_qp_range(min, max);
        }
        self.shared.screen_source.set_qp_range(min, max);
        (min, max)
    }

    /// Current target screen streaming content profile.
    pub fn screen_profile(&self) -> tetherly_core::ScreenContentProfile {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .content_profile()
    }

    /// Current target screen streaming QP range bounds.
    pub fn screen_qp_range(&self) -> (u8, u8) {
        self.shared.screen_engine.lock().expect("screen").qp_range()
    }

    /// Report current screen streaming content profile and QP quality bounds (ADR-085).
    pub fn screen_profile_report(&self) -> ScreenProfileReport {
        let (profile, min_qp, max_qp, is_custom_qp) = {
            let engine = self.shared.screen_engine.lock().expect("screen");
            let qc = engine.quality_controller();
            (qc.profile(), qc.min_qp(), qc.max_qp(), qc.is_custom())
        };
        ScreenProfileReport {
            ok: true,
            profile,
            min_qp,
            max_qp,
            is_custom_qp,
            auto_profile: self.shared.screen_profile_auto,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Set and report screen streaming content profile (ADR-085).
    pub fn screen_set_profile_reported(
        &self,
        profile: tetherly_core::ScreenContentProfile,
    ) -> ScreenProfileReport {
        self.screen_set_profile(profile);
        self.screen_profile_report()
    }

    /// Set and report screen streaming QP range bounds (ADR-085).
    pub fn screen_set_qp_range_reported(&self, min_qp: u8, max_qp: u8) -> ScreenProfileReport {
        self.screen_set_qp_range(min_qp, max_qp);
        self.screen_profile_report()
    }

    /// Report current 4D autonomous adaptation matrix coordinator state (ADR-086).
    pub fn screen_matrix_report(&self) -> ScreenMatrixReport {
        let engine = self.shared.screen_engine.lock().expect("screen");
        let stats = engine.stats();
        let coord = engine.matrix_coordinator();
        ScreenMatrixReport {
            ok: true,
            mode: coord.mode().as_str().to_string(),
            health: coord.health().as_str().to_string(),
            bitrate_bps: stats.bitrate_bps,
            fps_target: stats.fps_target,
            scale_pct: stats.scale_pct,
            profile: stats.content_profile().as_str().to_string(),
            qp_min: stats.qp_min,
            qp_max: stats.qp_max,
            auto_fps: coord.auto_fps(),
            auto_scale: coord.auto_scale(),
            consecutive_healthy_acks: coord.consecutive_healthy_acks(),
            consecutive_drops: coord.consecutive_drops(),
            in_flight: stats.in_flight,
            rtt_us: stats.rtt_latest_us,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current 4D autonomous adaptation matrix coordinator mode (ADR-086).
    pub fn screen_matrix_mode(&self) -> tetherly_core::ScreenMatrixMode {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .matrix_coordinator()
            .mode()
    }

    /// Set and report 4D autonomous adaptation matrix coordinator mode (ADR-086).
    pub fn screen_set_matrix_mode(
        &self,
        mode: tetherly_core::ScreenMatrixMode,
    ) -> ScreenMatrixReport {
        {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_matrix_mode(mode);
        }
        self.screen_matrix_report()
    }

    /// Report current receiver playout buffer and dejittering metrics (ADR-087).
    pub fn screen_playout_report(&self) -> ScreenPlayoutReport {
        let mode = *self
            .shared
            .screen_playout_mode
            .lock()
            .expect("playout mode");
        let stats = *self
            .shared
            .screen_playout_stats
            .lock()
            .expect("playout stats");
        ScreenPlayoutReport {
            ok: true,
            mode: mode.as_str().to_string(),
            target_delay_ms: (stats.target_delay_us as f64 / 1000.0 * 10.0).round() / 10.0,
            current_jitter_ms: (stats.current_jitter_us as f64 / 1000.0 * 100.0).round() / 10.0,
            target_delay_us: stats.target_delay_us,
            current_jitter_us: stats.current_jitter_us,
            buffered_frames: stats.buffered_frames,
            played_frames: stats.played_frames,
            dropped_late_frames: stats.dropped_late_frames,
            dropped_overflow_frames: stats.dropped_overflow_frames,
            custom_delay_us: if stats.custom_delay {
                Some(stats.target_delay_us)
            } else {
                None
            },
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current receiver playout buffer mode (ADR-087).
    pub fn screen_playout_mode(&self) -> tetherly_core::ScreenPlayoutMode {
        *self
            .shared
            .screen_playout_mode
            .lock()
            .expect("playout mode")
    }

    /// Set and report receiver playout buffer mode (ADR-087).
    pub fn screen_set_playout_mode(
        &self,
        mode: tetherly_core::ScreenPlayoutMode,
    ) -> ScreenPlayoutReport {
        *self
            .shared
            .screen_playout_mode
            .lock()
            .expect("playout mode") = mode;
        let mut stats = self
            .shared
            .screen_playout_stats
            .lock()
            .expect("playout stats");
        stats.mode = mode;
        if !stats.custom_delay {
            stats.target_delay_us = match mode {
                tetherly_core::ScreenPlayoutMode::Adaptive => {
                    tetherly_core::SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US
                }
                tetherly_core::ScreenPlayoutMode::ZeroLatency => {
                    tetherly_core::SCREEN_PLAYOUT_MIN_DELAY_US
                }
                tetherly_core::ScreenPlayoutMode::Smooth => {
                    tetherly_core::SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US
                }
            };
        }
        drop(stats);
        self.screen_playout_report()
    }

    /// Report current hardware cursor decoupling status and metrics (ADR-088).
    pub fn screen_cursor_report(&self) -> ScreenCursorReport {
        let mode = *self.shared.screen_cursor_mode.lock().expect("cursor mode");
        let stats = *self
            .shared
            .screen_cursor_stats
            .lock()
            .expect("cursor stats");
        ScreenCursorReport {
            ok: true,
            mode: mode.as_str().to_string(),
            x: stats.current_x,
            y: stats.current_y,
            visible: stats.current_visible,
            shape: stats.current_shape.as_str().to_string(),
            updates_sent: stats.updates_sent,
            updates_received: stats.updates_received,
            updates_dropped_dedup: stats.updates_dropped_dedup,
            updates_dropped_rate: stats.updates_dropped_rate,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current hardware cursor decoupling mode (ADR-088).
    pub fn screen_cursor_mode(&self) -> tetherly_core::ScreenCursorMode {
        *self.shared.screen_cursor_mode.lock().expect("cursor mode")
    }

    /// Set and report hardware cursor decoupling mode (ADR-088).
    pub fn screen_set_cursor_mode(
        &self,
        mode: tetherly_core::ScreenCursorMode,
    ) -> ScreenCursorReport {
        *self.shared.screen_cursor_mode.lock().expect("cursor mode") = mode;
        let mut stats = self
            .shared
            .screen_cursor_stats
            .lock()
            .expect("cursor stats");
        stats.mode = mode;
        drop(stats);
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_cursor_mode(mode);
        self.screen_cursor_report()
    }

    /// Inject or push a cursor update from source to the remote screen visitor (ADR-088).
    pub async fn screen_send_cursor(&self, update: ScreenCursorUpdate) -> Result<bool, CoreError> {
        let update_opt = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let now_us = self.shared.clock.unix_us();
            let res = engine.push_cursor_update_with_time(update, now_us);
            *self
                .shared
                .screen_cursor_stats
                .lock()
                .expect("cursor stats") = engine.cursor_stats();
            res
        };
        let Some(up) = update_opt else {
            return Ok(false);
        };
        let frame = up.to_frame()?;
        let tx = self
            .shared
            .screen_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen channel closed".into()))?;
        if tx.try_send(frame).is_err() {
            return Err(CoreError::Json("screen peer gone".into()));
        }
        Ok(true)
    }

    /// Report current screen static frame adaptive squelch status and metrics (ADR-089).
    pub fn screen_squelch_report(&self) -> ScreenSquelchReport {
        let mode = *self
            .shared
            .screen_squelch_mode
            .lock()
            .expect("squelch mode");
        let stats = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .squelch_stats();
        ScreenSquelchReport {
            ok: true,
            mode: mode.as_str().to_string(),
            is_squelched: stats.is_squelched,
            frames_evaluated: stats.frames_evaluated,
            frames_passed: stats.frames_passed,
            frames_squelched: stats.frames_squelched,
            keepalive_frames: stats.keepalive_frames,
            last_dirty_ratio: stats.last_dirty_ratio,
            consecutive_static_frames: stats.consecutive_static_frames,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen static frame adaptive squelch mode (ADR-089).
    pub fn screen_squelch_mode(&self) -> tetherly_core::ScreenSquelchMode {
        *self
            .shared
            .screen_squelch_mode
            .lock()
            .expect("squelch mode")
    }

    /// Set and report screen static frame adaptive squelch mode (ADR-089).
    pub fn screen_set_squelch_mode(
        &self,
        mode: tetherly_core::ScreenSquelchMode,
    ) -> ScreenSquelchReport {
        *self
            .shared
            .screen_squelch_mode
            .lock()
            .expect("squelch mode") = mode;
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_squelch_mode(mode);
        self.screen_squelch_report()
    }

    /// Wake up squelch controller immediately (e.g. upon user input or client interaction) (ADR-089).
    /// Uses try_lock to avoid stalling high-frequency input dispatch when the video engine is encoding.
    pub fn screen_force_wake_squelch(&self) {
        let now_us = self.shared.clock.unix_us();
        if let Ok(mut engine) = self.shared.screen_engine.try_lock() {
            engine.force_wake_squelch(now_us);
        }
    }

    /// Report current forward error correction (FEC) protection metrics and state (v2.04 / ADR-090).
    pub fn screen_fec_report(&self) -> ScreenFecReport {
        let mode = *self.shared.screen_fec_mode.lock().expect("fec mode");
        let stats = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .fec_stats();
        let rx_stats = self.screen_rx_stats();
        ScreenFecReport {
            ok: true,
            mode: mode.as_str().to_string(),
            redundancy_pct: stats.redundancy_pct,
            blocks_encoded: stats.blocks_encoded,
            data_shards_sent: stats.data_shards_sent,
            parity_shards_sent: stats.parity_shards_sent,
            blocks_decoded: rx_stats.fec_recovered_shards + rx_stats.fec_unrecoverable_blocks,
            recovered_shards: rx_stats.fec_recovered_shards,
            unrecoverable_blocks: rx_stats.fec_unrecoverable_blocks,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen forward error correction mode (v2.04 / ADR-090).
    pub fn screen_fec_mode(&self) -> tetherly_core::ScreenFecMode {
        *self.shared.screen_fec_mode.lock().expect("fec mode")
    }

    /// Set and report screen forward error correction mode (v2.04 / ADR-090).
    pub fn screen_set_fec_mode(&self, mode: tetherly_core::ScreenFecMode) -> ScreenFecReport {
        *self.shared.screen_fec_mode.lock().expect("fec mode") = mode;
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_fec_mode(mode);
        self.screen_fec_report()
    }

    /// Report current packet pacing and burst shaper metrics and state (v2.05 / ADR-091).
    pub fn screen_pacer_report(&self) -> ScreenPacerReport {
        let mode = *self.shared.screen_pacer_mode.lock().expect("pacer mode");
        let stats = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .pacer_stats();
        ScreenPacerReport {
            ok: true,
            mode: mode.as_str().to_string(),
            packets_paced: stats.packets_paced,
            bursts_smoothed: stats.bursts_smoothed,
            avg_delay_us: stats.avg_delay_us,
            last_delay_us: stats.last_delay_us,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen packet pacing mode (v2.05 / ADR-091).
    pub fn screen_pacer_mode(&self) -> tetherly_core::ScreenPacerMode {
        *self.shared.screen_pacer_mode.lock().expect("pacer mode")
    }

    /// Set and report screen packet pacing mode (v2.05 / ADR-091).
    pub fn screen_set_pacer_mode(&self, mode: tetherly_core::ScreenPacerMode) -> ScreenPacerReport {
        *self.shared.screen_pacer_mode.lock().expect("pacer mode") = mode;
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_pacer_mode(mode);
        self.screen_pacer_report()
    }

    /// Report current delay-based trendline bufferbloat metrics and state (v2.06 / ADR-092).
    pub fn screen_trendline_report(&self) -> ScreenTrendlineReport {
        let mode = *self
            .shared
            .screen_trendline_mode
            .lock()
            .expect("trendline mode");
        let stats = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .trendline_stats();
        ScreenTrendlineReport {
            ok: true,
            mode,
            state: stats.state,
            samples_count: stats.samples_count,
            current_slope_us_per_frame: stats.current_slope_us_per_frame,
            threshold_us_per_frame: stats.threshold_us_per_frame,
            overuse_count: stats.overuse_count,
            underuse_count: stats.underuse_count,
            accumulated_delay_us: stats.accumulated_delay_us,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen delay trendline filter mode (v2.06 / ADR-092).
    pub fn screen_trendline_mode(&self) -> tetherly_core::ScreenTrendlineMode {
        *self
            .shared
            .screen_trendline_mode
            .lock()
            .expect("trendline mode")
    }

    /// Set and report screen delay trendline filter mode (v2.06 / ADR-092).
    pub fn screen_set_trendline_mode(
        &self,
        mode: tetherly_core::ScreenTrendlineMode,
    ) -> ScreenTrendlineReport {
        *self
            .shared
            .screen_trendline_mode
            .lock()
            .expect("trendline mode") = mode;
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_trendline_mode(mode);
        self.screen_trendline_report()
    }

    /// Report current screen selective NACK and Hybrid ARQ (HARQ) metrics and state (v2.07 / ADR-093).
    pub fn screen_harq_report(&self) -> ScreenHarqReport {
        let mode = *self.shared.screen_harq_mode.lock().expect("harq mode");
        let mut rep = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .harq_report();
        rep.mode = mode;
        let rx_stats = *self.shared.screen_rx_stats.lock().expect("rx stats");
        rep.nacks_sent = rx_stats.harq_nacks_sent;
        rep.recovered_frames = rx_stats.harq_recovered_frames;
        rep.timestamp_ms = self.shared.clock.unix_ms();
        rep
    }

    /// Current screen HARQ selective retransmission mode (v2.07 / ADR-093).
    pub fn screen_harq_mode(&self) -> tetherly_core::ScreenHarqMode {
        *self.shared.screen_harq_mode.lock().expect("harq mode")
    }

    /// Set and report screen HARQ selective retransmission mode (v2.07 / ADR-093).
    pub fn screen_set_harq_mode(&self, mode: tetherly_core::ScreenHarqMode) -> ScreenHarqReport {
        *self.shared.screen_harq_mode.lock().expect("harq mode") = mode;
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .set_harq_mode(mode);
        self.screen_harq_report()
    }

    /// Report current receiver-driven transport-wide congestion control (TWCC) state and metrics (v2.08 / ADR-094).
    pub fn screen_twcc_report(&self) -> ScreenTwccReport {
        let mode = *self.shared.screen_twcc_mode.lock().expect("twcc mode");
        let sender_rep = self
            .shared
            .screen_engine
            .lock()
            .map(|e| e.twcc_report())
            .unwrap_or_default();
        let rx_stats = *self.shared.screen_rx_stats.lock().expect("screen rx");
        let local_twcc_stats = *self.shared.screen_twcc_stats.lock().expect("twcc stats");
        ScreenTwccReport {
            ok: true,
            mode,
            feedbacks_sent: rx_stats
                .twcc_feedbacks_sent
                .max(local_twcc_stats.feedbacks_sent),
            feedbacks_received: sender_rep.feedbacks_received,
            packets_tracked: sender_rep
                .packets_tracked
                .max(local_twcc_stats.packets_tracked),
            packets_acknowledged: sender_rep.packets_acknowledged,
            packets_lost: sender_rep.packets_lost,
            loss_ratio: sender_rep.loss_ratio,
            delivery_rate_bps: sender_rep.delivery_rate_bps,
            delay_gradient_us: sender_rep.delay_gradient_us,
            smoothed_rtt_us: sender_rep.smoothed_rtt_us,
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen TWCC feedback mode (v2.08 / ADR-094).
    pub fn screen_twcc_mode(&self) -> tetherly_core::ScreenTwccMode {
        *self.shared.screen_twcc_mode.lock().expect("twcc mode")
    }

    /// Set and report screen TWCC feedback mode (v2.08 / ADR-094).
    pub fn screen_set_twcc_mode(&self, mode: tetherly_core::ScreenTwccMode) -> ScreenTwccReport {
        *self.shared.screen_twcc_mode.lock().expect("twcc mode") = mode;
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_twcc_mode(mode);
        }
        self.screen_twcc_report()
    }

    /// Report current active bandwidth probing state and metrics (v2.09 / ADR-095).
    pub fn screen_probe_report(&self) -> ScreenProbeReport {
        let mode = *self.shared.screen_probe_mode.lock().expect("probe mode");
        let sender_rep = self
            .shared
            .screen_engine
            .lock()
            .map(|e| e.probe_report())
            .unwrap_or_else(|_| ScreenProbeReport {
                ok: true,
                mode,
                ..Default::default()
            });
        let rx_stats = *self.shared.screen_rx_stats.lock().expect("screen rx");
        let local_probe_stats = *self.shared.screen_probe_stats.lock().expect("probe stats");
        ScreenProbeReport {
            ok: true,
            mode,
            state: if sender_rep.state != ScreenProbeState::Idle {
                sender_rep.state
            } else {
                local_probe_stats.state
            },
            probes_triggered: sender_rep
                .probes_triggered
                .max(rx_stats.probes_triggered)
                .max(local_probe_stats.probes_triggered),
            probes_completed: sender_rep
                .probes_completed
                .max(local_probe_stats.probes_completed),
            probes_failed: sender_rep
                .probes_failed
                .max(local_probe_stats.probes_failed),
            last_target_bps: if sender_rep.last_target_bps > 0 {
                sender_rep.last_target_bps
            } else {
                local_probe_stats.last_target_bps
            },
            last_measured_bps: if sender_rep.last_measured_bps > 0 {
                sender_rep.last_measured_bps
            } else {
                local_probe_stats.last_measured_bps
            },
            last_rtt_us: sender_rep.last_rtt_us.max(local_probe_stats.last_rtt_us),
            last_loss_ratio: if sender_rep.last_loss_ratio > 0.0 {
                sender_rep.last_loss_ratio
            } else {
                local_probe_stats.last_loss_ratio
            },
            last_delay_gradient_us: if sender_rep.last_delay_gradient_us != 0 {
                sender_rep.last_delay_gradient_us
            } else {
                local_probe_stats.last_delay_gradient_us
            },
            active_cluster_id: sender_rep
                .active_cluster_id
                .or(local_probe_stats.active_cluster_id),
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen active bandwidth probe mode (v2.09 / ADR-095).
    pub fn screen_probe_mode(&self) -> tetherly_core::ScreenProbeMode {
        *self.shared.screen_probe_mode.lock().expect("probe mode")
    }

    /// Set and report screen active bandwidth probe mode (v2.09 / ADR-095).
    pub fn screen_set_probe_mode(&self, mode: tetherly_core::ScreenProbeMode) -> ScreenProbeReport {
        *self.shared.screen_probe_mode.lock().expect("probe mode") = mode;
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_probe_mode(mode);
        }
        self.screen_probe_report()
    }

    /// Manually trigger active bandwidth probe burst to target_bps (or auto-calculated next ladder step).
    pub async fn trigger_screen_probe(
        &self,
        target_bps: Option<u32>,
    ) -> Result<ScreenProbeReport, CoreError> {
        let now_us = self.shared.clock.unix_us();
        let target = match target_bps {
            Some(t) => t,
            None => {
                let cur = self
                    .shared
                    .screen_engine
                    .lock()
                    .map(|e| e.bitrate_bps())
                    .unwrap_or(2_000_000);
                (cur * 3 / 2).clamp(
                    tetherly_core::SCREEN_BITRATE_MIN_BPS,
                    tetherly_core::SCREEN_BITRATE_MAX_BPS,
                )
            }
        };
        let triggered = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.trigger_probe(target, now_us)
        };
        if triggered {
            let tx_opt = self.shared.screen_out.lock().await.clone();
            if let Some(tx) = tx_opt {
                let mut pf = Vec::new();
                {
                    let mut engine = self.shared.screen_engine.lock().expect("screen");
                    while let Some(probe) = engine.pop_probe_packet() {
                        if let Ok(f) = probe.to_inner() {
                            pf.push(f);
                        }
                    }
                }
                for f in pf {
                    let f_len = (f.payload.len() + 12) as u64;
                    if tx.try_send(f).is_ok() {
                        self.shared.telemetry.record_screen_tx(f_len);
                    }
                }
            }
        }
        Ok(self.screen_probe_report())
    }

    /// Report current audio-video playout lip-sync and clock drift state and metrics (v2.10 / ADR-096).
    pub fn screen_sync_report(&self) -> ScreenAudioSyncReport {
        let mode = *self.shared.screen_sync_mode.lock().expect("sync mode");
        let sender_rep = self
            .shared
            .screen_engine
            .lock()
            .map(|e| e.sync_report())
            .unwrap_or_else(|_| ScreenAudioSyncReport {
                ok: true,
                mode,
                ..Default::default()
            });
        let rx_stats = *self.shared.screen_rx_stats.lock().expect("screen rx");
        let local_sync_stats = *self.shared.screen_sync_stats.lock().expect("sync stats");

        ScreenAudioSyncReport {
            ok: true,
            mode,
            beacons_sent: sender_rep
                .beacons_sent
                .max(rx_stats.sync_beacons_sent)
                .max(local_sync_stats.beacons_sent),
            beacons_received: sender_rep
                .beacons_received
                .max(rx_stats.sync_beacons_received)
                .max(local_sync_stats.beacons_received),
            current_skew_us: if sender_rep.current_skew_us != 0 {
                sender_rep.current_skew_us
            } else if rx_stats.sync_skew_us != 0 {
                rx_stats.sync_skew_us
            } else {
                local_sync_stats.current_skew_us
            },
            clock_drift_ppm: if sender_rep.clock_drift_ppm.abs() > 0.001 {
                sender_rep.clock_drift_ppm
            } else if rx_stats.sync_clock_drift_ppm.abs() > 0.001 {
                rx_stats.sync_clock_drift_ppm
            } else {
                local_sync_stats.clock_drift_ppm
            },
            adjustments_applied: sender_rep
                .adjustments_applied
                .max(rx_stats.sync_adjustments_applied)
                .max(local_sync_stats.adjustments_applied),
            in_sync: if rx_stats.sync_beacons_received > 0 {
                rx_stats.in_sync
            } else if sender_rep.beacons_sent > 0 {
                sender_rep.in_sync
            } else {
                local_sync_stats.in_sync
            },
            timestamp_ms: self.shared.clock.unix_ms(),
        }
    }

    /// Current screen audio-video sync mode (v2.10 / ADR-096).
    pub fn screen_sync_mode(&self) -> tetherly_core::ScreenAudioSyncMode {
        *self.shared.screen_sync_mode.lock().expect("sync mode")
    }

    /// Set and report screen audio-video sync mode (v2.10 / ADR-096).
    pub fn screen_set_sync_mode(
        &self,
        mode: tetherly_core::ScreenAudioSyncMode,
    ) -> ScreenAudioSyncReport {
        *self.shared.screen_sync_mode.lock().expect("sync mode") = mode;
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_sync_mode(mode);
        }
        self.screen_sync_report()
    }

    /// Manually trigger a screen sync beacon.
    pub async fn trigger_screen_sync(&self) -> Result<ScreenAudioSyncReport, CoreError> {
        let now_us = self.shared.clock.unix_us();
        let beacon = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.trigger_sync_beacon(now_us)
        };
        let tx_opt = self.shared.screen_out.lock().await.clone();
        if let Some(tx) = tx_opt {
            if let Ok(f) = beacon.to_inner() {
                let s_len = (f.payload.len() + 12) as u64;
                if tx.try_send(f).is_ok() {
                    self.shared.telemetry.record_screen_tx(s_len);
                }
            }
        }
        Ok(self.screen_sync_report())
    }

    /// Return the active display ID being captured or viewed (ADR-097).
    pub fn screen_active_display_id(&self) -> u8 {
        self.shared.screen_active_display_id.load(Ordering::Relaxed)
    }

    /// List enumerated displays available on this host (ADR-097).
    pub fn screen_displays(&self) -> Vec<tetherly_core::ScreenDisplayInfo> {
        let displays = self
            .shared
            .screen_displays
            .lock()
            .expect("displays")
            .clone();
        if !displays.is_empty() {
            displays
        } else {
            self.shared.screen_source.displays()
        }
    }

    /// Update available displays list (e.g. when physical displays plug/unplug or virtual displays are created) (ADR-097).
    pub fn screen_set_displays(&self, displays: Vec<tetherly_core::ScreenDisplayInfo>) {
        self.shared.screen_source.set_displays(displays.clone());
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_displays(displays.clone());
        }
        *self.shared.screen_displays.lock().expect("displays") = displays;
    }

    /// Return count of display switch operations executed during runtime (ADR-097).
    pub fn screen_display_switches_count(&self) -> u64 {
        self.shared
            .screen_display_switches_count
            .load(Ordering::Relaxed)
    }

    /// Comprehensive status report for screen displays and active streaming target (v2.11 / ADR-097).
    pub fn screen_display_report(&self) -> tetherly_core::ScreenDisplayReport {
        let active_id = self.screen_active_display_id();
        let displays = self.screen_displays();
        let switches = self.screen_display_switches_count();
        let last_switch = self
            .shared
            .screen_last_display_switch_us
            .load(Ordering::Relaxed);
        tetherly_core::ScreenDisplayReport {
            ok: true,
            active_display_id: active_id,
            displays,
            switches_count: switches,
            last_switch_us: last_switch,
        }
    }

    /// Dynamically switch active display during screen streaming session (ADR-097).
    /// Dispatches ControlMsg::SwitchDisplay over wire if visitor, or switches capture source & forces keyframe if host.
    pub async fn screen_switch_display(
        &self,
        display_id: u8,
    ) -> Result<tetherly_core::ScreenDisplayReport, CoreError> {
        let now_us = self.shared.clock.unix_us();

        let displays = self.screen_displays();
        if !displays.is_empty() && !displays.iter().any(|d| d.display_id == display_id) {
            return Err(CoreError::ScreenRefused(format!(
                "Display {display_id} not found"
            )));
        }

        // 1. If connected as visitor/controller, send SwitchDisplay control message across ctl channel
        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let msg = ControlMsg::SwitchDisplay { display_id };
            if ctl.send(msg).await.is_err() {
                return Err(CoreError::NoConnectedPeer);
            }
            self.shared
                .screen_active_display_id
                .store(display_id, Ordering::Relaxed);
            self.shared
                .screen_display_switches_count
                .fetch_add(1, Ordering::Relaxed);
            self.shared
                .screen_last_display_switch_us
                .store(now_us, Ordering::Relaxed);
            return Ok(self.screen_display_report());
        }

        // 2. Otherwise (host or standalone), switch source directly
        self.shared.screen_source.switch_display(display_id)?;
        self.shared.screen_source.force_idr();
        if let Some(sel) = &self.shared.screen_encode_codec {
            sel.force_idr();
        }

        {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let _ = engine.switch_display(display_id, now_us);
        }

        self.shared
            .screen_active_display_id
            .store(display_id, Ordering::Relaxed);
        self.shared
            .screen_display_switches_count
            .fetch_add(1, Ordering::Relaxed);
        self.shared
            .screen_last_display_switch_us
            .store(now_us, Ordering::Relaxed);

        Ok(self.screen_display_report())
    }

    /// Return current privacy mode and host input lockout status (v2.12 / ADR-098).
    pub fn screen_privacy_report(&self) -> tetherly_core::ScreenPrivacyReport {
        let enabled = self.shared.screen_privacy_enabled.load(Ordering::Relaxed);
        let lock_input = self
            .shared
            .screen_privacy_lock_input
            .load(Ordering::Relaxed);
        let blanking_method = self
            .shared
            .screen_privacy_blanking_method
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let last_change_us = self
            .shared
            .screen_privacy_last_change_us
            .load(Ordering::Relaxed);
        tetherly_core::ScreenPrivacyReport {
            ok: true,
            enabled,
            lock_input,
            blanking_method,
            last_change_us,
        }
    }

    /// Dynamically configure screen privacy mode (curtain blanking) and host physical input lockout (v2.12 / ADR-098).
    /// If connected as visitor/controller, dispatches ControlMsg::SetPrivacyMode over wire.
    /// If host or standalone, applies configuration directly to screen_engine.
    pub async fn screen_set_privacy_mode(
        &self,
        enabled: bool,
        lock_input: bool,
    ) -> Result<tetherly_core::ScreenPrivacyReport, CoreError> {
        let now_us = self.shared.clock.unix_us();

        // 1. If connected as visitor/controller, send SetPrivacyMode control message across ctl channel
        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let msg = ControlMsg::SetPrivacyMode {
                enabled,
                lock_input,
            };
            if ctl.send(msg).await.is_err() {
                return Err(CoreError::NoConnectedPeer);
            }
            self.shared
                .screen_privacy_enabled
                .store(enabled, Ordering::Relaxed);
            self.shared
                .screen_privacy_lock_input
                .store(lock_input, Ordering::Relaxed);
            self.shared
                .screen_privacy_last_change_us
                .store(now_us, Ordering::Relaxed);
            return Ok(self.screen_privacy_report());
        }

        // 2. Otherwise (host or standalone), configure engine directly
        {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.set_privacy_mode(enabled, lock_input, now_us);
        }

        self.shared
            .screen_privacy_enabled
            .store(enabled, Ordering::Relaxed);
        self.shared
            .screen_privacy_lock_input
            .store(lock_input, Ordering::Relaxed);
        self.shared
            .screen_privacy_last_change_us
            .store(now_us, Ordering::Relaxed);

        Ok(self.screen_privacy_report())
    }

    /// Report microsecond end-to-end screen streaming latency breakdown & multi-stage statistics (v2.13 / ADR-099).
    pub fn screen_latency_report(&self) -> ScreenLatencyReport {
        let mode = *self
            .shared
            .screen_latency_mode
            .lock()
            .expect("latency mode");
        let sender_rep = self
            .shared
            .screen_engine
            .lock()
            .map(|e| e.latency_report())
            .unwrap_or_else(|_| ScreenLatencyReport {
                mode: mode.as_str().to_string(),
                ..Default::default()
            });
        let rx_stats = *self.shared.screen_rx_stats.lock().expect("screen rx");
        let local_latency_stats = self
            .shared
            .screen_latency_stats
            .lock()
            .expect("latency stats")
            .clone();

        if sender_rep.frames_profiled > 0 {
            sender_rep
        } else if local_latency_stats.frames_profiled > 0 {
            local_latency_stats
        } else if rx_stats.latency_frames_profiled > 0 {
            ScreenLatencyReport {
                ok: true,
                mode: mode.as_str().to_string(),
                frames_profiled: rx_stats.latency_frames_profiled,
                bottleneck: ScreenBottleneckStage::from_tag(rx_stats.latency_bottleneck)
                    .as_str()
                    .to_string(),
                current: ScreenLatencyBreakdown {
                    frame_seq: rx_stats.latency_frames_profiled,
                    total_e2e_us: rx_stats.latency_total_e2e_us,
                    ..Default::default()
                },
                avg_e2e_us: rx_stats.latency_total_e2e_us,
                p50_e2e_us: rx_stats.latency_total_e2e_us,
                p90_e2e_us: rx_stats.latency_p95_e2e_us,
                p95_e2e_us: rx_stats.latency_p95_e2e_us,
                p99_e2e_us: rx_stats.latency_p95_e2e_us,
                min_e2e_us: rx_stats.latency_total_e2e_us,
                max_e2e_us: rx_stats.latency_p95_e2e_us,
                avg_capture_us: 0,
                avg_encode_us: 0,
                avg_pacer_us: 0,
                avg_network_us: 0,
                avg_jitter_us: 0,
                avg_decode_us: 0,
                avg_render_us: 0,
            }
        } else {
            ScreenLatencyReport {
                ok: true,
                mode: mode.as_str().to_string(),
                ..Default::default()
            }
        }
    }

    /// Return current screen latency profiler mode (v2.13 / ADR-099).
    pub fn screen_latency_mode(&self) -> tetherly_core::ScreenProfilerMode {
        *self
            .shared
            .screen_latency_mode
            .lock()
            .expect("latency mode")
    }

    /// Dynamically configure screen latency profiler mode (v2.13 / ADR-099).
    pub fn screen_set_latency_mode(
        &self,
        mode: tetherly_core::ScreenProfilerMode,
    ) -> ScreenLatencyReport {
        *self
            .shared
            .screen_latency_mode
            .lock()
            .expect("latency mode") = mode;
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_latency_mode(mode);
        }
        self.screen_latency_report()
    }

    /// Reset screen streaming latency profiler stats and ring buffer (v2.13 / ADR-099).
    /// If connected as visitor/controller, sends ControlMsg::ResetLatencyProfile over wire.
    /// If host or standalone, resets local engine profiler.
    pub async fn screen_reset_latency(&self) -> Result<ScreenLatencyReport, CoreError> {
        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let msg = ControlMsg::ResetLatencyProfile;
            let _ = ctl.send(msg).await;
        }
        let report = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.reset_latency()
        };
        *self
            .shared
            .screen_latency_stats
            .lock()
            .expect("latency stats") = report;
        {
            let mut rx_stats = self.shared.screen_rx_stats.lock().expect("screen rx");
            rx_stats.latency_frames_profiled = 0;
            rx_stats.latency_total_e2e_us = 0;
            rx_stats.latency_p95_e2e_us = 0;
            rx_stats.latency_bottleneck = ScreenBottleneckStage::Balanced.tag();
        }
        Ok(self.screen_latency_report())
    }

    /// Report bottleneck-driven autonomous tuning state and statistics (v2.14 / ADR-100).
    pub fn screen_tuning_report(&self) -> ScreenTuningReport {
        let mode = *self.shared.screen_tuning_mode.lock().expect("tuning mode");
        let engine_rep = self
            .shared
            .screen_engine
            .lock()
            .map(|e| e.tuning_report())
            .unwrap_or_else(|_| ScreenTuningReport {
                mode: mode.as_str().to_string(),
                ..Default::default()
            });
        let local_stats = self
            .shared
            .screen_tuning_stats
            .lock()
            .expect("tuning stats")
            .clone();
        if engine_rep.actions_applied > 0 || engine_rep.last_decision.is_some() {
            engine_rep
        } else if local_stats.actions_applied > 0 || local_stats.last_decision.is_some() {
            local_stats
        } else {
            ScreenTuningReport {
                ok: true,
                mode: mode.as_str().to_string(),
                ..Default::default()
            }
        }
    }

    /// Return current bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    pub fn screen_tuning_mode(&self) -> ScreenTunerMode {
        *self.shared.screen_tuning_mode.lock().expect("tuning mode")
    }

    /// Dynamically configure the bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    /// If connected as visitor/controller with an active screen session, sends
    /// ControlMsg::SetTuningMode to the peer as well.
    pub async fn screen_set_tuning_mode(&self, mode: ScreenTunerMode) -> ScreenTuningReport {
        *self.shared.screen_tuning_mode.lock().expect("tuning mode") = mode;
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_tuning_mode(mode);
        }
        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let _ = ctl
                .send(ControlMsg::SetTuningMode { mode: mode.tag() })
                .await;
        }
        self.screen_tuning_report()
    }

    /// Reset screen streaming autonomous tuning stats and recovery stack (v2.14 / ADR-100).
    /// If connected as visitor/controller, sends ControlMsg::ResetTuningProfile over wire.
    pub async fn screen_reset_tuning(&self) -> Result<ScreenTuningReport, CoreError> {
        let ctl_opt = self.shared.screen_ctl.lock().await.clone();
        if let Some(ctl) = ctl_opt {
            let _ = ctl.send(ControlMsg::ResetTuningProfile).await;
        }
        let report = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.reset_tuning()
        };
        *self
            .shared
            .screen_tuning_stats
            .lock()
            .expect("tuning stats") = report;
        Ok(self.screen_tuning_report())
    }
}
