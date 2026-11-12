// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node start, run loop, UI/listener spawning and shutdown.

use super::*;
use crate::events::{file_offered, peer_up, UiEvent};
use crate::lan::{bind_many, bind_many_optional, control_bind_addrs};
use crate::platform::{MemoryClip, SystemClip};
use crate::sidecar::{OverlaySource, OverlayStatus};
use crate::store::{
    load_action_policy, load_audio_routing_policy, load_bandwidth_profiles, load_battery_policy,
    load_bench_policy, load_camera_policy, load_clipboard_policy, load_focus_policy,
    load_gesture_map, load_handoff_inbox, load_handoff_policy, load_input_policy,
    load_local_actions, load_locator_policy, load_media_policy, load_microphone_policy,
    load_topology_layout, load_topology_policy, load_trust, load_virtual_display_policy,
};
use crate::telemetry::TrafficTelemetry;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tetherly_core::frame::{ping, pong, CapsUpdate, InnerFrame};
use tetherly_core::liveness::Liveness;
use tetherly_core::ports::{Clip, Clock, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::{
    ancs_source_id, path_kind, AncsIngress, BatteryStateFrame, ClipHub, DeviceId, FileHub,
    FocusMode, FocusStateFrame, InputServer, MediaStateFrame, MemoryScreenSink, MemorySink,
    Notifier, NotifyHub, OpenAllowlist, PathKind, PlaybackState, PowerSource, ReplyView,
    ScreenAudioSyncStats, ScreenCodec, ScreenCursorStats, ScreenDamageTracker, ScreenLatencyReport,
    ScreenPlayoutMode, ScreenPlayoutStats, ScreenProbeStats, ScreenSender, ScreenSink,
    ScreenSource, ScreenStats, ScreenTuningReport, ScreenTwccStats, TcpLimiter,
};
use tetherly_net::dispatch::SessionEvent;
use tetherly_net::session::ActiveSession;
use tokio::sync::{broadcast, mpsc, watch, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

impl Node {
    pub fn start(cfg: NodeConfig) -> Result<Self, crate::store::NodeStoreError> {
        Self::start_with_clock(cfg, Arc::new(SystemClock))
    }

    pub fn start_with_clock(
        cfg: NodeConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, crate::store::NodeStoreError> {
        cleanup_stale_old_binaries();
        let identity = crate::cred::load_or_create_identity(&cfg.data_dir)?;
        let trust = load_trust(&cfg.data_dir)?;
        let (events, _) = broadcast::channel(64);
        let (shutdown, _) = watch::channel(false);
        let (rescan_trigger, _) = broadcast::channel(16);
        let clip: Arc<dyn Clip> = if cfg.memory_clip {
            Arc::new(MemoryClip::new())
        } else {
            Arc::new(SystemClip)
        };
        let notifier: Arc<dyn Notifier> = if cfg.memory_notifier || cfg.memory_clip {
            Arc::new(crate::platform::MemoryNotifier::new())
        } else {
            Arc::new(crate::platform::SystemNotifier)
        };
        let bound_control = AtomicU16::new(cfg.control_port);
        let bound_file = AtomicU16::new(cfg.file_port);
        let bound_input = AtomicU16::new(cfg.input_port);
        let bound_screen = AtomicU16::new(cfg.screen_port);
        let bound_ui = AtomicU16::new(0);
        let ancs_source = ancs_source_id(&cfg.ancs_peripheral);
        let ancs = cfg.ancs.clone().map(|t| AncsIngress::new(t, clock.clone()));
        let screen_encode_codec = cfg.screen_codec_h264.then(|| {
            Arc::new(crate::screen::NegotiatedCodec::new(
                true,
                cfg.screen_bitrate_kbps.saturating_mul(1000),
                false,
            ))
        });
        let screen_bitrate_auto = cfg.screen_bitrate_auto;
        let screen_fps = cfg.screen_fps;
        let screen_fps_auto = cfg.screen_fps_auto;
        let screen_scale_pct = cfg.screen_scale_pct;
        let screen_scale_auto = cfg.screen_scale_auto;
        let screen_profile = cfg.screen_profile;
        let screen_profile_auto = cfg.screen_profile_auto;
        let screen_qp_min = cfg.screen_qp_min;
        let screen_qp_max = cfg.screen_qp_max;
        let screen_matrix_mode = cfg.screen_matrix_mode;
        let screen_playout_mode = cfg.screen_playout_mode;
        let screen_cursor_mode = cfg.screen_cursor_mode;
        let screen_squelch_mode = cfg.screen_squelch_mode;
        let screen_fec_mode = cfg.screen_fec_mode;
        let screen_pacer_mode = cfg.screen_pacer_mode;
        let screen_trendline_mode = cfg.screen_trendline_mode;
        let screen_harq_mode = cfg.screen_harq_mode;
        let screen_twcc_mode = cfg.screen_twcc_mode;
        let screen_probe_mode = cfg.screen_probe_mode;
        let screen_sync_mode = cfg.screen_sync_mode;
        let screen_latency_mode = cfg.screen_latency_mode;
        let screen_tuning_mode = cfg.screen_tuning_mode;
        if let Some(sel) = &screen_encode_codec {
            sel.set_fps(screen_fps);
            sel.set_scale_pct(screen_scale_pct);
            sel.set_content_profile(screen_profile);
            if screen_qp_min != tetherly_core::SCREEN_QP_MIN_DEFAULT
                || screen_qp_max != tetherly_core::SCREEN_QP_MAX_DEFAULT
            {
                sel.set_qp_range(screen_qp_min, screen_qp_max);
            }
        }
        let screen_source: Arc<dyn ScreenSource> = cfg.screen_source.clone().unwrap_or_else(|| {
            // Production: real capture → negotiated codec (zstd, or H.264
            // when opted in and the peer advertised it) → wire. Tests inject
            // their own source.
            let codec: Arc<dyn ScreenCodec> = match &screen_encode_codec {
                Some(sel) => sel.clone(),
                None => Arc::new(crate::screen::ZstdCodec::default()),
            };
            Arc::new(crate::screen::EncodedSource::new(
                crate::screen::SystemSource::new(),
                codec,
            ))
        });
        screen_source.set_fps(screen_fps);
        screen_source.set_scale_pct(screen_scale_pct);
        screen_source.set_content_profile(screen_profile);
        if screen_qp_min != tetherly_core::SCREEN_QP_MIN_DEFAULT
            || screen_qp_max != tetherly_core::SCREEN_QP_MAX_DEFAULT
        {
            screen_source.set_qp_range(screen_qp_min, screen_qp_max);
        }
        let initial_display_id = cfg.screen_display_id;
        let initial_displays = screen_source.displays();
        if initial_display_id != 0 {
            let _ = screen_source.switch_display(initial_display_id);
        }
        let screen_codec: Arc<dyn ScreenCodec> = cfg
            .screen_codec
            .clone()
            .unwrap_or_else(|| Arc::new(crate::screen::ZstdCodec::default()));
        let screen_codec_h264: Arc<dyn ScreenCodec> = Arc::new(crate::screen::H264Codec::new(
            cfg.screen_bitrate_kbps * 1000,
        ));
        // The observer always records what was presented, so the local UI and
        // CI can assert on the stream without a display.
        let screen_observer = MemoryScreenSink::new();
        // Production passes SystemSink; CI leaves it None and presents to the
        // observer itself (the same shared state), so snapshots stay live.
        let screen_sink: Arc<dyn ScreenSink> = cfg
            .screen_sink
            .clone()
            .unwrap_or_else(|| Arc::new(screen_observer.clone()));
        let input_real_sink = cfg.input_sink.clone();
        let resume = load_resume_table(&cfg.data_dir);
        let initial_overlay = if cfg.overlay.enabled && !cfg.overlay.extra_peers.is_empty() {
            OverlayStatus {
                present: true,
                local_ips: Vec::new(),
                peers: cfg
                    .overlay
                    .extra_peers
                    .iter()
                    .filter_map(|ep| match ep {
                        SocketAddr::V4(v4) => Some(*v4.ip()),
                        _ => None,
                    })
                    .collect(),
                source: OverlaySource::Manual,
            }
        } else {
            OverlayStatus::lan_only()
        };
        let node_name = Mutex::new(cfg.name.clone());
        let advertise = AtomicBool::new(cfg.advertise);
        let reconnect = AtomicBool::new(cfg.reconnect);
        let initial_blocked = crate::store::load_blocked(&cfg.data_dir);
        let initial_profiles = crate::store::load_profiles(&cfg.data_dir);
        let initial_auto_accept = crate::store::load_auto_accept_policy(&cfg.data_dir);
        let initial_clipboard_policy = load_clipboard_policy(&cfg.data_dir);
        let initial_handoff_policy = load_handoff_policy(&cfg.data_dir);
        let initial_handoff_inbox = load_handoff_inbox(&cfg.data_dir);
        let initial_media_policy = load_media_policy(&cfg.data_dir);
        let initial_battery_policy = load_battery_policy(&cfg.data_dir);
        let initial_locator_policy = load_locator_policy(&cfg.data_dir);
        let initial_local_actions = load_local_actions(&cfg.data_dir);
        let initial_action_policy = load_action_policy(&cfg.data_dir);
        let initial_focus_policy = load_focus_policy(&cfg.data_dir);
        let initial_topology_layout = load_topology_layout(&cfg.data_dir);
        let initial_topology_policy = load_topology_policy(&cfg.data_dir);
        let initial_input_policy = load_input_policy(&cfg.data_dir);
        let initial_virtual_display_policy = load_virtual_display_policy(&cfg.data_dir);
        let initial_audio_routing_policy = load_audio_routing_policy(&cfg.data_dir);
        let initial_camera_policy = load_camera_policy(&cfg.data_dir);
        let initial_microphone_policy = load_microphone_policy(&cfg.data_dir);
        let initial_bench_policy = load_bench_policy(&cfg.data_dir);
        let initial_bandwidth_profiles = load_bandwidth_profiles(&cfg.data_dir);
        let initial_gesture_map = load_gesture_map(&cfg.data_dir);
        let shared = Arc::new(Shared {
            identity,
            cfg,
            trust: Arc::new(Mutex::new(trust)),
            lockout: Arc::new(Mutex::new(LockoutTable::default())),
            tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
            clock,
            hubs: Mutex::new(Hubs {
                notify: NotifyHub::default(),
                clip: ClipHub::default(),
                files: FileHub::default(),
                resume,
            }),
            clip,
            pin: Mutex::new(None),
            events,
            sessions: AsyncMutex::new(HashMap::new()),
            input_out: Mutex::new(None),
            input_reconnect_deadline: Mutex::new(None),
            input_buffered_frames: Mutex::new(VecDeque::with_capacity(64)),
            live: Mutex::new(HashMap::new()),
            ping_waiters: Mutex::new(HashMap::new()),
            pending_out: Mutex::new(HashMap::new()),
            active_transfers: Mutex::new(HashMap::new()),
            transfer_cancels: Mutex::new(HashMap::new()),
            transfer_stats: Mutex::new(TransferStats::default()),
            bound_control,
            bound_file,
            bound_input,
            bound_screen,
            bound_ui,
            input_seat: Mutex::new(None),
            input_sink: Mutex::new(MemorySink::default()),
            input_real_sink,
            input_engine: Mutex::new(InputServer::new(1, 1)),
            input_peer: Mutex::new(None),
            input_capture_active: AtomicBool::new(false),
            input_hook_control: Mutex::new(None),
            screen_out: AsyncMutex::new(None),
            screen_peer: Mutex::new(None),
            screen_engine: Mutex::new({
                let mut s = ScreenSender::with_fps(screen_fps);
                s.set_scale_pct(screen_scale_pct);
                s.set_content_profile(screen_profile);
                if screen_qp_min != tetherly_core::SCREEN_QP_MIN_DEFAULT
                    || screen_qp_max != tetherly_core::SCREEN_QP_MAX_DEFAULT
                {
                    s.set_qp_range(screen_qp_min, screen_qp_max);
                }
                s.set_matrix_mode(screen_matrix_mode);
                s.matrix_coordinator_mut().set_auto_fps(screen_fps_auto);
                s.matrix_coordinator_mut().set_auto_scale(screen_scale_auto);
                s.set_squelch_mode(screen_squelch_mode);
                s.set_fec_mode(screen_fec_mode);
                s.set_pacer_mode(screen_pacer_mode);
                s.set_trendline_mode(screen_trendline_mode);
                s.set_harq_mode(screen_harq_mode);
                s.set_twcc_mode(screen_twcc_mode);
                s.set_probe_mode(screen_probe_mode);
                s.set_sync_mode(screen_sync_mode);
                s.set_latency_mode(screen_latency_mode);
                s.set_tuning_mode(screen_tuning_mode);
                s.set_displays(initial_displays.clone());
                if initial_display_id != 0 {
                    let _ = s.switch_display(initial_display_id, 0);
                }
                s
            }),
            screen_source,
            screen_observer: Mutex::new(screen_observer),
            screen_ctl: AsyncMutex::new(None),
            screen_codec,
            screen_codec_h264,
            screen_encode_codec,
            screen_bitrate_auto,
            screen_bitrate_applied: std::sync::atomic::AtomicBool::new(false),
            screen_fps_auto,
            screen_fps_applied: std::sync::atomic::AtomicBool::new(false),
            screen_scale_auto,
            screen_scale_applied: std::sync::atomic::AtomicBool::new(false),
            screen_profile_auto,
            screen_profile_applied: std::sync::atomic::AtomicBool::new(false),
            screen_sink,
            screen_interval: Mutex::new(ScreenIntervalTracker::new()),
            screen_rx_stats: Mutex::new(ScreenStats::default()),
            screen_playout_mode: Mutex::new(screen_playout_mode),
            screen_playout_stats: Mutex::new(ScreenPlayoutStats {
                mode: screen_playout_mode,
                target_delay_us: match screen_playout_mode {
                    ScreenPlayoutMode::Adaptive => {
                        tetherly_core::SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US
                    }
                    ScreenPlayoutMode::ZeroLatency => tetherly_core::SCREEN_PLAYOUT_MIN_DELAY_US,
                    ScreenPlayoutMode::Smooth => tetherly_core::SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US,
                },
                current_jitter_us: 0,
                buffered_frames: 0,
                played_frames: 0,
                dropped_late_frames: 0,
                dropped_overflow_frames: 0,
                custom_delay: false,
            }),
            screen_cursor_mode: Mutex::new(screen_cursor_mode),
            screen_cursor_stats: Mutex::new(ScreenCursorStats::default()),
            screen_squelch_mode: Mutex::new(screen_squelch_mode),
            screen_fec_mode: Mutex::new(screen_fec_mode),
            screen_pacer_mode: Mutex::new(screen_pacer_mode),
            screen_trendline_mode: Mutex::new(screen_trendline_mode),
            screen_harq_mode: Mutex::new(screen_harq_mode),
            screen_twcc_mode: Mutex::new(screen_twcc_mode),
            screen_twcc_stats: Mutex::new(ScreenTwccStats {
                mode: screen_twcc_mode,
                ..Default::default()
            }),
            screen_probe_mode: Mutex::new(screen_probe_mode),
            screen_probe_stats: Mutex::new(ScreenProbeStats {
                mode: screen_probe_mode,
                ..Default::default()
            }),
            screen_sync_mode: Mutex::new(screen_sync_mode),
            screen_sync_stats: Mutex::new(ScreenAudioSyncStats {
                mode: screen_sync_mode,
                ..Default::default()
            }),
            screen_latency_mode: Mutex::new(screen_latency_mode),
            screen_latency_stats: Mutex::new(ScreenLatencyReport {
                mode: screen_latency_mode.as_str().to_string(),
                ..Default::default()
            }),
            screen_tuning_mode: Mutex::new(screen_tuning_mode),
            screen_tuning_stats: Mutex::new(ScreenTuningReport {
                mode: screen_tuning_mode.as_str().to_string(),
                ..Default::default()
            }),
            screen_recording: Mutex::new(None),
            screen_peer_recording: std::sync::atomic::AtomicBool::new(false),
            screen_start_refused: std::sync::atomic::AtomicBool::new(false),
            screen_start_ticket: std::sync::atomic::AtomicU64::new(0),
            screen_playback: Mutex::new(None),
            screen_last_frame: Mutex::new(None),
            screen_snapshot_count: AtomicU64::new(0),
            screen_watermark: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenWatermarkConfig::default(),
            )),
            screen_watermark_frames: AtomicU64::new(0),
            screen_peer_watermark_enforced: AtomicBool::new(false),
            screen_annotations: std::sync::Arc::new(std::sync::RwLock::new(Vec::new())),
            screen_whiteboard: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::WhiteboardHistory::default(),
            )),
            screen_whiteboard_undo_count: AtomicU64::new(0),
            screen_whiteboard_redo_count: AtomicU64::new(0),
            screen_laser_pointer: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_roi: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenRoi::default(),
            )),
            screen_roi_updates: AtomicU64::new(0),
            screen_color_mode: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenColorConfig::default(),
            )),
            screen_color_updates: AtomicU64::new(0),
            screen_delta_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenDeltaConfig::default(),
            )),
            screen_delta_frames_sent: AtomicU64::new(0),
            screen_delta_keyframes_sent: AtomicU64::new(0),
            screen_delta_frames_received: AtomicU64::new(0),
            screen_delta_keyframes_received: AtomicU64::new(0),
            screen_delta_saved_bytes: AtomicU64::new(0),
            screen_delta_counter: AtomicU64::new(0),
            screen_receiver_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_receiver_back_buffer: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_quality_metric_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenQualityMetricConfig::default(),
            )),
            screen_quality_metric_state: std::sync::Arc::new(std::sync::RwLock::new(
                ScreenQualityMetricState::default(),
            )),
            screen_metric_counter: AtomicU64::new(0),
            screen_metric_ref_frame: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_qoe_controller: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenQoeController::default(),
            )),
            screen_pir_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenPirConfig::default(),
            )),
            screen_pir_sweeps_completed: AtomicU64::new(0),
            screen_pir_slices_sent: AtomicU64::new(0),
            screen_pir_current_band: AtomicU8::new(0),
            screen_pir_active_sweep: AtomicBool::new(false),
            screen_ltr_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenLtrConfig::default(),
            )),
            screen_ltr_current_seq: AtomicU64::new(0),
            screen_ltr_anchors_established: AtomicU64::new(0),
            screen_ltr_recoveries_requested: AtomicU64::new(0),
            screen_ltr_recoveries_completed: AtomicU64::new(0),
            screen_ltr_saved_burst_bytes: AtomicU64::new(0),
            screen_ltr_recovery_pending: AtomicBool::new(false),
            screen_sender_ltr_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_receiver_ltr_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_svc_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenSvcConfig::default(),
            )),
            screen_svc_t0_frames_sent: AtomicU64::new(0),
            screen_svc_t1_frames_sent: AtomicU64::new(0),
            screen_svc_t1_frames_dropped: AtomicU64::new(0),
            screen_svc_saved_bytes: AtomicU64::new(0),
            screen_sender_t0_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_receiver_t0_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_motion_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenMotionConfig::default(),
            )),
            screen_motion_frames_sent: AtomicU64::new(0),
            screen_motion_fallback_frames_sent: AtomicU64::new(0),
            screen_motion_saved_bytes: AtomicU64::new(0),
            screen_sender_motion_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_tile_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileConfig::default(),
            )),
            screen_tile_frames_sent: AtomicU64::new(0),
            screen_tile_dirty_tiles_sent: AtomicU64::new(0),
            screen_tile_skipped_count: AtomicU64::new(0),
            screen_tile_saved_bytes: AtomicU64::new(0),
            screen_sender_tile_canvas: std::sync::Arc::new(std::sync::RwLock::new(None)),
            screen_tile_cache_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileCacheConfig::default(),
            )),
            screen_tile_cache_dict: std::sync::Arc::new(Mutex::new(
                tetherly_core::ScreenTileDictionary::new(
                    tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES,
                ),
            )),
            screen_tile_cache_receiver: std::sync::Arc::new(Mutex::new(vec![
                None;
                tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
                    as usize
            ])),
            screen_tile_cache_hits: AtomicU64::new(0),
            screen_tile_cache_misses: AtomicU64::new(0),
            screen_tile_cache_saved_bytes: AtomicU64::new(0),
            screen_focus_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenFocusConfig::default(),
            )),
            screen_focus_tiles_sent: AtomicU64::new(0),
            screen_focus_peripheral_sent: AtomicU64::new(0),
            screen_focus_peripheral_skipped: AtomicU64::new(0),
            screen_focus_saved_bytes: AtomicU64::new(0),
            screen_tile_palette_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTilePaletteConfig::default(),
            )),
            screen_tile_palette_solid_count: AtomicU64::new(0),
            screen_tile_palette_bilevel_count: AtomicU64::new(0),
            screen_tile_palette_palette_count: AtomicU64::new(0),
            screen_tile_palette_truecolor_count: AtomicU64::new(0),
            screen_tile_palette_saved_bytes: AtomicU64::new(0),
            screen_tile_predict_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTilePredictConfig::default(),
            )),
            screen_tile_predict_tiles_count: AtomicU64::new(0),
            screen_tile_predict_saved_bytes: AtomicU64::new(0),
            screen_tile_present_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTilePresentConfig::default(),
            )),
            screen_tile_present_committed_txs: AtomicU64::new(0),
            screen_tile_present_swapped_frames: AtomicU64::new(0),
            screen_tile_present_torn_prevented: AtomicU64::new(0),
            screen_tile_present_total_latency_us: AtomicU64::new(0),
            screen_tile_nack_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileNackConfig::default(),
            )),
            screen_tile_resend_buffer: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileResendBuffer::default(),
            )),
            screen_tile_nack_requests_sent: AtomicU64::new(0),
            screen_tile_nack_tiles_retransmitted: AtomicU64::new(0),
            screen_tile_nack_degraded_commits: AtomicU64::new(0),
            screen_tile_saliency_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileSaliencyConfig::default(),
            )),
            screen_tile_saliency_critical_tiles: AtomicU64::new(0),
            screen_tile_saliency_normal_tiles: AtomicU64::new(0),
            screen_tile_saliency_background_tiles: AtomicU64::new(0),
            screen_tile_saliency_background_skipped: AtomicU64::new(0),
            screen_tile_lod_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileLodConfig::default(),
            )),
            screen_tile_lod0_tiles: AtomicU64::new(0),
            screen_tile_lod1_tiles: AtomicU64::new(0),
            screen_tile_lod2_tiles: AtomicU64::new(0),
            screen_tile_lod_bytes_saved: AtomicU64::new(0),
            screen_tile_batch_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileBatchConfig::default(),
            )),
            screen_tile_batches_sent: AtomicU64::new(0),
            screen_tile_chunks_sent: AtomicU64::new(0),
            screen_tile_batch_bytes_total: AtomicU64::new(0),
            screen_tile_batch_pacing_pauses: AtomicU64::new(0),
            screen_tile_batch_receiver_state: Mutex::new(None),
            screen_tile_motion_config: std::sync::Arc::new(std::sync::RwLock::new(
                tetherly_core::ScreenTileMotionConfig::default(),
            )),
            screen_tile_motion_tiles_sent: AtomicU64::new(0),
            screen_tile_motion_fallback_tiles_sent: AtomicU64::new(0),
            screen_tile_motion_bytes_saved: AtomicU64::new(0),
            screen_annotation_count: AtomicU64::new(0),
            screen_laser_updates: AtomicU64::new(0),
            screen_client_out: AsyncMutex::new(None),
            screen_active_display_id: AtomicU8::new(initial_display_id),
            screen_displays: Mutex::new(initial_displays),
            screen_display_switches_count: AtomicU64::new(0),
            screen_last_display_switch_us: AtomicU64::new(0),
            screen_privacy_enabled: AtomicBool::new(false),
            screen_privacy_lock_input: AtomicBool::new(false),
            screen_privacy_blanking_method: Mutex::new("virtual_overlay".to_string()),
            screen_privacy_last_change_us: AtomicU64::new(0),
            screen_damage_tracker: Mutex::new(ScreenDamageTracker::new()),
            screen_prev_frame: Mutex::new(None),
            shutdown,
            overlay: Mutex::new(initial_overlay),
            ancs: Mutex::new(ancs),
            ancs_source,
            allowlist: Mutex::new(OpenAllowlist::default()),
            started_at: std::time::Instant::now(),
            node_name,
            advertise,
            reconnect,
            advertise_stop: Mutex::new(None),
            telemetry: Arc::new(TrafficTelemetry::new()),
            local_caps: Mutex::new({
                let mut s = std::collections::HashSet::new();
                s.insert("notify".to_string());
                s.insert("clip".to_string());
                s.insert("file".to_string());
                s.insert("input".to_string());
                s.insert("screen".to_string());
                s.insert("handoff".to_string());
                s.insert("media".to_string());
                s.insert("power".to_string());
                s
            }),
            blocked: Mutex::new(initial_blocked),
            auto_accept_trusted: AtomicBool::new(initial_auto_accept.enabled),
            auto_accepted_count: AtomicU64::new(0),
            discovered: Mutex::new(HashMap::new()),
            rescan_trigger,
            opener: Mutex::new(Arc::new(crate::platform::SystemOpener)),
            notifier: Mutex::new(notifier),
            notify_config: Mutex::new(NotifyBannerConfig::default()),
            notify_dispatched_count: AtomicU64::new(0),
            notify_suppressed_count: AtomicU64::new(0),
            sys_probe: Mutex::new(Arc::new(crate::platform::PlatformSysProbe)),
            heartbeat_config: Mutex::new(HeartbeatConfig::default()),
            link_trackers: Mutex::new(HashMap::new()),
            audit_engine: Mutex::new(AuditEngine::new(500)),
            message_engine: Mutex::new(MessageEngine::new(200)),
            outbox_engine: Mutex::new(OutboxEngine::new(200)),
            profiles: Mutex::new(initial_profiles),
            auto_accept_policy: Mutex::new(initial_auto_accept),
            clipboard_policy: Mutex::new(initial_clipboard_policy),
            clip_auto_synced_out_count: AtomicU64::new(0),
            clip_auto_synced_in_count: AtomicU64::new(0),
            clip_sensitive_blocked_count: AtomicU64::new(0),
            last_auto_sync_hash: Mutex::new(None),
            handoff_policy: Mutex::new(initial_handoff_policy),
            handoff_inbox: Mutex::new(initial_handoff_inbox),
            handoff_seq: AtomicU64::new(0),
            media_policy: Mutex::new(initial_media_policy),
            local_media_state: Mutex::new(MediaStateFrame {
                player_id: "default".into(),
                state: PlaybackState::Stopped,
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                position_ms: 0,
                duration_ms: 0,
                volume_percent: 70,
                muted: false,
                can_play: true,
                can_pause: false,
                can_go_next: true,
                can_go_previous: true,
                can_seek: true,
                timestamp_ms: 0,
            }),
            peer_media_states: Mutex::new(HashMap::new()),
            media_cmd_seq: AtomicU64::new(0),
            battery_policy: Mutex::new(initial_battery_policy),
            local_battery_state: Mutex::new(BatteryStateFrame {
                battery_level: 100,
                is_charging: false,
                power_source: PowerSource::Battery,
                battery_saver: false,
                health_percent: None,
                timestamp_ms: 0,
            }),
            peer_battery_states: Mutex::new(HashMap::new()),
            locator_policy: Mutex::new(initial_locator_policy),
            active_locate_ring: Mutex::new(None),
            peer_last_locate_req_ms: Mutex::new(HashMap::new()),
            locate_req_seq: AtomicU64::new(0),
            local_actions: Mutex::new(initial_local_actions),
            action_policy: Mutex::new(initial_action_policy),
            peer_remote_actions: Mutex::new(HashMap::new()),
            peer_last_action_trigger_ms: Mutex::new(HashMap::new()),
            action_trigger_seq: AtomicU64::new(0),
            action_results_cache: Mutex::new(VecDeque::with_capacity(100)),
            focus_policy: Mutex::new(initial_focus_policy),
            local_focus_state: Mutex::new(FocusStateFrame {
                enabled: false,
                mode: FocusMode::Dnd,
                expires_ms: None,
                reason: None,
                timestamp_ms: 0,
            }),
            peer_focus_states: Mutex::new(HashMap::new()),
            topology_layout: Mutex::new(initial_topology_layout),
            topology_policy: Mutex::new(initial_topology_policy),
            peer_topologies: Mutex::new(HashMap::new()),
            cursor_last_pos: Mutex::new((0, 0)),
            input_policy: Mutex::new(initial_input_policy),
            active_input_tunnel: Mutex::new(None),
            remote_session: Mutex::new(None),
            remote_session_keymap: Mutex::new(None),
            remote_session_auto_allowed: std::sync::atomic::AtomicBool::new(false),
            session_heartbeat_paused: std::sync::atomic::AtomicBool::new(false),
            gesture_swipe_last_ignored_ms: std::sync::atomic::AtomicU64::new(0),
            gesture_map: Mutex::new(initial_gesture_map),
            screen_seq_trace: Mutex::new(Vec::new()),
            input_trace: Mutex::new(VecDeque::with_capacity(super::input::INPUT_TRACE_CAP)),
            screen_channel_gen: std::sync::atomic::AtomicU64::new(0),
            screen_client_gen: std::sync::atomic::AtomicU64::new(0),
            input_client_task: Mutex::new(None),
            input_dialing: std::sync::atomic::AtomicBool::new(false),
            input_attach_kick_at: Mutex::new(None),
            input_attaching: std::sync::atomic::AtomicBool::new(false),
            last_auto_reverse: Mutex::new(None),
            reverse_arm_since: Mutex::new(None),
            reverse_last_tick: Mutex::new(None),
            reverse_pinned_touch: Mutex::new(None),
            reverse_zone_dirty: std::sync::atomic::AtomicBool::new(false),
            reverse_move_accum_px: std::sync::atomic::AtomicI64::new(0),
            roaming_own_crossing: std::sync::atomic::AtomicBool::new(false),
            reverse_last_move: Mutex::new(None),
            reverse_last_pos: Mutex::new(None),
            rt_handle: Mutex::new(tokio::runtime::Handle::try_current().ok()),
            virtual_display_policy: Mutex::new(initial_virtual_display_policy),
            active_virtual_display: Mutex::new(None),
            audio_routing_policy: Mutex::new(initial_audio_routing_policy),
            active_audio_routing: Mutex::new(None),
            camera_policy: Mutex::new(initial_camera_policy),
            active_camera: Mutex::new(None),
            microphone_policy: Mutex::new(initial_microphone_policy),
            active_microphone: Mutex::new(None),
            bench_policy: Mutex::new(initial_bench_policy),
            active_bench: Mutex::new(None),
            last_bench_report: Mutex::new(None),
            bandwidth_profiles: Mutex::new(initial_bandwidth_profiles),
            drag_controller: Mutex::new(DragController::new()),
            drag_target: Mutex::new(DragTarget::new()),
            pending_drag_drops: Mutex::new(HashMap::new()),
            transfer_drag_ids: Mutex::new(HashMap::new()),
        });
        Ok(Self { shared })
    }

    pub fn emit(&self, ev: UiEvent) {
        let _ = self.shared.events.send(ev);
    }

    /// Subscribe to the shutdown signal.
    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shared.shutdown.subscribe()
    }

    pub fn clone_handle(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        self.spawn_listeners().await?;
        if self.shared.cfg.ui_port != 0 {
            // Bind before returning so the port is real the moment `run()`
            // reports the node up; `tetherly pin` reaches it via this port.
            self.spawn_ui().await.map_err(|e| {
                anyhow::anyhow!("ui port {} unavailable: {e}", self.shared.cfg.ui_port)
            })?;
        }
        // Pinned-dwell heartbeat: completes the reverse-takeover dwell while
        // the OS stops reporting moves on a clamped edge push (v0.2.25).
        self.spawn_reverse_pinned_heartbeat();
        info!(
            device_id = %self.shared.identity.device_id(),
            "node running"
        );
        let mut stop = self.shared.shutdown.subscribe();
        while !*stop.borrow() {
            if stop.changed().await.is_err() {
                break;
            }
        }
        info!(
            device_id = %self.shared.identity.device_id(),
            "node shutdown complete"
        );
        Ok(())
    }

    /// Bind and serve the loopback UI API, recording the port actually bound (a
    /// `cfg.ui_port` of 0 resolves to an ephemeral one). Callers that need the
    /// UI before `run()` — integration tests driving `/api/pin` — call this
    /// directly; `run()` calls it when a concrete port is configured.
    pub async fn spawn_ui(&self) -> std::io::Result<u16> {
        let bound =
            crate::uihttp::bind_and_spawn(self.clone_handle(), self.shared.cfg.ui_port).await?;
        self.shared.bound_ui.store(bound, Ordering::SeqCst);
        Ok(bound)
    }

    pub async fn spawn_listeners(&self) -> anyhow::Result<()> {
        let addrs = if self.shared.cfg.loopback_only {
            vec![SocketAddr::from((
                [127, 0, 0, 1],
                self.shared.cfg.control_port,
            ))]
        } else {
            control_bind_addrs(self.shared.cfg.control_port)
        };
        let listeners = bind_many(&addrs).await?;
        if let Some(first) = listeners.first() {
            self.shared
                .bound_control
                .store(first.local_addr()?.port(), Ordering::SeqCst);
        }

        macro_rules! data_listeners {
            ($port:expr, $bound:expr) => {{
                let port = $port;
                let addrs = if self.shared.cfg.loopback_only {
                    vec![SocketAddr::from(([127, 0, 0, 1], port))]
                } else {
                    control_bind_addrs(port)
                };
                let ls = bind_many_optional(&addrs).await;
                if let Some(a) = ls.first().and_then(|l| l.local_addr().ok()) {
                    $bound.store(a.port(), Ordering::SeqCst);
                }
                ls
            }};
        }
        let file_listeners = data_listeners!(self.shared.cfg.file_port, self.shared.bound_file);
        let input_listeners = data_listeners!(self.shared.cfg.input_port, self.shared.bound_input);
        let screen_listeners =
            data_listeners!(self.shared.cfg.screen_port, self.shared.bound_screen);

        // One startup line per address left out of the LAN set, so a user whose
        // only path is a CGNAT overlay (Tailscale/NetBird) can see why nothing
        // was discovered instead of guessing.
        crate::lan::log_skipped_interfaces();
        if self.is_advertise_enabled() {
            self.advertise_mdns();
        }
        self.spawn_discovery_browser();
        if self.shared.cfg.overlay.enabled {
            self.spawn_overlay();
        }
        self.spawn_reconnect();
        self.spawn_clipboard_auto_sync();

        for l in listeners {
            node_accept_loop(self.clone_handle(), l, "control", |node, s, a| async move {
                node.accept_one(s, a).await
            });
        }
        for l in file_listeners {
            node_accept_loop(self.clone_handle(), l, "file", |node, s, a| async move {
                node.accept_file(s, a).await
            });
        }
        for l in input_listeners {
            node_accept_loop(self.clone_handle(), l, "input", |node, s, a| async move {
                node.accept_input(s, a).await
            });
        }
        for l in screen_listeners {
            node_accept_loop(self.clone_handle(), l, "screen", |node, s, a| async move {
                node.accept_screen(s, a).await
            });
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let _ = self.shared.shutdown.send(true);
        if let Some(tx) = self
            .shared
            .advertise_stop
            .lock()
            .expect("advertise_stop")
            .take()
        {
            let _ = tx.send(true);
        }
        if let Ok(mut sessions) = self.shared.sessions.try_lock() {
            sessions.clear();
        }
        self.shared.live.lock().expect("live").clear();
        self.shared
            .ping_waiters
            .lock()
            .expect("ping_waiters")
            .clear();
    }

    pub fn restart_daemon(
        &self,
        replacement_path: Option<&str>,
        delay_ms: Option<u64>,
    ) -> Result<crate::uihttp::DaemonRestartReport, String> {
        // Single-flight gate: two concurrent restarts would interleave the
        // rename/copy sequence and can leave a corrupt or missing daemon
        // binary. The second caller fails fast instead.
        if RESTART_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("restart already in progress".to_string());
        }
        let result = self.restart_daemon_inner(replacement_path, delay_ms);
        RESTART_IN_PROGRESS.store(false, Ordering::SeqCst);
        result
    }

    fn restart_daemon_inner(
        &self,
        replacement_path: Option<&str>,
        delay_ms: Option<u64>,
    ) -> Result<crate::uihttp::DaemonRestartReport, String> {
        let old_pid = std::process::id();
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("cannot determine current executable path: {e}"))?;

        let action = if let Some(rep) = replacement_path {
            let rep_path = std::path::PathBuf::from(rep);
            if !rep_path.is_file() {
                return Err(format!("replacement binary not found at {:?}", rep_path));
            }
            // Security gate: the replacement must live inside the daemon's
            // own update staging directory. A loopback HTTP caller must never
            // be able to point the daemon at an arbitrary executable on disk
            // and have it replace (and run as) the daemon binary.
            let staging = update_staging_dir(&self.shared.cfg.data_dir);
            let canonical = rep_path
                .canonicalize()
                .map_err(|e| format!("cannot resolve replacement path {rep_path:?}: {e}"))?;
            if !canonical.starts_with(&staging) {
                return Err(format!(
                    "replacement binary must be placed inside the update staging directory {:?}",
                    staging
                ));
            }

            // On Windows, running binaries are write-locked, but NTFS allows
            // renaming a running binary. Sequence: rename the running binary
            // aside, atomically rename the replacement into place, and roll
            // back to the backup on any failure so a half-updated state can
            // never persist.
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            #[cfg(windows)]
            let old_backup = current_exe.with_extension(format!("exe.old.{timestamp}"));
            #[cfg(not(windows))]
            let old_backup = current_exe.with_extension(format!("old.{timestamp}"));

            std::fs::rename(&current_exe, &old_backup).map_err(|e| {
                format!("failed to rename current running binary to {old_backup:?}: {e}")
            })?;

            let staging_tmp = current_exe.with_extension("new.tmp");
            let placed = std::fs::copy(&canonical, &staging_tmp)
                .map_err(|e| format!("failed to stage replacement binary: {e}"))
                .and_then(|_| {
                    if let Ok(f) = std::fs::File::open(&staging_tmp) {
                        let _ = f.sync_all();
                    }
                    std::fs::rename(&staging_tmp, &current_exe).map_err(|e| {
                        format!("failed to place replacement binary at {current_exe:?}: {e}")
                    })
                });
            if let Err(e) = placed {
                rollback_binary(&old_backup, &current_exe);
                return Err(e);
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&current_exe, std::fs::Permissions::from_mode(0o755));
            }

            // Readiness preflight: run the new binary in a cheap `--help`
            // mode before the old daemon schedules its own exit. A broken
            // build (bad link, missing runtime, corrupt image) fails here
            // while the old daemon is still alive to serve requests.
            if let Err(e) = preflight_new_binary(&current_exe) {
                rollback_binary(&old_backup, &current_exe);
                return Err(format!("replacement binary failed preflight: {e}"));
            }

            format!("replaced_and_restarted (from {})", canonical.display())
        } else {
            "restarted".to_string()
        };

        let mut cmd = std::process::Command::new(&current_exe);
        cmd.args(std::env::args_os().skip(1));
        cmd.stdin(std::process::Stdio::null());
        // Detach the new daemon from this process's console/job so a parent
        // console close or job teardown cannot take the replacement with it.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    // New session: immune to terminal SIGHUP propagation.
                    let _ = libc::setsid();
                    Ok(())
                });
            }
        }

        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn new daemon process {:?}: {e}", current_exe))?;
        let new_pid = child.id();

        let node = self.clone_handle();
        // Clamp the hand-off window: the port bind retry budget on the new
        // daemon is ~1s (20 x 50ms), so a much larger delay would guarantee
        // the new process loses the bind race and dies while this one lives
        // on, reporting a restart that never actually happened.
        let wait_ms = delay_ms.unwrap_or(50).min(1000);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
            node.shutdown();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            std::process::exit(0);
        });

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(crate::uihttp::DaemonRestartReport {
            ok: true,
            action,
            old_pid,
            new_pid,
            message: format!(
                "daemon spawned successfully with PID {new_pid}; terminating parent PID {old_pid}"
            ),
            timestamp_ms: now_ms,
        })
    }

    pub(super) fn attach<'a>(
        &'a self,
        mut sess: ActiveSession,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let peer_id = sess.peer_id.clone();
            let name = sess.peer_hello.name.clone();
            let platform = sess.peer_hello.platform.clone();
            let addr = sess.peer_addr;
            let incoming = path_kind(addr, &self.shared.cfg.overlay.cidrs);
            {
                let live = self.shared.live.lock().expect("live");
                if let Some(cur) = live.get(&peer_id) {
                    if cur.path == PathKind::Lan && incoming == PathKind::Overlay {
                        debug!(peer = %peer_id, %addr, "keep LAN; drop overlay attach");
                        return;
                    }
                }
            }
            {
                let mut live = self.shared.live.lock().expect("live");
                live.insert(
                    peer_id.clone(),
                    LivePeer {
                        device_id: peer_id.clone(),
                        name: name.clone(),
                        platform: platform.clone(),
                        addr,
                        handshake_hash: *sess.handshake_hash(),
                        file_port: None,
                        input_port: None,
                        screen_port: None,
                        caps: Vec::new(),
                        path: incoming,
                        rtt_ms: None,
                        last_seen_ms: self.shared.clock.unix_ms(),
                    },
                );
            }
            self.emit(peer_up(&peer_id, &name, &platform));
            match incoming {
                PathKind::Lan => info!(peer = %peer_id, %addr, "path lan"),
                PathKind::Overlay => info!(peer = %peer_id, %addr, "path overlay"),
            }
            let (tx, mut rx) = mpsc::channel::<InnerFrame>(SESSION_QUEUE_DEPTH);
            {
                self.shared
                    .sessions
                    .lock()
                    .await
                    .insert(peer_id.clone(), tx.clone());
            }
            let file_port = self.file_port();
            let input_port = self.input_port();
            let screen_port = self.screen_port();
            let mut initial_caps: Vec<String> = {
                let local = self.shared.local_caps.lock().expect("local_caps");
                let mut list: Vec<String> = local.iter().cloned().collect();
                list.sort();
                list
            };
            initial_caps.push(format!("fileport={file_port}"));
            initial_caps.push(format!("inputport={input_port}"));

            initial_caps.push("screencodecs=zstd,h264".to_string());
            initial_caps.push(format!("screenport={screen_port}"));
            let my_lan_ips = crate::lan::lan_ips()
                .into_iter()
                .filter(|ip| !ip.is_loopback())
                .map(|ip| ip.to_string())
                .collect::<Vec<_>>()
                .join(",");
            if !my_lan_ips.is_empty() {
                initial_caps.push(format!("lanips={my_lan_ips}"));
            }
            let caps = CapsUpdate { caps: initial_caps };
            if let Ok(frame) = caps.to_frame() {
                let _ = tx.try_send(frame);
            }
            let node = self.clone_handle();
            let clock = self.shared.clock.clone();
            let mut stop = self.shared.shutdown.subscribe();
            let sess_addr = addr;
            // Keepalive: a ping every heartbeat keeps the NAT mapping warm and, more
            // importantly, gives the peer something to answer, so `Liveness` can tell
            // a merely-quiet session from a dead one. The task ends when the session
            // task drops its receiver (try_send then reports Closed) or on shutdown.
            let keep_tx = tx.clone();
            let keep_clock = clock.clone();
            let mut keep_stop = self.shared.shutdown.subscribe();
            let keep_node = self.clone_handle();
            let keep_peer = peer_id.clone();
            tokio::spawn(async move {
                loop {
                    let interval_secs = keep_node
                        .shared
                        .heartbeat_config
                        .lock()
                        .expect("heartbeat")
                        .interval_secs
                        .max(1);
                    tokio::select! {
                        _ = keep_stop.changed() => {
                            if *keep_stop.borrow() {
                                break;
                            }
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_secs(interval_secs)) => {
                            let now_ms = keep_clock.unix_ms();
                            match keep_tx.try_send(ping(now_ms)) {
                                Ok(()) => {
                                    let mut trackers =
                                        keep_node.shared.link_trackers.lock().expect("link_trackers");
                                    trackers
                                        .entry(keep_peer.clone())
                                        .or_insert_with(|| PeerLinkTracker::new(now_ms))
                                        .record_ping_sent(now_ms);
                                }
                                // A full queue is backpressure, not a dead peer.
                                Err(mpsc::error::TrySendError::Full(_)) => {}
                                Err(mpsc::error::TrySendError::Closed(_)) => break,
                            }
                        }
                    }
                }
            });
            self.shared.telemetry.record_session_established();
            tokio::spawn(async move {
                let hb = node.shared.heartbeat_config.lock().expect("hb").clone();
                let mut live = Liveness::with(
                    std::time::Duration::from_secs(hb.interval_secs.max(1)),
                    std::time::Duration::from_secs(hb.deadline_secs.max(hb.interval_secs)),
                    clock.unix_ms(),
                );
                loop {
                    let current_deadline = node
                        .shared
                        .heartbeat_config
                        .lock()
                        .expect("hb")
                        .deadline_secs
                        .max(1);
                    if live.deadline().as_secs() != current_deadline {
                        live.set_deadline(std::time::Duration::from_secs(current_deadline));
                    }
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        outbound = rx.recv() => {
                            match outbound {
                                Some(frame) => {
                                    if live.is_expired(clock.unix_ms()) {
                                        warn!(peer = %peer_id, "session liveness lost");
                                        break;
                                    }
                                    let frame_len = (frame.payload.len() + 12) as u64;
                                    let frame_ty = frame.ty;
                                    if let Err(e) = sess.send_bounded(frame).await {
                                        warn!(%e, "send");
                                        break;
                                    }
                                    node.shared.telemetry.record_control_tx(frame_len, frame_ty);
                                }
                                None => break,
                            }
                        }
                        inbound = sess.recv_event() => {
                            match inbound {
                                Ok(ev) => {
                                    live.on_inbound(clock.unix_ms());
                                    {
                                        let mut live_map = node.shared.live.lock().expect("live");
                                        if let Some(p) = live_map.get_mut(&peer_id) {
                                            p.last_seen_ms = clock.unix_ms();
                                        }
                                    }
                                    if let Err(e) = node.handle_event(&peer_id, ev, &mut sess).await {
                                        warn!(%e, "event");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    debug!(%e, "recv end");
                                    break;
                                }
                            }
                        }
                    }
                }
                node.shared.telemetry.record_session_closed();
                let still_mine = node
                    .shared
                    .live
                    .lock()
                    .expect("live")
                    .get(&peer_id)
                    .map(|p| p.addr)
                    == Some(sess_addr);
                if still_mine {
                    node.shared.sessions.lock().await.remove(&peer_id);
                    node.shared.live.lock().expect("live").remove(&peer_id);
                    node.shared
                        .ping_waiters
                        .lock()
                        .expect("ping_waiters")
                        .remove(&peer_id);
                    *node.shared.input_seat.lock().expect("seat") = None;
                    // Drag reverse path: a peer going down must not leave
                    // pending drop registrations or an in-flight target
                    // session behind — a stale registration would mis-claim
                    // an arbitrary later FileOffer from the (re)connected
                    // peer and apply MOTW + reveal to a normal transfer.
                    {
                        let mut pending = node
                            .shared
                            .pending_drag_drops
                            .lock()
                            .expect("pending_drag_drops");
                        let stale_ids: Vec<u64> = pending
                            .iter()
                            .filter(|(_, p)| p.peer == peer_id)
                            .map(|(id, _)| *id)
                            .collect();
                        for id in &stale_ids {
                            pending.remove(id);
                        }
                        let active_drag_id = node
                            .shared
                            .drag_target
                            .lock()
                            .expect("drag_target")
                            .active_session()
                            .map(|s| s.drag_id);
                        if active_drag_id.is_some_and(|id| stale_ids.contains(&id)) {
                            node.shared.drag_target.lock().expect("drag_target").reset();
                        }
                    }
                    node.emit(UiEvent::PeerDown {
                        device_id: peer_id.to_string(),
                    });
                }
            });
        })
    }

    pub(super) async fn handle_event(
        &self,
        peer: &DeviceId,
        ev: SessionEvent,
        sess: &mut ActiveSession,
    ) -> Result<(), tetherly_net::NetError> {
        let (ev_bytes, ev_ty) = match &ev {
            SessionEvent::Ping(_) => (20u64, tetherly_core::frame::TYPE_PING),
            SessionEvent::Pong(_) => (20u64, tetherly_core::frame::TYPE_PONG),
            SessionEvent::Notify(p) => (
                (p.app_id.len() + p.title.len() + p.body.len() + 32) as u64,
                tetherly_core::frame::TYPE_NOTIFY_PUSH,
            ),
            SessionEvent::Dismiss(_) => (24u64, tetherly_core::frame::TYPE_NOTIFY_DISMISS),
            SessionEvent::Reply(r) => (
                (r.app_id.len() + r.text.len() + 32) as u64,
                tetherly_core::frame::TYPE_NOTIFY_REPLY,
            ),
            SessionEvent::Clip(c) => (
                (c.text.as_ref().map_or(0, |s| s.len()) + 32) as u64,
                tetherly_core::frame::TYPE_CLIP_SET,
            ),
            SessionEvent::TextMessage(m) => (
                (m.text.len() + 32) as u64,
                tetherly_core::frame::TYPE_TEXT_MESSAGE,
            ),
            SessionEvent::ResourceHandoff(h) => (
                (h.url.len() + h.title.len() + 32) as u64,
                tetherly_core::frame::TYPE_RESOURCE_HANDOFF,
            ),
            SessionEvent::MediaCommand(c) => (
                (c.id.len() + 32) as u64,
                tetherly_core::frame::TYPE_MEDIA_COMMAND,
            ),
            SessionEvent::MediaState(s) => (
                (s.title.len() + s.artist.len() + s.album.len() + 48) as u64,
                tetherly_core::frame::TYPE_MEDIA_STATE,
            ),
            SessionEvent::BatteryState(_) => (48u64, tetherly_core::frame::TYPE_BATTERY_STATE),
            SessionEvent::LocateRequest(r) => (
                (r.request_id.len() + r.message.as_ref().map(|m| m.len()).unwrap_or(0) + 48) as u64,
                tetherly_core::frame::TYPE_LOCATE_REQUEST,
            ),
            SessionEvent::LocateStop(s) => (
                (s.request_id.as_ref().map(|r| r.len()).unwrap_or(0) + s.reason.len() + 32) as u64,
                tetherly_core::frame::TYPE_LOCATE_STOP,
            ),
            SessionEvent::FileOffer(f) => (
                (f.transfer_id.len()
                    + f.files.iter().map(|item| item.name.len()).sum::<usize>()
                    + 64) as u64,
                tetherly_core::frame::TYPE_FILE_OFFER,
            ),
            SessionEvent::FileAccept(_) => (32u64, tetherly_core::frame::TYPE_FILE_ACCEPT),
            SessionEvent::FileReject(_) => (32u64, tetherly_core::frame::TYPE_FILE_REJECT),
            SessionEvent::FileDone(_) => (32u64, tetherly_core::frame::TYPE_FILE_DONE),
            SessionEvent::DragControl(_) => (64u64, tetherly_core::frame::TYPE_DRAG_CONTROL),
            SessionEvent::Caps(_) => (32u64, tetherly_core::frame::TYPE_CAPS_UPDATE),
            SessionEvent::ActionListReq(r) => (
                (r.request_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_ACTION_LIST_REQ,
            ),
            SessionEvent::ActionListResp(r) => (
                (r.request_id.len() + r.actions.len() * 64 + 32) as u64,
                tetherly_core::frame::TYPE_ACTION_LIST_RESP,
            ),
            SessionEvent::ActionTrigger(t) => (
                (t.trigger_id.len() + t.action_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_ACTION_TRIGGER,
            ),
            SessionEvent::ActionResult(res) => (
                (res.trigger_id.len()
                    + res.action_id.len()
                    + res.output_summary.as_ref().map(|s| s.len()).unwrap_or(0)
                    + res.error_message.as_ref().map(|s| s.len()).unwrap_or(0)
                    + 48) as u64,
                tetherly_core::frame::TYPE_ACTION_RESULT,
            ),
            SessionEvent::FocusState(st) => (
                (st.reason.as_ref().map(|s| s.len()).unwrap_or(0) + 32) as u64,
                tetherly_core::frame::TYPE_FOCUS_STATE,
            ),
            SessionEvent::FocusSyncReq(req) => (
                (req.request_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_FOCUS_SYNC_REQ,
            ),
            SessionEvent::TopologyState(st) => (
                (st.device_id.len() + st.neighbors.len() * 48 + 64) as u64,
                tetherly_core::frame::TYPE_TOPOLOGY_STATE,
            ),
            SessionEvent::TopologySyncReq(req) => (
                (req.request_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_TOPOLOGY_SYNC_REQ,
            ),
            SessionEvent::InputTunnelRequest(req) => (
                (req.session_id.len() + req.client_device_id.len() + 48) as u64,
                tetherly_core::frame::TYPE_INPUT_TUNNEL_REQUEST,
            ),
            SessionEvent::InputTunnelResp(resp) => (
                (resp.session_id.len() + resp.reject_reason.as_ref().map_or(0, |s| s.len()) + 48)
                    as u64,
                tetherly_core::frame::TYPE_INPUT_TUNNEL_RESP,
            ),
            SessionEvent::InputTunnelRelease(rel) => (
                (rel.session_id.len() + rel.reason.len() + 48) as u64,
                tetherly_core::frame::TYPE_INPUT_TUNNEL_RELEASE,
            ),
            SessionEvent::InputTunnelHeartbeat(hb) => (
                (hb.session_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_INPUT_TUNNEL_HEARTBEAT,
            ),
            SessionEvent::RemoteSessionRequest(req) => (
                (req.session_id.len() + req.client_device_id.len() + 48) as u64,
                tetherly_core::frame::TYPE_REMOTE_SESSION_REQUEST,
            ),
            SessionEvent::RemoteSessionResp(resp) => (
                (resp.session_id.len() + resp.reject_reason.as_ref().map_or(0, |s| s.len()) + 64)
                    as u64,
                tetherly_core::frame::TYPE_REMOTE_SESSION_RESP,
            ),
            SessionEvent::RemoteSessionClose(close) => (
                (close.session_id.len() + close.reason.len() + 48) as u64,
                tetherly_core::frame::TYPE_REMOTE_SESSION_CLOSE,
            ),
            SessionEvent::VirtualDisplayAnnounce(ann) => (
                (ann.display_name.len() + 64) as u64,
                tetherly_core::frame::TYPE_VIRTUAL_DISPLAY_ANNOUNCE,
            ),
            SessionEvent::VirtualDisplayAttach(att) => (
                (att.session_id.len() + att.peer_id.len() + 64) as u64,
                tetherly_core::frame::TYPE_VIRTUAL_DISPLAY_ATTACH,
            ),
            SessionEvent::VirtualDisplayDetach(det) => (
                (det.session_id.len() + det.reason.as_ref().map_or(0, |r| r.len()) + 48) as u64,
                tetherly_core::frame::TYPE_VIRTUAL_DISPLAY_DETACH,
            ),
            SessionEvent::VirtualDisplayFrameAck(ack) => (
                (ack.session_id.len() + 32) as u64,
                tetherly_core::frame::TYPE_VIRTUAL_DISPLAY_FRAME_ACK,
            ),
            SessionEvent::AudioRoutingAnnounce(ann) => (
                (ann.device_name.len() + 64) as u64,
                tetherly_core::frame::TYPE_AUDIO_ROUTING_ANNOUNCE,
            ),
            SessionEvent::AudioRoutingAttach(att) => (
                (att.session_id.len() + att.source_id.len() + 64) as u64,
                tetherly_core::frame::TYPE_AUDIO_ROUTING_ATTACH,
            ),
            SessionEvent::AudioRoutingDetach(det) => (
                (det.session_id.len() + det.reason.as_ref().map_or(0, |r| r.len()) + 48) as u64,
                tetherly_core::frame::TYPE_AUDIO_ROUTING_DETACH,
            ),
            SessionEvent::AudioRoutingChunk(chunk) => (
                (chunk.payload_bytes.len() + 32) as u64,
                tetherly_core::frame::TYPE_AUDIO_ROUTING_CHUNK,
            ),
            SessionEvent::CameraAnnounce(ann) => (
                (ann.device_name.len() + 64) as u64,
                tetherly_core::frame::TYPE_CAMERA_ANNOUNCE,
            ),
            SessionEvent::CameraAttach(att) => (
                (att.session_id.len() + att.peer_id.len() + 64) as u64,
                tetherly_core::frame::TYPE_CAMERA_ATTACH,
            ),
            SessionEvent::CameraDetach(det) => (
                (det.session_id.len() + det.reason.as_ref().map_or(0, |r| r.len()) + 48) as u64,
                tetherly_core::frame::TYPE_CAMERA_DETACH,
            ),
            SessionEvent::CameraFrameChunk(chunk) => (
                (chunk.data.len() + 32) as u64,
                tetherly_core::frame::TYPE_CAMERA_FRAME_CHUNK,
            ),
            SessionEvent::MicAnnounce(ann) => (
                (ann.device_name.len() + 64) as u64,
                tetherly_core::frame::TYPE_MIC_ANNOUNCE,
            ),
            SessionEvent::MicAttach(att) => (
                (att.session_id.len() + att.peer_id.len() + 64) as u64,
                tetherly_core::frame::TYPE_MIC_ATTACH,
            ),
            SessionEvent::MicDetach(det) => (
                (det.session_id.len() + det.reason.as_ref().map_or(0, |r| r.len()) + 48) as u64,
                tetherly_core::frame::TYPE_MIC_DETACH,
            ),
            SessionEvent::MicAudioChunk(chunk) => (
                (chunk.data.len() + 32) as u64,
                tetherly_core::frame::TYPE_MIC_AUDIO_CHUNK,
            ),
            SessionEvent::BenchRequest(req) => (
                (req.session_id.len() + req.peer_id.len() + 48) as u64,
                tetherly_core::frame::TYPE_BENCH_REQUEST,
            ),
            SessionEvent::BenchAccept(acc) => (
                (acc.session_id.len() + 48) as u64,
                tetherly_core::frame::TYPE_BENCH_ACCEPT,
            ),
            SessionEvent::BenchData(data) => (
                (data.data.len() + 32) as u64,
                tetherly_core::frame::TYPE_BENCH_DATA,
            ),
            SessionEvent::BenchReport(rep) => (
                (rep.session_id.len() + 96) as u64,
                tetherly_core::frame::TYPE_BENCH_REPORT,
            ),
            SessionEvent::Unknown(ty) => (32u64, *ty),
        };
        self.shared.telemetry.record_control_rx(ev_bytes, ev_ty);
        match ev {
            SessionEvent::Ping(ts) => {
                let p = pong(ts);
                let p_len = (p.payload.len() + 12) as u64;
                sess.send_bounded(p).await?;
                self.shared
                    .telemetry
                    .record_control_tx(p_len, tetherly_core::frame::TYPE_PONG);
            }
            SessionEvent::Pong(ts) => {
                let now_ms = self.shared.clock.unix_ms();
                let rtt = now_ms.saturating_sub(ts);
                {
                    let mut live = self.shared.live.lock().expect("live");
                    if let Some(p) = live.get_mut(peer) {
                        p.rtt_ms = Some(rtt);
                        p.last_seen_ms = now_ms;
                    }
                }
                self.emit(UiEvent::PeerLatency {
                    device_id: peer.to_string(),
                    rtt_ms: rtt,
                });
                let quality_event = {
                    let mut trackers = self.shared.link_trackers.lock().expect("link_trackers");
                    let tracker = trackers
                        .entry(peer.clone())
                        .or_insert_with(|| PeerLinkTracker::new(now_ms));
                    tracker.record_pong_recv(rtt);
                    let min_rtt_ms = tracker.min_rtt();
                    let max_rtt_ms = tracker.max_rtt();
                    let avg_rtt_ms = tracker.avg_rtt();
                    let jitter_ms = tracker.jitter_ms();
                    let packet_loss_rate = tracker.packet_loss_rate();
                    let grade = tracker.evaluate_grade(true);
                    UiEvent::PeerLinkQualityRefreshed {
                        device_id: peer.to_string(),
                        rtt_ms: rtt,
                        min_rtt_ms,
                        max_rtt_ms,
                        avg_rtt_ms,
                        jitter_ms,
                        packet_loss_rate,
                        grade: format!("{:?}", grade).to_lowercase(),
                        timestamp_ms: now_ms,
                    }
                };
                self.emit(quality_event);
                let waiters = {
                    let mut pw = self.shared.ping_waiters.lock().expect("ping_waiters");
                    pw.remove(peer)
                };
                if let Some(senders) = waiters {
                    for tx in senders {
                        let _ = tx.send(rtt);
                    }
                }
            }
            SessionEvent::Notify(push) => {
                let _ = self.ingest_local_push(peer.clone(), push).await;
            }
            SessionEvent::Dismiss(d) => {
                let dismissed = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.dismiss_wire_candidates(peer, &d)
                };
                for c in dismissed {
                    self.emit(UiEvent::CandidateDismissed {
                        id: c.id,
                        source: c.source.to_string(),
                        app_id: c.app_id,
                        uid: c.uid,
                    });
                }
            }
            SessionEvent::Reply(reply) => {
                let text_len = reply.text.len();
                let accepted = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.ingest_reply(peer, reply.clone())
                };
                if accepted {
                    // Never log the text; only its length and identity.
                    info!(
                        source = %peer,
                        app_id = %reply.app_id,
                        uid = %reply.uid,
                        text_len,
                        "notify reply queued"
                    );
                    self.emit(UiEvent::ReplyReceived {
                        reply: crate::events::ReplyViewDto::from(ReplyView {
                            id: tetherly_core::notify::reply_id(peer, &reply.app_id, &reply.uid),
                            source: peer.clone(),
                            app_id: reply.app_id.clone(),
                            uid: reply.uid.clone(),
                        }),
                    });
                }
            }
            SessionEvent::Clip(clip) => {
                self.apply_clip(peer, clip)?;
            }
            SessionEvent::TextMessage(msg) => {
                self.receive_message(peer.as_str(), msg);
            }
            SessionEvent::FileOffer(offer) => {
                let total: u64 = offer.files.iter().map(|f| f.size).sum();
                let tid = offer.transfer_id.clone();
                let hint = {
                    let hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.resume.claim(peer, &tid, total)
                };
                if let Some(off) = hint {
                    info!(transfer = %tid, offset = off, "resuming interrupted transfer");
                }
                // A refused offer (unsafe name, bad digest, over `FILE_MAX`, or a
                // full candidate table) is a peer-side protocol fault, not a
                // session fault: log and drop this one offer, keep the session.
                let offered = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.files.offer_resumable(
                        peer.clone(),
                        offer,
                        self.shared.clock.unix_ms(),
                        hint,
                    )
                };
                match offered {
                    Ok(t) => {
                        // Claim a pending drag drop: if this inbound offer is the
                        // first file of a drag-originated drop, bind its
                        // transfer_id to the drag_id so the completion hook can
                        // settle MOTW + reveal.
                        self.claim_drag_offer(peer, &tid);
                        self.emit(file_offered(&t));
                        let is_auto_accept = {
                            let policy = self
                                .shared
                                .auto_accept_policy
                                .lock()
                                .expect("auto_accept_policy");
                            if policy.enabled && self.is_trusted(peer) {
                                let profile = self.peer_profile(peer);
                                policy.is_eligible(&profile, total)
                            } else {
                                false
                            }
                        };
                        if is_auto_accept {
                            if let Err(e) = self.user_accept_file(&tid).await {
                                warn!(%e, transfer = %tid, "auto-accept failed");
                            } else {
                                self.shared
                                    .auto_accepted_count
                                    .fetch_add(1, Ordering::Relaxed);
                                info!(
                                    transfer = %tid,
                                    "auto-accepted file from trusted peer based on policy"
                                );
                            }
                        }
                    }
                    Err(e) => warn!(%e, transfer = %tid, "refused file offer"),
                }
            }
            SessionEvent::FileAccept(d) => {
                debug!(transfer = %d.transfer_id, resume = ?d.resume_from, "peer accepted");
                self.on_peer_accepted_file(&d).await;
            }
            SessionEvent::FileReject(d) => {
                info!(transfer = %d.transfer_id, "peer rejected file");
                // Drop the buffered outbound bytes: without this the rejected
                // file's whole body stays in `pending_out` for the node's life.
                self.shared
                    .pending_out
                    .lock()
                    .expect("pending")
                    .remove(&d.transfer_id);
                let cancel_tx = self
                    .shared
                    .transfer_cancels
                    .lock()
                    .expect("cancels")
                    .remove(&d.transfer_id);
                if let Some(tx) = cancel_tx {
                    let _ = tx.send(());
                }
                {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.files.forget(&d.transfer_id);
                }
                self.set_transfer_phase(
                    &d.transfer_id,
                    TransferPhase::Failed,
                    Some("rejected by peer".into()),
                );
                self.emit(UiEvent::FileRejected {
                    transfer_id: d.transfer_id.clone(),
                    source: peer.to_string(),
                    reason: Some("rejected by peer".into()),
                });
            }
            SessionEvent::FileDone(done) => {
                let mut hubs = self.shared.hubs.lock().expect("hubs");
                match hubs.files.mark_done(&done) {
                    Ok(_) => {
                        self.set_transfer_phase(&done.transfer_id, TransferPhase::Completed, None);
                        self.emit(UiEvent::FileDone {
                            transfer_id: done.transfer_id,
                        });
                    }
                    Err(e) => warn!(%e, "file done"),
                }
            }
            SessionEvent::Caps(update) => {
                let file_port = parse_file_port(&update.caps);
                let input_port = parse_input_port(&update.caps);
                let screen_port = parse_screen_port(&update.caps);
                let features = extract_features(&update.caps);
                {
                    if let Some(live) = self.shared.live.lock().expect("live").get_mut(peer) {
                        if let Some(port) = file_port {
                            live.file_port = Some(port);
                        }
                        if let Some(port) = input_port {
                            live.input_port = Some(port);
                        }
                        if let Some(port) = screen_port {
                            live.screen_port = Some(port);
                        }
                        live.caps = update.caps.clone();
                    }
                }
                // ADR-079: the outbound screen encoder upgrades to H.264 only
                // when the peer advertises it. Cheap token scan, caps lists
                // are a dozen entries.
                if let Some(sel) = &self.shared.screen_encode_codec {
                    let supports = update.caps.iter().any(|c| {
                        c.split('=')
                            .nth(1)
                            .is_some_and(|v| v.split(',').any(|n| n == "h264"))
                            && c.starts_with("screencodecs=")
                    });
                    sel.set_peer_supports_h264(supports);
                }
                self.emit(UiEvent::PeerCapsChanged {
                    device_id: peer.to_string(),
                    caps: update.caps.clone(),
                    features,
                });
                if let Some(lan_ips_str) =
                    update.caps.iter().find_map(|c| c.strip_prefix("lanips="))
                {
                    let is_overlay = {
                        self.shared
                            .live
                            .lock()
                            .expect("live")
                            .get(peer)
                            .map(|p| p.path == PathKind::Overlay)
                            .unwrap_or(false)
                    };
                    if is_overlay {
                        let ips: Vec<String> = lan_ips_str
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let node = self.clone_handle();
                        let peer_cloned = peer.clone();
                        tokio::spawn(async move {
                            for ip_str in ips {
                                if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
                                    let target_addr =
                                        SocketAddr::new(ip, tetherly_net::CONTROL_PORT);
                                    if let Ok(Ok(stream)) = tokio::time::timeout(
                                        std::time::Duration::from_millis(200),
                                        tokio::net::TcpStream::connect(target_addr),
                                    )
                                    .await
                                    {
                                        let _ = stream.set_nodelay(true);
                                        info!(peer = %peer_cloned, %target_addr, "lan probe succeeded; promoting from overlay to direct lan");
                                        let mut cfg = node.session_config_inner(true);
                                        cfg.pin = None;
                                        if let Ok(sess) =
                                            tetherly_net::dial_session(stream, target_addr, &cfg)
                                                .await
                                        {
                                            node.attach(sess).await;
                                            if node.input_channel_open().await {
                                                let _ = node.open_input(&peer_cloned).await;
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        });
                    }
                }
            }
            SessionEvent::ResourceHandoff(h) => {
                self.receive_resource_handoff(peer, h);
            }
            SessionEvent::MediaCommand(cmd) => {
                self.receive_media_command(peer, cmd);
            }
            SessionEvent::MediaState(st) => {
                self.receive_media_state(peer, st);
            }
            SessionEvent::BatteryState(st) => {
                self.receive_battery_state(peer, st);
            }
            SessionEvent::LocateRequest(r) => {
                self.receive_locate_request(peer, r);
            }
            SessionEvent::LocateStop(s) => {
                self.receive_locate_stop(peer, s);
            }
            SessionEvent::ActionListReq(r) => {
                self.receive_action_list_req(peer, r);
            }
            SessionEvent::ActionListResp(r) => {
                self.receive_action_list_resp(peer, r);
            }
            SessionEvent::ActionTrigger(t) => {
                self.receive_action_trigger(peer, t);
            }
            SessionEvent::ActionResult(res) => {
                self.receive_action_result(peer, res);
            }
            SessionEvent::FocusState(st) => {
                self.receive_focus_state(peer, st);
            }
            SessionEvent::FocusSyncReq(req) => {
                self.receive_focus_sync_req(peer, req);
            }
            SessionEvent::TopologyState(st) => {
                self.receive_topology_state(peer, st);
            }
            SessionEvent::TopologySyncReq(req) => {
                self.receive_topology_sync_req(peer, req);
            }
            SessionEvent::InputTunnelRequest(req) => {
                self.receive_input_tunnel_request(peer, req).await;
            }
            SessionEvent::InputTunnelResp(resp) => {
                self.receive_input_tunnel_resp(peer, resp).await;
            }
            SessionEvent::InputTunnelRelease(rel) => {
                self.receive_input_tunnel_release(peer, rel).await;
            }
            SessionEvent::InputTunnelHeartbeat(hb) => {
                self.receive_input_tunnel_heartbeat(peer, hb);
            }
            SessionEvent::RemoteSessionRequest(req) => {
                self.receive_remote_session_request(peer, req).await;
            }
            SessionEvent::RemoteSessionResp(resp) => {
                self.receive_remote_session_resp(peer, resp).await;
            }
            SessionEvent::RemoteSessionClose(close) => {
                self.receive_remote_session_close(peer, close).await;
            }
            SessionEvent::VirtualDisplayAnnounce(ann) => {
                self.receive_virtual_display_announce(peer, ann);
            }
            SessionEvent::VirtualDisplayAttach(att) => {
                self.receive_virtual_display_attach(peer, att).await;
            }
            SessionEvent::VirtualDisplayDetach(det) => {
                self.receive_virtual_display_detach(peer, det).await;
            }
            SessionEvent::VirtualDisplayFrameAck(ack) => {
                self.receive_virtual_display_frame_ack(peer, ack);
            }
            SessionEvent::AudioRoutingAnnounce(ann) => {
                self.receive_audio_routing_announce(peer, ann);
            }
            SessionEvent::AudioRoutingAttach(att) => {
                self.receive_audio_routing_attach(peer, att).await;
            }
            SessionEvent::AudioRoutingDetach(det) => {
                self.receive_audio_routing_detach(peer, det).await;
            }
            SessionEvent::AudioRoutingChunk(chunk) => {
                self.receive_audio_routing_chunk(peer, chunk);
            }
            SessionEvent::CameraAnnounce(ann) => {
                self.receive_camera_announce(peer, ann);
            }
            SessionEvent::CameraAttach(att) => {
                self.receive_camera_attach(peer, att).await;
            }
            SessionEvent::CameraDetach(det) => {
                self.receive_camera_detach(peer, det).await;
            }
            SessionEvent::CameraFrameChunk(chunk) => {
                self.receive_camera_frame_chunk(peer, chunk);
            }
            SessionEvent::MicAnnounce(ann) => {
                self.receive_mic_announce(peer, ann);
            }
            SessionEvent::MicAttach(att) => {
                self.receive_mic_attach(peer, att).await;
            }
            SessionEvent::MicDetach(det) => {
                self.receive_mic_detach(peer, det).await;
            }
            SessionEvent::MicAudioChunk(chunk) => {
                self.receive_mic_audio_chunk(peer, chunk);
            }
            SessionEvent::BenchRequest(req) => {
                self.receive_bench_request(peer, req).await;
            }
            SessionEvent::BenchAccept(acc) => {
                self.receive_bench_accept(peer, acc).await;
            }
            SessionEvent::BenchData(data) => {
                self.receive_bench_data(peer, data);
            }
            SessionEvent::BenchReport(rep) => {
                self.receive_bench_report(peer, rep).await;
            }
            SessionEvent::DragControl(msg) => {
                self.receive_drag_control(peer, msg).await;
            }
            SessionEvent::Unknown(_) => {}
        }
        Ok(())
    }

    pub fn control_port(&self) -> u16 {
        self.shared.bound_control.load(Ordering::SeqCst)
    }

    /// Loopback UI API port. Reads the port actually bound by `run()`; falls
    /// back to the configured value before the UI is up, and 0 when this node
    /// serves no UI (tests, or a second node launched with `--ui-port 0`).
    pub fn ui_port(&self) -> u16 {
        let bound = self.shared.bound_ui.load(Ordering::SeqCst);
        if bound != 0 {
            bound
        } else {
            self.shared.cfg.ui_port
        }
    }

    pub fn file_port(&self) -> u16 {
        self.shared.bound_file.load(Ordering::SeqCst)
    }

    pub fn control_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.control_port()))
    }

    pub fn file_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.file_port()))
    }

    /// Whether the host screen channel is currently congested (ADR-080).
    pub fn is_screen_congested(&self) -> bool {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_congested()
    }

    /// Sets an explicit congestion override for testing and deterministic simulation (v2.37 / ADR-123).
    pub fn set_screen_congestion_override(&self, forced: Option<bool>) {
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.set_congested_override(forced);
        }
    }
}

/// Single-flight gate for `restart_daemon` (see `restart_daemon` for why).
static RESTART_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The only directory a replacement binary may be staged into. It lives
/// under the daemon's data dir, so planting a file there requires the same
/// privilege the daemon already has — a loopback HTTP caller (browser
/// context) can name the path but cannot create the file.
fn update_staging_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    let dir = data_dir.join("updates");
    let _ = std::fs::create_dir_all(&dir);
    dir.canonicalize().unwrap_or(dir)
}

/// Restore the previous binary after a failed hot replacement: drop the
/// broken image and rename the backup back into place.
fn rollback_binary(old_backup: &std::path::Path, current_exe: &std::path::Path) {
    let _ = std::fs::remove_file(current_exe);
    let _ = std::fs::rename(old_backup, current_exe);
}

/// Run the freshly placed binary with `--help` and require a clean exit.
/// This exercises process creation, dynamic linking and the CLI parser —
/// the failure modes of a broken hot update — while the old daemon is
/// still alive to serve requests and roll the file back.
fn preflight_new_binary(exe: &std::path::Path) -> Result<(), String> {
    let mut child = std::process::Command::new(exe)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("new binary failed to start: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("new binary exited with {status}"))
                };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("new binary did not exit within 5s".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(format!("preflight wait failed: {e}")),
        }
    }
}

fn cleanup_stale_old_binaries() {
    if let Ok(exe_path) = std::env::current_exe() {
        if let (Some(parent), Some(stem)) = (exe_path.parent(), exe_path.file_stem()) {
            let cutoff =
                std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 60 * 60);
            cleanup_old_binaries_in(parent, &stem.to_string_lossy(), cutoff);
        }
    }
}

fn cleanup_old_binaries_in(parent: &std::path::Path, stem: &str, cutoff: std::time::SystemTime) {
    // Filesystems with coarse mtime granularity (whole-second ext4/overlayfs
    // mounts) can report a file created *after* `cutoff` with a truncated
    // mtime that compares *before* it. Grade "recent" with a small slack so
    // the rollback path is never deleted over a timestamp rounding artifact
    // (and so minor wall-clock skew between processes cannot brick rollback).
    const RECENT_SLACK: std::time::Duration = std::time::Duration::from_secs(2);
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Only ever touch this binary's own backups
            // (`tetherly.exe.old.<ts>` / `tetherly.old.<ts>`), never
            // unrelated `.old.` files in the same directory.
            // Recent backups are the only rollback path after a bad
            // hot replacement, so they are retained for a day.
            if !name_str.starts_with(stem) || !name_str.contains(".old.") {
                continue;
            }
            let recent = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t + RECENT_SLACK >= cutoff)
                .unwrap_or(true);
            if recent {
                continue;
            }
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_restart_rejects_replacement_outside_staging() {
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node("restart-gate");
        // A file that exists but is outside the daemon-managed staging
        // directory must be refused: the loopback HTTP surface must never be
        // able to point the daemon at an arbitrary executable.
        let outside = dir.join("evil.bin");
        std::fs::write(&outside, b"not-a-binary").unwrap();
        let err = node
            .restart_daemon(Some(outside.to_str().unwrap()), Some(0))
            .unwrap_err();
        assert!(
            err.contains("update staging directory"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_daemon_restart_report_dto() {
        let rep = crate::uihttp::DaemonRestartReport {
            ok: true,
            action: "restarted".into(),
            old_pid: 1234,
            new_pid: 5678,
            message: "daemon spawned".into(),
            timestamp_ms: 100000,
        };
        let json = serde_json::to_string(&rep).unwrap();
        let decoded: crate::uihttp::DaemonRestartReport = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, rep);
    }

    #[test]
    fn test_cleanup_stale_old_binaries() {
        let tmp = std::env::temp_dir().join(format!(
            "tetherly_test_cleanup_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let old1 = tmp.join("tetherly.exe.old.12345");
        let old2 = tmp.join("tetherly.old.67890");
        let keep = tmp.join("tetherly.exe");
        let unrelated = tmp.join("notes.old.txt");
        let recent_backup = tmp.join("tetherly.exe.old.99999999");
        // The cutoff is taken before creating the fixtures, so every file
        // created afterwards counts as "recent" unless its mtime is rewound.
        let cutoff = std::time::SystemTime::now();
        let stale = cutoff - std::time::Duration::from_secs(3600);
        for f in [&old1, &old2, &keep, &unrelated, &recent_backup] {
            std::fs::write(f, b"x").unwrap();
        }

        // Backups older than the retention window are removed; recent
        // backups (the rollback path) and unrelated files survive.
        for f in [&old1, &old2] {
            std::fs::File::options()
                .write(true)
                .open(f)
                .unwrap()
                .set_modified(stale)
                .unwrap();
        }

        super::cleanup_old_binaries_in(&tmp, "tetherly", cutoff);

        assert!(!old1.exists());
        assert!(!old2.exists());
        assert!(keep.exists());
        assert!(unrelated.exists());
        assert!(recent_backup.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
