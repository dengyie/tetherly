// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen ROI, scale, codec, pacing and tuner controls.

use super::*;
use std::sync::atomic::Ordering;
use tetherly_core::{ControlMsg, CoreError};

impl Node {
    /// Report dynamic Screen Region-of-Interest (ROI) status and dimensions (v2.21 / ADR-107).
    pub fn screen_roi_report(&self) -> tetherly_core::ScreenRoiReport {
        let roi = *self.shared.screen_roi.read().expect("lock");
        let updates = self.shared.screen_roi_updates.load(Ordering::Relaxed);
        let (src_w, src_h) = {
            let last = self.shared.screen_last_frame.lock().expect("last_frame");
            if let Some(f) = &*last {
                (f.rect.w, f.rect.h)
            } else {
                (0, 0)
            }
        };
        let (eff_w, eff_h) = if roi.active {
            (roi.w, roi.h)
        } else {
            (src_w, src_h)
        };
        tetherly_core::ScreenRoiReport {
            active: roi.active,
            x: roi.x,
            y: roi.y,
            w: roi.w,
            h: roi.h,
            zoom_percent: roi.zoom_percent,
            source_w: src_w,
            source_h: src_h,
            effective_w: eff_w,
            effective_h: eff_h,
            update_count: updates,
        }
    }

    /// Set dynamic Region-of-Interest (ROI) viewport cropping bounds and optional zoom (v2.21 / ADR-107).
    pub async fn screen_set_roi(
        &self,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        zoom_percent: Option<u16>,
    ) -> Result<tetherly_core::ScreenRoiReport, CoreError> {
        let (src_w, src_h) = {
            let last = self.shared.screen_last_frame.lock().expect("last_frame");
            if let Some(f) = &*last {
                (f.rect.w, f.rect.h)
            } else {
                (3840, 2160) // fallback max bounding box
            }
        };
        let mut new_roi = tetherly_core::ScreenRoi {
            active: true,
            x,
            y,
            w,
            h,
            zoom_percent: zoom_percent
                .unwrap_or_else(|| self.shared.screen_roi.read().expect("lock").zoom_percent),
        };
        new_roi.sanitize(src_w, src_h);
        {
            let mut roi = self.shared.screen_roi.write().expect("lock");
            *roi = new_roi;
            self.shared
                .screen_roi_updates
                .fetch_add(1, Ordering::Relaxed);
        }

        // Send 14-byte ControlMsg Tag 16
        let msg = ControlMsg::SetScreenRoi {
            x: new_roi.x,
            y: new_roi.y,
            w: new_roi.w,
            h: new_roi.h,
        };
        let _ = self.send_screen_control(msg).await;
        if let Some(zp) = zoom_percent {
            let zoom_msg = ControlMsg::SetScreenRoiZoom { zoom_percent: zp };
            let _ = self.send_screen_control(zoom_msg).await;
        }

        Ok(self.screen_roi_report())
    }

    /// Reset Region-of-Interest (ROI) back to full screen capture (v2.21 / ADR-107).
    pub async fn screen_reset_roi(&self) -> Result<tetherly_core::ScreenRoiReport, CoreError> {
        {
            let mut roi = self.shared.screen_roi.write().expect("lock");
            roi.active = false;
            roi.zoom_percent = 100;
            self.shared
                .screen_roi_updates
                .fetch_add(1, Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenRoi;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_roi_report())
    }

    /// Set Region-of-Interest (ROI) zoom scale percentage (v2.21 / ADR-107).
    pub async fn screen_set_roi_zoom(
        &self,
        zoom_percent: u16,
    ) -> Result<tetherly_core::ScreenRoiReport, CoreError> {
        let clamped = zoom_percent.clamp(50, 800);
        {
            let mut roi = self.shared.screen_roi.write().expect("lock");
            roi.zoom_percent = clamped;
            self.shared
                .screen_roi_updates
                .fetch_add(1, Ordering::Relaxed);
        }
        let msg = ControlMsg::SetScreenRoiZoom {
            zoom_percent: clamped,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_roi_report())
    }

    /// Report screen adaptive color depth & quantization configuration (v2.22 / ADR-108).
    pub fn screen_color_report(&self) -> tetherly_core::ScreenColorReport {
        let cfg = *self.shared.screen_color_mode.read().expect("lock");
        let updates = self.shared.screen_color_updates.load(Ordering::Relaxed);
        tetherly_core::ScreenColorReport {
            ok: true,
            mode: cfg.mode,
            threshold: cfg.threshold,
            is_active: cfg.is_active(),
            update_count: updates,
        }
    }

    /// Set screen adaptive color depth mode and optional binarization threshold (v2.22 / ADR-108).
    pub async fn screen_set_color_mode(
        &self,
        mode: tetherly_core::ScreenColorMode,
        threshold: Option<u8>,
    ) -> Result<tetherly_core::ScreenColorReport, CoreError> {
        let th = threshold
            .unwrap_or(tetherly_core::SCREEN_COLOR_DEFAULT_THRESHOLD)
            .clamp(1, 254);
        {
            let mut cfg = self.shared.screen_color_mode.write().expect("lock");
            cfg.mode = mode;
            cfg.threshold = th;
            self.shared
                .screen_color_updates
                .fetch_add(1, Ordering::Relaxed);
        }
        let msg = ControlMsg::SetScreenColorMode {
            mode: mode.tag(),
            threshold: th,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_color_report())
    }

    /// Reset screen adaptive color depth back to 32-bit native full color (v2.22 / ADR-108).
    pub async fn screen_reset_color_mode(
        &self,
    ) -> Result<tetherly_core::ScreenColorReport, CoreError> {
        {
            let mut cfg = self.shared.screen_color_mode.write().expect("lock");
            *cfg = tetherly_core::ScreenColorConfig::default();
            self.shared
                .screen_color_updates
                .fetch_add(1, Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenColorMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_color_report())
    }

    /// Report screen differential delta patch configuration and telemetry (v2.23 / ADR-109).
    pub fn screen_delta_report(&self) -> tetherly_core::ScreenDeltaReport {
        let cfg = *self.shared.screen_delta_config.read().expect("lock");
        tetherly_core::ScreenDeltaReport {
            ok: true,
            enabled: cfg.enabled,
            keyframe_interval: cfg.keyframe_interval,
            delta_frames_sent: self.shared.screen_delta_frames_sent.load(Ordering::Relaxed),
            keyframes_sent: self
                .shared
                .screen_delta_keyframes_sent
                .load(Ordering::Relaxed),
            delta_frames_received: self
                .shared
                .screen_delta_frames_received
                .load(Ordering::Relaxed),
            keyframes_received: self
                .shared
                .screen_delta_keyframes_received
                .load(Ordering::Relaxed),
            saved_bytes: self.shared.screen_delta_saved_bytes.load(Ordering::Relaxed),
        }
    }

    /// Set screen differential delta patch mode and keyframe interval (v2.23 / ADR-109).
    pub async fn screen_set_delta_mode(
        &self,
        enabled: bool,
        keyframe_interval: Option<u16>,
    ) -> Result<tetherly_core::ScreenDeltaReport, CoreError> {
        let interval = keyframe_interval
            .unwrap_or(tetherly_core::SCREEN_DELTA_DEFAULT_KEYFRAME_INTERVAL)
            .clamp(1, 600);
        {
            let mut cfg = self.shared.screen_delta_config.write().expect("lock");
            cfg.enabled = enabled;
            cfg.keyframe_interval = interval;
            self.shared.screen_delta_counter.store(0, Ordering::Relaxed);
        }
        let msg = ControlMsg::SetScreenDeltaMode {
            enabled,
            keyframe_interval: interval,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_delta_report())
    }

    /// Reset screen differential delta patch mode back to default full-frame streaming (v2.23 / ADR-109).
    pub async fn screen_reset_delta_mode(
        &self,
    ) -> Result<tetherly_core::ScreenDeltaReport, CoreError> {
        {
            let mut cfg = self.shared.screen_delta_config.write().expect("lock");
            *cfg = tetherly_core::ScreenDeltaConfig::default();
            self.shared.screen_delta_counter.store(0, Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenDeltaMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_delta_report())
    }

    /// Report screen objective visual quality metric configuration and evaluation telemetry (v2.24 / ADR-110).
    pub fn screen_quality_metric_report(&self) -> tetherly_core::ScreenQualityMetricReport {
        let cfg = *self
            .shared
            .screen_quality_metric_config
            .read()
            .expect("lock");
        let state = self
            .shared
            .screen_quality_metric_state
            .read()
            .unwrap()
            .clone();
        let (avg_mse, avg_psnr_db, avg_ssim) = if state.evaluated_frames > 0 {
            (
                state.sum_mse / (state.evaluated_frames as f64),
                state.sum_psnr_db / (state.evaluated_frames as f64),
                state.sum_ssim / (state.evaluated_frames as f64),
            )
        } else {
            (0.0, 100.0, 1.0)
        };
        let quality_warning = cfg.enabled
            && state.evaluated_frames > 0
            && (state.last_psnr_db < 30.0 || state.last_ssim < 0.85);

        tetherly_core::ScreenQualityMetricReport {
            ok: true,
            enabled: cfg.enabled,
            sample_interval: cfg.sample_interval,
            evaluated_frames: state.evaluated_frames,
            avg_mse,
            avg_psnr_db,
            avg_ssim,
            last_psnr_db: state.last_psnr_db,
            last_ssim: state.last_ssim,
            quality_warning,
        }
    }

    /// Set screen objective visual quality metric mode and sample interval (v2.24 / ADR-110).
    pub async fn screen_set_quality_metric_mode(
        &self,
        enabled: bool,
        sample_interval: Option<u8>,
    ) -> Result<tetherly_core::ScreenQualityMetricReport, CoreError> {
        let interval = sample_interval
            .unwrap_or(tetherly_core::SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL)
            .clamp(1, 120);
        {
            let mut cfg = self
                .shared
                .screen_quality_metric_config
                .write()
                .expect("lock");
            cfg.enabled = enabled;
            cfg.sample_interval = interval;
            self.shared
                .screen_metric_counter
                .store(0, Ordering::Relaxed);
        }
        let msg = ControlMsg::SetScreenQualityMetricMode {
            enabled,
            sample_interval: interval,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_quality_metric_report())
    }

    /// Reset screen objective visual quality metric mode back to default disabled (v2.24 / ADR-110).
    pub async fn screen_reset_quality_metric_mode(
        &self,
    ) -> Result<tetherly_core::ScreenQualityMetricReport, CoreError> {
        {
            let mut cfg = self
                .shared
                .screen_quality_metric_config
                .write()
                .expect("lock");
            *cfg = tetherly_core::ScreenQualityMetricConfig::default();
            self.shared
                .screen_metric_counter
                .store(0, Ordering::Relaxed);
            let mut state = self
                .shared
                .screen_quality_metric_state
                .write()
                .expect("lock");
            *state = ScreenQualityMetricState::default();
            *self.shared.screen_metric_ref_frame.write().expect("lock") = None;
        }
        let msg = ControlMsg::ResetScreenQualityMetricMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_quality_metric_report())
    }

    /// Report screen autonomous closed-loop QoE adaptation configuration and controller status (v2.25 / ADR-111).
    pub fn screen_qoe_report(&self) -> tetherly_core::ScreenQoeReport {
        let now_us = self.shared.clock.unix_us();
        self.shared
            .screen_qoe_controller
            .read()
            .unwrap()
            .report(now_us)
    }

    /// Configure screen autonomous closed-loop QoE adaptation policy mode and targets (v2.25 / ADR-111).
    pub async fn screen_set_qoe_mode(
        &self,
        mode: tetherly_core::ScreenQoeMode,
        target_psnr_db: Option<u8>,
        target_ssim_pct: Option<u8>,
    ) -> Result<tetherly_core::ScreenQoeReport, CoreError> {
        let (psnr, ssim) = {
            let mut qoe = self.shared.screen_qoe_controller.write().expect("lock");
            let cfg = tetherly_core::ScreenQoeConfig::new(mode, target_psnr_db, target_ssim_pct);
            qoe.set_config(cfg);
            (cfg.target_psnr_db, cfg.target_ssim_pct)
        };

        if mode.is_active() {
            let mut metric_cfg = self
                .shared
                .screen_quality_metric_config
                .write()
                .expect("lock");
            metric_cfg.enabled = true;
        }

        let msg = ControlMsg::SetScreenQoeMode {
            mode: mode.tag(),
            target_psnr_db: psnr,
            target_ssim_pct: ssim,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_qoe_report())
    }

    /// Reset screen autonomous closed-loop QoE adaptation back to default disabled state (v2.25 / ADR-111).
    pub async fn screen_reset_qoe_mode(&self) -> Result<tetherly_core::ScreenQoeReport, CoreError> {
        {
            let mut qoe = self.shared.screen_qoe_controller.write().expect("lock");
            qoe.reset();
        }
        let msg = ControlMsg::ResetScreenQoeMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_qoe_report())
    }

    /// Report screen streaming Progressive Intra-Refresh (PIR) configuration and status (v2.26 / ADR-112).
    pub fn screen_pir_report(&self) -> tetherly_core::ScreenPirReport {
        let cfg = *self.shared.screen_pir_config.read().expect("lock");
        tetherly_core::ScreenPirReport {
            ok: true,
            mode: cfg.mode.as_str().to_string(),
            bands: cfg.bands,
            sweeps_completed: self
                .shared
                .screen_pir_sweeps_completed
                .load(std::sync::atomic::Ordering::Relaxed),
            slices_sent: self
                .shared
                .screen_pir_slices_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            current_band: self
                .shared
                .screen_pir_current_band
                .load(std::sync::atomic::Ordering::Relaxed),
            active_sweep: self
                .shared
                .screen_pir_active_sweep
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming Progressive Intra-Refresh (PIR) mode and bands (v2.26 / ADR-112).
    pub async fn screen_set_pir_mode(
        &self,
        mode: tetherly_core::ScreenPirMode,
        bands: Option<u8>,
    ) -> Result<tetherly_core::ScreenPirReport, CoreError> {
        let b = bands
            .unwrap_or(tetherly_core::SCREEN_PIR_DEFAULT_BANDS)
            .clamp(
                tetherly_core::SCREEN_PIR_MIN_BANDS,
                tetherly_core::SCREEN_PIR_MAX_BANDS,
            );
        {
            let mut cfg = self.shared.screen_pir_config.write().expect("lock");
            cfg.mode = mode;
            cfg.bands = b;
            if mode == tetherly_core::ScreenPirMode::OnDemand {
                self.shared
                    .screen_pir_active_sweep
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                self.shared
                    .screen_pir_current_band
                    .store(0, std::sync::atomic::Ordering::Relaxed);
            } else if mode == tetherly_core::ScreenPirMode::Continuous {
                self.shared
                    .screen_pir_active_sweep
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                self.shared
                    .screen_pir_active_sweep
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let msg = ControlMsg::SetScreenPirMode {
            mode: mode.tag(),
            bands: b,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_pir_report())
    }

    /// Reset screen streaming Progressive Intra-Refresh (PIR) back to default disabled state (v2.26 / ADR-112).
    pub async fn screen_reset_pir_mode(&self) -> Result<tetherly_core::ScreenPirReport, CoreError> {
        {
            let mut cfg = self.shared.screen_pir_config.write().expect("lock");
            *cfg = tetherly_core::ScreenPirConfig::default();
            self.shared
                .screen_pir_active_sweep
                .store(false, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_pir_current_band
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenPirMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_pir_report())
    }

    /// Report screen streaming Long-Term Reference (LTR) configuration and metrics (v2.27 / ADR-113).
    pub fn screen_ltr_report(&self) -> tetherly_core::ScreenLtrReport {
        let cfg = *self.shared.screen_ltr_config.read().expect("lock");
        tetherly_core::ScreenLtrReport {
            ok: true,
            enabled: cfg.enabled,
            auto_interval: cfg.auto_interval,
            current_ltr_seq: self
                .shared
                .screen_ltr_current_seq
                .load(std::sync::atomic::Ordering::Relaxed),
            anchors_established: self
                .shared
                .screen_ltr_anchors_established
                .load(std::sync::atomic::Ordering::Relaxed),
            recoveries_requested: self
                .shared
                .screen_ltr_recoveries_requested
                .load(std::sync::atomic::Ordering::Relaxed),
            recoveries_completed: self
                .shared
                .screen_ltr_recoveries_completed
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_burst_bytes: self
                .shared
                .screen_ltr_saved_burst_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming Long-Term Reference (LTR) mode and auto-anchor interval (v2.27 / ADR-113).
    pub async fn screen_set_ltr_mode(
        &self,
        enabled: bool,
        auto_interval: Option<u16>,
    ) -> Result<tetherly_core::ScreenLtrReport, CoreError> {
        let interval = auto_interval
            .unwrap_or(tetherly_core::SCREEN_LTR_DEFAULT_AUTO_INTERVAL)
            .clamp(
                tetherly_core::SCREEN_LTR_MIN_INTERVAL,
                tetherly_core::SCREEN_LTR_MAX_INTERVAL,
            );
        {
            let mut cfg = self.shared.screen_ltr_config.write().expect("lock");
            cfg.enabled = enabled;
            cfg.auto_interval = interval;
            if !enabled {
                *self.shared.screen_sender_ltr_canvas.write().expect("lock") = None;
                *self
                    .shared
                    .screen_receiver_ltr_canvas
                    .write()
                    .expect("lock") = None;
                self.shared
                    .screen_ltr_current_seq
                    .store(0, std::sync::atomic::Ordering::Relaxed);
                self.shared
                    .screen_ltr_recovery_pending
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let msg = ControlMsg::SetScreenLtrMode {
            enabled,
            auto_interval: interval,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_ltr_report())
    }

    /// Explicitly trigger anchoring of the current frame as the golden LTR reference (v2.27 / ADR-113).
    pub async fn screen_anchor_ltr(&self) -> Result<tetherly_core::ScreenLtrReport, CoreError> {
        let ltr_seq = {
            let maybe_canvas = self
                .shared
                .screen_receiver_canvas
                .read()
                .unwrap()
                .clone()
                .or_else(|| {
                    self.shared
                        .screen_sender_ltr_canvas
                        .read()
                        .expect("lock")
                        .clone()
                });
            if let Some(canvas) = maybe_canvas {
                *self.shared.screen_sender_ltr_canvas.write().expect("lock") = Some(canvas.clone());
                *self
                    .shared
                    .screen_receiver_ltr_canvas
                    .write()
                    .expect("lock") = Some(canvas.clone());
                self.shared
                    .screen_ltr_current_seq
                    .store(canvas.seq, std::sync::atomic::Ordering::Relaxed);
                self.shared
                    .screen_ltr_anchors_established
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                canvas.seq
            } else {
                let seq = self
                    .shared
                    .screen_ltr_current_seq
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                self.shared
                    .screen_ltr_anchors_established
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                seq
            }
        };
        let msg = ControlMsg::LtrAnchorSync { ltr_seq };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_ltr_report())
    }

    /// Request instant differential recovery against the current golden LTR anchor (v2.27 / ADR-113).
    pub async fn screen_request_ltr_recovery(
        &self,
    ) -> Result<tetherly_core::ScreenLtrReport, CoreError> {
        let ltr_seq = self
            .shared
            .screen_ltr_current_seq
            .load(std::sync::atomic::Ordering::Relaxed);
        self.shared
            .screen_ltr_recoveries_requested
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.shared
            .screen_ltr_recovery_pending
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let msg = ControlMsg::LtrRecoveryRequest { ltr_seq };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_ltr_report())
    }

    /// Reset screen streaming LTR mode back to default disabled state and purge golden canvases (v2.27 / ADR-113).
    pub async fn screen_reset_ltr_mode(&self) -> Result<tetherly_core::ScreenLtrReport, CoreError> {
        {
            let mut cfg = self.shared.screen_ltr_config.write().expect("lock");
            *cfg = tetherly_core::ScreenLtrConfig::default();
            *self.shared.screen_sender_ltr_canvas.write().expect("lock") = None;
            *self
                .shared
                .screen_receiver_ltr_canvas
                .write()
                .expect("lock") = None;
            self.shared
                .screen_ltr_current_seq
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_ltr_recovery_pending
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenLtrMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_ltr_report())
    }

    /// Report screen streaming 2-Layer Temporal Scalability (SVC-T) status and telemetry (v2.28 / ADR-114).
    pub fn screen_svc_report(&self) -> tetherly_core::ScreenSvcReport {
        let cfg = *self.shared.screen_svc_config.read().expect("lock");
        tetherly_core::ScreenSvcReport {
            ok: true,
            enabled: cfg.is_enabled(),
            mode: cfg.mode.as_str().to_string(),
            drop_t1: cfg.drop_t1,
            t0_frames_sent: self
                .shared
                .screen_svc_t0_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            t1_frames_sent: self
                .shared
                .screen_svc_t1_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            t1_frames_dropped: self
                .shared
                .screen_svc_t1_frames_dropped
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_svc_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming SVC mode and T1 frame drop policy (v2.28 / ADR-114).
    pub async fn screen_set_svc_mode(
        &self,
        mode: tetherly_core::ScreenSvcMode,
        drop_t1: bool,
    ) -> Result<tetherly_core::ScreenSvcReport, CoreError> {
        {
            let mut cfg = self.shared.screen_svc_config.write().expect("lock");
            cfg.mode = mode;
            cfg.drop_t1 = drop_t1;
            if !cfg.is_enabled() {
                *self.shared.screen_sender_t0_canvas.write().expect("lock") = None;
                *self.shared.screen_receiver_t0_canvas.write().expect("lock") = None;
            }
        }
        let msg = ControlMsg::SetScreenSvcMode {
            mode: mode.as_u8(),
            drop_t1,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_svc_report())
    }

    /// Set or toggle T1 frame drop policy under SVC mode (v2.28 / ADR-114).
    pub async fn screen_set_svc_drop_t1(
        &self,
        drop_t1: bool,
    ) -> Result<tetherly_core::ScreenSvcReport, CoreError> {
        let mode = self.shared.screen_svc_config.read().expect("lock").mode;
        self.screen_set_svc_mode(mode, drop_t1).await
    }

    /// Reset screen streaming SVC mode to default disabled state and reset metrics (v2.28 / ADR-114).
    pub async fn screen_reset_svc_mode(&self) -> Result<tetherly_core::ScreenSvcReport, CoreError> {
        {
            let mut cfg = self.shared.screen_svc_config.write().expect("lock");
            *cfg = tetherly_core::ScreenSvcConfig::default();
            *self.shared.screen_sender_t0_canvas.write().expect("lock") = None;
            *self.shared.screen_receiver_t0_canvas.write().expect("lock") = None;
            self.shared
                .screen_svc_t0_frames_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_svc_t1_frames_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_svc_t1_frames_dropped
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_svc_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenSvcMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_svc_report())
    }

    /// Report screen streaming 2D motion vector estimation and scroll acceleration status (v2.29 / ADR-115).
    pub fn screen_motion_report(&self) -> tetherly_core::ScreenMotionReport {
        let cfg = *self.shared.screen_motion_config.read().expect("lock");
        tetherly_core::ScreenMotionReport {
            ok: true,
            enabled: cfg.enabled,
            max_displacement: cfg.max_displacement,
            motion_frames_sent: self
                .shared
                .screen_motion_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            fallback_frames_sent: self
                .shared
                .screen_motion_fallback_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_motion_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming 2D motion vector estimation mode (v2.29 / ADR-115).
    pub async fn screen_set_motion_mode(
        &self,
        enabled: bool,
        max_displacement: u16,
    ) -> Result<tetherly_core::ScreenMotionReport, CoreError> {
        let max_displacement = max_displacement.clamp(
            tetherly_core::SCREEN_MOTION_MIN_DISPLACEMENT,
            tetherly_core::SCREEN_MOTION_MAX_DISPLACEMENT,
        );
        {
            let mut cfg = self.shared.screen_motion_config.write().expect("lock");
            cfg.enabled = enabled;
            cfg.max_displacement = max_displacement;
            if !enabled {
                *self
                    .shared
                    .screen_sender_motion_canvas
                    .write()
                    .expect("lock") = None;
            }
        }
        let msg = ControlMsg::SetScreenMotionMode {
            enabled,
            max_displacement,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_motion_report())
    }

    /// Reset screen streaming 2D motion vector estimation mode to default and clear metrics (v2.29 / ADR-115).
    pub async fn screen_reset_motion_mode(
        &self,
    ) -> Result<tetherly_core::ScreenMotionReport, CoreError> {
        {
            let mut cfg = self.shared.screen_motion_config.write().expect("lock");
            *cfg = tetherly_core::ScreenMotionConfig::default();
            *self
                .shared
                .screen_sender_motion_canvas
                .write()
                .expect("lock") = None;
            self.shared
                .screen_motion_frames_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_motion_fallback_frames_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_motion_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenMotionMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_motion_report())
    }

    /// Report screen streaming spatial tile-grid decomposition status (v2.30 / ADR-116).
    pub fn screen_tile_report(&self) -> tetherly_core::ScreenTileReport {
        let cfg = *self.shared.screen_tile_config.read().expect("lock");
        tetherly_core::ScreenTileReport {
            ok: true,
            enabled: cfg.enabled,
            tile_size: cfg.tile_size,
            grid_frames_sent: self
                .shared
                .screen_tile_frames_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            dirty_tiles_sent: self
                .shared
                .screen_tile_dirty_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            skipped_tiles_count: self
                .shared
                .screen_tile_skipped_count
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_tile_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming spatial tile-grid decomposition mode (v2.30 / ADR-116).
    pub async fn screen_set_tile_mode(
        &self,
        enabled: bool,
        tile_size: u16,
    ) -> Result<tetherly_core::ScreenTileReport, CoreError> {
        let tile_size = if tile_size == 0 {
            tetherly_core::SCREEN_TILE_DEFAULT_SIZE
        } else {
            tile_size.clamp(
                tetherly_core::SCREEN_TILE_MIN_SIZE,
                tetherly_core::SCREEN_TILE_MAX_SIZE,
            )
        };
        {
            let mut cfg = self.shared.screen_tile_config.write().expect("lock");
            cfg.enabled = enabled;
            cfg.tile_size = tile_size;
            if !enabled {
                *self.shared.screen_sender_tile_canvas.write().expect("lock") = None;
            }
        }
        let msg = ControlMsg::SetScreenTileMode { enabled, tile_size };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_report())
    }

    /// Reset screen streaming spatial tile-grid decomposition mode to default and clear metrics (v2.30 / ADR-116).
    pub async fn screen_reset_tile_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileConfig::default();
            *self.shared.screen_sender_tile_canvas.write().expect("lock") = None;
            self.shared
                .screen_tile_frames_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_dirty_tiles_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_skipped_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_report())
    }

    /// Report screen streaming tile dictionary cache status (v2.31 / ADR-117).
    pub fn screen_tile_cache_report(&self) -> tetherly_core::ScreenTileCacheReport {
        let cfg = *self.shared.screen_tile_cache_config.read().expect("lock");
        let hits = self
            .shared
            .screen_tile_cache_hits
            .load(std::sync::atomic::Ordering::Relaxed);
        let misses = self
            .shared
            .screen_tile_cache_misses
            .load(std::sync::atomic::Ordering::Relaxed);
        let total = hits + misses;
        let hit_ratio_pct = if total > 0 {
            (hits as f32 / total as f32) * 100.0
        } else {
            0.0
        };
        let current_entries = self
            .shared
            .screen_tile_cache_dict
            .lock()
            .unwrap()
            .current_entries();
        tetherly_core::ScreenTileCacheReport {
            ok: true,
            enabled: cfg.enabled,
            max_entries: cfg.max_entries,
            current_entries,
            cache_hits: hits,
            cache_misses: misses,
            hit_ratio_pct,
            saved_bandwidth_bytes: self
                .shared
                .screen_tile_cache_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming tile dictionary cache mode (v2.31 / ADR-117).
    pub async fn screen_set_tile_cache_mode(
        &self,
        enabled: bool,
        max_entries: u16,
    ) -> Result<tetherly_core::ScreenTileCacheReport, CoreError> {
        let max_entries = if max_entries == 0 {
            tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        } else {
            max_entries.clamp(
                tetherly_core::SCREEN_TILE_CACHE_MIN_ENTRIES,
                tetherly_core::SCREEN_TILE_CACHE_MAX_ENTRIES,
            )
        };
        {
            let mut cfg = self.shared.screen_tile_cache_config.write().expect("lock");
            cfg.enabled = enabled;
            cfg.max_entries = max_entries;
            self.shared
                .screen_tile_cache_dict
                .lock()
                .unwrap()
                .resize(max_entries);
            self.shared
                .screen_tile_cache_receiver
                .lock()
                .unwrap()
                .resize(max_entries as usize, None);
        }
        let msg = ControlMsg::SetScreenTileCacheMode {
            enabled,
            max_entries,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_cache_report())
    }

    /// Reset screen streaming tile dictionary cache mode to default disabled and purge dictionary (v2.31 / ADR-117).
    pub async fn screen_reset_tile_cache_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileCacheReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_cache_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileCacheConfig::default();
            self.shared
                .screen_tile_cache_dict
                .lock()
                .expect("lock")
                .clear();
            let mut rx = self.shared.screen_tile_cache_receiver.lock().expect("lock");
            rx.clear();
            rx.resize(
                tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES as usize,
                None,
            );
            self.shared
                .screen_tile_cache_hits
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_cache_misses
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_cache_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileCacheMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_cache_report())
    }

    /// Report screen streaming foveated focus awareness and spatial priority scheduling status (v2.32 / ADR-118).
    pub fn screen_focus_report(&self) -> tetherly_core::ScreenFocusReport {
        let cfg = *self.shared.screen_focus_config.read().expect("lock");
        tetherly_core::ScreenFocusReport {
            ok: true,
            enabled: cfg.enabled,
            mode: cfg.mode,
            radius: cfg.radius,
            peripheral_skip_ratio: cfg.peripheral_skip_ratio,
            box_x: cfg.box_x,
            box_y: cfg.box_y,
            box_w: cfg.box_w,
            box_h: cfg.box_h,
            focus_tiles_sent: self
                .shared
                .screen_focus_tiles_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            peripheral_tiles_sent: self
                .shared
                .screen_focus_peripheral_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            peripheral_tiles_skipped: self
                .shared
                .screen_focus_peripheral_skipped
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_focus_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming foveated focus mode and parameters (v2.32 / ADR-118).
    pub async fn screen_set_focus_mode(
        &self,
        enabled: bool,
        mode: Option<tetherly_core::ScreenFocusMode>,
        radius: Option<u16>,
        peripheral_skip_ratio: Option<u8>,
        box_coords: Option<(u16, u16, u16, u16)>,
    ) -> Result<tetherly_core::ScreenFocusReport, CoreError> {
        let (mode, radius, peripheral_skip_ratio, (box_x, box_y, box_w, box_h)) = {
            let mut cfg = self.shared.screen_focus_config.write().expect("lock");
            cfg.enabled = enabled;
            if let Some(m) = mode {
                cfg.mode = m;
            }
            if let Some(r) = radius {
                cfg.radius = r.clamp(
                    tetherly_core::SCREEN_FOCUS_MIN_RADIUS,
                    tetherly_core::SCREEN_FOCUS_MAX_RADIUS,
                );
            }
            if let Some(s) = peripheral_skip_ratio {
                cfg.peripheral_skip_ratio = s.clamp(0, tetherly_core::SCREEN_FOCUS_MAX_SKIP_RATIO);
            }
            if let Some((bx, by, bw, bh)) = box_coords {
                cfg.box_x = bx;
                cfg.box_y = by;
                cfg.box_w = bw;
                cfg.box_h = bh;
            }
            (
                cfg.mode,
                cfg.radius,
                cfg.peripheral_skip_ratio,
                (cfg.box_x, cfg.box_y, cfg.box_w, cfg.box_h),
            )
        };
        let msg = ControlMsg::SetScreenFocusMode {
            enabled,
            mode: mode.to_u8(),
            radius,
            peripheral_skip_ratio,
        };
        let _ = self.send_screen_control(msg).await;
        if box_coords.is_some() {
            let box_msg = ControlMsg::SetScreenFocusBox {
                box_x,
                box_y,
                box_w,
                box_h,
            };
            let _ = self.send_screen_control(box_msg).await;
        }
        Ok(self.screen_focus_report())
    }

    /// Reset screen streaming foveated focus mode to default disabled and clear statistics (v2.32 / ADR-118).
    pub async fn screen_reset_focus_mode(
        &self,
    ) -> Result<tetherly_core::ScreenFocusReport, CoreError> {
        {
            let mut cfg = self.shared.screen_focus_config.write().expect("lock");
            *cfg = tetherly_core::ScreenFocusConfig::default();
            self.shared
                .screen_focus_tiles_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_focus_peripheral_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_focus_peripheral_skipped
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_focus_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenFocusMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_focus_report())
    }

    /// Report screen streaming tile-level adaptive palette compression status (v2.33 / ADR-119).
    pub fn screen_tile_palette_report(&self) -> tetherly_core::ScreenTilePaletteReport {
        let cfg = *self.shared.screen_tile_palette_config.read().expect("lock");
        tetherly_core::ScreenTilePaletteReport {
            ok: true,
            enabled: cfg.enabled,
            max_colors: cfg.max_colors,
            enable_rle: cfg.enable_rle,
            solid_tiles_count: self
                .shared
                .screen_tile_palette_solid_count
                .load(std::sync::atomic::Ordering::Relaxed),
            bilevel_tiles_count: self
                .shared
                .screen_tile_palette_bilevel_count
                .load(std::sync::atomic::Ordering::Relaxed),
            palette_tiles_count: self
                .shared
                .screen_tile_palette_palette_count
                .load(std::sync::atomic::Ordering::Relaxed),
            truecolor_tiles_count: self
                .shared
                .screen_tile_palette_truecolor_count
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_tile_palette_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming tile-level adaptive palette mode and parameters (v2.33 / ADR-119).
    pub async fn screen_set_tile_palette_mode(
        &self,
        enabled: bool,
        max_colors: Option<u8>,
        enable_rle: Option<bool>,
    ) -> Result<tetherly_core::ScreenTilePaletteReport, CoreError> {
        let (max_c, rle) = {
            let mut cfg = self
                .shared
                .screen_tile_palette_config
                .write()
                .expect("lock");
            cfg.enabled = enabled;
            if let Some(mc) = max_colors {
                cfg.max_colors = mc.clamp(2, 16);
            }
            if let Some(r) = enable_rle {
                cfg.enable_rle = r;
            }
            (cfg.max_colors, cfg.enable_rle)
        };
        let msg = ControlMsg::SetScreenTilePaletteMode {
            enabled,
            max_colors: max_c,
            enable_rle: rle,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_palette_report())
    }

    /// Reset screen streaming tile-level adaptive palette mode to default disabled and clear statistics (v2.33 / ADR-119).
    pub async fn screen_reset_tile_palette_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTilePaletteReport, CoreError> {
        {
            let mut cfg = self
                .shared
                .screen_tile_palette_config
                .write()
                .expect("lock");
            *cfg = tetherly_core::ScreenTilePaletteConfig::default();
            self.shared
                .screen_tile_palette_solid_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_palette_bilevel_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_palette_palette_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_palette_truecolor_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_palette_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTilePaletteMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_palette_report())
    }

    /// Report current screen streaming tile predictive filter configuration and statistics (v2.34 / ADR-120).
    pub fn screen_tile_predict_report(&self) -> tetherly_core::ScreenTilePredictReport {
        let cfg = *self.shared.screen_tile_predict_config.read().expect("lock");
        tetherly_core::ScreenTilePredictReport {
            ok: true,
            enabled: cfg.enabled,
            filter_mode: cfg.filter_mode,
            enable_rct: cfg.enable_rct,
            predict_tiles_count: self
                .shared
                .screen_tile_predict_tiles_count
                .load(std::sync::atomic::Ordering::Relaxed),
            saved_bandwidth_bytes: self
                .shared
                .screen_tile_predict_saved_bytes
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Configure screen streaming tile-level predictive filter & reversible color transform mode (v2.34 / ADR-120).
    pub async fn screen_set_tile_predict_mode(
        &self,
        enabled: bool,
        filter_mode: Option<tetherly_core::ScreenTilePredictFilter>,
        enable_rct: Option<bool>,
    ) -> Result<tetherly_core::ScreenTilePredictReport, CoreError> {
        let (filter, rct) = {
            let mut cfg = self
                .shared
                .screen_tile_predict_config
                .write()
                .expect("lock");
            cfg.enabled = enabled;
            if let Some(fm) = filter_mode {
                cfg.filter_mode = fm;
            }
            if let Some(r) = enable_rct {
                cfg.enable_rct = r;
            }
            (cfg.filter_mode, cfg.enable_rct)
        };
        let msg = ControlMsg::SetScreenTilePredictMode {
            enabled,
            filter_mode: filter.to_u8(),
            enable_rct: rct,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_predict_report())
    }

    /// Reset screen streaming tile-level predictive filter mode to default disabled and clear statistics (v2.34 / ADR-120).
    pub async fn screen_reset_tile_predict_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTilePredictReport, CoreError> {
        {
            let mut cfg = self
                .shared
                .screen_tile_predict_config
                .write()
                .expect("lock");
            *cfg = tetherly_core::ScreenTilePredictConfig::default();
            self.shared
                .screen_tile_predict_tiles_count
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_predict_saved_bytes
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTilePredictMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_predict_report())
    }

    /// Report current screen streaming tile presentation transaction and double-buffer state (v2.35 / ADR-121).
    pub fn screen_tile_present_report(&self) -> tetherly_core::ScreenTilePresentReport {
        let cfg = *self.shared.screen_tile_present_config.read().expect("lock");
        let committed = self
            .shared
            .screen_tile_present_committed_txs
            .load(std::sync::atomic::Ordering::Relaxed);
        let swapped = self
            .shared
            .screen_tile_present_swapped_frames
            .load(std::sync::atomic::Ordering::Relaxed);
        let torn = self
            .shared
            .screen_tile_present_torn_prevented
            .load(std::sync::atomic::Ordering::Relaxed);
        let total_lat = self
            .shared
            .screen_tile_present_total_latency_us
            .load(std::sync::atomic::Ordering::Relaxed);
        let avg_lat = total_lat.checked_div(swapped).unwrap_or(0);
        tetherly_core::ScreenTilePresentReport {
            ok: true,
            enabled: cfg.enabled,
            vsync_mode: cfg.vsync_mode.as_str().to_string(),
            double_buffer: cfg.double_buffer,
            tear_prevention: cfg.tear_prevention,
            committed_transactions: committed,
            swapped_frames: swapped,
            torn_frames_prevented: torn,
            avg_swap_latency_us: avg_lat,
        }
    }

    /// Configure screen streaming tile presentation transaction & double-buffer mode (v2.35 / ADR-121).
    pub async fn screen_set_tile_present_mode(
        &self,
        enabled: bool,
        vsync_mode: Option<tetherly_core::ScreenTilePresentVsyncMode>,
        double_buffer: Option<bool>,
        tear_prevention: Option<bool>,
    ) -> Result<tetherly_core::ScreenTilePresentReport, CoreError> {
        let (v_mode, db, tp) = {
            let mut cfg = self
                .shared
                .screen_tile_present_config
                .write()
                .expect("lock");
            cfg.enabled = enabled;
            if let Some(vm) = vsync_mode {
                cfg.vsync_mode = vm;
            }
            if let Some(d) = double_buffer {
                cfg.double_buffer = d;
            }
            if let Some(t) = tear_prevention {
                cfg.tear_prevention = t;
            }
            (cfg.vsync_mode, cfg.double_buffer, cfg.tear_prevention)
        };
        let msg = ControlMsg::SetScreenTilePresentMode {
            enabled,
            vsync_mode: v_mode.to_u8(),
            double_buffer: db,
            tear_prevention: tp,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_present_report())
    }

    /// Reset screen streaming tile presentation mode to default disabled and clear statistics (v2.35 / ADR-121).
    pub async fn screen_reset_tile_present_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTilePresentReport, CoreError> {
        {
            let mut cfg = self
                .shared
                .screen_tile_present_config
                .write()
                .expect("lock");
            *cfg = tetherly_core::ScreenTilePresentConfig::default();
            self.shared
                .screen_tile_present_committed_txs
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_present_swapped_frames
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_present_torn_prevented
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_present_total_latency_us
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTilePresentMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_present_report())
    }

    /// Report current screen streaming tile NACK and fast retransmission state (v2.36 / ADR-122).
    pub fn screen_tile_nack_report(&self) -> tetherly_core::ScreenTileNackReport {
        let cfg = *self.shared.screen_tile_nack_config.read().expect("lock");
        let sent = self
            .shared
            .screen_tile_nack_requests_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let retried = self
            .shared
            .screen_tile_nack_tiles_retransmitted
            .load(std::sync::atomic::Ordering::Relaxed);
        let degraded = self
            .shared
            .screen_tile_nack_degraded_commits
            .load(std::sync::atomic::Ordering::Relaxed);
        tetherly_core::ScreenTileNackReport {
            ok: true,
            enabled: cfg.enabled,
            timeout_ms: cfg.timeout_ms,
            max_retries: cfg.max_retries,
            nack_requests_sent: sent,
            tiles_retransmitted: retried,
            degraded_commits: degraded,
        }
    }

    /// Configure screen streaming tile NACK & fast retransmission mode (v2.36 / ADR-122).
    pub async fn screen_set_tile_nack_mode(
        &self,
        enabled: bool,
        timeout_ms: Option<u16>,
        max_retries: Option<u8>,
    ) -> Result<tetherly_core::ScreenTileNackReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_nack_config.write().expect("lock");
            cfg.enabled = enabled;
            if let Some(t) = timeout_ms {
                cfg.timeout_ms = t;
            }
            if let Some(r) = max_retries {
                cfg.max_retries = r;
            }
        }
        let msg = ControlMsg::TileNackSync {
            nack_requests: self
                .shared
                .screen_tile_nack_requests_sent
                .load(std::sync::atomic::Ordering::Relaxed),
            retransmitted_tiles: self
                .shared
                .screen_tile_nack_tiles_retransmitted
                .load(std::sync::atomic::Ordering::Relaxed),
            degraded_commits: self
                .shared
                .screen_tile_nack_degraded_commits
                .load(std::sync::atomic::Ordering::Relaxed),
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_nack_report())
    }

    /// Reset screen streaming tile NACK mode to default disabled and clear statistics (v2.36 / ADR-122).
    pub async fn screen_reset_tile_nack_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileNackReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_nack_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileNackConfig::default();
            self.shared
                .screen_tile_resend_buffer
                .write()
                .unwrap()
                .clear();
            self.shared
                .screen_tile_nack_requests_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_nack_tiles_retransmitted
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_nack_degraded_commits
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileNackMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_nack_report())
    }

    /// Report current screen streaming tile saliency and tiered scheduling state (v2.37 / ADR-123).
    pub fn screen_tile_saliency_report(&self) -> tetherly_core::ScreenTileSaliencyReport {
        let cfg = *self
            .shared
            .screen_tile_saliency_config
            .read()
            .expect("lock");
        let crit = self
            .shared
            .screen_tile_saliency_critical_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let norm = self
            .shared
            .screen_tile_saliency_normal_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let bg = self
            .shared
            .screen_tile_saliency_background_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let skipped = self
            .shared
            .screen_tile_saliency_background_skipped
            .load(std::sync::atomic::Ordering::Relaxed);
        tetherly_core::ScreenTileSaliencyReport {
            ok: true,
            enabled: cfg.enabled,
            high_threshold: cfg.high_threshold,
            low_threshold: cfg.low_threshold,
            drop_background_on_congestion: cfg.drop_background_on_congestion,
            critical_tiles_sent: crit,
            normal_tiles_sent: norm,
            background_tiles_sent: bg,
            background_tiles_skipped: skipped,
        }
    }

    /// Configure screen streaming tile saliency & tiered scheduling mode (v2.37 / ADR-123).
    pub async fn screen_set_tile_saliency_mode(
        &self,
        enabled: bool,
        high_threshold: Option<u8>,
        low_threshold: Option<u8>,
        drop_background_on_congestion: Option<bool>,
    ) -> Result<tetherly_core::ScreenTileSaliencyReport, CoreError> {
        let (high, low, drop_bg) = {
            let mut cfg = self
                .shared
                .screen_tile_saliency_config
                .write()
                .expect("lock");
            cfg.enabled = enabled;
            if let Some(h) = high_threshold {
                cfg.high_threshold = h;
            }
            if let Some(l) = low_threshold {
                cfg.low_threshold = l;
            }
            if let Some(d) = drop_background_on_congestion {
                cfg.drop_background_on_congestion = d;
            }
            if cfg.high_threshold < cfg.low_threshold {
                cfg.high_threshold = cfg.low_threshold;
            }
            (
                cfg.high_threshold,
                cfg.low_threshold,
                cfg.drop_background_on_congestion,
            )
        };
        let msg = ControlMsg::SetScreenTileSaliencyMode {
            enabled,
            high_threshold: high,
            low_threshold: low,
            drop_background_on_congestion: drop_bg,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_saliency_report())
    }

    /// Reset screen streaming tile saliency mode to default disabled and clear statistics (v2.37 / ADR-123).
    pub async fn screen_reset_tile_saliency_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileSaliencyReport, CoreError> {
        {
            let mut cfg = self
                .shared
                .screen_tile_saliency_config
                .write()
                .expect("lock");
            *cfg = tetherly_core::ScreenTileSaliencyConfig::default();
            self.shared
                .screen_tile_saliency_critical_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_saliency_normal_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_saliency_background_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_saliency_background_skipped
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileSaliencyMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_saliency_report())
    }

    /// Report current screen streaming tile Level-of-Detail (LoD) downsampling state (v2.38 / ADR-124).
    pub fn screen_tile_lod_report(&self) -> tetherly_core::ScreenTileLodReport {
        let cfg = *self.shared.screen_tile_lod_config.read().expect("lock");
        let lod0 = self
            .shared
            .screen_tile_lod0_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let lod1 = self
            .shared
            .screen_tile_lod1_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let lod2 = self
            .shared
            .screen_tile_lod2_tiles
            .load(std::sync::atomic::Ordering::Relaxed);
        let bytes_saved = self
            .shared
            .screen_tile_lod_bytes_saved
            .load(std::sync::atomic::Ordering::Relaxed);
        tetherly_core::ScreenTileLodReport {
            ok: true,
            enabled: cfg.enabled,
            max_lod: cfg.max_lod,
            auto_lod_on_congestion: cfg.auto_lod_on_congestion,
            lod0_tiles_sent: lod0,
            lod1_tiles_sent: lod1,
            lod2_tiles_sent: lod2,
            bytes_saved,
        }
    }

    /// Configure screen streaming tile Level-of-Detail (LoD) downsampling mode (v2.38 / ADR-124).
    pub async fn screen_set_tile_lod_mode(
        &self,
        enabled: bool,
        max_lod: Option<u8>,
        auto_lod_on_congestion: Option<bool>,
    ) -> Result<tetherly_core::ScreenTileLodReport, CoreError> {
        let (max_lod_val, auto_lod_val) = {
            let mut cfg = self.shared.screen_tile_lod_config.write().expect("lock");
            cfg.enabled = enabled;
            if let Some(m) = max_lod {
                cfg.max_lod = m.clamp(1, 2);
            }
            if let Some(a) = auto_lod_on_congestion {
                cfg.auto_lod_on_congestion = a;
            }
            (cfg.max_lod, cfg.auto_lod_on_congestion)
        };
        let msg = ControlMsg::SetScreenTileLodMode {
            enabled,
            max_lod: max_lod_val,
            auto_lod_on_congestion: auto_lod_val,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_lod_report())
    }

    /// Reset screen streaming tile Level-of-Detail (LoD) mode to default and clear statistics (v2.38 / ADR-124).
    pub async fn screen_reset_tile_lod_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileLodReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_lod_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileLodConfig::default();
            self.shared
                .screen_tile_lod0_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_lod1_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_lod2_tiles
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_lod_bytes_saved
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileLodMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_lod_report())
    }

    /// Dynamic tile batching and pacing status report (v2.39 / ADR-125).
    pub fn screen_tile_batch_report(&self) -> tetherly_core::ScreenTileBatchReport {
        let cfg = *self.shared.screen_tile_batch_config.read().expect("lock");
        let batches = self
            .shared
            .screen_tile_batches_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let chunks = self
            .shared
            .screen_tile_chunks_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let bytes = self
            .shared
            .screen_tile_batch_bytes_total
            .load(std::sync::atomic::Ordering::Relaxed);
        let pacing_pauses = self
            .shared
            .screen_tile_batch_pacing_pauses
            .load(std::sync::atomic::Ordering::Relaxed);
        let avg_chunk_bytes = bytes.checked_div(chunks).unwrap_or(0) as u32;
        tetherly_core::ScreenTileBatchReport {
            ok: true,
            enabled: cfg.enabled,
            target_chunk_bytes: cfg.target_chunk_bytes,
            pacing_interval_us: cfg.pacing_interval_us,
            batches_sent: batches,
            chunks_sent: chunks,
            avg_chunk_bytes,
            pacing_pauses,
        }
    }

    /// Configure screen streaming tile batching, MTU chunking and pacing (v2.39 / ADR-125).
    pub async fn screen_set_tile_batch_mode(
        &self,
        enabled: bool,
        target_chunk_bytes: Option<u16>,
        pacing_interval_us: Option<u16>,
    ) -> Result<tetherly_core::ScreenTileBatchReport, CoreError> {
        let (target_val, pacing_val) = {
            let mut cfg = self.shared.screen_tile_batch_config.write().expect("lock");
            cfg.enabled = enabled;
            if let Some(tb) = target_chunk_bytes {
                cfg.target_chunk_bytes = tb.clamp(
                    tetherly_core::SCREEN_TILE_BATCH_MIN_CHUNK_BYTES,
                    tetherly_core::SCREEN_TILE_BATCH_MAX_CHUNK_BYTES,
                );
            }
            if let Some(pu) = pacing_interval_us {
                cfg.pacing_interval_us =
                    pu.clamp(0, tetherly_core::SCREEN_TILE_BATCH_MAX_PACING_US);
            }
            (cfg.target_chunk_bytes, cfg.pacing_interval_us)
        };
        let msg = ControlMsg::SetScreenTileBatchMode {
            enabled,
            target_chunk_bytes: target_val,
            pacing_interval_us: pacing_val,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_batch_report())
    }

    /// Reset screen streaming tile batching mode to default and clear statistics (v2.39 / ADR-125).
    pub async fn screen_reset_tile_batch_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileBatchReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_batch_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileBatchConfig::default();
            self.shared
                .screen_tile_batches_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_chunks_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_batch_bytes_total
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_batch_pacing_pauses
                .store(0, std::sync::atomic::Ordering::Relaxed);
            *self
                .shared
                .screen_tile_batch_receiver_state
                .lock()
                .expect("lock") = None;
        }
        let msg = ControlMsg::ResetScreenTileBatchMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_batch_report())
    }

    /// Dynamic tile motion estimation and displacement compensation status report (v2.40 / ADR-126).
    pub fn screen_tile_motion_report(&self) -> tetherly_core::ScreenTileMotionReport {
        let cfg = *self.shared.screen_tile_motion_config.read().expect("lock");
        let motion = self
            .shared
            .screen_tile_motion_tiles_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let fallback = self
            .shared
            .screen_tile_motion_fallback_tiles_sent
            .load(std::sync::atomic::Ordering::Relaxed);
        let bytes = self
            .shared
            .screen_tile_motion_bytes_saved
            .load(std::sync::atomic::Ordering::Relaxed);
        tetherly_core::ScreenTileMotionReport {
            ok: true,
            enabled: cfg.enabled,
            max_displacement: cfg.max_displacement,
            min_match_ratio: cfg.min_match_ratio,
            motion_tiles_sent: motion,
            fallback_tiles_sent: fallback,
            bytes_saved: bytes,
        }
    }

    /// Configure screen streaming tile motion estimation and displacement compensation (v2.40 / ADR-126).
    pub async fn screen_set_tile_motion_mode(
        &self,
        enabled: bool,
        max_displacement: Option<u16>,
        min_match_ratio: Option<u8>,
    ) -> Result<tetherly_core::ScreenTileMotionReport, CoreError> {
        let (disp_val, ratio_val) = {
            let mut cfg = self.shared.screen_tile_motion_config.write().expect("lock");
            cfg.enabled = enabled;
            if let Some(md) = max_displacement {
                cfg.max_displacement = md.clamp(
                    tetherly_core::SCREEN_TILE_MOTION_MIN_DISPLACEMENT,
                    tetherly_core::SCREEN_TILE_MOTION_MAX_DISPLACEMENT,
                );
            }
            if let Some(mr) = min_match_ratio {
                cfg.min_match_ratio = mr.clamp(1, 100);
            }
            (cfg.max_displacement, cfg.min_match_ratio)
        };
        let msg = ControlMsg::SetScreenTileMotionMode {
            enabled,
            max_displacement: disp_val,
            min_match_ratio: ratio_val,
        };
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_motion_report())
    }

    /// Reset screen streaming tile motion mode to default and clear statistics (v2.40 / ADR-126).
    pub async fn screen_reset_tile_motion_mode(
        &self,
    ) -> Result<tetherly_core::ScreenTileMotionReport, CoreError> {
        {
            let mut cfg = self.shared.screen_tile_motion_config.write().expect("lock");
            *cfg = tetherly_core::ScreenTileMotionConfig::default();
            self.shared
                .screen_tile_motion_tiles_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_motion_fallback_tiles_sent
                .store(0, std::sync::atomic::Ordering::Relaxed);
            self.shared
                .screen_tile_motion_bytes_saved
                .store(0, std::sync::atomic::Ordering::Relaxed);
        }
        let msg = ControlMsg::ResetScreenTileMotionMode;
        let _ = self.send_screen_control(msg).await;
        Ok(self.screen_tile_motion_report())
    }
}
