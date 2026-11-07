// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen capture streaming: session setup, frame send, stats.

use super::*;
use crate::events::UiEvent;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tetherly_core::frame::InnerFrame;
use tetherly_core::{
    ControlMsg, CoreError, DeviceId, MemoryScreenSink, ScreenAssembler, ScreenAudioSyncFrame,
    ScreenFecShard, ScreenFrame, ScreenNack, ScreenPlayoutAction, ScreenProbePacket,
    ScreenProfileFrame, ScreenReceiver, ScreenRecordingMeta, ScreenRecordingWriter, ScreenState,
    ScreenStats, ScreenTunerMode, ScreenTwccFeedback, ScreenTwccStats, TYPE_SCREEN_CONTROL,
    TYPE_SCREEN_FEC, TYPE_SCREEN_NACK, TYPE_SCREEN_PROBE, TYPE_SCREEN_PROFILE, TYPE_SCREEN_SYNC,
    TYPE_SCREEN_TWCC,
};
use tetherly_net::session::{accept_session, dial_session, ActiveSession};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::warn;

impl Node {
    pub fn screen_port(&self) -> u16 {
        self.shared.bound_screen.load(Ordering::SeqCst)
    }

    pub fn screen_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.screen_port()))
    }

    /// Open the resume-only screen channel 45720 as the **visitor**: this node
    /// dials a trusted peer to view its screen. Pairing on 45720 is refused.
    /// Frames only start flowing after the host calls `allow()` locally and the
    /// host responds to our `Start`. Real capture is Manual-required.
    pub async fn open_screen(&self, peer: &DeviceId) -> Result<(), tetherly_net::NetError> {
        let addr = {
            let live = self.shared.live.lock().expect("live");
            let p = live
                .get(peer)
                .ok_or(tetherly_net::NetError::ScreenRequiresTrust)?;
            let port = p.screen_port.ok_or(tetherly_net::NetError::Handshake)?;
            SocketAddr::new(p.addr.ip(), port)
        };
        let stream = TcpStream::connect(addr).await?;
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        cfg.screen_only = true;
        let sess = match dial_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        self.attach_screen_client(sess).await;
        Ok(())
    }

    /// Accept a 45720 screen connection. The peer wants to view this machine's
    /// screen. We refuse to stream until the local user clicks `allow()`.
    pub(super) async fn accept_screen(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        cfg.screen_only = true;
        let sess = match accept_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        self.attach_screen_server(sess).await;
        Ok(())
    }

    /// Host side of a 45720 session: this machine is being viewed. Consent is
    /// enforced here — the peer's `Start` is refused until `screen_allow()` has
    /// run locally. Frames are pushed by the app calling `screen_send_frame()`
    /// at its own cadence (CLI loop on real hardware).
    pub(super) async fn attach_screen_server(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let (tx, mut rx) = mpsc::channel::<InnerFrame>(SCREEN_QUEUE_DEPTH);
        // Generation tag: when this loop exits it must only run the
        // engine-level `on_peer_gone` if it is STILL the active channel — a
        // replacement attach (new dial) must not be reset by this loop's
        // teardown racing behind it. The fetch_add happens INSIDE the engine
        // lock so an old loop's exit check (same lock) can never interleave
        // between "gen still looks current" and this attach's
        // `on_peer_connected` — the m6_7 CI race: a middle generation's
        // teardown ran its check before the next attach bumped the gen, then
        // its `on_peer_gone` landed after the new attach, wiping the fresh
        // consent and wedging the engine back at `Requested`/`Idle`.
        let my_gen = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let gen = self
                .shared
                .screen_channel_gen
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let displays = self.screen_displays();
            if !displays.is_empty() {
                engine.set_displays(displays);
            }
            engine.on_peer_connected();
            eprintln!("[m6dbg] server attach: gen={} peer={}", gen, peer_id);
            gen
        };
        *self.shared.screen_out.lock().await = Some(tx);
        *self.shared.screen_peer.lock().expect("screen_peer") = Some(peer_id.clone());
        // A new screen session re-evaluates every ADR-083/084/085 bandwidth
        // tier from scratch — not just bitrate. The fps/scale/profile flags
        // gate first-frame tier selection too; leaving them set would pin a
        // second session to the previous session's adaptive tier.
        self.shared
            .screen_bitrate_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.shared
            .screen_fps_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.shared
            .screen_scale_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.shared
            .screen_profile_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let state = self.screen_state();
        let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
        self.emit(UiEvent::screen_state_changed(
            state.as_str(),
            allowed,
            Some(peer_id.to_string()),
        ));
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        // Pipe outbound frames to the session and inbound controls into the
        // engine. The session's send/recv is the wire; the pump task below
        // produces frames on the timer.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    outbound = rx.recv() => {
                        match outbound {
                            Some(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                if sess.send_bounded(frame).await.is_err() {
                                    break;
                                }
                                node.shared.telemetry.record_screen_tx(f_len);
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                node.shared.telemetry.record_screen_rx(f_len);
                                if frame.ty == TYPE_SCREEN_NACK {
                                    if let Ok(nack) = ScreenNack::decode(&frame.payload) {
                                        let now_us = node.shared.clock.unix_us();
                                        let retransmit_frames = {
                                            let mut engine = node
                                                .shared
                                                .screen_engine
                                                .lock()
                                                .expect("screen");
                                            engine.handle_nack(&nack, now_us)
                                        };
                                        if nack.requires_keyframe_fallback {
                                            node.shared.screen_source.force_idr();
                                            if let Some(sel) = &node.shared.screen_encode_codec {
                                                sel.force_idr();
                                            }
                                        }
                                        for r_frame in retransmit_frames {
                                            let r_len = (r_frame.payload.len() + 12) as u64;
                                            if sess.send_bounded(r_frame).await.is_ok() {
                                                node.shared.telemetry.record_screen_tx(r_len);
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if frame.ty == tetherly_core::TYPE_SCREEN_ANNOTATION {
                                    if let Ok(msg) = tetherly_core::ScreenAnnotationMsg::from_frame(&frame) {
                                        match msg {
                                            tetherly_core::ScreenAnnotationMsg::Add(item) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let _ = wb.add_item(item);
                                                node.shared.screen_annotation_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Remove { id } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                wb.active_items_mut().retain(|it| it.id != id);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Undo { author } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let a_opt = if author.is_empty() { None } else { Some(author.as_str()) };
                                                let undone = wb.undo(a_opt);
                                                if undone.is_some() {
                                                    node.shared.screen_whiteboard_undo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Redo { author } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let a_opt = if author.is_empty() { None } else { Some(author.as_str()) };
                                                let redone = wb.redo(a_opt);
                                                if redone.is_some() {
                                                    node.shared.screen_whiteboard_redo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Clear => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                if wb.policy().allow_peer_clear {
                                                    wb.clear();
                                                    node.shared.screen_annotations.write().unwrap().clear();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Snapshot(snapshot) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let _ = wb.import_snapshot(snapshot, true);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Policy(policy) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                wb.set_policy(policy);
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if frame.ty == TYPE_SCREEN_PROFILE {
                                    if let Ok(profile) = ScreenProfileFrame::decode(&frame.payload) {
                                        let mut engine = node
                                            .shared
                                            .screen_engine
                                            .lock()
                                            .expect("screen");
                                        engine.on_profile_frame(&profile);
                                        *node.shared.screen_latency_stats.lock().expect("latency stats") =
                                            engine.latency_report();
                                        *node.shared.screen_tuning_stats.lock().expect("tuning stats") =
                                            engine.tuning_report();
                                    }
                                    continue;
                                }
                                if frame.ty == TYPE_SCREEN_TWCC {
                                    if let Ok(feedback) = ScreenTwccFeedback::decode(&frame.payload) {
                                        let now_us = node.shared.clock.unix_us();
                                        let report = {
                                            let mut engine = node
                                                .shared
                                                .screen_engine
                                                .lock()
                                                .expect("screen");
                                            engine.handle_twcc_feedback(&feedback, now_us)
                                        };
                                        *node.shared.screen_twcc_stats.lock().expect("twcc stats") = ScreenTwccStats {
                                            mode: report.mode,
                                            feedbacks_sent: report.feedbacks_sent,
                                            feedbacks_received: report.feedbacks_received,
                                            packets_tracked: report.packets_tracked,
                                            packets_acknowledged: report.packets_acknowledged,
                                            packets_lost: report.packets_lost,
                                            loss_ratio: report.loss_ratio,
                                            delivery_rate_bps: report.delivery_rate_bps,
                                            delay_gradient_us: report.delay_gradient_us,
                                            smoothed_rtt_us: report.smoothed_rtt_us,
                                        };
                                        *node.shared.screen_probe_stats.lock().expect("probe stats") = {
                                            let engine = node.shared.screen_engine.lock().expect("screen");
                                            engine.probe_stats()
                                        };
                                        if let Some(sel) = &node.shared.screen_encode_codec {
                                            let dynamic_bitrate = {
                                                let engine = node.shared.screen_engine.lock().expect("screen");
                                                engine.bitrate_bps()
                                            };
                                            if sel.bitrate_bps() != dynamic_bitrate {
                                                sel.set_bitrate_bps(dynamic_bitrate);
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if let Ok(msg) = ControlMsg::from_frame(&frame) {
                                    let now_us = node.shared.clock.unix_us();
                                    if matches!(msg, ControlMsg::KeyframeRequest) {
                                        node.shared.screen_source.force_idr();
                                        if let Some(sel) = &node.shared.screen_encode_codec {
                                            sel.force_idr();
                                        }
                                    }
                                    if let ControlMsg::SwitchDisplay { display_id } = msg {
                                        let displays = node.screen_displays();
                                        if displays.is_empty() || displays.iter().any(|d| d.display_id == display_id) {
                                            let _ = node.shared.screen_source.switch_display(display_id);
                                            node.shared.screen_source.force_idr();
                                            if let Some(sel) = &node.shared.screen_encode_codec {
                                                sel.force_idr();
                                            }
                                            node.shared.screen_active_display_id.store(display_id, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_display_switches_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_last_display_switch_us.store(now_us, std::sync::atomic::Ordering::Relaxed);
                                        }
                                    }
                                    if let ControlMsg::SetPrivacyMode { enabled, lock_input } = msg {
                                        node.shared.screen_privacy_enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
                                        node.shared.screen_privacy_lock_input.store(lock_input, std::sync::atomic::Ordering::Relaxed);
                                        node.shared.screen_privacy_last_change_us.store(now_us, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    if matches!(msg, ControlMsg::Stop) {
                                        node.shared.screen_privacy_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
                                        node.shared.screen_privacy_lock_input.store(false, std::sync::atomic::Ordering::Relaxed);
                                        node.shared.screen_privacy_last_change_us.store(now_us, std::sync::atomic::Ordering::Relaxed);
                                        node.shared.screen_peer_watermark_enforced.store(false, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    if let ControlMsg::SetWatermarkState { enabled, host_enforced } = msg {
                                        let mut cfg = node.shared.screen_watermark.write().unwrap();
                                        cfg.enabled = enabled;
                                        cfg.host_enforced = host_enforced;
                                        node.shared.screen_peer_watermark_enforced.store(host_enforced, std::sync::atomic::Ordering::Relaxed);
                                        tracing::info!(enabled, host_enforced, "host updated screen watermark state from peer");
                                    }
                                    if matches!(msg, ControlMsg::ResetLatencyProfile) {
                                        *node.shared.screen_latency_stats.lock().expect("latency stats") = {
                                            let engine = node.shared.screen_engine.lock().expect("screen");
                                            engine.latency_report()
                                        };
                                    }
                                    if let ControlMsg::SetTuningMode { mode } = msg {
                                        let m = ScreenTunerMode::from_tag(mode);
                                        *node.shared.screen_tuning_mode.lock().expect("tuning mode") = m;
                                        if let Ok(mut engine) = node.shared.screen_engine.lock() {
                                            engine.set_tuning_mode(m);
                                            *node.shared.screen_tuning_stats.lock().expect("tuning stats") =
                                                engine.tuning_report();
                                        }
                                    }
                                    if matches!(msg, ControlMsg::ResetTuningProfile) {
                                        if let Ok(mut engine) = node.shared.screen_engine.lock() {
                                            engine.reset_tuning();
                                            *node.shared.screen_tuning_stats.lock().expect("tuning stats") =
                                                engine.tuning_report();
                                        }
                                    }
                                    if let ControlMsg::SetRecordingState { active } = msg {
                                        node.shared
                                            .screen_peer_recording
                                            .store(active, std::sync::atomic::Ordering::Relaxed);
                                        tracing::info!(active, "peer screen recording state updated");
                                    }
                                    match &msg {
                                        ControlMsg::SetLaserPointer { x, y, active, color_rgb } => {
                                            let state = if *active {
                                                Some(tetherly_core::LaserPointerState {
                                                    x: *x,
                                                    y: *y,
                                                    active: true,
                                                    color_rgb: *color_rgb,
                                                    radius: 12,
                                                })
                                            } else {
                                                None
                                            };
                                            *node.shared.screen_laser_pointer.write().unwrap() = state;
                                            node.shared.screen_laser_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::ClearAnnotations => {
                                            let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                            if wb.policy().allow_peer_clear {
                                                wb.clear();
                                                node.shared.screen_annotations.write().unwrap().clear();
                                            }
                                        }
                                        ControlMsg::WhiteboardUndo { author_hash } => {
                                            let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                            let undone = if *author_hash == 0 {
                                                wb.undo(None)
                                            } else {
                                                let target = wb
                                                    .active_items()
                                                    .iter()
                                                    .rfind(|it| tetherly_core::hash_author(&it.author) == *author_hash)
                                                    .map(|it| it.author.clone());
                                                wb.undo(target.as_deref())
                                            };
                                            if undone.is_some() {
                                                node.shared.screen_whiteboard_undo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                        }
                                        ControlMsg::WhiteboardRedo { author_hash: _ } => {
                                            let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                            let redone = wb.redo(None);
                                            if redone.is_some() {
                                                node.shared.screen_whiteboard_redo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                        }
                                        ControlMsg::SetWhiteboardPolicy {
                                            read_only,
                                            allow_peer_clear,
                                            max_items,
                                        } => {
                                            let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                            wb.set_policy(tetherly_core::WhiteboardPolicy {
                                                read_only: *read_only,
                                                allow_peer_clear: *allow_peer_clear,
                                                max_items: *max_items,
                                            });
                                        }
                                        ControlMsg::SetScreenRoi { x, y, w, h } => {
                                            let mut roi = node.shared.screen_roi.write().unwrap();
                                            roi.active = true;
                                            roi.x = *x;
                                            roi.y = *y;
                                            roi.w = *w;
                                            roi.h = *h;
                                            node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::ResetScreenRoi => {
                                            let mut roi = node.shared.screen_roi.write().unwrap();
                                            roi.active = false;
                                            roi.zoom_percent = 100;
                                            node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenRoiZoom { zoom_percent } => {
                                            let mut roi = node.shared.screen_roi.write().unwrap();
                                            roi.zoom_percent = *zoom_percent;
                                            node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenColorMode { mode, threshold } => {
                                            let mut cfg = node.shared.screen_color_mode.write().unwrap();
                                            cfg.mode = tetherly_core::ScreenColorMode::from_tag(*mode);
                                            cfg.threshold = *threshold;
                                            node.shared.screen_color_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::ResetScreenColorMode => {
                                            let mut cfg = node.shared.screen_color_mode.write().unwrap();
                                            *cfg = tetherly_core::ScreenColorConfig::default();
                                            node.shared.screen_color_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenDeltaMode { enabled, keyframe_interval } => {
                                            let mut cfg = node.shared.screen_delta_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.keyframe_interval = *keyframe_interval;
                                            node.shared.screen_delta_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::ResetScreenDeltaMode => {
                                            let mut cfg = node.shared.screen_delta_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenDeltaConfig::default();
                                            node.shared.screen_delta_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenQualityMetricMode { enabled, sample_interval } => {
                                            let mut cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.sample_interval = *sample_interval;
                                            node.shared.screen_metric_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::ResetScreenQualityMetricMode => {
                                            let mut cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenQualityMetricConfig::default();
                                            node.shared.screen_metric_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                            let mut state = node.shared.screen_quality_metric_state.write().unwrap();
                                            *state = ScreenQualityMetricState::default();
                                            *node.shared.screen_metric_ref_frame.write().unwrap() = None;
                                        }
                                        ControlMsg::SetScreenQoeMode { mode, target_psnr_db, target_ssim_pct } => {
                                            let qoe_mode = tetherly_core::ScreenQoeMode::from_tag(*mode);
                                            let mut qoe = node.shared.screen_qoe_controller.write().unwrap();
                                            let cfg = tetherly_core::ScreenQoeConfig {
                                                mode: qoe_mode,
                                                target_psnr_db: *target_psnr_db,
                                                target_ssim_pct: *target_ssim_pct,
                                            };
                                            qoe.set_config(cfg);
                                            if qoe_mode.is_active() {
                                                let mut metric_cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                                metric_cfg.enabled = true;
                                            }
                                        }
                                        ControlMsg::ResetScreenQoeMode => {
                                            let mut qoe = node.shared.screen_qoe_controller.write().unwrap();
                                            qoe.reset();
                                        }
                                        ControlMsg::SetScreenPirMode { mode, bands } => {
                                            let pir_mode = tetherly_core::ScreenPirMode::from_u8(*mode);
                                            let mut pir_cfg = node.shared.screen_pir_config.write().unwrap();
                                            pir_cfg.mode = pir_mode;
                                            pir_cfg.bands = *bands;
                                            if pir_mode == tetherly_core::ScreenPirMode::OnDemand {
                                                node.shared.screen_pir_active_sweep.store(true, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_pir_current_band.store(0, std::sync::atomic::Ordering::Relaxed);
                                            } else if pir_mode == tetherly_core::ScreenPirMode::Continuous {
                                                node.shared.screen_pir_active_sweep.store(true, std::sync::atomic::Ordering::Relaxed);
                                            } else {
                                                node.shared.screen_pir_active_sweep.store(false, std::sync::atomic::Ordering::Relaxed);
                                            }
                                        }
                                        ControlMsg::ResetScreenPirMode => {
                                            let mut pir_cfg = node.shared.screen_pir_config.write().unwrap();
                                            *pir_cfg = tetherly_core::ScreenPirConfig::default();
                                            node.shared.screen_pir_active_sweep.store(false, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_pir_current_band.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenLtrMode { enabled, auto_interval } => {
                                            let mut cfg = node.shared.screen_ltr_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.auto_interval = *auto_interval;
                                            if !*enabled {
                                                *node.shared.screen_sender_ltr_canvas.write().unwrap() = None;
                                                *node.shared.screen_receiver_ltr_canvas.write().unwrap() = None;
                                                node.shared.screen_ltr_current_seq.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_ltr_recovery_pending.store(false, std::sync::atomic::Ordering::Relaxed);
                                            }
                                        }
                                        ControlMsg::ResetScreenLtrMode => {
                                            let mut cfg = node.shared.screen_ltr_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenLtrConfig::default();
                                            *node.shared.screen_sender_ltr_canvas.write().unwrap() = None;
                                            *node.shared.screen_receiver_ltr_canvas.write().unwrap() = None;
                                            node.shared.screen_ltr_current_seq.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_ltr_recovery_pending.store(false, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::LtrRecoveryRequest { ltr_seq: _ } => {
                                            node.shared.screen_ltr_recoveries_requested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_ltr_recovery_pending.store(true, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::LtrAnchorSync { ltr_seq } => {
                                            let cur_canvas = node.shared.screen_receiver_canvas.read().unwrap().clone();
                                            if let Some(canvas) = cur_canvas {
                                                *node.shared.screen_receiver_ltr_canvas.write().unwrap() = Some(canvas);
                                            }
                                            node.shared.screen_ltr_current_seq.store(*ltr_seq, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_ltr_anchors_established.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenSvcMode { mode, drop_t1 } => {
                                            let mut cfg = node.shared.screen_svc_config.write().unwrap();
                                            cfg.mode = tetherly_core::ScreenSvcMode::from_u8(*mode);
                                            cfg.drop_t1 = *drop_t1;
                                            if !cfg.is_enabled() {
                                                *node.shared.screen_sender_t0_canvas.write().unwrap() = None;
                                                *node.shared.screen_receiver_t0_canvas.write().unwrap() = None;
                                            }
                                        }
                                        ControlMsg::ResetScreenSvcMode => {
                                            let mut cfg = node.shared.screen_svc_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenSvcConfig::default();
                                            *node.shared.screen_sender_t0_canvas.write().unwrap() = None;
                                            *node.shared.screen_receiver_t0_canvas.write().unwrap() = None;
                                            node.shared.screen_svc_t0_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_svc_t1_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_svc_t1_frames_dropped.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_svc_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SvcT1DropSync { dropped_count } => {
                                            node.shared.screen_svc_t1_frames_dropped.store(*dropped_count, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenMotionMode { enabled, max_displacement } => {
                                            let mut cfg = node.shared.screen_motion_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.max_displacement = (*max_displacement).clamp(
                                                tetherly_core::SCREEN_MOTION_MIN_DISPLACEMENT,
                                                tetherly_core::SCREEN_MOTION_MAX_DISPLACEMENT,
                                            );
                                            if !*enabled {
                                                *node.shared.screen_sender_motion_canvas.write().unwrap() = None;
                                            }
                                        }
                                        ControlMsg::ResetScreenMotionMode => {
                                            let mut cfg = node.shared.screen_motion_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenMotionConfig::default();
                                            *node.shared.screen_sender_motion_canvas.write().unwrap() = None;
                                            node.shared.screen_motion_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_motion_fallback_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_motion_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::MotionScrollSync { scroll_frames_sent } => {
                                            node.shared.screen_motion_frames_sent.store(*scroll_frames_sent, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileMode { enabled, tile_size } => {
                                            let mut cfg = node.shared.screen_tile_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.tile_size = (*tile_size).clamp(
                                                tetherly_core::SCREEN_TILE_MIN_SIZE,
                                                tetherly_core::SCREEN_TILE_MAX_SIZE,
                                            );
                                            if !*enabled {
                                                *node.shared.screen_sender_tile_canvas.write().unwrap() = None;
                                            }
                                        }
                                        ControlMsg::ResetScreenTileMode => {
                                            let mut cfg = node.shared.screen_tile_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileConfig::default();
                                            *node.shared.screen_sender_tile_canvas.write().unwrap() = None;
                                            node.shared.screen_tile_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_dirty_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_skipped_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileGridSync { tiles_sent } => {
                                            node.shared.screen_tile_dirty_tiles_sent.store(*tiles_sent, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileCacheMode { enabled, max_entries } => {
                                            let mut cfg = node.shared.screen_tile_cache_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.max_entries = (*max_entries).clamp(
                                                tetherly_core::SCREEN_TILE_CACHE_MIN_ENTRIES,
                                                tetherly_core::SCREEN_TILE_CACHE_MAX_ENTRIES,
                                            );
                                            node.shared.screen_tile_cache_dict.lock().unwrap().resize(cfg.max_entries);
                                            node.shared.screen_tile_cache_receiver.lock().unwrap().resize(cfg.max_entries as usize, None);
                                        }
                                        ControlMsg::ResetScreenTileCacheMode => {
                                            let mut cfg = node.shared.screen_tile_cache_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileCacheConfig::default();
                                            node.shared.screen_tile_cache_dict.lock().unwrap().clear();
                                            let mut rx = node.shared.screen_tile_cache_receiver.lock().unwrap();
                                            rx.clear();
                                            rx.resize(tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES as usize, None);
                                            node.shared.screen_tile_cache_hits.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_cache_misses.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_cache_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileCacheSync { hits } => {
                                            node.shared.screen_tile_cache_hits.store(*hits, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenFocusMode { enabled, mode, radius, peripheral_skip_ratio } => {
                                            let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.mode = tetherly_core::ScreenFocusMode::from_u8(*mode);
                                            cfg.radius = (*radius).clamp(
                                                tetherly_core::SCREEN_FOCUS_MIN_RADIUS,
                                                tetherly_core::SCREEN_FOCUS_MAX_RADIUS,
                                            );
                                            cfg.peripheral_skip_ratio = (*peripheral_skip_ratio).clamp(
                                                0,
                                                tetherly_core::SCREEN_FOCUS_MAX_SKIP_RATIO,
                                            );
                                        }
                                        ControlMsg::SetScreenFocusBox { box_x, box_y, box_w, box_h } => {
                                            let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                            cfg.box_x = *box_x;
                                            cfg.box_y = *box_y;
                                            cfg.box_w = *box_w;
                                            cfg.box_h = *box_h;
                                        }
                                        ControlMsg::ResetScreenFocusMode => {
                                            let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenFocusConfig::default();
                                            node.shared.screen_focus_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_focus_peripheral_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_focus_peripheral_skipped.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_focus_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::FocusSync { focus_tiles_sent } => {
                                            node.shared.screen_focus_tiles_sent.store(*focus_tiles_sent, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::FocusPeripheralSync { peripheral_skipped } => {
                                            node.shared.screen_focus_peripheral_skipped.store(*peripheral_skipped, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTilePaletteMode { enabled, max_colors, enable_rle } => {
                                            let mut cfg = node.shared.screen_tile_palette_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.max_colors = (*max_colors).clamp(2, 16);
                                            cfg.enable_rle = *enable_rle;
                                        }
                                        ControlMsg::ResetScreenTilePaletteMode => {
                                            let mut cfg = node.shared.screen_tile_palette_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTilePaletteConfig::default();
                                            node.shared.screen_tile_palette_solid_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_palette_bilevel_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_palette_palette_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_palette_truecolor_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_palette_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TilePaletteSync { saved_bytes, .. } => {
                                            node.shared.screen_tile_palette_saved_bytes.store(*saved_bytes, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTilePredictMode { enabled, filter_mode, enable_rct } => {
                                            let mut cfg = node.shared.screen_tile_predict_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.filter_mode = tetherly_core::ScreenTilePredictFilter::from_u8(*filter_mode);
                                            cfg.enable_rct = *enable_rct;
                                        }
                                        ControlMsg::ResetScreenTilePredictMode => {
                                            let mut cfg = node.shared.screen_tile_predict_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTilePredictConfig::default();
                                            node.shared.screen_tile_predict_tiles_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_predict_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TilePredictSync { saved_bytes, .. } => {
                                            node.shared.screen_tile_predict_saved_bytes.store(*saved_bytes, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTilePresentMode { enabled, vsync_mode, double_buffer, tear_prevention } => {
                                            let mut cfg = node.shared.screen_tile_present_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.vsync_mode = tetherly_core::ScreenTilePresentVsyncMode::from_u8(*vsync_mode);
                                            cfg.double_buffer = *double_buffer;
                                            cfg.tear_prevention = *tear_prevention;
                                        }
                                        ControlMsg::ResetScreenTilePresentMode => {
                                            let mut cfg = node.shared.screen_tile_present_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTilePresentConfig::default();
                                            node.shared.screen_tile_present_committed_txs.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_present_swapped_frames.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_present_torn_prevented.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_present_total_latency_us.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TilePresentSync { committed_txs, swapped_frames } => {
                                            node.shared.screen_tile_present_committed_txs.store(*committed_txs, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_present_swapped_frames.store(*swapped_frames, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileNackRequest { seq, tile_index, .. } => {
                                            let tile_opt = {
                                                let buf = node.shared.screen_tile_resend_buffer.read().unwrap();
                                                buf.get_tile(*seq, *tile_index)
                                            };
                                            if let Some(tile) = tile_opt {
                                                let w = tile.w.max(64);
                                                let h = tile.h.max(64);
                                                let resent_frame = tetherly_core::ScreenFrame::encode_tile_grid(
                                                    *seq as u64,
                                                    tetherly_core::ScreenRect::new(tile.x as i32, tile.y as i32, w, h).unwrap_or_else(|_| tetherly_core::ScreenRect::new(0, 0, 64, 64).unwrap()),
                                                    tetherly_core::PixelFormat::Rgba8,
                                                    64,
                                                    1,
                                                    &[tile],
                                                );
                                                if let Ok(f) = resent_frame.to_frame() {
                                                    let _ = sess.send_bounded(f).await;
                                                    node.shared.screen_tile_nack_tiles_retransmitted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                }
                                            }
                                        }
                                        ControlMsg::ResetScreenTileNackMode => {
                                            let mut cfg = node.shared.screen_tile_nack_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileNackConfig::default();
                                            node.shared.screen_tile_resend_buffer.write().unwrap().clear();
                                            node.shared.screen_tile_nack_requests_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_nack_tiles_retransmitted.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_nack_degraded_commits.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileNackSync { nack_requests, retransmitted_tiles, degraded_commits } => {
                                            node.shared.screen_tile_nack_requests_sent.store(*nack_requests, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_nack_tiles_retransmitted.store(*retransmitted_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_nack_degraded_commits.store(*degraded_commits, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileSaliencyMode { enabled, high_threshold, low_threshold, drop_background_on_congestion } => {
                                            let mut cfg = node.shared.screen_tile_saliency_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.high_threshold = *high_threshold;
                                            cfg.low_threshold = *low_threshold;
                                            cfg.drop_background_on_congestion = *drop_background_on_congestion;
                                        }
                                        ControlMsg::ResetScreenTileSaliencyMode => {
                                            let mut cfg = node.shared.screen_tile_saliency_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileSaliencyConfig::default();
                                            node.shared.screen_tile_saliency_critical_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_normal_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_background_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_background_skipped.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileSaliencySync { critical_tiles, normal_tiles, background_tiles, background_tiles_skipped } => {
                                            node.shared.screen_tile_saliency_critical_tiles.store(*critical_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_normal_tiles.store(*normal_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_background_tiles.store(*background_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_saliency_background_skipped.store(*background_tiles_skipped, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileLodMode { enabled, max_lod, auto_lod_on_congestion } => {
                                            let mut cfg = node.shared.screen_tile_lod_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.max_lod = *max_lod;
                                            cfg.auto_lod_on_congestion = *auto_lod_on_congestion;
                                        }
                                        ControlMsg::ResetScreenTileLodMode => {
                                            let mut cfg = node.shared.screen_tile_lod_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileLodConfig::default();
                                            node.shared.screen_tile_lod0_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_lod1_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_lod2_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_lod_bytes_saved.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileLodSync { bytes_saved, .. } => {
                                            node.shared.screen_tile_lod_bytes_saved.store(*bytes_saved, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileBatchMode { enabled, target_chunk_bytes, pacing_interval_us } => {
                                            let mut cfg = node.shared.screen_tile_batch_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.target_chunk_bytes = *target_chunk_bytes;
                                            cfg.pacing_interval_us = *pacing_interval_us;
                                        }
                                        ControlMsg::ResetScreenTileBatchMode => {
                                            let mut cfg = node.shared.screen_tile_batch_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileBatchConfig::default();
                                            node.shared.screen_tile_batches_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_chunks_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_batch_bytes_total.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_batch_pacing_pauses.store(0, std::sync::atomic::Ordering::Relaxed);
                                            *node.shared.screen_tile_batch_receiver_state.lock().unwrap() = None;
                                        }
                                        ControlMsg::TileBatchSync { batches_sent, chunks_sent, avg_chunk_bytes, pacing_pauses } => {
                                            node.shared.screen_tile_batches_sent.store(*batches_sent, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_chunks_sent.store(*chunks_sent, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_batch_bytes_total.store((*avg_chunk_bytes as u64) * (*chunks_sent), std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_batch_pacing_pauses.store(*pacing_pauses, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::SetScreenTileMotionMode { enabled, max_displacement, min_match_ratio } => {
                                            let mut cfg = node.shared.screen_tile_motion_config.write().unwrap();
                                            cfg.enabled = *enabled;
                                            cfg.max_displacement = *max_displacement;
                                            cfg.min_match_ratio = *min_match_ratio;
                                        }
                                        ControlMsg::ResetScreenTileMotionMode => {
                                            let mut cfg = node.shared.screen_tile_motion_config.write().unwrap();
                                            *cfg = tetherly_core::ScreenTileMotionConfig::default();
                                            node.shared.screen_tile_motion_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_motion_fallback_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_motion_bytes_saved.store(0, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        ControlMsg::TileMotionSync { motion_tiles, fallback_tiles, bytes_saved } => {
                                            node.shared.screen_tile_motion_tiles_sent.store(*motion_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_motion_fallback_tiles_sent.store(*fallback_tiles, std::sync::atomic::Ordering::Relaxed);
                                            node.shared.screen_tile_motion_bytes_saved.store(*bytes_saved, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        _ => {}
                                    }
                                    let (reply, dynamic_bitrate, dynamic_fps, dynamic_scale) = {
                                        let mut engine = node
                                            .shared
                                            .screen_engine
                                            .lock()
                                            .expect("screen");
                                        let reply = engine.on_control_with_time(msg, now_us);
                                        let bitrate = engine.bitrate_bps();
                                        let fps = engine.fps_target();
                                        let scale = engine.scale_pct();
                                        (reply, bitrate, fps, scale)
                                    };
                                    if let Some(sel) = &node.shared.screen_encode_codec {
                                        if sel.bitrate_bps() != dynamic_bitrate {
                                            sel.set_bitrate_bps(dynamic_bitrate);
                                        }
                                        if sel.fps() != dynamic_fps {
                                            sel.set_fps(dynamic_fps);
                                        }
                                        if sel.scale_pct() != dynamic_scale {
                                            sel.set_scale_pct(dynamic_scale);
                                        }
                                    }
                                    node.shared.screen_source.set_bitrate_bps(dynamic_bitrate);
                                    node.shared.screen_source.set_fps(dynamic_fps);
                                    node.shared.screen_source.set_scale_pct(dynamic_scale);
                                    let cur_state = node.screen_state();
                                    let cur_allowed = matches!(
                                        cur_state,
                                        ScreenState::Allowed | ScreenState::Streaming
                                    );
                                    node.emit(UiEvent::screen_state_changed(
                                        cur_state.as_str(),
                                        cur_allowed,
                                        Some(sess.peer_id.to_string()),
                                    ));
                                    if let Ok(Some(reply)) = reply {
                                        if let Ok(f) = reply.to_frame() {
                                            let r_len = (f.payload.len() + 12) as u64;
                                            if sess.send_bounded(f).await.is_ok() {
                                                node.shared.telemetry.record_screen_tx(r_len);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            // Only reset the engine when this loop still owns the channel:
            // a newer attach has incremented the generation past ours. The
            // check runs INSIDE the engine lock, the same lock the attach
            // holds while bumping the generation — so an exit can never
            // observe "still newest" and then land its `on_peer_gone` after
            // a new attach's `on_peer_connected` (the m6_7 CI race). The
            // async channel/peer clears happen after the guard is dropped
            // (MutexGuard is not Send across the awaits below).
            let still_active = {
                let mut engine = node.shared.screen_engine.lock().expect("screen");
                let current = node
                    .shared
                    .screen_channel_gen
                    .load(std::sync::atomic::Ordering::SeqCst);
                eprintln!(
                    "[m6dbg] server loop exit: my_gen={} current={} (still_newest={})",
                    my_gen,
                    current,
                    current == my_gen + 1
                );
                if current == my_gen + 1 {
                    engine.on_peer_gone();
                }
                current == my_gen + 1
            };
            if still_active {
                *node.shared.screen_peer.lock().expect("screen_peer") = None;
                *node.shared.screen_out.lock().await = None;
            }
            node.shared
                .screen_privacy_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_privacy_lock_input
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let final_state = node.screen_state();
            let final_allowed =
                matches!(final_state, ScreenState::Allowed | ScreenState::Streaming);
            node.emit(UiEvent::screen_state_changed(
                final_state.as_str(),
                final_allowed,
                None,
            ));
        });
    }

    /// Visitor side of a 45720 session: this machine views a peer's screen.
    /// Reassembles `TMF1` fragments, decodes the codec, presents through the
    /// sink, and acks the sequence it accepted.
    pub(super) async fn attach_screen_client(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        // Client-side generation tag (mirror of the server-side
        // `screen_channel_gen`): this loop may only clear the registration
        // triple on exit if it is STILL the newest client channel. Bumped
        // before the fields are written so a replaced loop's exit check
        // always observes a newer generation.
        let my_client_gen = self
            .shared
            .screen_client_gen
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        eprintln!(
            "[m6dbg] client attach: gen={} peer={}",
            my_client_gen, peer_id
        );
        let (ctl_tx, mut ctl_rx) = mpsc::channel::<ControlMsg>(SCREEN_QUEUE_DEPTH);
        *self.shared.screen_ctl.lock().await = Some(ctl_tx);
        let (client_frame_tx, mut client_frame_rx) =
            mpsc::channel::<InnerFrame>(SCREEN_QUEUE_DEPTH);
        *self.shared.screen_client_out.lock().await = Some(client_frame_tx);
        *self.shared.screen_peer.lock().expect("screen_peer") = Some(peer_id.clone());
        // A new screen session re-evaluates the ADR-078 bitrate tier.
        self.shared
            .screen_bitrate_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.emit(UiEvent::screen_state_changed(
            "connected",
            false,
            Some(peer_id.to_string()),
        ));
        let initial_playout_mode = *self
            .shared
            .screen_playout_mode
            .lock()
            .expect("playout mode");
        let initial_cursor_mode = *self.shared.screen_cursor_mode.lock().expect("cursor mode");
        let initial_harq_mode = *self.shared.screen_harq_mode.lock().expect("harq mode");
        let initial_twcc_mode = *self.shared.screen_twcc_mode.lock().expect("twcc mode");
        let initial_probe_mode = *self.shared.screen_probe_mode.lock().expect("probe mode");
        let initial_sync_mode = *self.shared.screen_sync_mode.lock().expect("sync mode");
        let initial_latency_mode = *self
            .shared
            .screen_latency_mode
            .lock()
            .expect("latency mode");
        let mut receiver = ScreenReceiver::with_playout_mode(initial_playout_mode);
        receiver.set_cursor_mode(initial_cursor_mode);
        receiver.set_harq_mode(initial_harq_mode);
        receiver.set_twcc_mode(initial_twcc_mode);
        receiver.set_probe_mode(initial_probe_mode);
        receiver.set_sync_mode(initial_sync_mode);
        receiver.set_latency_mode(initial_latency_mode);
        let initial_tuning_mode = *self.shared.screen_tuning_mode.lock().expect("tuning mode");
        receiver.set_tuning_mode(initial_tuning_mode);
        let mut assembler = ScreenAssembler::new();
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        let mut playout_ticker = tokio::time::interval(std::time::Duration::from_millis(5));
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    _ = playout_ticker.tick() => {
                        let target_mode = *node.shared.screen_playout_mode.lock().expect("playout mode");
                        if receiver.playout_mode() != target_mode {
                            receiver.set_playout_mode(target_mode);
                        }
                        let target_cursor_mode = *node.shared.screen_cursor_mode.lock().expect("cursor mode");
                        if receiver.cursor_mode() != target_cursor_mode {
                            receiver.set_cursor_mode(target_cursor_mode);
                        }
                        let target_harq_mode = *node.shared.screen_harq_mode.lock().expect("harq mode");
                        if receiver.harq_mode() != target_harq_mode {
                            receiver.set_harq_mode(target_harq_mode);
                        }
                        let target_twcc_mode = *node.shared.screen_twcc_mode.lock().expect("twcc mode");
                        if receiver.twcc_mode() != target_twcc_mode {
                            receiver.set_twcc_mode(target_twcc_mode);
                        }
                        let target_probe_mode = *node.shared.screen_probe_mode.lock().expect("probe mode");
                        if receiver.probe_mode() != target_probe_mode {
                            receiver.set_probe_mode(target_probe_mode);
                        }
                        let target_sync_mode = *node.shared.screen_sync_mode.lock().expect("sync mode");
                        if receiver.sync_mode() != target_sync_mode {
                            receiver.set_sync_mode(target_sync_mode);
                        }
                        let target_latency_mode = *node.shared.screen_latency_mode.lock().expect("latency mode");
                        if receiver.latency_mode() != target_latency_mode {
                            receiver.set_latency_mode(target_latency_mode);
                        }
                        let target_tuning_mode = *node.shared.screen_tuning_mode.lock().expect("tuning mode");
                        if receiver.tuning_mode() != target_tuning_mode {
                            receiver.set_tuning_mode(target_tuning_mode);
                        }
                        *node.shared.screen_tuning_stats.lock().expect("tuning stats") = receiver.tuning_report();
                        let now_us = node.shared.clock.unix_us();
                        if let Some(twcc_fb) = receiver.generate_twcc_feedback(now_us) {
                            *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                            *node.shared.screen_twcc_stats.lock().expect("twcc stats") = receiver.twcc_stats();
                            if let Ok(twcc_frame) = twcc_fb.to_inner() {
                                let t_len = (twcc_frame.payload.len() + 12) as u64;
                                if sess.send_bounded(twcc_frame).await.is_ok() {
                                    node.shared.telemetry.record_screen_tx(t_len);
                                }
                            }
                        }
                        let mut played = false;
                        while let ScreenPlayoutAction::Play(ready_frame) = receiver.pop_playout(now_us) {
                            let mut presented_frame = ready_frame;
                            {
                                let wm_cfg = node.shared.screen_watermark.read().unwrap();
                                if wm_cfg.enabled {
                                    let peer_id_str = node
                                        .shared
                                        .screen_peer
                                        .lock()
                                        .ok()
                                        .and_then(|g| g.clone())
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| "peer".to_string());
                                    let time_str = format!("{}", now_us / 1_000_000);
                                    if let Ok(applied) = presented_frame.apply_watermark(&wm_cfg, &peer_id_str, &time_str) {
                                        if applied {
                                            node.shared.screen_watermark_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                    }
                                }
                            }
                            // Collaborative whiteboard annotations & transient virtual laser pointer (ADR-105)
                            {
                                let now_ms = now_us / 1_000;
                                let annotations = node.shared.screen_annotations.read().unwrap();
                                if !annotations.is_empty() {
                                    let _ = presented_frame.apply_annotations(&annotations, now_ms);
                                }
                                if let Some(laser) = node.shared.screen_laser_pointer.read().unwrap().as_ref() {
                                    let _ = presented_frame.apply_laser_pointer(laser);
                                }
                            }
                            let _ = node.shared.screen_sink.present(&presented_frame);
                            *node.shared.screen_last_frame.lock().expect("last_frame") = Some(presented_frame.clone());
                            if let Ok(mut slot) = node.shared.screen_recording.try_lock() {
                                if let Some(sess) = slot.as_mut() {
                                    if sess.writer.is_none() {
                                        let meta =
                                            ScreenRecordingMeta::from_frame(&presented_frame, sess.started_at_us);
                                        match std::fs::File::create(&sess.path) {
                                            Ok(file) => {
                                                match ScreenRecordingWriter::new(
                                                    meta,
                                                    std::io::BufWriter::new(file),
                                                ) {
                                                    Ok(w) => sess.writer = Some(w),
                                                    Err(e) => tracing::warn!(%e, "screen recording header write failed"),
                                                }
                                            }
                                            Err(e) => tracing::warn!(
                                                %e,
                                                path = %sess.path.display(),
                                                "screen recording file create failed"
                                            ),
                                        }
                                    }
                                    if let Some(w) = sess.writer.as_mut() {
                                        if w.append(&presented_frame, now_us).is_ok() {
                                            sess.frames = sess.frames.saturating_add(1);
                                            sess.bytes = sess
                                                .bytes
                                                .saturating_add(presented_frame.bytes.len() as u64 + 32);
                                            sess.last_seq = presented_frame.seq;
                                        }
                                    }
                                }
                            }
                            played = true;
                        }
                        if played || receiver.playout_stats().buffered_frames > 0 {
                            *node.shared.screen_playout_stats.lock().expect("playout stats") = receiver.playout_stats();
                        }
                    }
                    ctl = ctl_rx.recv() => {
                        match ctl {
                            Some(msg) => {
                                if matches!(msg, ControlMsg::ResetLatencyProfile) {
                                    receiver.reset_latency();
                                    *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                    *node.shared.screen_latency_stats.lock().expect("latency stats") = receiver.latency_report();
                                }
                                if let ControlMsg::SetTuningMode { mode } = msg {
                                    let m = ScreenTunerMode::from_tag(mode);
                                    *node.shared.screen_tuning_mode.lock().expect("tuning mode") = m;
                                    receiver.set_tuning_mode(m);
                                    *node.shared.screen_tuning_stats.lock().expect("tuning stats") = receiver.tuning_report();
                                }
                                if matches!(msg, ControlMsg::ResetTuningProfile) {
                                    receiver.reset_tuning();
                                    *node.shared.screen_tuning_stats.lock().expect("tuning stats") = receiver.tuning_report();
                                }
                                if let ControlMsg::SetRecordingState { active } = msg {
                                    node.shared
                                        .screen_peer_recording
                                        .store(active, std::sync::atomic::Ordering::Relaxed);
                                }
                                match msg.to_frame() {
                                    Ok(f) => {
                                        let f_len = (f.payload.len() + 12) as u64;
                                        if sess.send_bounded(f).await.is_err() {
                                            break;
                                        }
                                        node.shared.telemetry.record_screen_tx(f_len);
                                    }
                                    Err(_) => continue,
                                }
                            }
                            None => break,
                        }
                    }
                    client_frame = client_frame_rx.recv() => {
                        match client_frame {
                            Some(f) => {
                                let f_len = (f.payload.len() + 12) as u64;
                                if sess.send_bounded(f).await.is_err() {
                                    break;
                                }
                                node.shared.telemetry.record_screen_tx(f_len);
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                node.shared.telemetry.record_screen_rx(f_len);
                                if frame.ty == TYPE_SCREEN_CONTROL {
                                    if let Ok(msg) = ControlMsg::from_frame(&frame) {
                                        if let ControlMsg::SetWatermarkState { enabled, host_enforced } = msg {
                                            let mut cfg = node.shared.screen_watermark.write().unwrap();
                                            cfg.enabled = enabled;
                                            cfg.host_enforced = host_enforced;
                                            node.shared.screen_peer_watermark_enforced.store(host_enforced, std::sync::atomic::Ordering::Relaxed);
                                            tracing::info!(enabled, host_enforced, "viewer received screen watermark state from host");
                                        }
                                        if let ControlMsg::SetRecordingState { active } = msg {
                                            node.shared.screen_peer_recording.store(active, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        if matches!(msg, ControlMsg::Stop) {
                                            // ADR-138: an unsolicited `Stop` from the host
                                            // on the control channel is the Start-refusal
                                            // reply (consent revoked mid-session). Latch
                                            // it so a pending remote-session start rolls
                                            // back instead of presenting a dead screen.
                                            // Ticket protocol: `screen_start` claims a
                                            // fresh ticket BEFORE sending Start and
                                            // records it as the open window; the reply
                                            // may only latch the refusal while the
                                            // window it belongs to is still open — a
                                            // leftover refusal from a PREVIOUS channel
                                            // arriving after a newer start claimed its
                                            // own ticket is discarded (m6_7 CI race).
                                            let current = node
                                                .shared
                                                .screen_start_ticket
                                                .load(std::sync::atomic::Ordering::SeqCst);
                                            if current % 2 == 1 {
                                                // Odd = a start attempt is in flight (its
                                                // claim made the value odd); latch the refusal.
                                                node.shared
                                                    .screen_start_refused
                                                    .store(true, std::sync::atomic::Ordering::SeqCst);
                                                tracing::info!("viewer: host refused screen start (consent stop reply)");
                                            } else {
                                                tracing::info!("viewer: ignoring stale screen-start Stop reply (no start in flight)");
                                            }
                                        }
                                        match &msg {
                                            ControlMsg::SetLaserPointer { x, y, active, color_rgb } => {
                                                let state = if *active {
                                                    Some(tetherly_core::LaserPointerState {
                                                        x: *x,
                                                        y: *y,
                                                        active: true,
                                                        color_rgb: *color_rgb,
                                                        radius: 12,
                                                    })
                                                } else {
                                                    None
                                                };
                                                *node.shared.screen_laser_pointer.write().unwrap() = state;
                                                node.shared.screen_laser_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::ClearAnnotations => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                if wb.policy().allow_peer_clear {
                                                    wb.clear();
                                                    node.shared.screen_annotations.write().unwrap().clear();
                                                }
                                            }
                                            ControlMsg::WhiteboardUndo { author_hash } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let undone = if *author_hash == 0 {
                                                    wb.undo(None)
                                                } else {
                                                    let target = wb
                                                        .active_items()
                                                        .iter()
                                                        .rfind(|it| tetherly_core::hash_author(&it.author) == *author_hash)
                                                        .map(|it| it.author.clone());
                                                    wb.undo(target.as_deref())
                                                };
                                                if undone.is_some() {
                                                    node.shared.screen_whiteboard_undo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            ControlMsg::WhiteboardRedo { author_hash: _ } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let redone = wb.redo(None);
                                                if redone.is_some() {
                                                    node.shared.screen_whiteboard_redo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            ControlMsg::SetWhiteboardPolicy {
                                                read_only,
                                                allow_peer_clear,
                                                max_items,
                                            } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                wb.set_policy(tetherly_core::WhiteboardPolicy {
                                                    read_only: *read_only,
                                                    allow_peer_clear: *allow_peer_clear,
                                                    max_items: *max_items,
                                                });
                                            }
                                            ControlMsg::SetScreenRoi { x, y, w, h } => {
                                                let mut roi = node.shared.screen_roi.write().unwrap();
                                                roi.active = true;
                                                roi.x = *x;
                                                roi.y = *y;
                                                roi.w = *w;
                                                roi.h = *h;
                                                node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::ResetScreenRoi => {
                                                let mut roi = node.shared.screen_roi.write().unwrap();
                                                roi.active = false;
                                                roi.zoom_percent = 100;
                                                node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenRoiZoom { zoom_percent } => {
                                                let mut roi = node.shared.screen_roi.write().unwrap();
                                                roi.zoom_percent = *zoom_percent;
                                                node.shared.screen_roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenColorMode { mode, threshold } => {
                                                let mut cfg = node.shared.screen_color_mode.write().unwrap();
                                                cfg.mode = tetherly_core::ScreenColorMode::from_tag(*mode);
                                                cfg.threshold = *threshold;
                                                node.shared.screen_color_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::ResetScreenColorMode => {
                                                let mut cfg = node.shared.screen_color_mode.write().unwrap();
                                                *cfg = tetherly_core::ScreenColorConfig::default();
                                                node.shared.screen_color_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenDeltaMode { enabled, keyframe_interval } => {
                                                let mut cfg = node.shared.screen_delta_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.keyframe_interval = *keyframe_interval;
                                                node.shared.screen_delta_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::ResetScreenDeltaMode => {
                                                let mut cfg = node.shared.screen_delta_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenDeltaConfig::default();
                                                node.shared.screen_delta_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenQualityMetricMode { enabled, sample_interval } => {
                                                let mut cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.sample_interval = *sample_interval;
                                                node.shared.screen_metric_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::ResetScreenQualityMetricMode => {
                                                let mut cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenQualityMetricConfig::default();
                                                node.shared.screen_metric_counter.store(0, std::sync::atomic::Ordering::Relaxed);
                                                let mut state = node.shared.screen_quality_metric_state.write().unwrap();
                                                *state = ScreenQualityMetricState::default();
                                                *node.shared.screen_metric_ref_frame.write().unwrap() = None;
                                            }
                                            ControlMsg::SetScreenQoeMode { mode, target_psnr_db, target_ssim_pct } => {
                                                let qoe_mode = tetherly_core::ScreenQoeMode::from_tag(*mode);
                                                let mut qoe = node.shared.screen_qoe_controller.write().unwrap();
                                                let cfg = tetherly_core::ScreenQoeConfig {
                                                    mode: qoe_mode,
                                                    target_psnr_db: *target_psnr_db,
                                                    target_ssim_pct: *target_ssim_pct,
                                                };
                                                qoe.set_config(cfg);
                                                if qoe_mode.is_active() {
                                                    let mut metric_cfg = node.shared.screen_quality_metric_config.write().unwrap();
                                                    metric_cfg.enabled = true;
                                                }
                                            }
                                            ControlMsg::ResetScreenQoeMode => {
                                                let mut qoe = node.shared.screen_qoe_controller.write().unwrap();
                                                qoe.reset();
                                            }
                                            ControlMsg::SetScreenPirMode { mode, bands } => {
                                                let pir_mode = tetherly_core::ScreenPirMode::from_u8(*mode);
                                                let mut pir_cfg = node.shared.screen_pir_config.write().unwrap();
                                                pir_cfg.mode = pir_mode;
                                                pir_cfg.bands = *bands;
                                                if pir_mode == tetherly_core::ScreenPirMode::OnDemand {
                                                    node.shared.screen_pir_active_sweep.store(true, std::sync::atomic::Ordering::Relaxed);
                                                    node.shared.screen_pir_current_band.store(0, std::sync::atomic::Ordering::Relaxed);
                                                } else if pir_mode == tetherly_core::ScreenPirMode::Continuous {
                                                    node.shared.screen_pir_active_sweep.store(true, std::sync::atomic::Ordering::Relaxed);
                                                } else {
                                                    node.shared.screen_pir_active_sweep.store(false, std::sync::atomic::Ordering::Relaxed);
                                                }
                                            }
                                            ControlMsg::ResetScreenPirMode => {
                                                let mut pir_cfg = node.shared.screen_pir_config.write().unwrap();
                                                *pir_cfg = tetherly_core::ScreenPirConfig::default();
                                                node.shared.screen_pir_active_sweep.store(false, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_pir_current_band.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenLtrMode { enabled, auto_interval } => {
                                                let mut cfg = node.shared.screen_ltr_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.auto_interval = *auto_interval;
                                                if !*enabled {
                                                    *node.shared.screen_sender_ltr_canvas.write().unwrap() = None;
                                                    *node.shared.screen_receiver_ltr_canvas.write().unwrap() = None;
                                                    node.shared.screen_ltr_current_seq.store(0, std::sync::atomic::Ordering::Relaxed);
                                                    node.shared.screen_ltr_recovery_pending.store(false, std::sync::atomic::Ordering::Relaxed);
                                                }
                                            }
                                            ControlMsg::ResetScreenLtrMode => {
                                                let mut cfg = node.shared.screen_ltr_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenLtrConfig::default();
                                                *node.shared.screen_sender_ltr_canvas.write().unwrap() = None;
                                                *node.shared.screen_receiver_ltr_canvas.write().unwrap() = None;
                                                node.shared.screen_ltr_current_seq.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_ltr_recovery_pending.store(false, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::LtrRecoveryRequest { ltr_seq: _ } => {
                                                node.shared.screen_ltr_recoveries_requested.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_ltr_recovery_pending.store(true, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::LtrAnchorSync { ltr_seq } => {
                                                let cur_canvas = node.shared.screen_receiver_canvas.read().unwrap().clone();
                                                if let Some(canvas) = cur_canvas {
                                                    *node.shared.screen_receiver_ltr_canvas.write().unwrap() = Some(canvas);
                                                }
                                                node.shared.screen_ltr_current_seq.store(*ltr_seq, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_ltr_anchors_established.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenSvcMode { mode, drop_t1 } => {
                                                let mut cfg = node.shared.screen_svc_config.write().unwrap();
                                                cfg.mode = tetherly_core::ScreenSvcMode::from_u8(*mode);
                                                cfg.drop_t1 = *drop_t1;
                                                if !cfg.is_enabled() {
                                                    *node.shared.screen_sender_t0_canvas.write().unwrap() = None;
                                                    *node.shared.screen_receiver_t0_canvas.write().unwrap() = None;
                                                }
                                            }
                                            ControlMsg::ResetScreenSvcMode => {
                                                let mut cfg = node.shared.screen_svc_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenSvcConfig::default();
                                                *node.shared.screen_sender_t0_canvas.write().unwrap() = None;
                                                *node.shared.screen_receiver_t0_canvas.write().unwrap() = None;
                                                node.shared.screen_svc_t0_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_svc_t1_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_svc_t1_frames_dropped.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_svc_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SvcT1DropSync { dropped_count } => {
                                                node.shared.screen_svc_t1_frames_dropped.store(*dropped_count, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenMotionMode { enabled, max_displacement } => {
                                                let mut cfg = node.shared.screen_motion_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.max_displacement = (*max_displacement).clamp(
                                                    tetherly_core::SCREEN_MOTION_MIN_DISPLACEMENT,
                                                    tetherly_core::SCREEN_MOTION_MAX_DISPLACEMENT,
                                                );
                                                if !*enabled {
                                                    *node.shared.screen_sender_motion_canvas.write().unwrap() = None;
                                                }
                                            }
                                            ControlMsg::ResetScreenMotionMode => {
                                                let mut cfg = node.shared.screen_motion_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenMotionConfig::default();
                                                *node.shared.screen_sender_motion_canvas.write().unwrap() = None;
                                                node.shared.screen_motion_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_motion_fallback_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_motion_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::MotionScrollSync { scroll_frames_sent } => {
                                                node.shared.screen_motion_frames_sent.store(*scroll_frames_sent, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileMode { enabled, tile_size } => {
                                                let mut cfg = node.shared.screen_tile_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.tile_size = (*tile_size).clamp(
                                                    tetherly_core::SCREEN_TILE_MIN_SIZE,
                                                    tetherly_core::SCREEN_TILE_MAX_SIZE,
                                                );
                                                if !*enabled {
                                                    *node.shared.screen_sender_tile_canvas.write().unwrap() = None;
                                                }
                                            }
                                            ControlMsg::ResetScreenTileMode => {
                                                let mut cfg = node.shared.screen_tile_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileConfig::default();
                                                *node.shared.screen_sender_tile_canvas.write().unwrap() = None;
                                                node.shared.screen_tile_frames_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_dirty_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_skipped_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileGridSync { tiles_sent } => {
                                                node.shared.screen_tile_dirty_tiles_sent.store(*tiles_sent, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileCacheMode { enabled, max_entries } => {
                                                let mut cfg = node.shared.screen_tile_cache_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.max_entries = (*max_entries).clamp(
                                                    tetherly_core::SCREEN_TILE_CACHE_MIN_ENTRIES,
                                                    tetherly_core::SCREEN_TILE_CACHE_MAX_ENTRIES,
                                                );
                                                node.shared.screen_tile_cache_dict.lock().unwrap().resize(cfg.max_entries);
                                                node.shared.screen_tile_cache_receiver.lock().unwrap().resize(cfg.max_entries as usize, None);
                                            }
                                            ControlMsg::ResetScreenTileCacheMode => {
                                                let mut cfg = node.shared.screen_tile_cache_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileCacheConfig::default();
                                                node.shared.screen_tile_cache_dict.lock().unwrap().clear();
                                                let mut rx = node.shared.screen_tile_cache_receiver.lock().unwrap();
                                                rx.clear();
                                                rx.resize(tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES as usize, None);
                                                node.shared.screen_tile_cache_hits.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_cache_misses.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_cache_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileCacheSync { hits } => {
                                                node.shared.screen_tile_cache_hits.store(*hits, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenFocusMode { enabled, mode, radius, peripheral_skip_ratio } => {
                                                let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.mode = tetherly_core::ScreenFocusMode::from_u8(*mode);
                                                cfg.radius = (*radius).clamp(
                                                    tetherly_core::SCREEN_FOCUS_MIN_RADIUS,
                                                    tetherly_core::SCREEN_FOCUS_MAX_RADIUS,
                                                );
                                                cfg.peripheral_skip_ratio = (*peripheral_skip_ratio).clamp(
                                                    0,
                                                    tetherly_core::SCREEN_FOCUS_MAX_SKIP_RATIO,
                                                );
                                            }
                                            ControlMsg::SetScreenFocusBox { box_x, box_y, box_w, box_h } => {
                                                let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                                cfg.box_x = *box_x;
                                                cfg.box_y = *box_y;
                                                cfg.box_w = *box_w;
                                                cfg.box_h = *box_h;
                                            }
                                            ControlMsg::ResetScreenFocusMode => {
                                                let mut cfg = node.shared.screen_focus_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenFocusConfig::default();
                                                node.shared.screen_focus_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_focus_peripheral_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_focus_peripheral_skipped.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_focus_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::FocusSync { focus_tiles_sent } => {
                                                node.shared.screen_focus_tiles_sent.store(*focus_tiles_sent, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::FocusPeripheralSync { peripheral_skipped } => {
                                                node.shared.screen_focus_peripheral_skipped.store(*peripheral_skipped, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTilePaletteMode { enabled, max_colors, enable_rle } => {
                                                let mut cfg = node.shared.screen_tile_palette_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.max_colors = (*max_colors).clamp(2, 16);
                                                cfg.enable_rle = *enable_rle;
                                            }
                                            ControlMsg::ResetScreenTilePaletteMode => {
                                                let mut cfg = node.shared.screen_tile_palette_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTilePaletteConfig::default();
                                                node.shared.screen_tile_palette_solid_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_palette_bilevel_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_palette_palette_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_palette_truecolor_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_palette_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TilePaletteSync { saved_bytes, .. } => {
                                                node.shared.screen_tile_palette_saved_bytes.store(*saved_bytes, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTilePredictMode { enabled, filter_mode, enable_rct } => {
                                                let mut cfg = node.shared.screen_tile_predict_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.filter_mode = tetherly_core::ScreenTilePredictFilter::from_u8(*filter_mode);
                                                cfg.enable_rct = *enable_rct;
                                            }
                                            ControlMsg::ResetScreenTilePredictMode => {
                                                let mut cfg = node.shared.screen_tile_predict_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTilePredictConfig::default();
                                                node.shared.screen_tile_predict_tiles_count.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_predict_saved_bytes.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TilePredictSync { saved_bytes, .. } => {
                                                node.shared.screen_tile_predict_saved_bytes.store(*saved_bytes, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTilePresentMode { enabled, vsync_mode, double_buffer, tear_prevention } => {
                                                let mut cfg = node.shared.screen_tile_present_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.vsync_mode = tetherly_core::ScreenTilePresentVsyncMode::from_u8(*vsync_mode);
                                                cfg.double_buffer = *double_buffer;
                                                cfg.tear_prevention = *tear_prevention;
                                            }
                                            ControlMsg::ResetScreenTilePresentMode => {
                                                let mut cfg = node.shared.screen_tile_present_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTilePresentConfig::default();
                                                node.shared.screen_tile_present_committed_txs.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_present_swapped_frames.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_present_torn_prevented.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_present_total_latency_us.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TilePresentSync { committed_txs, swapped_frames } => {
                                                node.shared.screen_tile_present_committed_txs.store(*committed_txs, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_present_swapped_frames.store(*swapped_frames, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileNackRequest { seq, tile_index, .. } => {
                                                let tile_opt = {
                                                    let buf = node.shared.screen_tile_resend_buffer.read().unwrap();
                                                    buf.get_tile(*seq, *tile_index)
                                                };
                                                if let Some(tile) = tile_opt {
                                                    let w = tile.w.max(64);
                                                    let h = tile.h.max(64);
                                                    let resent_frame = tetherly_core::ScreenFrame::encode_tile_grid(
                                                        *seq as u64,
                                                        tetherly_core::ScreenRect::new(tile.x as i32, tile.y as i32, w, h).unwrap_or_else(|_| tetherly_core::ScreenRect::new(0, 0, 64, 64).unwrap()),
                                                        tetherly_core::PixelFormat::Rgba8,
                                                        64,
                                                        1,
                                                        &[tile],
                                                    );
                                                    if let Ok(f) = resent_frame.to_frame() {
                                                        let _ = sess.send_bounded(f).await;
                                                        node.shared.screen_tile_nack_tiles_retransmitted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    }
                                                }
                                            }
                                            ControlMsg::ResetScreenTileNackMode => {
                                                let mut cfg = node.shared.screen_tile_nack_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileNackConfig::default();
                                                node.shared.screen_tile_resend_buffer.write().unwrap().clear();
                                                node.shared.screen_tile_nack_requests_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_nack_tiles_retransmitted.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_nack_degraded_commits.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileNackSync { nack_requests, retransmitted_tiles, degraded_commits } => {
                                                node.shared.screen_tile_nack_requests_sent.store(*nack_requests, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_nack_tiles_retransmitted.store(*retransmitted_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_nack_degraded_commits.store(*degraded_commits, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileSaliencyMode { enabled, high_threshold, low_threshold, drop_background_on_congestion } => {
                                                let mut cfg = node.shared.screen_tile_saliency_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.high_threshold = *high_threshold;
                                                cfg.low_threshold = *low_threshold;
                                                cfg.drop_background_on_congestion = *drop_background_on_congestion;
                                            }
                                            ControlMsg::ResetScreenTileSaliencyMode => {
                                                let mut cfg = node.shared.screen_tile_saliency_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileSaliencyConfig::default();
                                                node.shared.screen_tile_saliency_critical_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_normal_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_background_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_background_skipped.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileSaliencySync { critical_tiles, normal_tiles, background_tiles, background_tiles_skipped } => {
                                                node.shared.screen_tile_saliency_critical_tiles.store(*critical_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_normal_tiles.store(*normal_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_background_tiles.store(*background_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_saliency_background_skipped.store(*background_tiles_skipped, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileLodMode { enabled, max_lod, auto_lod_on_congestion } => {
                                                let mut cfg = node.shared.screen_tile_lod_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.max_lod = *max_lod;
                                                cfg.auto_lod_on_congestion = *auto_lod_on_congestion;
                                            }
                                            ControlMsg::ResetScreenTileLodMode => {
                                                let mut cfg = node.shared.screen_tile_lod_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileLodConfig::default();
                                                node.shared.screen_tile_lod0_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_lod1_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_lod2_tiles.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_lod_bytes_saved.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileLodSync { bytes_saved, .. } => {
                                                node.shared.screen_tile_lod_bytes_saved.store(*bytes_saved, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileBatchMode { enabled, target_chunk_bytes, pacing_interval_us } => {
                                                let mut cfg = node.shared.screen_tile_batch_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.target_chunk_bytes = *target_chunk_bytes;
                                                cfg.pacing_interval_us = *pacing_interval_us;
                                            }
                                            ControlMsg::ResetScreenTileBatchMode => {
                                                let mut cfg = node.shared.screen_tile_batch_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileBatchConfig::default();
                                                node.shared.screen_tile_batches_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_chunks_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_batch_bytes_total.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_batch_pacing_pauses.store(0, std::sync::atomic::Ordering::Relaxed);
                                                *node.shared.screen_tile_batch_receiver_state.lock().unwrap() = None;
                                            }
                                            ControlMsg::TileBatchSync { batches_sent, chunks_sent, avg_chunk_bytes, pacing_pauses } => {
                                                node.shared.screen_tile_batches_sent.store(*batches_sent, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_chunks_sent.store(*chunks_sent, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_batch_bytes_total.store((*avg_chunk_bytes as u64) * (*chunks_sent), std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_batch_pacing_pauses.store(*pacing_pauses, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::SetScreenTileMotionMode { enabled, max_displacement, min_match_ratio } => {
                                                let mut cfg = node.shared.screen_tile_motion_config.write().unwrap();
                                                cfg.enabled = *enabled;
                                                cfg.max_displacement = *max_displacement;
                                                cfg.min_match_ratio = *min_match_ratio;
                                            }
                                            ControlMsg::ResetScreenTileMotionMode => {
                                                let mut cfg = node.shared.screen_tile_motion_config.write().unwrap();
                                                *cfg = tetherly_core::ScreenTileMotionConfig::default();
                                                node.shared.screen_tile_motion_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_motion_fallback_tiles_sent.store(0, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_motion_bytes_saved.store(0, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            ControlMsg::TileMotionSync { motion_tiles, fallback_tiles, bytes_saved } => {
                                                node.shared.screen_tile_motion_tiles_sent.store(*motion_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_motion_fallback_tiles_sent.store(*fallback_tiles, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_tile_motion_bytes_saved.store(*bytes_saved, std::sync::atomic::Ordering::Relaxed);
                                            }
                                            _ => {}
                                        }
                                    }
                                    continue;
                                }
                                if frame.ty == tetherly_core::TYPE_SCREEN_ANNOTATION {
                                    if let Ok(msg) = tetherly_core::ScreenAnnotationMsg::from_frame(&frame) {
                                        match msg {
                                            tetherly_core::ScreenAnnotationMsg::Add(item) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let _ = wb.add_item(item);
                                                node.shared.screen_annotation_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Remove { id } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                wb.active_items_mut().retain(|it| it.id != id);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Undo { author } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let a_opt = if author.is_empty() { None } else { Some(author.as_str()) };
                                                let undone = wb.undo(a_opt);
                                                if undone.is_some() {
                                                    node.shared.screen_whiteboard_undo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Redo { author } => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let a_opt = if author.is_empty() { None } else { Some(author.as_str()) };
                                                let redone = wb.redo(a_opt);
                                                if redone.is_some() {
                                                    node.shared.screen_whiteboard_redo_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                    *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Clear => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                if wb.policy().allow_peer_clear {
                                                    wb.clear();
                                                    node.shared.screen_annotations.write().unwrap().clear();
                                                }
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Snapshot(snapshot) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                let _ = wb.import_snapshot(snapshot, true);
                                                *node.shared.screen_annotations.write().unwrap() = wb.active_items().to_vec();
                                            }
                                            tetherly_core::ScreenAnnotationMsg::Policy(policy) => {
                                                let mut wb = node.shared.screen_whiteboard.write().unwrap();
                                                wb.set_policy(policy);
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if frame.ty == tetherly_core::TYPE_SCREEN_CURSOR {
                                    if let Ok(update) = tetherly_core::ScreenCursorUpdate::from_frame(&frame) {
                                        receiver.on_cursor(update);
                                        *node.shared.screen_cursor_stats.lock().expect("cursor stats") = receiver.cursor_stats();
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                    }
                                    continue;
                                }
                                if frame.ty == TYPE_SCREEN_PROBE {
                                    if let Ok(probe) = ScreenProbePacket::decode(&frame.payload) {
                                        let now_us = node.shared.clock.unix_us();
                                        receiver.on_probe_packet_with_time(probe, now_us);
                                        *node.shared.screen_probe_stats.lock().expect("probe stats") = receiver.probe_stats();
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                        if let Some(twcc_fb) = receiver.generate_twcc_feedback(now_us) {
                                            *node.shared.screen_twcc_stats.lock().expect("twcc stats") = receiver.twcc_stats();
                                            if let Ok(twcc_frame) = twcc_fb.to_inner() {
                                                let t_len = (twcc_frame.payload.len() + 12) as u64;
                                                if sess.send_bounded(twcc_frame).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(t_len);
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                                if frame.ty == TYPE_SCREEN_SYNC {
                                    if let Ok(sync_beacon) = ScreenAudioSyncFrame::decode(&frame.payload) {
                                        let now_us = node.shared.clock.unix_us();
                                        receiver.on_sync_beacon_with_time(&sync_beacon, now_us);
                                        *node.shared.screen_sync_stats.lock().expect("sync stats") = receiver.sync_stats();
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                        *node.shared.screen_playout_stats.lock().expect("playout stats") = receiver.playout_stats();
                                    }
                                    continue;
                                }
                                if frame.ty == TYPE_SCREEN_PROFILE {
                                    if let Ok(profile) = ScreenProfileFrame::decode(&frame.payload) {
                                        receiver.on_profile_frame(&profile);
                                        *node.shared.screen_latency_stats.lock().expect("latency stats") = receiver.latency_report();
                                        *node.shared.screen_tuning_stats.lock().expect("tuning stats") = receiver.tuning_report();
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                    }
                                    continue;
                                }
                                let assembled_opt = if frame.ty == TYPE_SCREEN_FEC {
                                    let now_us = node.shared.clock.unix_us();
                                    match ScreenFecShard::from_inner_frame(&frame) {
                                        Ok(shard) => match receiver.on_fec_shard_with_time(shard, now_us) {
                                            Ok(opt) => {
                                                *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                                opt
                                            }
                                            Err(e) => {
                                                tracing::warn!(%e, "screen fec assembler error; closing viewer");
                                                break;
                                            }
                                        },
                                        Err(e) => {
                                            tracing::warn!(%e, "invalid screen fec shard frame");
                                            continue;
                                        }
                                    }
                                } else {
                                    match assembler.push(&frame) {
                                        Ok(opt) => opt,
                                        Err(e) => {
                                            tracing::warn!(%e, "screen assembler error; closing viewer");
                                            break;
                                        }
                                    }
                                };
                                let now_us = node.shared.clock.unix_us();
                                if let Some(twcc_fb) = receiver.generate_twcc_feedback(now_us) {
                                    *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
                                    *node.shared.screen_twcc_stats.lock().expect("twcc stats") = receiver.twcc_stats();
                                    if let Ok(twcc_frame) = twcc_fb.to_inner() {
                                        let t_len = (twcc_frame.payload.len() + 12) as u64;
                                        if sess.send_bounded(twcc_frame).await.is_ok() {
                                            node.shared.telemetry.record_screen_tx(t_len);
                                        }
                                    }
                                }
                                if assembled_opt.is_none() {
                                    let rtt_estimate_us = node
                                        .shared
                                        .screen_engine
                                        .lock()
                                        .map(|e| e.stats().rtt_latest_us.unwrap_or(20_000))
                                        .unwrap_or(20_000);
                                    let nack_opt = if frame.ty == TYPE_SCREEN_FEC {
                                        receiver.generate_nack_on_fec(now_us, rtt_estimate_us)
                                    } else {
                                        receiver.generate_nack_on_frag(&assembler, now_us, rtt_estimate_us)
                                    };
                                    if let Some(nack) = nack_opt {
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") =
                                            receiver.stats();
                                        if let Ok(nack_frame) = nack.to_inner() {
                                            let n_len = (nack_frame.payload.len() + 12) as u64;
                                            if sess.send_bounded(nack_frame).await.is_ok() {
                                                node.shared.telemetry.record_screen_tx(n_len);
                                            }
                                        }
                                    }
                                }
                                if let Some(sf) = assembled_opt {
                                    if receiver.harq_stats().nacks_sent > 0 {
                                        receiver.record_recovered_frame();
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") =
                                            receiver.stats();
                                    }
                                    // Ordering gate: a frame no newer than the last
                                    // accepted one is a stale duplicate (a NACK
                                    // retransmission racing the original, a whole
                                    // keyframe replayed by recovery). It must not
                                    // reach *any* decoder below: the canvas is the
                                    // reference picture for tile and delta frames,
                                    // so rebuilding it from an older frame rewinds
                                    // it and every following delta then decodes on
                                    // the wrong base — a permanently corrupt view.
                                    // `apply` still records the drop, but nothing
                                    // is decoded, applied to the canvas or pushed
                                    // to playout.
                                    if sf.seq <= receiver.last_seq() {
                                        let _ = receiver.apply(&sf);
                                        *node.shared.screen_rx_stats.lock().expect("screen rx") =
                                            receiver.stats();
                                        continue;
                                    }
                                    let codec_tag = sf.codec_tag;
                                    // Decode by the frame's wire tag:
                                    // zstd (1) or H.264 (3, ADR-079).
                                    let present_frame = if sf.codec_tag == tetherly_core::SCREEN_COMPRESS_MOTION_SCROLL
                                        || sf.codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_GRID
                                        || sf.codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_CACHE
                                        || sf.codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_BATCH
                                    {
                                        sf
                                    } else if sf.compressed() {
                                        let codec = if sf.codec_tag
                                            == tetherly_core::SCREEN_COMPRESS_H264
                                        {
                                            node.shared.screen_codec_h264.clone()
                                        } else {
                                            node.shared.screen_codec.clone()
                                        };
                                        match codec.decompress_frame(
                                            &sf.bytes,
                                            sf.rect.w,
                                            sf.rect.h,
                                            sf.raw_len(),
                                        ) {
                                            Ok(raw) => ScreenFrame {
                                                bytes: raw,
                                                codec_tag: tetherly_core::SCREEN_COMPRESS_NONE,
                                                ..sf
                                            },
                                            Err(e) => {
                                                tracing::warn!(%e, seq = sf.seq, "screen decompress failed; requesting keyframe recovery");
                                                let req = ControlMsg::KeyframeRequest;
                                                if let Ok(f) = req.to_frame() {
                                                    let r_len = (f.payload.len() + 12) as u64;
                                                    if sess.send_bounded(f).await.is_ok() {
                                                        node.shared.telemetry.record_screen_tx(r_len);
                                                    }
                                                }
                                                continue;
                                            }
                                        }
                                    } else {
                                        sf
                                    };
                                    let present_frame = if codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_GRID {
                                        let (need_recovery, frame_opt, nack_opt) = {
                                            match present_frame.decode_tile_grid() {
                                                Ok((_tile_size, total_tiles, dirty_tiles)) => {
                                                    let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                                    match canvas_guard.as_mut() {
                                                        Some(canvas) => {
                                                            let mut apply_ok = true;
                                                            let mut failed_tile_idx: Option<u16> = None;
                                                            let count = dirty_tiles.len() as u64;
                                                            let bpp = canvas.format.bpp();
                                                            let present_cfg = *node.shared.screen_tile_present_config.read().unwrap();
                                                            let mut back_opt = if present_cfg.enabled {
                                                                let expected_len = (canvas.rect.w as usize) * (canvas.rect.h as usize) * bpp;
                                                                let mut bg = node.shared.screen_receiver_back_buffer.write().unwrap();
                                                                if bg.is_none()
                                                                    || bg.as_ref().map(|b| b.rect != canvas.rect || b.bytes.len() != expected_len).unwrap_or(true)
                                                                {
                                                                    *bg = Some(canvas.clone());
                                                                }
                                                                Some(bg)
                                                            } else {
                                                                None
                                                            };
                                                            for tile in dirty_tiles {
                                                                let raw_len = (tile.w as usize) * (tile.h as usize) * bpp;
                                                                let raw_data = if tile.data.len() == raw_len {
                                                                    tile.data
                                                                } else {
                                                                    let subcodec = if !tile.data.is_empty() { tile.data[0] } else { 0 };
                                                                    if subcodec == tetherly_core::TILE_SUBCODEC_SOLID
                                                                        || subcodec == tetherly_core::TILE_SUBCODEC_BILEVEL
                                                                        || subcodec == tetherly_core::TILE_SUBCODEC_PALETTE16
                                                                    {
                                                                        match tetherly_core::decompress_tile_palette(subcodec, &tile.data, tile.w, tile.h, raw_len) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(_) => {
                                                                                match node.shared.screen_codec.decompress_frame(
                                                                                    &tile.data,
                                                                                    tile.w,
                                                                                    tile.h,
                                                                                    raw_len,
                                                                                ) {
                                                                                    Ok(decomp) => decomp,
                                                                                    Err(e) => {
                                                                                        tracing::warn!(%e, "tile patch decompress failed");
                                                                                        failed_tile_idx = Some(tile.index);
                                                                                        apply_ok = false;
                                                                                        break;
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD {
                                                                        match tetherly_core::decompress_tile_predictive(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                            node.shared.screen_codec.as_ref(),
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile predictive decompress failed");
                                                                                failed_tile_idx = Some(tile.index);
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_LOD {
                                                                        match tetherly_core::decompress_tile_lod(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                            node.shared.screen_codec.as_ref(),
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile lod decompress failed");
                                                                                failed_tile_idx = Some(tile.index);
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_MOTION {
                                                                        match tetherly_core::decompress_tile_motion(
                                                                            &tile.data,
                                                                            tile.x,
                                                                            tile.y,
                                                                            tile.w,
                                                                            tile.h,
                                                                            canvas,
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile motion decompress failed");
                                                                                failed_tile_idx = Some(tile.index);
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else {
                                                                        match node.shared.screen_codec.decompress_frame(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile patch decompress failed");
                                                                                failed_tile_idx = Some(tile.index);
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    }
                                                                };
                                                                let raw_tile = tetherly_core::DirtyTile {
                                                                    data: raw_data,
                                                                    ..tile
                                                                };
                                                                let res = if let Some(ref mut bg) = back_opt {
                                                                    bg.as_mut().unwrap().apply_dirty_tile(&raw_tile)
                                                                } else {
                                                                    canvas.apply_dirty_tile(&raw_tile)
                                                                };
                                                                if let Err(e) = res {
                                                                    tracing::warn!(%e, "apply_dirty_tile failed; requesting keyframe recovery");
                                                                    failed_tile_idx = Some(tile.index);
                                                                    apply_ok = false;
                                                                    break;
                                                                }
                                                            }
                                                            if apply_ok {
                                                                node.shared.screen_tile_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                node.shared.screen_tile_dirty_tiles_sent.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
                                                                let skipped = (total_tiles as u64).saturating_sub(count);
                                                                node.shared.screen_tile_skipped_count.fetch_add(skipped, std::sync::atomic::Ordering::Relaxed);
                                                                if let Some(mut bg) = back_opt {
                                                                    let t0 = std::time::Instant::now();
                                                                    if present_cfg.tear_prevention && count > 1 {
                                                                        node.shared.screen_tile_present_torn_prevented.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    }
                                                                    node.shared.screen_tile_present_committed_txs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    node.shared.screen_tile_present_swapped_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    let swap_lat = t0.elapsed().as_micros().max(1) as u64;
                                                                    node.shared.screen_tile_present_total_latency_us.fetch_add(swap_lat, std::sync::atomic::Ordering::Relaxed);
                                                                    if let Some(back) = bg.as_mut() {
                                                                        back.seq = present_frame.seq;
                                                                        canvas.bytes.copy_from_slice(&back.bytes);
                                                                    }
                                                                }
                                                                canvas.seq = present_frame.seq;
                                                                let updated = canvas.clone();
                                                                let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                                if svc_enabled {
                                                                    *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                                }
                                                                (false, Some(updated), None)
                                                            } else {
                                                                let nack_cfg = *node.shared.screen_tile_nack_config.read().unwrap();
                                                                if nack_cfg.enabled {
                                                                    let tidx = failed_tile_idx.unwrap_or(0);
                                                                    node.shared.screen_tile_nack_requests_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    node.shared.screen_tile_nack_degraded_commits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    if let Some(mut bg) = back_opt {
                                                                        if let Some(back) = bg.as_mut() {
                                                                            back.seq = present_frame.seq;
                                                                            canvas.bytes.copy_from_slice(&back.bytes);
                                                                        }
                                                                    }
                                                                    canvas.seq = present_frame.seq;
                                                                    let updated = canvas.clone();
                                                                    let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                                    if svc_enabled {
                                                                        *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                                    }
                                                                    (false, Some(updated), Some((present_frame.seq as u32, tidx)))
                                                                } else {
                                                                    (true, None, None)
                                                                }
                                                            }
                                                        }
                                                        None => {
                                                            tracing::warn!("received tile grid frame before base keyframe; requesting keyframe recovery");
                                                            (true, None, None)
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(%e, "decode_tile_grid failed; requesting keyframe recovery");
                                                    (true, None, None)
                                                }
                                            }
                                        };
                                        if let Some((s, tidx)) = nack_opt {
                                            let nack = ControlMsg::TileNackRequest {
                                                seq: s,
                                                tile_index: tidx,
                                                retry_count: 1,
                                            };
                                            if let Ok(f) = nack.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                        }
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_BATCH {
                                        let (need_recovery, frame_opt) = {
                                            match present_frame.decode_tile_batch() {
                                                Ok((batch_id, chunk_index, total_chunks, _tile_size, _total_tiles, dirty_tiles)) => {
                                                    let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                                    match canvas_guard.as_mut() {
                                                        Some(canvas) => {
                                                            let bpp = canvas.format.bpp();
                                                            let expected_len = (canvas.rect.w as usize) * (canvas.rect.h as usize) * bpp;
                                                            let mut bg = node.shared.screen_receiver_back_buffer.write().unwrap();
                                                            if bg.is_none()
                                                                || bg.as_ref().map(|b| b.rect != canvas.rect || b.bytes.len() != expected_len).unwrap_or(true)
                                                            {
                                                                *bg = Some(canvas.clone());
                                                            }

                                                            let mut apply_ok = true;
                                                            for tile in dirty_tiles {
                                                                let raw_len = (tile.w as usize) * (tile.h as usize) * bpp;
                                                                let raw_data = if tile.data.len() == raw_len {
                                                                    tile.data
                                                                } else {
                                                                    let subcodec = if !tile.data.is_empty() { tile.data[0] } else { 0 };
                                                                    if subcodec == tetherly_core::TILE_SUBCODEC_SOLID
                                                                        || subcodec == tetherly_core::TILE_SUBCODEC_BILEVEL
                                                                        || subcodec == tetherly_core::TILE_SUBCODEC_PALETTE16
                                                                    {
                                                                        match tetherly_core::decompress_tile_palette(subcodec, &tile.data, tile.w, tile.h, raw_len) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(_) => {
                                                                                match node.shared.screen_codec.decompress_frame(
                                                                                    &tile.data,
                                                                                    tile.w,
                                                                                    tile.h,
                                                                                    raw_len,
                                                                                ) {
                                                                                    Ok(decomp) => decomp,
                                                                                    Err(e) => {
                                                                                        tracing::warn!(%e, "tile batch patch decompress failed");
                                                                                        apply_ok = false;
                                                                                        break;
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD {
                                                                        match tetherly_core::decompress_tile_predictive(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                            node.shared.screen_codec.as_ref(),
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile batch predictive decompress failed");
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_LOD {
                                                                        match tetherly_core::decompress_tile_lod(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                            node.shared.screen_codec.as_ref(),
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile batch lod decompress failed");
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else if subcodec == tetherly_core::TILE_SUBCODEC_MOTION {
                                                                        match tetherly_core::decompress_tile_motion(
                                                                            &tile.data,
                                                                            tile.x,
                                                                            tile.y,
                                                                            tile.w,
                                                                            tile.h,
                                                                            canvas,
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile batch motion decompress failed");
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    } else {
                                                                        match node.shared.screen_codec.decompress_frame(
                                                                            &tile.data,
                                                                            tile.w,
                                                                            tile.h,
                                                                            raw_len,
                                                                        ) {
                                                                            Ok(decomp) => decomp,
                                                                            Err(e) => {
                                                                                tracing::warn!(%e, "tile batch decompress failed");
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        }
                                                                    }
                                                                };
                                                                let raw_tile = tetherly_core::DirtyTile {
                                                                    data: raw_data,
                                                                    ..tile
                                                                };
                                                                if let Err(e) = bg.as_mut().unwrap().apply_dirty_tile(&raw_tile) {
                                                                    tracing::warn!(%e, "batch apply_dirty_tile failed");
                                                                    apply_ok = false;
                                                                    break;
                                                                }
                                                            }

                                                            if !apply_ok {
                                                                (true, None)
                                                            } else {
                                                                let mut rx_state_guard = node.shared.screen_tile_batch_receiver_state.lock().unwrap();
                                                                let state = rx_state_guard.get_or_insert_with(|| TileBatchReceiverState {
                                                                    batch_id,
                                                                    total_chunks,
                                                                    chunks_received: std::collections::BTreeSet::new(),
                                                                });
                                                                if state.batch_id != batch_id {
                                                                    *state = TileBatchReceiverState {
                                                                        batch_id,
                                                                        total_chunks,
                                                                        chunks_received: std::collections::BTreeSet::new(),
                                                                    };
                                                                }
                                                                state.chunks_received.insert(chunk_index);
                                                                let all_done = state.chunks_received.len() >= (total_chunks as usize);
                                                                if all_done {
                                                                    *rx_state_guard = None;
                                                                    if let Some(back) = bg.as_mut() {
                                                                        back.seq = present_frame.seq;
                                                                        canvas.bytes.copy_from_slice(&back.bytes);
                                                                    }
                                                                    canvas.seq = present_frame.seq;
                                                                    let updated = canvas.clone();
                                                                    let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                                    if svc_enabled {
                                                                        *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                                    }
                                                                    (false, Some(updated))
                                                                } else {
                                                                    (false, None)
                                                                }
                                                            }
                                                        }
                                                        None => {
                                                            tracing::warn!("received tile batch chunk before base keyframe; requesting keyframe recovery");
                                                            (true, None)
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(%e, "decode_tile_batch failed; requesting keyframe recovery");
                                                    (true, None)
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        match frame_opt {
                                            Some(frame) => frame,
                                            None => continue,
                                        }
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_CACHE {
                                        let (need_recovery, frame_opt) = {
                                            match present_frame.decode_tile_cache() {
                                                Ok((_tile_size, _total_tiles, entries)) => {
                                                    let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                                    match canvas_guard.as_mut() {
                                                        Some(canvas) => {
                                                            let mut apply_ok = true;
                                                            let present_cfg = *node.shared.screen_tile_present_config.read().unwrap();
                                                            let mut back_opt = if present_cfg.enabled {
                                                                let bpp = canvas.format.bpp();
                                                                let expected_len = (canvas.rect.w as usize) * (canvas.rect.h as usize) * bpp;
                                                                let mut bg = node.shared.screen_receiver_back_buffer.write().unwrap();
                                                                if bg.is_none()
                                                                    || bg.as_ref().map(|b| b.rect != canvas.rect || b.bytes.len() != expected_len).unwrap_or(true)
                                                                {
                                                                    *bg = Some(canvas.clone());
                                                                }
                                                                Some(bg)
                                                            } else {
                                                                None
                                                            };
                                                            let mut rx_guard = node.shared.screen_tile_cache_receiver.lock().unwrap();
                                                            for entry in &entries {
                                                                if entry.cache_hit {
                                                                    let id = entry.cache_id as usize;
                                                                    if id < rx_guard.len() {
                                                                        if let Some(cached_data) = &rx_guard[id] {
                                                                            let res = if let Some(ref mut bg) = back_opt {
                                                                                bg.as_mut().unwrap().apply_cached_tile(entry.x, entry.y, entry.w, entry.h, cached_data)
                                                                            } else {
                                                                                canvas.apply_cached_tile(entry.x, entry.y, entry.w, entry.h, cached_data)
                                                                            };
                                                                            if let Err(e) = res {
                                                                                tracing::warn!(%e, "apply_cached_tile hit failed; requesting keyframe recovery");
                                                                                apply_ok = false;
                                                                                break;
                                                                            }
                                                                        } else {
                                                                            tracing::warn!(cache_id = entry.cache_id, "tile cache hit missing in receiver dictionary; requesting keyframe recovery");
                                                                            apply_ok = false;
                                                                            break;
                                                                        }
                                                                    } else {
                                                                        tracing::warn!(cache_id = entry.cache_id, "tile cache id exceeds dictionary size; requesting keyframe recovery");
                                                                        apply_ok = false;
                                                                        break;
                                                                    }
                                                                } else {
                                                                    let bpp = canvas.format.bpp();
                                                                    let raw_len = (entry.w as usize) * (entry.h as usize) * bpp;
                                                                    let raw_data = if entry.data.len() == raw_len {
                                                                        entry.data.clone()
                                                                    } else {
                                                                        let subcodec = if !entry.data.is_empty() { entry.data[0] } else { 0 };
                                                                        if subcodec == tetherly_core::TILE_SUBCODEC_SOLID
                                                                            || subcodec == tetherly_core::TILE_SUBCODEC_BILEVEL
                                                                            || subcodec == tetherly_core::TILE_SUBCODEC_PALETTE16
                                                                        {
                                                                            match tetherly_core::decompress_tile_palette(subcodec, &entry.data, entry.w, entry.h, raw_len) {
                                                                                Ok(decomp) => decomp,
                                                                                Err(_) => {
                                                                                    match node.shared.screen_codec.decompress_frame(
                                                                                        &entry.data,
                                                                                        entry.w,
                                                                                        entry.h,
                                                                                        raw_len,
                                                                                    ) {
                                                                                        Ok(decomp) => decomp,
                                                                                        Err(e) => {
                                                                                            tracing::warn!(%e, "tile cache patch decompress failed");
                                                                                            apply_ok = false;
                                                                                            break;
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        } else if subcodec == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD {
                                                                            match tetherly_core::decompress_tile_predictive(
                                                                                &entry.data,
                                                                                entry.w,
                                                                                entry.h,
                                                                                raw_len,
                                                                                node.shared.screen_codec.as_ref(),
                                                                            ) {
                                                                                Ok(decomp) => decomp,
                                                                                Err(e) => {
                                                                                    tracing::warn!(%e, "tile cache predictive decompress failed");
                                                                                    apply_ok = false;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        } else if subcodec == tetherly_core::TILE_SUBCODEC_LOD {
                                                                            match tetherly_core::decompress_tile_lod(
                                                                                &entry.data,
                                                                                entry.w,
                                                                                entry.h,
                                                                                raw_len,
                                                                                node.shared.screen_codec.as_ref(),
                                                                            ) {
                                                                                Ok(decomp) => decomp,
                                                                                Err(e) => {
                                                                                    tracing::warn!(%e, "tile cache lod decompress failed");
                                                                                    apply_ok = false;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        } else if subcodec == tetherly_core::TILE_SUBCODEC_MOTION {
                                                                            match tetherly_core::decompress_tile_motion(
                                                                                &entry.data,
                                                                                entry.x,
                                                                                entry.y,
                                                                                entry.w,
                                                                                entry.h,
                                                                                canvas,
                                                                            ) {
                                                                                Ok(decomp) => decomp,
                                                                                Err(e) => {
                                                                                    tracing::warn!(%e, "tile cache motion decompress failed");
                                                                                    apply_ok = false;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        } else {
                                                                            match node.shared.screen_codec.decompress_frame(
                                                                                &entry.data,
                                                                                entry.w,
                                                                                entry.h,
                                                                                raw_len,
                                                                            ) {
                                                                                Ok(decomp) => decomp,
                                                                                Err(e) => {
                                                                                    tracing::warn!(%e, "tile cache patch decompress failed");
                                                                                    apply_ok = false;
                                                                                    break;
                                                                                }
                                                                            }
                                                                        }
                                                                    };
                                                                    let id = entry.cache_id as usize;
                                                                    if id >= rx_guard.len() {
                                                                        rx_guard.resize(id + 1, None);
                                                                    }
                                                                    rx_guard[id] = Some(raw_data.clone());
                                                                    let res = if let Some(ref mut bg) = back_opt {
                                                                        bg.as_mut().unwrap().apply_cached_tile(entry.x, entry.y, entry.w, entry.h, &raw_data)
                                                                    } else {
                                                                        canvas.apply_cached_tile(entry.x, entry.y, entry.w, entry.h, &raw_data)
                                                                    };
                                                                    if let Err(e) = res {
                                                                        tracing::warn!(%e, "apply_cached_tile miss failed; requesting keyframe recovery");
                                                                        apply_ok = false;
                                                                        break;
                                                                    }
                                                                }
                                                            }
                                                            drop(rx_guard);
                                                            if apply_ok {
                                                                if let Some(mut bg) = back_opt {
                                                                    let t0 = std::time::Instant::now();
                                                                    if present_cfg.tear_prevention && entries.len() > 1 {
                                                                        node.shared.screen_tile_present_torn_prevented.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    }
                                                                    node.shared.screen_tile_present_committed_txs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    node.shared.screen_tile_present_swapped_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                    let swap_lat = t0.elapsed().as_micros().max(1) as u64;
                                                                    node.shared.screen_tile_present_total_latency_us.fetch_add(swap_lat, std::sync::atomic::Ordering::Relaxed);
                                                                    if let Some(back) = bg.as_mut() {
                                                                        back.seq = present_frame.seq;
                                                                        canvas.bytes.copy_from_slice(&back.bytes);
                                                                    }
                                                                }
                                                                canvas.seq = present_frame.seq;
                                                                let updated = canvas.clone();
                                                                let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                                if svc_enabled {
                                                                    *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                                }
                                                                (false, Some(updated))
                                                            } else {
                                                                (true, None)
                                                            }
                                                        }
                                                        None => {
                                                            tracing::warn!("received tile cache frame before base keyframe; requesting keyframe recovery");
                                                            (true, None)
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(%e, "decode_tile_cache failed; requesting keyframe recovery");
                                                    (true, None)
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_MOTION_SCROLL {
                                        let (need_recovery, frame_opt) = {
                                            match present_frame.decode_motion_scroll() {
                                                Ok((dx, dy, exposed_rect, compressed_patch)) => {
                                                    let patch_raw_len = (exposed_rect.width as usize) * (exposed_rect.height as usize) * 4;
                                                    let patch_res = node.shared.screen_codec.decompress_frame(
                                                        compressed_patch,
                                                        exposed_rect.width,
                                                        exposed_rect.height,
                                                        patch_raw_len,
                                                    );
                                                    match patch_res {
                                                        Ok(raw_patch_bytes) => {
                                                            let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                                            match canvas_guard.as_mut() {
                                                                Some(canvas) => {
                                                                    canvas.scroll_canvas(dx, dy);
                                                                    if let Ok(patch_rect) = tetherly_core::ScreenRect::new(
                                                                        exposed_rect.x as i32,
                                                                        exposed_rect.y as i32,
                                                                        exposed_rect.width,
                                                                        exposed_rect.height,
                                                                    ) {
                                                                        if let Ok(patch_frame) = ScreenFrame::raw(
                                                                            present_frame.seq,
                                                                            patch_rect,
                                                                            canvas.format,
                                                                            raw_patch_bytes,
                                                                        ) {
                                                                            if let Ok(()) = canvas.apply_sub_rect(&patch_frame) {
                                                                                node.shared.screen_motion_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                                canvas.seq = present_frame.seq;
                                                                                let updated = canvas.clone();
                                                                                let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                                                if svc_enabled {
                                                                                    *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                                                }
                                                                                (false, Some(updated))
                                                                            } else {
                                                                                tracing::warn!("motion scroll patch apply failed; requesting keyframe recovery");
                                                                                (true, None)
                                                                            }
                                                                        } else {
                                                                            tracing::warn!("failed to construct motion patch frame");
                                                                            (true, None)
                                                                        }
                                                                    } else {
                                                                        tracing::warn!("invalid motion exposed rect");
                                                                        (true, None)
                                                                    }
                                                                }
                                                                None => {
                                                                    tracing::warn!("received motion scroll frame before base keyframe; requesting keyframe recovery");
                                                                    (true, None)
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            tracing::warn!(%e, "motion scroll patch decompress failed");
                                                            (true, None)
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(%e, "decode_motion_scroll failed; requesting keyframe recovery");
                                                    (true, None)
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_LTR {
                                        let (need_recovery, frame_opt) = {
                                            let mut ltr_guard = node.shared.screen_receiver_ltr_canvas.write().unwrap();
                                            match ltr_guard.as_mut() {
                                                Some(golden) => match golden.apply_sub_rect(&present_frame) {
                                                    Ok(()) => {
                                                        node.shared.screen_ltr_recoveries_completed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                        golden.seq = present_frame.seq;
                                                        let updated = golden.clone();
                                                        *node.shared.screen_receiver_canvas.write().unwrap() = Some(updated.clone());
                                                        (false, Some(updated))
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(%e, "LTR recovery patch blit failed; requesting keyframe recovery");
                                                        (true, None)
                                                    }
                                                },
                                                None => {
                                                    tracing::warn!("received LTR recovery frame before golden reference anchor; requesting keyframe recovery");
                                                    (true, None)
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_SVC_T1 {
                                        let (need_recovery, frame_opt) = {
                                            let t0_guard = node.shared.screen_receiver_t0_canvas.read().unwrap();
                                            match t0_guard.as_ref() {
                                                Some(t0_base) => {
                                                    let mut display_canvas = t0_base.clone();
                                                    match display_canvas.apply_sub_rect(&present_frame) {
                                                        Ok(()) => {
                                                            node.shared.screen_svc_t1_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                            display_canvas.seq = present_frame.seq;
                                                            *node.shared.screen_receiver_canvas.write().unwrap() = Some(display_canvas.clone());
                                                            (false, Some(display_canvas))
                                                        }
                                                        Err(e) => {
                                                            tracing::warn!(%e, "SVC T1 patch blit failed; requesting keyframe recovery");
                                                            (true, None)
                                                        }
                                                    }
                                                }
                                                None => {
                                                    let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                                    match canvas_guard.as_mut() {
                                                        Some(canvas) => match canvas.apply_sub_rect(&present_frame) {
                                                            Ok(()) => {
                                                                node.shared.screen_svc_t1_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                canvas.seq = present_frame.seq;
                                                                (false, Some(canvas.clone()))
                                                            }
                                                            Err(e) => {
                                                                tracing::warn!(%e, "SVC T1 fallback blit failed; requesting keyframe recovery");
                                                                (true, None)
                                                            }
                                                        },
                                                        None => {
                                                            tracing::warn!("received SVC T1 frame before base keyframe; requesting keyframe recovery");
                                                            (true, None)
                                                        }
                                                    }
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else if codec_tag == tetherly_core::SCREEN_COMPRESS_DELTA {
                                        let (need_recovery, frame_opt) = {
                                            let mut canvas_guard = node.shared.screen_receiver_canvas.write().unwrap();
                                            match canvas_guard.as_mut() {
                                                Some(canvas) => match canvas.apply_sub_rect(&present_frame) {
                                                    Ok(()) => {
                                                        node.shared.screen_delta_frames_received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                        canvas.seq = present_frame.seq;
                                                        let updated = canvas.clone();
                                                        let svc_enabled = node.shared.screen_svc_config.read().unwrap().is_enabled();
                                                        if svc_enabled {
                                                            *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(updated.clone());
                                                            node.shared.screen_svc_t0_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                        }
                                                        (false, Some(updated))
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(%e, "delta patch blit failed; requesting keyframe recovery");
                                                        (true, None)
                                                    }
                                                },
                                                None => {
                                                    tracing::warn!("received delta frame before base keyframe; requesting keyframe recovery");
                                                    (true, None)
                                                }
                                            }
                                        };
                                        if need_recovery {
                                            let req = ControlMsg::KeyframeRequest;
                                            if let Ok(f) = req.to_frame() {
                                                let r_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_screen_tx(r_len);
                                                }
                                            }
                                            continue;
                                        }
                                        frame_opt.unwrap()
                                    } else {
                                        *node.shared.screen_receiver_canvas.write().unwrap() = Some(present_frame.clone());
                                        *node.shared.screen_receiver_back_buffer.write().unwrap() = Some(present_frame.clone());
                                        node.shared.screen_delta_keyframes_received.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        let svc_cfg = *node.shared.screen_svc_config.read().unwrap();
                                        if svc_cfg.is_enabled() {
                                            *node.shared.screen_receiver_t0_canvas.write().unwrap() = Some(present_frame.clone());
                                            node.shared.screen_svc_t0_frames_sent.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        let ltr_cfg = *node.shared.screen_ltr_config.read().unwrap();
                                        if ltr_cfg.enabled {
                                            let mut ltr_guard = node.shared.screen_receiver_ltr_canvas.write().unwrap();
                                            if ltr_guard.is_none() {
                                                *ltr_guard = Some(present_frame.clone());
                                                node.shared.screen_ltr_current_seq.store(present_frame.seq, std::sync::atomic::Ordering::Relaxed);
                                                node.shared.screen_ltr_anchors_established.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            }
                                        }
                                        present_frame
                                    };
                                    // ADR-110 / v2.24: Objective Visual Quality Metric sampling & distortion profiling
                                    node.shared.record_quality_metric(&present_frame);
                                    let is_gap = receiver.last_seq() != 0
                                        && present_frame.seq > receiver.last_seq() + 1;
                                    match receiver.apply(&present_frame) {
                                        Ok(acked) => {
                                            // The default sink is the
                                            // observer itself (shared
                                            // inner), so presenting once
                                            // updates the CI snapshot too.
                                            *node
                                                .shared
                                                .screen_rx_stats
                                                .lock()
                                                .expect("screen rx") =
                                                receiver.stats();
                                            *node
                                                .shared
                                                .screen_latency_stats
                                                .lock()
                                                .expect("latency stats") =
                                                receiver.latency_report();
                                            if is_gap
                                                && codec_tag
                                                    == tetherly_core::SCREEN_COMPRESS_H264
                                            {
                                                let req = ControlMsg::KeyframeRequest;
                                                if let Ok(f) = req.to_frame() {
                                                    let r_len = (f.payload.len() + 12) as u64;
                                                    if sess.send_bounded(f).await.is_ok() {
                                                        node.shared.telemetry.record_screen_tx(r_len);
                                                    }
                                                }
                                            }
                                            if let Some(seq) = acked {
                                                let arrival_us = node.shared.clock.unix_us();
                                                receiver.push_playout(present_frame, arrival_us);
                                                while let ScreenPlayoutAction::Play(ready_frame) = receiver.pop_playout(arrival_us) {
                                                    let mut presented_frame = ready_frame;
                                                    {
                                                        let wm_cfg = node.shared.screen_watermark.read().unwrap();
                                                        if wm_cfg.enabled {
                                                            let peer_id_str = node
                                                                .shared
                                                                .screen_peer
                                                                .lock()
                                                                .ok()
                                                                .and_then(|g| g.clone())
                                                                .map(|p| p.to_string())
                                                                .unwrap_or_else(|| "peer".to_string());
                                                            let time_str = format!("{}", arrival_us / 1_000_000);
                                                            if let Ok(applied) = presented_frame.apply_watermark(&wm_cfg, &peer_id_str, &time_str) {
                                                                if applied {
                                                                    node.shared.screen_watermark_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    {
                                                        let now_ms = arrival_us / 1_000;
                                                        let annotations = node.shared.screen_annotations.read().unwrap();
                                                        if !annotations.is_empty() {
                                                            let _ = presented_frame.apply_annotations(&annotations, now_ms);
                                                        }
                                                        if let Some(laser) = node.shared.screen_laser_pointer.read().unwrap().as_ref() {
                                                            let _ = presented_frame.apply_laser_pointer(laser);
                                                        }
                                                    }
                                                    let _ = node
                                                        .shared
                                                        .screen_sink
                                                        .present(&presented_frame);
                                                    *node.shared.screen_last_frame.lock().expect("last_frame") = Some(presented_frame.clone());
                                                }
                                                *node
                                                    .shared
                                                    .screen_playout_stats
                                                    .lock()
                                                    .expect("playout stats") =
                                                    receiver.playout_stats();

                                                let ack = ControlMsg::Ack { seq };
                                                if let Ok(f) = ack.to_frame() {
                                                    let a_len = (f.payload.len() + 12) as u64;
                                                    if sess.send_bounded(f).await.is_ok() {
                                                        node.shared.telemetry.record_screen_tx(a_len);
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(%e, "screen receiver rejected frame; closing viewer");
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "screen session recv ended; closing viewer");
                                break;
                            }
                        }
                    }
                }
            }
            receiver.on_peer_gone();
            *node
                .shared
                .screen_playout_stats
                .lock()
                .expect("playout stats") = receiver.playout_stats();
            *node
                .shared
                .screen_cursor_stats
                .lock()
                .expect("cursor stats") = receiver.cursor_stats();
            *node.shared.screen_rx_stats.lock().expect("screen rx") = receiver.stats();
            *node.shared.screen_sync_stats.lock().expect("sync stats") = receiver.sync_stats();
            // Only clear the registration triple when this loop is still the
            // newest client channel. A replaced loop's late exit must not wipe
            // the fields its successor just wrote: `screen_ctl=None` breaks
            // every control send of the new session, and `screen_peer=None`
            // defeats the cross-session channel-reuse check in
            // `remote_session_start`.
            let still_newest_client = node
                .shared
                .screen_client_gen
                .load(std::sync::atomic::Ordering::SeqCst)
                == my_client_gen;
            if still_newest_client {
                *node.shared.screen_peer.lock().expect("screen_peer") = None;
                *node.shared.screen_ctl.lock().await = None;
                *node.shared.screen_client_out.lock().await = None;
            }
            node.shared
                .screen_privacy_enabled
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_privacy_lock_input
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_bitrate_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_fps_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_scale_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_profile_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_peer_watermark_enforced
                .store(false, std::sync::atomic::Ordering::Relaxed);
            if still_newest_client {
                node.emit(UiEvent::screen_state_changed("idle", false, None));
            }
        });
    }

    /// Grab one frame from the source and push it toward the visitor. Returns
    /// `true` only when a frame was queued (i.e. while streaming). Deterministic
    /// in CI via `MemoryScreenSource`; real capture is Manual-required.
    pub async fn screen_send_frame(&self) -> Result<bool, CoreError> {
        // Grab is the expensive step (DXGI map, adb pipe read): skip it
        // entirely unless a visitor is actually streaming.
        if !self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .state()
            .is_streaming()
        {
            return Ok(false);
        }
        // ADR-083 / v1.97: Frame pacing check. Skip capture if pacing interval has not elapsed.
        let now_us = self.shared.clock.unix_us();
        if !self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_pacing_ready(now_us)
        {
            return Ok(false);
        }
        // ADR-078 / v1.92: the first frame of a stream picks the H.264 tier
        // from the viewer's measured bandwidth profile (the manual flag
        // wins when `screen_bitrate_auto` is off).
        if self.shared.screen_bitrate_auto
            && !self
                .shared
                .screen_bitrate_applied
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            if let Some(sel) = &self.shared.screen_encode_codec {
                let mbps = {
                    let peer = self.shared.screen_peer.lock().expect("screen_peer").clone();
                    peer.and_then(|p| {
                        self.shared
                            .bandwidth_profiles
                            .lock()
                            .expect("bandwidth_profiles")
                            .get(&p.to_string())
                            .map(|pr| pr.measured_mbps)
                    })
                };
                sel.set_bitrate_bps(crate::screen::screen_bitrate_from_profile(mbps));
            }
        }
        // ADR-083 / v1.97: the first frame of a stream also picks the target FPS tier
        // from the viewer's measured bandwidth profile when `screen_fps_auto` is on.
        if self.shared.screen_fps_auto
            && !self
                .shared
                .screen_fps_applied
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let mbps = {
                let peer = self.shared.screen_peer.lock().expect("screen_peer").clone();
                peer.and_then(|p| {
                    self.shared
                        .bandwidth_profiles
                        .lock()
                        .expect("bandwidth_profiles")
                        .get(&p.to_string())
                        .map(|pr| pr.measured_mbps)
                })
            };
            self.screen_set_fps(crate::screen::screen_fps_from_profile(mbps));
        }
        // ADR-084 / v1.98: the first frame of a stream also picks the target resolution scale tier
        // from the viewer's measured bandwidth profile when `screen_scale_auto` is on.
        if self.shared.screen_scale_auto
            && !self
                .shared
                .screen_scale_applied
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let mbps = {
                let peer = self.shared.screen_peer.lock().expect("screen_peer").clone();
                peer.and_then(|p| {
                    self.shared
                        .bandwidth_profiles
                        .lock()
                        .expect("bandwidth_profiles")
                        .get(&p.to_string())
                        .map(|pr| pr.measured_mbps)
                })
            };
            self.screen_set_scale_pct(crate::screen::screen_scale_from_profile(mbps));
        }
        // ADR-085 / v1.99: the first frame of a stream also picks the content profile
        // from the viewer's measured bandwidth profile when `screen_profile_auto` is on.
        if self.shared.screen_profile_auto
            && !self
                .shared
                .screen_profile_applied
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            let mbps = {
                let peer = self.shared.screen_peer.lock().expect("screen_peer").clone();
                peer.and_then(|p| {
                    self.shared
                        .bandwidth_profiles
                        .lock()
                        .expect("bandwidth_profiles")
                        .get(&p.to_string())
                        .map(|pr| pr.measured_mbps)
                })
            };
            self.screen_set_profile(crate::screen::screen_profile_from_profile(mbps));
        }
        // Grab can block for a long time (adb pipe reads in particular), so
        // run it on the blocking pool with a hard timeout. On timeout, call
        // `interrupt` so a wedged device stream EOFs and unwinds instead of
        // stalling the pump forever.
        let source = Arc::clone(&self.shared.screen_source);
        let grab_task = source.clone();
        let grab = tokio::task::spawn_blocking(move || grab_task.grab());
        let frame = match tokio::time::timeout(SCREEN_GRAB_TIMEOUT, grab).await {
            Ok(Ok(inner)) => inner?,
            Ok(Err(join)) => {
                return Err(CoreError::ScreenRefused(format!(
                    "screen grab task: {join}"
                )));
            }
            Err(_) => {
                source.interrupt();
                return Err(CoreError::ScreenRefused("screen grab timed out".into()));
            }
        };
        let Some(frame) = frame else {
            return Ok(false);
        };
        let full_w = frame.rect.w;
        let full_h = frame.rect.h;
        *self.shared.screen_last_frame.lock().expect("last_frame") = Some(frame.clone());

        // ADR-107 / v2.21: Dynamic Region-of-Interest (ROI) viewport cropping
        let mut frame = {
            let mut roi = *self.shared.screen_roi.read().unwrap();
            if roi.active {
                roi.sanitize(full_w, full_h);
                frame.crop_roi(&roi)?
            } else {
                frame
            }
        };

        // ADR-108 / v2.22: Adaptive Color Depth & Grayscale Economy Pipeline
        {
            let color_cfg = *self.shared.screen_color_mode.read().unwrap();
            if color_cfg.is_active() {
                frame.apply_color_mode(&color_cfg)?;
            }
        }

        // ADR-110 / v2.24: Objective Visual Quality Metric sampling & distortion profiling
        // and ADR-111 / v2.25: Autonomous Closed-Loop QoE Adaptation
        self.shared.record_quality_metric(&frame);

        // ADR-113 / v2.27: Long-Term Reference (LTR) Frame Anchoring
        let ltr_cfg = *self.shared.screen_ltr_config.read().unwrap();
        let ltr_sync_seq = if ltr_cfg.enabled && !frame.compressed() {
            let mut sender_ltr = self.shared.screen_sender_ltr_canvas.write().unwrap();
            let should_anchor = sender_ltr.is_none()
                || (ltr_cfg.auto_interval > 0
                    && frame.seq > 0
                    && frame.seq % (ltr_cfg.auto_interval as u64) == 0);
            if should_anchor {
                *sender_ltr = Some(frame.clone());
                self.shared
                    .screen_ltr_current_seq
                    .store(frame.seq, std::sync::atomic::Ordering::Relaxed);
                self.shared
                    .screen_ltr_anchors_established
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Some(frame.seq)
            } else {
                None
            }
        } else {
            None
        };

        // ADR-114 / v2.28: Screen Streaming 2-Layer Temporal Scalability (SVC-T) Graceful Degradation
        let svc_cfg = *self.shared.screen_svc_config.read().unwrap();
        if svc_cfg.is_enabled() && (frame.seq % 2 != 0) && svc_cfg.should_drop_t1() {
            self.shared
                .screen_svc_t1_frames_dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let raw_bytes =
                (frame.rect.w as u64) * (frame.rect.h as u64) * (frame.format.bpp() as u64);
            self.shared
                .screen_svc_saved_bytes
                .fetch_add(raw_bytes, std::sync::atomic::Ordering::Relaxed);
            return Ok(false);
        }

        // ADR-123 / v2.37: When tile saliency tiering is enabled, per-tile priority scheduling and
        // background dropping protects interactive critical tiles without dropping whole frames.
        let saliency_cfg = *self.shared.screen_tile_saliency_config.read().unwrap();
        // ADR-124 / v2.38: Dynamic multi-resolution Level-of-Detail downsampling under congestion.
        let lod_cfg = *self.shared.screen_tile_lod_config.read().unwrap();

        let outbound = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let now_us = self.shared.clock.unix_us();
            // Congestion control / adaptive backpressure (ADR-080 / ADR-082):
            // When link is congested (in_flight >= 4 or RTT >= 250ms), drop non-initial frames
            // to eliminate bufferbloat and socket stall while maintaining presentation continuity.
            // On drop, AIMD multiplicative decrease cuts bitrate by 20% and arms an immediate IDR.
            if engine.is_congested()
                && engine.stats().sent > 0
                && !saliency_cfg.enabled
                && !lod_cfg.enabled
            {
                if svc_cfg.is_enabled() && (frame.seq % 2 != 0) {
                    self.shared
                        .screen_svc_t1_frames_dropped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let raw_bytes =
                        (frame.rect.w as u64) * (frame.rect.h as u64) * (frame.format.bpp() as u64);
                    self.shared
                        .screen_svc_saved_bytes
                        .fetch_add(raw_bytes, std::sync::atomic::Ordering::Relaxed);
                    return Ok(false);
                }
                engine.record_drop_with_time(now_us);
                engine.consume_pacing(now_us);
                let new_rate = engine.bitrate_bps();
                let new_fps = engine.fps_target();
                let new_scale = engine.scale_pct();
                self.shared.screen_source.force_idr();
                if let Some(sel) = &self.shared.screen_encode_codec {
                    sel.force_idr();
                    sel.set_bitrate_bps(new_rate);
                    sel.set_fps(new_fps);
                    sel.set_scale_pct(new_scale);
                }
                self.shared.screen_source.set_bitrate_bps(new_rate);
                self.shared.screen_source.set_fps(new_fps);
                self.shared.screen_source.set_scale_pct(new_scale);
                warn!(
                    in_flight = engine.in_flight_count(),
                    rtt_latest = ?engine.stats().rtt_latest_us,
                    bitrate_bps = new_rate,
                    fps_target = new_fps,
                    scale_pct = new_scale,
                    health = %engine.network_health().as_str(),
                    "screen channel congested; dropping non-initial capture frame; flagged IDR and adapted 4D matrix"
                );
                return Ok(false);
            }

            // ADR-089 Screen Damage Detection & Adaptive Squelch:
            let (is_dirty, dirty_ratio, bounding_rect) = {
                let mut prev_guard = self.shared.screen_prev_frame.lock().expect("prev frame");
                let mut tracker = self
                    .shared
                    .screen_damage_tracker
                    .lock()
                    .expect("damage tracker");
                let (width, height) = (frame.rect.w as u32, frame.rect.h as u32);
                let report = match &*prev_guard {
                    None => {
                        *prev_guard = Some(frame.bytes.clone());
                        tracker.evaluate_hint(true, width, height)
                    }
                    Some(prev_bytes) => {
                        let is_diff = prev_bytes != &frame.bytes;
                        if !is_diff {
                            tracker.evaluate_hint(false, width, height)
                        } else if !frame.compressed() {
                            let r = tracker.evaluate_rgba(prev_bytes, &frame.bytes, width, height);
                            *prev_guard = Some(frame.bytes.clone());
                            r
                        } else {
                            *prev_guard = Some(frame.bytes.clone());
                            tracker.evaluate_hint(true, width, height)
                        }
                    }
                };
                (!report.is_static, report.dirty_ratio, report.bounding_rect)
            };

            // ADR-112 / v2.26: Screen Streaming Progressive Intra-Refresh (PIR) Anti-Burst Pipeline
            let pir_cfg = *self.shared.screen_pir_config.read().unwrap();
            let is_pir_active = match pir_cfg.mode {
                tetherly_core::ScreenPirMode::Disabled => false,
                tetherly_core::ScreenPirMode::Continuous => true,
                tetherly_core::ScreenPirMode::OnDemand => self
                    .shared
                    .screen_pir_active_sweep
                    .load(std::sync::atomic::Ordering::Relaxed),
            };

            // ADR-113 / v2.27: Screen Streaming LTR Recovery Pipeline
            let is_ltr_recovery = ltr_cfg.enabled
                && self
                    .shared
                    .screen_ltr_recovery_pending
                    .swap(false, std::sync::atomic::Ordering::SeqCst);

            // ADR-115 / v2.29: 2D Motion Vector Estimation & Scroll Acceleration Pipeline
            let motion_cfg = *self.shared.screen_motion_config.read().unwrap();
            let motion_frame_opt = if motion_cfg.enabled && !frame.compressed() {
                let prev_opt = self
                    .shared
                    .screen_sender_motion_canvas
                    .read()
                    .unwrap()
                    .clone();
                let mut encoded_motion = None;
                if let Some(prev) = prev_opt {
                    if prev.rect.w == frame.rect.w
                        && prev.rect.h == frame.rect.h
                        && prev.format == frame.format
                    {
                        if let Some(mv) = tetherly_core::estimate_scroll_motion(
                            &prev,
                            &frame,
                            motion_cfg.max_displacement,
                            tetherly_core::SCREEN_MOTION_MIN_MATCH_RATIO,
                        ) {
                            if mv.dx != 0 || mv.dy != 0 {
                                if let Some(exposed_rect) = tetherly_core::compute_exposed_strip(
                                    frame.rect.w,
                                    frame.rect.h,
                                    mv.dx,
                                    mv.dy,
                                ) {
                                    if let Ok(patch) = frame.extract_sub_rect(
                                        exposed_rect.x,
                                        exposed_rect.y,
                                        exposed_rect.width,
                                        exposed_rect.height,
                                    ) {
                                        if let Ok(compressed) =
                                            self.shared.screen_codec.compress_frame(
                                                &patch.bytes,
                                                patch.rect.w,
                                                patch.rect.h,
                                            )
                                        {
                                            let full_raw_bytes = (frame.rect.w as u64)
                                                * (frame.rect.h as u64)
                                                * (frame.format.bpp() as u64);
                                            let encoded_bytes = (compressed.len() + 12) as u64;
                                            if full_raw_bytes > encoded_bytes {
                                                self.shared.screen_motion_saved_bytes.fetch_add(
                                                    full_raw_bytes - encoded_bytes,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }
                                            self.shared
                                                .screen_motion_frames_sent
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            *self
                                                .shared
                                                .screen_sender_motion_canvas
                                                .write()
                                                .unwrap() = Some(frame.clone());
                                            encoded_motion =
                                                Some(ScreenFrame::encode_motion_scroll(
                                                    frame.rect,
                                                    frame.format,
                                                    frame.seq,
                                                    mv.dx,
                                                    mv.dy,
                                                    exposed_rect,
                                                    &compressed,
                                                ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if encoded_motion.is_none() {
                    if is_dirty {
                        self.shared
                            .screen_motion_fallback_frames_sent
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    *self.shared.screen_sender_motion_canvas.write().unwrap() = Some(frame.clone());
                }
                encoded_motion
            } else {
                if !frame.compressed() {
                    *self.shared.screen_sender_motion_canvas.write().unwrap() = Some(frame.clone());
                }
                None
            };
            let is_motion = motion_frame_opt.is_some();

            // ADR-117 / v2.31 takes precedence over basic Tile-Grid when tile_cache_cfg.enabled
            let mut tile_resend_entry: Option<(u32, Vec<tetherly_core::DirtyTile>)> = None;
            let tile_cache_cfg = *self.shared.screen_tile_cache_config.read().unwrap();
            let tile_batch_cfg = *self.shared.screen_tile_batch_config.read().unwrap();
            let tile_motion_cfg = *self.shared.screen_tile_motion_config.read().unwrap();
            let tile_cfg = *self.shared.screen_tile_config.read().unwrap();
            let focus_cfg = *self.shared.screen_focus_config.read().unwrap();
            let palette_cfg = *self.shared.screen_tile_palette_config.read().unwrap();
            let predict_cfg = *self.shared.screen_tile_predict_config.read().unwrap();
            let is_congested = engine.is_congested();
            let cursor_pos = if focus_cfg.enabled {
                let stats = self
                    .shared
                    .screen_cursor_stats
                    .lock()
                    .ok()
                    .map(|s| (s.current_x, s.current_y));
                if let Some((cx, cy)) = stats {
                    if cx != 0 || cy != 0 {
                        Some((cx, cy))
                    } else {
                        self.shared.cursor_last_pos.lock().ok().map(|p| *p)
                    }
                } else {
                    self.shared.cursor_last_pos.lock().ok().map(|p| *p)
                }
            } else {
                None
            };
            let focus_box = if focus_cfg.enabled {
                match focus_cfg.mode {
                    tetherly_core::ScreenFocusMode::Box => Some((
                        focus_cfg.box_x,
                        focus_cfg.box_y,
                        focus_cfg.box_w,
                        focus_cfg.box_h,
                    )),
                    tetherly_core::ScreenFocusMode::Cursor => cursor_pos.map(|(cx, cy)| {
                        let r = focus_cfg.radius;
                        let x = cx.max(0) as u16;
                        let y = cy.max(0) as u16;
                        let bx = x.saturating_sub(r);
                        let by = y.saturating_sub(r);
                        let bw = r.saturating_mul(2);
                        let bh = r.saturating_mul(2);
                        (bx, by, bw, bh)
                    }),
                }
            } else {
                None
            };

            let (tile_cache_frame_opt, tile_frame_opt, tile_batch_chunks_opt) = if tile_cache_cfg
                .enabled
                && !frame.compressed()
                && !is_motion
            {
                let prev_opt = self
                    .shared
                    .screen_sender_tile_canvas
                    .read()
                    .unwrap()
                    .clone();
                let mut encoded_tile_cache = None;
                if let Some(prev) = &prev_opt {
                    if prev.rect.w == frame.rect.w
                        && prev.rect.h == frame.rect.h
                        && prev.format == frame.format
                    {
                        let tile_size = if tile_cfg.enabled {
                            tile_cfg.tile_size
                        } else {
                            tetherly_core::SCREEN_TILE_DEFAULT_SIZE
                        };
                        let (total_tiles, raw_dirty_tiles) =
                            tetherly_core::extract_dirty_tiles(prev, &frame, tile_size);
                        let (dirty_tiles, any_peripheral_skipped) = if focus_cfg.enabled {
                            let cadence = (focus_cfg.peripheral_skip_ratio as u64) + 1;
                            let send_peripheral = (frame.seq % cadence) == 0;
                            let mut kept = Vec::with_capacity(raw_dirty_tiles.len());
                            let mut skipped = false;
                            for tile in raw_dirty_tiles {
                                let in_focus = tetherly_core::is_tile_in_focus(
                                    tile.x, tile.y, tile.w, tile.h, &focus_cfg, cursor_pos,
                                );
                                if in_focus {
                                    self.shared
                                        .screen_focus_tiles_sent
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    kept.push(tile);
                                } else if send_peripheral {
                                    self.shared
                                        .screen_focus_peripheral_sent
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    kept.push(tile);
                                } else {
                                    self.shared
                                        .screen_focus_peripheral_skipped
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    self.shared.screen_focus_saved_bytes.fetch_add(
                                        tile.data.len() as u64,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                    skipped = true;
                                }
                            }
                            (kept, skipped)
                        } else {
                            (raw_dirty_tiles, false)
                        };

                        let (dirty_tiles, any_saliency_skipped) = if saliency_cfg.enabled {
                            let (scheduled, crit, norm, bg, skipped) =
                                tetherly_core::ScreenTileSaliencyEvaluator::schedule_tiles(
                                    dirty_tiles,
                                    focus_box,
                                    &saliency_cfg,
                                    is_congested,
                                );
                            self.shared
                                .screen_tile_saliency_critical_tiles
                                .fetch_add(crit, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_normal_tiles
                                .fetch_add(norm, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_background_tiles
                                .fetch_add(bg, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_background_skipped
                                .fetch_add(skipped, std::sync::atomic::Ordering::Relaxed);
                            (scheduled, skipped > 0)
                        } else {
                            (dirty_tiles, false)
                        };
                        let any_skipped = any_peripheral_skipped || any_saliency_skipped;

                        if !dirty_tiles.is_empty() {
                            let mut cached_entries = Vec::with_capacity(dirty_tiles.len());
                            let mut dict = self.shared.screen_tile_cache_dict.lock().unwrap();
                            let mut any_lod_downsampled = false;
                            let mut lod_flags = Vec::with_capacity(dirty_tiles.len());
                            for tile in &dirty_tiles {
                                let hash = tetherly_core::fnv1a_hash64(&tile.data);
                                let (hit, cache_id) = dict.lookup_or_insert(hash);
                                if hit {
                                    lod_flags.push(false);
                                    self.shared
                                        .screen_tile_cache_hits
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let saved = (tile.data.len() as u64).saturating_sub(17);
                                    self.shared
                                        .screen_tile_cache_saved_bytes
                                        .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                    cached_entries.push(tetherly_core::CachedTileEntry {
                                        index: tile.index,
                                        x: tile.x,
                                        y: tile.y,
                                        w: tile.w,
                                        h: tile.h,
                                        cache_hit: true,
                                        cache_id,
                                        data: Vec::new(),
                                    });
                                } else {
                                    self.shared
                                        .screen_tile_cache_misses
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let raw_len = tile.data.len();
                                    let motion_opt = if tile_motion_cfg.enabled {
                                        tetherly_core::compress_tile_motion(
                                            &tile.data,
                                            tile.x,
                                            tile.y,
                                            tile.w,
                                            tile.h,
                                            prev,
                                            &tile_motion_cfg,
                                        )
                                    } else {
                                        None
                                    };

                                    let (to_send, subc) = if let Some((
                                        motion_subc,
                                        motion_payload,
                                    )) = motion_opt
                                    {
                                        self.shared
                                            .screen_tile_motion_tiles_sent
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        let saved = (raw_len as u64)
                                            .saturating_sub(motion_payload.len() as u64);
                                        self.shared
                                            .screen_tile_motion_bytes_saved
                                            .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                        lod_flags.push(false);
                                        (motion_payload, motion_subc)
                                    } else {
                                        if tile_motion_cfg.enabled {
                                            self.shared
                                                .screen_tile_motion_fallback_tiles_sent
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        let (target_lod, is_lod) = if lod_cfg.enabled {
                                            let (_, tier) = if saliency_cfg.enabled {
                                                tetherly_core::ScreenTileSaliencyEvaluator::evaluate_tile(tile, focus_box, &saliency_cfg)
                                            } else {
                                                (100, tetherly_core::TileSaliencyTier::Normal)
                                            };
                                            if tier == tetherly_core::TileSaliencyTier::Critical {
                                                (tetherly_core::TileLodLevel::Lod0, false)
                                            } else if lod_cfg.auto_lod_on_congestion {
                                                if is_congested {
                                                    if tier == tetherly_core::TileSaliencyTier::Background && lod_cfg.max_lod >= 2 {
                                                        (tetherly_core::TileLodLevel::Lod2, true)
                                                    } else if lod_cfg.max_lod >= 1 {
                                                        if !saliency_cfg.enabled && lod_cfg.max_lod >= 2 {
                                                            (tetherly_core::TileLodLevel::Lod2, true)
                                                        } else {
                                                            (tetherly_core::TileLodLevel::Lod1, true)
                                                        }
                                                    } else {
                                                        (tetherly_core::TileLodLevel::Lod0, false)
                                                    }
                                                } else {
                                                    (tetherly_core::TileLodLevel::Lod0, false)
                                                }
                                            } else {
                                                if (tier
                                                    == tetherly_core::TileSaliencyTier::Background
                                                    || !saliency_cfg.enabled)
                                                    && lod_cfg.max_lod >= 2
                                                {
                                                    (tetherly_core::TileLodLevel::Lod2, true)
                                                } else if lod_cfg.max_lod >= 1 {
                                                    (tetherly_core::TileLodLevel::Lod1, true)
                                                } else {
                                                    (tetherly_core::TileLodLevel::Lod0, false)
                                                }
                                            }
                                        } else {
                                            (tetherly_core::TileLodLevel::Lod0, false)
                                        };
                                        if is_lod {
                                            any_lod_downsampled = true;
                                        }
                                        lod_flags.push(is_lod);

                                        if target_lod != tetherly_core::TileLodLevel::Lod0 {
                                            let (subc, comp) = tetherly_core::compress_tile_lod(
                                                &tile.data,
                                                tile.w,
                                                tile.h,
                                                target_lod,
                                                self.shared.screen_codec.as_ref(),
                                            );
                                            let saved =
                                                (raw_len as u64).saturating_sub(comp.len() as u64);
                                            self.shared.screen_tile_lod_bytes_saved.fetch_add(
                                                saved,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            match target_lod {
                                                tetherly_core::TileLodLevel::Lod1 => {
                                                    self.shared.screen_tile_lod1_tiles.fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                }
                                                tetherly_core::TileLodLevel::Lod2 => {
                                                    self.shared.screen_tile_lod2_tiles.fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                }
                                                _ => {}
                                            }
                                            (comp, subc)
                                        } else {
                                            if lod_cfg.enabled {
                                                self.shared.screen_tile_lod0_tiles.fetch_add(
                                                    1,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }
                                            if palette_cfg.enabled {
                                                let (subcodec, encoded) =
                                                    tetherly_core::compress_tile_palette(
                                                        &tile.data,
                                                        tile.w,
                                                        tile.h,
                                                        &palette_cfg,
                                                    );
                                                if subcodec == tetherly_core::TILE_SUBCODEC_SOLID
                                                    || subcodec
                                                        == tetherly_core::TILE_SUBCODEC_BILEVEL
                                                    || subcodec
                                                        == tetherly_core::TILE_SUBCODEC_PALETTE16
                                                {
                                                    (encoded, subcodec)
                                                } else if predict_cfg.enabled {
                                                    let (pred_subc, pred_data) =
                                                        tetherly_core::compress_tile_predictive(
                                                            &tile.data,
                                                            tile.w,
                                                            tile.h,
                                                            &predict_cfg,
                                                            self.shared.screen_codec.as_ref(),
                                                        );
                                                    if pred_subc
                                                        == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                                        && pred_data.len() < raw_len
                                                    {
                                                        (pred_data, tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD)
                                                    } else {
                                                        let comp = if let Ok(compressed) =
                                                            self.shared.screen_codec.compress_frame(
                                                                &tile.data, tile.w, tile.h,
                                                            ) {
                                                            if compressed.len() < raw_len {
                                                                compressed
                                                            } else {
                                                                tile.data.clone()
                                                            }
                                                        } else {
                                                            tile.data.clone()
                                                        };
                                                        (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                                    }
                                                } else {
                                                    let comp = if let Ok(compressed) = self
                                                        .shared
                                                        .screen_codec
                                                        .compress_frame(&tile.data, tile.w, tile.h)
                                                    {
                                                        if compressed.len() < raw_len {
                                                            compressed
                                                        } else {
                                                            tile.data.clone()
                                                        }
                                                    } else {
                                                        tile.data.clone()
                                                    };
                                                    (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                                }
                                            } else if predict_cfg.enabled {
                                                let (pred_subc, pred_data) =
                                                    tetherly_core::compress_tile_predictive(
                                                        &tile.data,
                                                        tile.w,
                                                        tile.h,
                                                        &predict_cfg,
                                                        self.shared.screen_codec.as_ref(),
                                                    );
                                                if pred_subc
                                                    == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                                    && pred_data.len() < raw_len
                                                {
                                                    (
                                                        pred_data,
                                                        tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD,
                                                    )
                                                } else {
                                                    let comp = if let Ok(compressed) = self
                                                        .shared
                                                        .screen_codec
                                                        .compress_frame(&tile.data, tile.w, tile.h)
                                                    {
                                                        if compressed.len() < raw_len {
                                                            compressed
                                                        } else {
                                                            tile.data.clone()
                                                        }
                                                    } else {
                                                        tile.data.clone()
                                                    };
                                                    (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                                }
                                            } else {
                                                let comp = if let Ok(compressed) = self
                                                    .shared
                                                    .screen_codec
                                                    .compress_frame(&tile.data, tile.w, tile.h)
                                                {
                                                    if compressed.len() < raw_len {
                                                        compressed
                                                    } else {
                                                        tile.data.clone()
                                                    }
                                                } else {
                                                    tile.data.clone()
                                                };
                                                (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                            }
                                        }
                                    };
                                    if palette_cfg.enabled {
                                        let saved =
                                            (raw_len as u64).saturating_sub(to_send.len() as u64);
                                        match subc {
                                            tetherly_core::TILE_SUBCODEC_SOLID => {
                                                self.shared
                                                    .screen_tile_palette_solid_count
                                                    .fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                self.shared
                                                    .screen_tile_palette_saved_bytes
                                                    .fetch_add(
                                                        saved,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                            }
                                            tetherly_core::TILE_SUBCODEC_BILEVEL => {
                                                self.shared
                                                    .screen_tile_palette_bilevel_count
                                                    .fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                self.shared
                                                    .screen_tile_palette_saved_bytes
                                                    .fetch_add(
                                                        saved,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                            }
                                            tetherly_core::TILE_SUBCODEC_PALETTE16 => {
                                                self.shared
                                                    .screen_tile_palette_palette_count
                                                    .fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                                self.shared
                                                    .screen_tile_palette_saved_bytes
                                                    .fetch_add(
                                                        saved,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                            }
                                            _ => {
                                                self.shared
                                                    .screen_tile_palette_truecolor_count
                                                    .fetch_add(
                                                        1,
                                                        std::sync::atomic::Ordering::Relaxed,
                                                    );
                                            }
                                        }
                                    }
                                    if predict_cfg.enabled
                                        && subc == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                    {
                                        let saved =
                                            (raw_len as u64).saturating_sub(to_send.len() as u64);
                                        self.shared
                                            .screen_tile_predict_tiles_count
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        self.shared
                                            .screen_tile_predict_saved_bytes
                                            .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    cached_entries.push(tetherly_core::CachedTileEntry {
                                        index: tile.index,
                                        x: tile.x,
                                        y: tile.y,
                                        w: tile.w,
                                        h: tile.h,
                                        cache_hit: false,
                                        cache_id,
                                        data: to_send,
                                    });
                                }
                            }
                            drop(dict);
                            if any_skipped || any_lod_downsampled {
                                let mut prev_canvas = prev.clone();
                                for (t, was_lod) in dirty_tiles.iter().zip(lod_flags.iter()) {
                                    if !*was_lod {
                                        let _ = prev_canvas
                                            .apply_cached_tile(t.x, t.y, t.w, t.h, &t.data);
                                    }
                                }
                                *self.shared.screen_sender_tile_canvas.write().unwrap() =
                                    Some(prev_canvas);
                            } else {
                                *self.shared.screen_sender_tile_canvas.write().unwrap() =
                                    Some(frame.clone());
                            }
                            encoded_tile_cache = Some(ScreenFrame::encode_tile_cache(
                                frame.seq,
                                frame.rect,
                                frame.format,
                                tile_size,
                                total_tiles,
                                &cached_entries,
                            ));
                        }
                    }
                }
                if encoded_tile_cache.is_none()
                    && (prev_opt.is_none()
                        || (!focus_cfg.enabled
                            && !saliency_cfg.enabled
                            && !lod_cfg.enabled
                            && !tile_motion_cfg.enabled))
                {
                    *self.shared.screen_sender_tile_canvas.write().unwrap() = Some(frame.clone());
                }
                (encoded_tile_cache, None, None)
            } else if (tile_cfg.enabled
                || focus_cfg.enabled
                || palette_cfg.enabled
                || predict_cfg.enabled
                || saliency_cfg.enabled
                || lod_cfg.enabled
                || tile_batch_cfg.enabled
                || tile_motion_cfg.enabled)
                && !frame.compressed()
                && !is_motion
            {
                let prev_opt = self
                    .shared
                    .screen_sender_tile_canvas
                    .read()
                    .unwrap()
                    .clone();
                let mut encoded_tile = None;
                let mut encoded_tile_batch = None;
                if let Some(prev) = &prev_opt {
                    if prev.rect.w == frame.rect.w
                        && prev.rect.h == frame.rect.h
                        && prev.format == frame.format
                    {
                        let tile_size = if tile_cfg.enabled {
                            tile_cfg.tile_size
                        } else {
                            tetherly_core::SCREEN_TILE_DEFAULT_SIZE
                        };
                        let (total_tiles, raw_dirty_tiles) =
                            tetherly_core::extract_dirty_tiles(prev, &frame, tile_size);
                        let (dirty_tiles, any_peripheral_skipped) = if focus_cfg.enabled {
                            let cadence = (focus_cfg.peripheral_skip_ratio as u64) + 1;
                            let send_peripheral = (frame.seq % cadence) == 0;
                            let mut kept = Vec::with_capacity(raw_dirty_tiles.len());
                            let mut skipped = false;
                            for tile in raw_dirty_tiles {
                                let in_focus = tetherly_core::is_tile_in_focus(
                                    tile.x, tile.y, tile.w, tile.h, &focus_cfg, cursor_pos,
                                );
                                if in_focus {
                                    self.shared
                                        .screen_focus_tiles_sent
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    kept.push(tile);
                                } else if send_peripheral {
                                    self.shared
                                        .screen_focus_peripheral_sent
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    kept.push(tile);
                                } else {
                                    self.shared
                                        .screen_focus_peripheral_skipped
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    self.shared.screen_focus_saved_bytes.fetch_add(
                                        tile.data.len() as u64,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                    skipped = true;
                                }
                            }
                            (kept, skipped)
                        } else {
                            (raw_dirty_tiles, false)
                        };

                        let (dirty_tiles, any_saliency_skipped) = if saliency_cfg.enabled {
                            let (scheduled, crit, norm, bg, skipped) =
                                tetherly_core::ScreenTileSaliencyEvaluator::schedule_tiles(
                                    dirty_tiles,
                                    focus_box,
                                    &saliency_cfg,
                                    is_congested,
                                );
                            self.shared
                                .screen_tile_saliency_critical_tiles
                                .fetch_add(crit, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_normal_tiles
                                .fetch_add(norm, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_background_tiles
                                .fetch_add(bg, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_saliency_background_skipped
                                .fetch_add(skipped, std::sync::atomic::Ordering::Relaxed);
                            (scheduled, skipped > 0)
                        } else {
                            (dirty_tiles, false)
                        };
                        let any_skipped = any_peripheral_skipped || any_saliency_skipped;

                        let threshold_ok = focus_cfg.enabled
                            || palette_cfg.enabled
                            || predict_cfg.enabled
                            || saliency_cfg.enabled
                            || lod_cfg.enabled
                            || tile_batch_cfg.enabled
                            || tile_motion_cfg.enabled
                            || dirty_tiles.len() < ((total_tiles as usize) * 3 / 4).max(1);
                        if !dirty_tiles.is_empty() && threshold_ok {
                            let mut compressed_tiles = Vec::with_capacity(dirty_tiles.len());
                            let mut payload_bytes = 6u64;
                            let mut any_lod_downsampled = false;
                            let mut lod_flags = Vec::with_capacity(dirty_tiles.len());
                            for tile in &dirty_tiles {
                                let raw_len = tile.data.len();
                                let motion_opt = if tile_motion_cfg.enabled {
                                    tetherly_core::compress_tile_motion(
                                        &tile.data,
                                        tile.x,
                                        tile.y,
                                        tile.w,
                                        tile.h,
                                        prev,
                                        &tile_motion_cfg,
                                    )
                                } else {
                                    None
                                };

                                let (to_send, subc) = if let Some((motion_subc, motion_payload)) =
                                    motion_opt
                                {
                                    self.shared
                                        .screen_tile_motion_tiles_sent
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let saved = (raw_len as u64)
                                        .saturating_sub(motion_payload.len() as u64);
                                    self.shared
                                        .screen_tile_motion_bytes_saved
                                        .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                    lod_flags.push(false);
                                    (motion_payload, motion_subc)
                                } else {
                                    if tile_motion_cfg.enabled {
                                        self.shared
                                            .screen_tile_motion_fallback_tiles_sent
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    let (target_lod, is_lod) = if lod_cfg.enabled {
                                        let (_, tier) = if saliency_cfg.enabled {
                                            tetherly_core::ScreenTileSaliencyEvaluator::evaluate_tile(tile, focus_box, &saliency_cfg)
                                        } else {
                                            (100, tetherly_core::TileSaliencyTier::Normal)
                                        };
                                        if tier == tetherly_core::TileSaliencyTier::Critical {
                                            (tetherly_core::TileLodLevel::Lod0, false)
                                        } else if lod_cfg.auto_lod_on_congestion {
                                            if is_congested {
                                                if tier
                                                    == tetherly_core::TileSaliencyTier::Background
                                                    && lod_cfg.max_lod >= 2
                                                {
                                                    (tetherly_core::TileLodLevel::Lod2, true)
                                                } else if lod_cfg.max_lod >= 1 {
                                                    if !saliency_cfg.enabled && lod_cfg.max_lod >= 2
                                                    {
                                                        (tetherly_core::TileLodLevel::Lod2, true)
                                                    } else {
                                                        (tetherly_core::TileLodLevel::Lod1, true)
                                                    }
                                                } else {
                                                    (tetherly_core::TileLodLevel::Lod0, false)
                                                }
                                            } else {
                                                (tetherly_core::TileLodLevel::Lod0, false)
                                            }
                                        } else {
                                            if (tier == tetherly_core::TileSaliencyTier::Background
                                                || !saliency_cfg.enabled)
                                                && lod_cfg.max_lod >= 2
                                            {
                                                (tetherly_core::TileLodLevel::Lod2, true)
                                            } else if lod_cfg.max_lod >= 1 {
                                                (tetherly_core::TileLodLevel::Lod1, true)
                                            } else {
                                                (tetherly_core::TileLodLevel::Lod0, false)
                                            }
                                        }
                                    } else {
                                        (tetherly_core::TileLodLevel::Lod0, false)
                                    };
                                    if is_lod {
                                        any_lod_downsampled = true;
                                    }
                                    lod_flags.push(is_lod);

                                    if target_lod != tetherly_core::TileLodLevel::Lod0 {
                                        let (subc, comp) = tetherly_core::compress_tile_lod(
                                            &tile.data,
                                            tile.w,
                                            tile.h,
                                            target_lod,
                                            self.shared.screen_codec.as_ref(),
                                        );
                                        let saved =
                                            (raw_len as u64).saturating_sub(comp.len() as u64);
                                        self.shared
                                            .screen_tile_lod_bytes_saved
                                            .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                        match target_lod {
                                            tetherly_core::TileLodLevel::Lod1 => {
                                                self.shared.screen_tile_lod1_tiles.fetch_add(
                                                    1,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }
                                            tetherly_core::TileLodLevel::Lod2 => {
                                                self.shared.screen_tile_lod2_tiles.fetch_add(
                                                    1,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }
                                            _ => {}
                                        }
                                        (comp, subc)
                                    } else {
                                        if lod_cfg.enabled {
                                            self.shared
                                                .screen_tile_lod0_tiles
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                        if palette_cfg.enabled {
                                            let (subcodec, encoded) =
                                                tetherly_core::compress_tile_palette(
                                                    &tile.data,
                                                    tile.w,
                                                    tile.h,
                                                    &palette_cfg,
                                                );
                                            if subcodec == tetherly_core::TILE_SUBCODEC_SOLID
                                                || subcodec == tetherly_core::TILE_SUBCODEC_BILEVEL
                                                || subcodec
                                                    == tetherly_core::TILE_SUBCODEC_PALETTE16
                                            {
                                                (encoded, subcodec)
                                            } else if predict_cfg.enabled {
                                                let (pred_subc, pred_data) =
                                                    tetherly_core::compress_tile_predictive(
                                                        &tile.data,
                                                        tile.w,
                                                        tile.h,
                                                        &predict_cfg,
                                                        self.shared.screen_codec.as_ref(),
                                                    );
                                                if pred_subc
                                                    == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                                    && pred_data.len() < raw_len
                                                {
                                                    (
                                                        pred_data,
                                                        tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD,
                                                    )
                                                } else {
                                                    let comp = if let Ok(compressed) = self
                                                        .shared
                                                        .screen_codec
                                                        .compress_frame(&tile.data, tile.w, tile.h)
                                                    {
                                                        if compressed.len() < raw_len {
                                                            compressed
                                                        } else {
                                                            tile.data.clone()
                                                        }
                                                    } else {
                                                        tile.data.clone()
                                                    };
                                                    (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                                }
                                            } else {
                                                let comp = if let Ok(compressed) = self
                                                    .shared
                                                    .screen_codec
                                                    .compress_frame(&tile.data, tile.w, tile.h)
                                                {
                                                    if compressed.len() < raw_len {
                                                        compressed
                                                    } else {
                                                        tile.data.clone()
                                                    }
                                                } else {
                                                    tile.data.clone()
                                                };
                                                (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                            }
                                        } else if predict_cfg.enabled {
                                            let (pred_subc, pred_data) =
                                                tetherly_core::compress_tile_predictive(
                                                    &tile.data,
                                                    tile.w,
                                                    tile.h,
                                                    &predict_cfg,
                                                    self.shared.screen_codec.as_ref(),
                                                );
                                            if pred_subc
                                                == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                                && pred_data.len() < raw_len
                                            {
                                                (
                                                    pred_data,
                                                    tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD,
                                                )
                                            } else {
                                                let comp = if let Ok(compressed) = self
                                                    .shared
                                                    .screen_codec
                                                    .compress_frame(&tile.data, tile.w, tile.h)
                                                {
                                                    if compressed.len() < raw_len {
                                                        compressed
                                                    } else {
                                                        tile.data.clone()
                                                    }
                                                } else {
                                                    tile.data.clone()
                                                };
                                                (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                            }
                                        } else {
                                            let comp = if let Ok(compressed) = self
                                                .shared
                                                .screen_codec
                                                .compress_frame(&tile.data, tile.w, tile.h)
                                            {
                                                if compressed.len() < raw_len {
                                                    compressed
                                                } else {
                                                    tile.data.clone()
                                                }
                                            } else {
                                                tile.data.clone()
                                            };
                                            (comp, tetherly_core::TILE_SUBCODEC_RAW)
                                        }
                                    }
                                };

                                if palette_cfg.enabled {
                                    let saved =
                                        (raw_len as u64).saturating_sub(to_send.len() as u64);
                                    match subc {
                                        tetherly_core::TILE_SUBCODEC_SOLID => {
                                            self.shared
                                                .screen_tile_palette_solid_count
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            self.shared.screen_tile_palette_saved_bytes.fetch_add(
                                                saved,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                        }
                                        tetherly_core::TILE_SUBCODEC_BILEVEL => {
                                            self.shared
                                                .screen_tile_palette_bilevel_count
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            self.shared.screen_tile_palette_saved_bytes.fetch_add(
                                                saved,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                        }
                                        tetherly_core::TILE_SUBCODEC_PALETTE16 => {
                                            self.shared
                                                .screen_tile_palette_palette_count
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                            self.shared.screen_tile_palette_saved_bytes.fetch_add(
                                                saved,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                        }
                                        _ => {
                                            self.shared
                                                .screen_tile_palette_truecolor_count
                                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        }
                                    }
                                }
                                if predict_cfg.enabled
                                    && subc == tetherly_core::TILE_SUBCODEC_PREDICT_ZSTD
                                {
                                    let saved =
                                        (raw_len as u64).saturating_sub(to_send.len() as u64);
                                    self.shared
                                        .screen_tile_predict_tiles_count
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    self.shared
                                        .screen_tile_predict_saved_bytes
                                        .fetch_add(saved, std::sync::atomic::Ordering::Relaxed);
                                }

                                payload_bytes += 14 + to_send.len() as u64;
                                compressed_tiles.push(tetherly_core::DirtyTile {
                                    data: to_send,
                                    ..tile.clone()
                                });
                            }
                            let full_raw_bytes = (frame.rect.w as u64)
                                * (frame.rect.h as u64)
                                * (frame.format.bpp() as u64);
                            if full_raw_bytes > payload_bytes {
                                self.shared.screen_tile_saved_bytes.fetch_add(
                                    full_raw_bytes - payload_bytes,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                            let count = compressed_tiles.len() as u64;
                            self.shared
                                .screen_tile_frames_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            self.shared
                                .screen_tile_dirty_tiles_sent
                                .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
                            let skipped = (total_tiles as u64).saturating_sub(count);
                            self.shared
                                .screen_tile_skipped_count
                                .fetch_add(skipped, std::sync::atomic::Ordering::Relaxed);

                            if any_skipped || any_lod_downsampled {
                                let mut prev_canvas = prev.clone();
                                for (t, was_lod) in dirty_tiles.iter().zip(lod_flags.iter()) {
                                    if !*was_lod {
                                        let _ = prev_canvas
                                            .apply_cached_tile(t.x, t.y, t.w, t.h, &t.data);
                                    }
                                }
                                *self.shared.screen_sender_tile_canvas.write().unwrap() =
                                    Some(prev_canvas);
                            } else {
                                *self.shared.screen_sender_tile_canvas.write().unwrap() =
                                    Some(frame.clone());
                            }

                            tile_resend_entry = Some((frame.seq as u32, compressed_tiles.clone()));
                            self.shared
                                .screen_tile_resend_buffer
                                .write()
                                .unwrap()
                                .push_frame(frame.seq as u32, compressed_tiles.clone());
                            if tile_batch_cfg.enabled {
                                let batch_id = self
                                    .shared
                                    .screen_tile_batches_sent
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                    as u32
                                    + 1;
                                let chunks = tetherly_core::chunk_dirty_tiles(
                                    frame.seq,
                                    frame.rect,
                                    frame.format,
                                    compressed_tiles.clone(),
                                    tile_batch_cfg.target_chunk_bytes as usize,
                                    tile_size,
                                    total_tiles,
                                    batch_id,
                                );
                                let chunk_count = chunks.len() as u64;
                                self.shared
                                    .screen_tile_chunks_sent
                                    .fetch_add(chunk_count, std::sync::atomic::Ordering::Relaxed);
                                let total_bytes: u64 =
                                    chunks.iter().map(|c| c.bytes.len() as u64).sum();
                                self.shared
                                    .screen_tile_batch_bytes_total
                                    .fetch_add(total_bytes, std::sync::atomic::Ordering::Relaxed);
                                encoded_tile_batch = Some(chunks);
                            } else {
                                encoded_tile = Some(ScreenFrame::encode_tile_grid(
                                    frame.seq,
                                    frame.rect,
                                    frame.format,
                                    tile_size,
                                    total_tiles,
                                    &compressed_tiles,
                                ));
                            }
                        }
                    }
                }
                if encoded_tile.is_none()
                    && encoded_tile_batch.is_none()
                    && (prev_opt.is_none()
                        || (!focus_cfg.enabled
                            && !saliency_cfg.enabled
                            && !lod_cfg.enabled
                            && !tile_batch_cfg.enabled))
                {
                    *self.shared.screen_sender_tile_canvas.write().unwrap() = Some(frame.clone());
                }
                (None, encoded_tile, encoded_tile_batch)
            } else {
                if !frame.compressed() {
                    *self.shared.screen_sender_tile_canvas.write().unwrap() = Some(frame.clone());
                }
                (None, None, None)
            };
            let is_tile = tile_frame_opt.is_some();
            let is_tile_cache = tile_cache_frame_opt.is_some();
            let is_tile_batch = tile_batch_chunks_opt.is_some();

            let frame = if let Some(cf) = tile_cache_frame_opt {
                cf
            } else if let Some(chunks) = &tile_batch_chunks_opt {
                chunks[0].clone()
            } else if let Some(mf) = motion_frame_opt {
                mf
            } else if let Some(tf) = tile_frame_opt {
                tf
            } else if is_ltr_recovery && !frame.compressed() {
                let golden_opt = self.shared.screen_sender_ltr_canvas.read().unwrap().clone();
                match golden_opt {
                    Some(golden)
                        if golden.rect.w == frame.rect.w
                            && golden.rect.h == frame.rect.h
                            && golden.format == frame.format =>
                    {
                        let mut tracker = self
                            .shared
                            .screen_damage_tracker
                            .lock()
                            .expect("damage tracker");
                        let report = tracker.evaluate_rgba(
                            &golden.bytes,
                            &frame.bytes,
                            frame.rect.w as u32,
                            frame.rect.h as u32,
                        );
                        drop(tracker);
                        let rect =
                            report
                                .bounding_rect
                                .unwrap_or(tetherly_core::ScreenDamageRect {
                                    x: 0,
                                    y: 0,
                                    width: 1.min(frame.rect.w),
                                    height: 1.min(frame.rect.h),
                                });
                        let patch =
                            frame.extract_sub_rect(rect.x, rect.y, rect.width, rect.height)?;
                        let compressed = self.shared.screen_codec.compress_frame(
                            &patch.bytes,
                            patch.rect.w,
                            patch.rect.h,
                        )?;
                        let full_raw_bytes = (frame.rect.w as u64)
                            * (frame.rect.h as u64)
                            * (frame.format.bpp() as u64);
                        let patch_bytes = compressed.len() as u64;
                        if full_raw_bytes > patch_bytes {
                            self.shared.screen_ltr_saved_burst_bytes.fetch_add(
                                full_raw_bytes - patch_bytes,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                        }
                        self.shared
                            .screen_ltr_recoveries_completed
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        ScreenFrame {
                            codec_tag: tetherly_core::SCREEN_COMPRESS_LTR,
                            bytes: compressed,
                            ..patch
                        }
                    }
                    _ => {
                        self.shared
                            .screen_delta_counter
                            .store(0, std::sync::atomic::Ordering::Relaxed);
                        self.shared
                            .screen_delta_keyframes_sent
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        frame
                    }
                }
            } else if is_pir_active && !frame.compressed() {
                let cur_band = self
                    .shared
                    .screen_pir_current_band
                    .load(std::sync::atomic::Ordering::Relaxed);
                let bands = pir_cfg.bands.clamp(
                    tetherly_core::SCREEN_PIR_MIN_BANDS,
                    tetherly_core::SCREEN_PIR_MAX_BANDS,
                );
                let (bx, by, bw, bh) = tetherly_core::compute_pir_band_rect(
                    frame.rect.w as u32,
                    frame.rect.h as u32,
                    bands,
                    cur_band,
                );
                let slice = frame.extract_sub_rect(bx as u16, by as u16, bw as u16, bh as u16)?;
                let compressed = self.shared.screen_codec.compress_frame(
                    &slice.bytes,
                    slice.rect.w,
                    slice.rect.h,
                )?;
                let next_band = (cur_band + 1) % bands;
                self.shared
                    .screen_pir_current_band
                    .store(next_band, std::sync::atomic::Ordering::Relaxed);
                self.shared
                    .screen_pir_slices_sent
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if next_band == 0 {
                    self.shared
                        .screen_pir_sweeps_completed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if pir_cfg.mode == tetherly_core::ScreenPirMode::OnDemand {
                        self.shared
                            .screen_pir_active_sweep
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                ScreenFrame {
                    codec_tag: tetherly_core::SCREEN_COMPRESS_DELTA,
                    bytes: compressed,
                    ..slice
                }
            } else if svc_cfg.is_enabled() && !frame.compressed() {
                // ADR-114 / v2.28: 2-Layer Temporal Scalability (SVC-T) Hierarchical Delta & Leaf Branch
                let is_t1 = (frame.seq % 2) != 0;
                if is_t1 {
                    // Enhancement Layer (T1): strictly references latest T0 anchor; never updates T0 anchor
                    let t0_opt = self.shared.screen_sender_t0_canvas.read().unwrap().clone();
                    match t0_opt {
                        Some(t0_ref)
                            if t0_ref.rect.w == frame.rect.w
                                && t0_ref.rect.h == frame.rect.h
                                && t0_ref.format == frame.format =>
                        {
                            let mut tracker = self
                                .shared
                                .screen_damage_tracker
                                .lock()
                                .expect("damage tracker");
                            let report = tracker.evaluate_rgba(
                                &t0_ref.bytes,
                                &frame.bytes,
                                frame.rect.w as u32,
                                frame.rect.h as u32,
                            );
                            drop(tracker);
                            let rect =
                                report
                                    .bounding_rect
                                    .unwrap_or(tetherly_core::ScreenDamageRect {
                                        x: 0,
                                        y: 0,
                                        width: 1.min(frame.rect.w),
                                        height: 1.min(frame.rect.h),
                                    });
                            let patch =
                                frame.extract_sub_rect(rect.x, rect.y, rect.width, rect.height)?;
                            let compressed = self.shared.screen_codec.compress_frame(
                                &patch.bytes,
                                patch.rect.w,
                                patch.rect.h,
                            )?;
                            let full_raw_bytes = (frame.rect.w as u64)
                                * (frame.rect.h as u64)
                                * (frame.format.bpp() as u64);
                            let patch_bytes = compressed.len() as u64;
                            if full_raw_bytes > patch_bytes {
                                self.shared.screen_svc_saved_bytes.fetch_add(
                                    full_raw_bytes - patch_bytes,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                            self.shared
                                .screen_svc_t1_frames_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            ScreenFrame {
                                codec_tag: tetherly_core::SCREEN_COMPRESS_SVC_T1,
                                bytes: compressed,
                                ..patch
                            }
                        }
                        _ => {
                            self.shared
                                .screen_svc_t1_frames_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            frame
                        }
                    }
                } else {
                    // Base Layer (T0): strictly references previous T0 anchor and updates T0 canvas
                    self.shared
                        .screen_svc_t0_frames_sent
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let prev_t0_opt = self.shared.screen_sender_t0_canvas.read().unwrap().clone();
                    match prev_t0_opt {
                        Some(prev_t0)
                            if prev_t0.rect.w == frame.rect.w
                                && prev_t0.rect.h == frame.rect.h
                                && prev_t0.format == frame.format =>
                        {
                            let mut tracker = self
                                .shared
                                .screen_damage_tracker
                                .lock()
                                .expect("damage tracker");
                            let report = tracker.evaluate_rgba(
                                &prev_t0.bytes,
                                &frame.bytes,
                                frame.rect.w as u32,
                                frame.rect.h as u32,
                            );
                            drop(tracker);
                            if let Some(rect) = report
                                .bounding_rect
                                .filter(|_| !report.is_static && report.dirty_ratio < 0.70)
                            {
                                let patch = frame.extract_sub_rect(
                                    rect.x,
                                    rect.y,
                                    rect.width,
                                    rect.height,
                                )?;
                                let compressed = self.shared.screen_codec.compress_frame(
                                    &patch.bytes,
                                    patch.rect.w,
                                    patch.rect.h,
                                )?;
                                let full_raw_bytes = (frame.rect.w as u64)
                                    * (frame.rect.h as u64)
                                    * (frame.format.bpp() as u64);
                                let patch_bytes = compressed.len() as u64;
                                if full_raw_bytes > patch_bytes {
                                    self.shared.screen_delta_saved_bytes.fetch_add(
                                        full_raw_bytes - patch_bytes,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                }
                                *self.shared.screen_sender_t0_canvas.write().unwrap() =
                                    Some(frame.clone());
                                ScreenFrame {
                                    codec_tag: tetherly_core::SCREEN_COMPRESS_DELTA,
                                    bytes: compressed,
                                    ..patch
                                }
                            } else {
                                *self.shared.screen_sender_t0_canvas.write().unwrap() =
                                    Some(frame.clone());
                                frame
                            }
                        }
                        _ => {
                            *self.shared.screen_sender_t0_canvas.write().unwrap() =
                                Some(frame.clone());
                            frame
                        }
                    }
                }
            } else {
                // ADR-109 / v2.23: Dynamic Differential Delta Patch & Temporal Compression
                let delta_cfg = *self.shared.screen_delta_config.read().unwrap();
                if delta_cfg.enabled && !frame.compressed() {
                    let cur_counter = self
                        .shared
                        .screen_delta_counter
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if let Some(rect) = bounding_rect.filter(|_| {
                        cur_counter < delta_cfg.keyframe_interval as u64
                            && is_dirty
                            && dirty_ratio < 0.70
                    }) {
                        let patch =
                            frame.extract_sub_rect(rect.x, rect.y, rect.width, rect.height)?;
                        let compressed = self.shared.screen_codec.compress_frame(
                            &patch.bytes,
                            patch.rect.w,
                            patch.rect.h,
                        )?;
                        let full_raw_bytes = (frame.rect.w as u64)
                            * (frame.rect.h as u64)
                            * (frame.format.bpp() as u64);
                        let patch_bytes = compressed.len() as u64;
                        if full_raw_bytes > patch_bytes {
                            self.shared.screen_delta_saved_bytes.fetch_add(
                                full_raw_bytes - patch_bytes,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                        }
                        self.shared
                            .screen_delta_counter
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        self.shared
                            .screen_delta_frames_sent
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        ScreenFrame {
                            codec_tag: tetherly_core::SCREEN_COMPRESS_DELTA,
                            bytes: compressed,
                            ..patch
                        }
                    } else {
                        self.shared
                            .screen_delta_counter
                            .store(0, std::sync::atomic::Ordering::Relaxed);
                        self.shared
                            .screen_delta_keyframes_sent
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        frame
                    }
                } else {
                    frame
                }
            };

            let effective_dirty = is_dirty
                || is_pir_active
                || is_ltr_recovery
                || svc_cfg.is_enabled()
                || is_motion
                || is_tile
                || is_tile_cache
                || is_tile_batch;
            let pushed = engine.push_with_damage_time(frame, effective_dirty, dirty_ratio, now_us);
            if pushed.is_none() {
                // Suppressed by adaptive squelch due to static screen
                engine.consume_pacing(now_us);
                return Ok(false);
            }
            if let Some(pseq) = pushed {
                if let Some((orig_seq, tiles)) = tile_resend_entry {
                    let mut buf = self.shared.screen_tile_resend_buffer.write().unwrap();
                    buf.push_frame(pseq as u32, tiles.clone());
                    if (pseq as u32) != orig_seq {
                        buf.push_frame(orig_seq, tiles);
                    }
                }
            }
            engine.consume_pacing(now_us);
            let mut outbound = engine.drain();
            if let Some(chunks) = tile_batch_chunks_opt {
                if let Some(pseq) = pushed {
                    if let Some(first) = outbound.first_mut() {
                        first.seq = pseq;
                    }
                    for mut chunk in chunks.into_iter().skip(1) {
                        chunk.seq = pseq;
                        outbound.push(chunk);
                    }
                }
            }
            outbound
        };
        self.shared
            .screen_interval
            .lock()
            .expect("interval")
            .record();
        self.send_screen_frames(outbound).await?;
        if let Some(seq) = ltr_sync_seq {
            let sync_msg = ControlMsg::LtrAnchorSync { ltr_seq: seq };
            let _ = self.send_screen_control(sync_msg).await;
        }
        Ok(true)
    }

    pub(super) async fn send_screen_frames(
        &self,
        frames: Vec<ScreenFrame>,
    ) -> Result<(), CoreError> {
        let tx = self
            .shared
            .screen_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen channel closed".into()))?;
        let pacing_us = self
            .shared
            .screen_tile_batch_config
            .read()
            .unwrap()
            .pacing_interval_us;
        for (f_idx, f) in frames.into_iter().enumerate() {
            if f_idx > 0
                && f.codec_tag == tetherly_core::SCREEN_COMPRESS_TILE_BATCH
                && pacing_us > 0
            {
                tokio::time::sleep(std::time::Duration::from_micros(pacing_us as u64)).await;
                self.shared
                    .screen_tile_batch_pacing_pauses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            let (frags, delays_us) = {
                let now_us = self.shared.clock.unix_us();
                let mut engine = self.shared.screen_engine.lock().expect("screen");
                let loss_ratio = match engine.network_health() {
                    tetherly_core::NetworkHealth::Healthy => 0.0,
                    tetherly_core::NetworkHealth::Warning => 0.015,
                    tetherly_core::NetworkHealth::Congested => 0.05,
                    tetherly_core::NetworkHealth::Critical => 0.10,
                };
                let frags =
                    engine.encode_frame_to_inner_frames_with_time(&f, loss_ratio, now_us)?;
                let delays = engine.compute_packet_delays(frags.len());
                (frags, delays)
            };
            // Live video: under backpressure drop the whole frame rather than
            // stall the pump or grow memory without bound. A partial frame is
            // never queued, so the peer's assembler is never left mid-frame.
            if tx.capacity() < frags.len() {
                let now_us = self.shared.clock.unix_us();
                let (new_rate, new_fps, new_scale) = {
                    let mut engine = self.shared.screen_engine.lock().expect("screen");
                    engine.record_drop_with_time(now_us);
                    (
                        engine.bitrate_bps(),
                        engine.fps_target(),
                        engine.scale_pct(),
                    )
                };
                self.shared.screen_source.force_idr();
                if let Some(sel) = &self.shared.screen_encode_codec {
                    sel.force_idr();
                    sel.set_bitrate_bps(new_rate);
                    sel.set_fps(new_fps);
                    sel.set_scale_pct(new_scale);
                }
                self.shared.screen_source.set_bitrate_bps(new_rate);
                self.shared.screen_source.set_fps(new_fps);
                self.shared.screen_source.set_scale_pct(new_scale);
                warn!(
                    need = frags.len(),
                    have = tx.capacity(),
                    bitrate_bps = new_rate,
                    fps_target = new_fps,
                    scale_pct = new_scale,
                    "screen queue full; dropping frame; flagged IDR and adapted 4D matrix"
                );
                continue;
            }
            let mut prev_delay = 0u64;
            for (idx, frag) in frags.into_iter().enumerate() {
                if let Some(&delay_us) = delays_us.get(idx) {
                    let step = delay_us.saturating_sub(prev_delay);
                    if step > 0 {
                        tokio::time::sleep(std::time::Duration::from_micros(step)).await;
                    }
                    prev_delay = delay_us;
                }
                if tx.try_send(frag).is_err() {
                    return Err(CoreError::Json("screen peer gone".into()));
                }
            }
        }
        let probe_frames: Vec<InnerFrame> = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let mut pf = Vec::new();
            while let Some(probe) = engine.pop_probe_packet() {
                if let Ok(f) = probe.to_inner() {
                    pf.push(f);
                }
            }
            pf
        };
        for pf in probe_frames {
            let p_len = (pf.payload.len() + 12) as u64;
            if tx.try_send(pf).is_err() {
                return Err(CoreError::Json("screen peer gone".into()));
            }
            self.shared.telemetry.record_screen_tx(p_len);
        }
        let sync_frames: Vec<InnerFrame> = {
            let now_us = self.shared.clock.unix_us();
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            engine.record_video_pts(now_us);
            engine.maybe_generate_sync_beacon(now_us);
            let mut sf = Vec::new();
            while let Some(sync_beacon) = engine.pop_sync_beacon() {
                if let Ok(f) = sync_beacon.to_inner() {
                    sf.push(f);
                }
            }
            sf
        };
        for sf in sync_frames {
            let s_len = (sf.payload.len() + 12) as u64;
            if tx.try_send(sf).is_err() {
                return Err(CoreError::Json("screen peer gone".into()));
            }
            self.shared.telemetry.record_screen_tx(s_len);
        }
        Ok(())
    }

    pub async fn send_screen_control(&self, msg: ControlMsg) -> Result<(), CoreError> {
        if let Some(tx) = self.shared.screen_ctl.lock().await.clone() {
            return match tx.try_send(msg) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("screen control queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("screen peer gone".into()))
                }
            };
        }
        if let Some(tx) = self.shared.screen_out.lock().await.clone() {
            let frame = msg
                .to_frame()
                .map_err(|e| CoreError::Json(format!("bad control frame: {e}")))?;
            return match tx.try_send(frame) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("screen frame queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("screen peer gone".into()))
                }
            };
        }
        Err(CoreError::Json("screen control channel closed".into()))
    }

    pub(super) async fn send_screen_frame_raw(&self, frame: InnerFrame) -> Result<(), CoreError> {
        if let Some(tx) = self.shared.screen_out.lock().await.clone() {
            return match tx.try_send(frame) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("screen frame queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("screen peer gone".into()))
                }
            };
        }
        if let Some(tx) = self.shared.screen_client_out.lock().await.clone() {
            return match tx.try_send(frame) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("screen frame queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("screen peer gone".into()))
                }
            };
        }
        Err(CoreError::Json("screen channel closed".into()))
    }

    /// Check if target frame pacing interval has elapsed for the outbound screen pump (ADR-083).
    pub fn screen_is_pacing_ready(&self) -> bool {
        let now_us = self.shared.clock.unix_us();
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_pacing_ready(now_us)
    }

    #[cfg(test)]
    pub fn set_screen_last_frame_for_test(&self, frame: ScreenFrame) {
        *self.shared.screen_last_frame.lock().unwrap() = Some(frame);
    }

    #[cfg(test)]
    pub fn set_screen_peer_watermark_enforced_for_test(&self, enforced: bool) {
        self.shared
            .screen_peer_watermark_enforced
            .store(enforced, Ordering::Relaxed);
    }

    pub fn screen_state(&self) -> ScreenState {
        self.shared.screen_engine.lock().expect("screen").state()
    }

    pub fn screen_stats(&self) -> ScreenStats {
        self.shared.screen_engine.lock().expect("screen").stats()
    }

    /// Visitor-side receiver counters. All zeros until this node views a peer.
    pub fn screen_rx_stats(&self) -> ScreenStats {
        *self.shared.screen_rx_stats.lock().expect("screen rx")
    }

    /// 95th-percentile host-side frame interval in microseconds, or `None`
    /// before two frames have shipped. Wall-clock: this is the M5.1 budget
    /// measurement on real hardware.
    pub fn screen_latency_p95_us(&self) -> Option<u64> {
        self.shared
            .screen_interval
            .lock()
            .expect("interval")
            .p95_us()
    }

    pub fn screen_sink_snapshot(&self) -> MemoryScreenSink {
        self.shared.screen_observer.lock().expect("sink").clone()
    }
}
