// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen DTOs and per-session state records.

use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex};
use tetherly_core::{ScreenPlaybackReport, ScreenPlaybackState, ScreenRecordingWriter};

/// Comprehensive status report for remote screen streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ScreenStatusReport {
    pub ok: bool,
    pub state: String,
    pub channel_open: bool,
    pub allowed: bool,
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
    pub refused: u64,
    pub bytes: u64,
    pub rx_bytes: u64,
    pub p95_frame_interval_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_peer: Option<String>,
    #[serde(default)]
    pub acked: u64,
    #[serde(default)]
    pub in_flight: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_latest_us: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_p95_us: Option<u64>,
    #[serde(default)]
    pub is_congested: bool,
    #[serde(default)]
    pub keyframes_requested: u64,
    #[serde(default)]
    pub gaps: u64,
    #[serde(default)]
    pub bitrate_bps: u32,
    #[serde(default)]
    pub fps_target: u32,
    #[serde(default)]
    pub scale_pct: u32,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub qp_min: u8,
    #[serde(default)]
    pub qp_max: u8,
    #[serde(default)]
    pub matrix_mode: String,
    #[serde(default)]
    pub network_health: String,
    #[serde(default)]
    pub playout_mode: String,
    #[serde(default)]
    pub playout_delay_ms: f64,
    #[serde(default)]
    pub playout_jitter_ms: f64,
    #[serde(default)]
    pub playout_buffered: usize,
    #[serde(default)]
    pub playout_dropped_late: u64,
    #[serde(default)]
    pub cursor_mode: String,
    #[serde(default)]
    pub cursor_x: i32,
    #[serde(default)]
    pub cursor_y: i32,
    #[serde(default)]
    pub cursor_visible: bool,
    #[serde(default)]
    pub cursor_shape: String,
    #[serde(default)]
    pub cursor_updates_sent: u64,
    #[serde(default)]
    pub cursor_updates_received: u64,
    #[serde(default)]
    pub squelch_mode: String,
    #[serde(default)]
    pub is_squelched: bool,
    #[serde(default)]
    pub squelched_frames: u64,
    #[serde(default)]
    pub fec_mode: String,
    #[serde(default)]
    pub fec_redundancy_pct: u8,
    #[serde(default)]
    pub fec_recovered_shards: u64,
    #[serde(default)]
    pub fec_unrecoverable_blocks: u64,
    #[serde(default)]
    pub pacer_mode: String,
    #[serde(default)]
    pub pacer_packets_paced: u64,
    #[serde(default)]
    pub pacer_bursts_smoothed: u64,
    #[serde(default)]
    pub pacer_avg_delay_us: u64,
    #[serde(default)]
    pub trendline_mode: String,
    #[serde(default)]
    pub trendline_state: String,
    #[serde(default)]
    pub trendline_slope_us: f64,
    #[serde(default)]
    pub trendline_overuse_count: u64,
    #[serde(default)]
    pub harq_mode: String,
    #[serde(default)]
    pub harq_nacks_sent: u64,
    #[serde(default)]
    pub harq_nacks_received: u64,
    #[serde(default)]
    pub harq_shards_retransmitted: u64,
    #[serde(default)]
    pub harq_recovered_frames: u64,
    #[serde(default)]
    pub harq_retransmit_timeouts: u64,
    #[serde(default)]
    pub harq_buffer_occupancy: usize,
    #[serde(default)]
    pub twcc_mode: String,
    #[serde(default)]
    pub twcc_feedbacks_sent: u64,
    #[serde(default)]
    pub twcc_feedbacks_received: u64,
    #[serde(default)]
    pub twcc_delivery_rate_bps: u64,
    #[serde(default)]
    pub twcc_delay_gradient_us: i64,
    #[serde(default)]
    pub twcc_loss_ratio: f32,
    #[serde(default)]
    pub probe_mode: String,
    #[serde(default)]
    pub probe_state: String,
    #[serde(default)]
    pub probes_triggered: u64,
    #[serde(default)]
    pub probes_completed: u64,
    #[serde(default)]
    pub probes_failed: u64,
    #[serde(default)]
    pub probe_target_bps: u32,
    #[serde(default)]
    pub probe_measured_bps: u64,
    #[serde(default)]
    pub sync_mode: String,
    #[serde(default)]
    pub sync_beacons_sent: u64,
    #[serde(default)]
    pub sync_beacons_received: u64,
    #[serde(default)]
    pub sync_skew_us: i64,
    #[serde(default)]
    pub sync_clock_drift_ppm: f64,
    #[serde(default)]
    pub sync_adjustments_applied: u64,
    #[serde(default)]
    pub in_sync: bool,
    #[serde(default)]
    pub active_display_id: u8,
    #[serde(default)]
    pub display_count: usize,
    #[serde(default)]
    pub display_switches_count: u64,
    #[serde(default)]
    pub privacy_enabled: bool,
    #[serde(default)]
    pub privacy_input_locked: bool,
    #[serde(default)]
    pub privacy_blanking_method: String,
    #[serde(default)]
    pub latency_mode: String,
    #[serde(default)]
    pub latency_bottleneck: String,
    #[serde(default)]
    pub latency_total_e2e_us: u64,
    #[serde(default)]
    pub latency_p95_e2e_us: u64,
    #[serde(default)]
    pub latency_frames_profiled: u64,
    #[serde(default)]
    pub tuning_mode: String,
    #[serde(default)]
    pub tuning_actions_total: u64,
    #[serde(default)]
    pub tuning_recoveries: u64,
    #[serde(default)]
    pub tuning_last_stage: String,
    #[serde(default)]
    pub tuning_last_action: String,
    #[serde(default)]
    pub recording_active: bool,
    #[serde(default)]
    pub peer_recording: bool,
    #[serde(default)]
    pub recording_frames: u64,
    #[serde(default)]
    pub playback_active: bool,
    #[serde(default)]
    pub playback_state: String,
    #[serde(default)]
    pub playback_frame: u64,
    #[serde(default)]
    pub snapshot_count: u64,
    #[serde(default)]
    pub watermark_enabled: bool,
    #[serde(default)]
    pub watermark_enforced: bool,
    #[serde(default)]
    pub watermark_frames: u64,
    #[serde(default)]
    pub annotations_count: usize,
    #[serde(default)]
    pub laser_active: bool,
    #[serde(default)]
    pub annotations_total: u64,
    #[serde(default)]
    pub laser_updates_total: u64,
    #[serde(default)]
    pub whiteboard_undo_count: u64,
    #[serde(default)]
    pub whiteboard_redo_count: u64,
    #[serde(default)]
    pub whiteboard_read_only: bool,
    #[serde(default)]
    pub whiteboard_max_items: u16,
    #[serde(default)]
    pub roi_active: bool,
    #[serde(default)]
    pub roi_x: u16,
    #[serde(default)]
    pub roi_y: u16,
    #[serde(default)]
    pub roi_w: u16,
    #[serde(default)]
    pub roi_h: u16,
    #[serde(default)]
    pub roi_zoom_percent: u16,
    #[serde(default)]
    pub roi_updates: u64,
    #[serde(default)]
    pub color_mode: String,
    #[serde(default)]
    pub color_threshold: u8,
    #[serde(default)]
    pub color_updates: u64,
    #[serde(default)]
    pub delta_enabled: bool,
    #[serde(default)]
    pub delta_keyframe_interval: u16,
    #[serde(default)]
    pub delta_frames_sent: u64,
    #[serde(default)]
    pub delta_keyframes_sent: u64,
    #[serde(default)]
    pub delta_frames_received: u64,
    #[serde(default)]
    pub delta_keyframes_received: u64,
    #[serde(default)]
    pub delta_saved_bytes: u64,
    #[serde(default)]
    pub metric_enabled: bool,
    #[serde(default)]
    pub metric_sample_interval: u8,
    #[serde(default)]
    pub metric_evaluated_frames: u64,
    #[serde(default)]
    pub metric_psnr_db: f64,
    #[serde(default)]
    pub metric_ssim: f64,
    #[serde(default)]
    pub metric_quality_warning: bool,
    #[serde(default)]
    pub qoe_enabled: bool,
    #[serde(default)]
    pub qoe_mode: String,
    #[serde(default)]
    pub qoe_target_psnr_db: u8,
    #[serde(default)]
    pub qoe_target_ssim_pct: u8,
    #[serde(default)]
    pub qoe_actions_total: u64,
    #[serde(default)]
    pub qoe_last_action: String,
    #[serde(default)]
    pub pir_enabled: bool,
    #[serde(default)]
    pub pir_mode: String,
    #[serde(default)]
    pub pir_bands: u8,
    #[serde(default)]
    pub pir_sweeps_completed: u64,
    #[serde(default)]
    pub pir_slices_sent: u64,
    #[serde(default)]
    pub ltr_enabled: bool,
    #[serde(default)]
    pub ltr_auto_interval: u16,
    #[serde(default)]
    pub ltr_current_seq: u64,
    #[serde(default)]
    pub ltr_anchors_established: u64,
    #[serde(default)]
    pub ltr_recoveries_completed: u64,
    #[serde(default)]
    pub svc_enabled: bool,
    #[serde(default)]
    pub svc_mode: String,
    #[serde(default)]
    pub svc_drop_t1: bool,
    #[serde(default)]
    pub svc_t0_sent: u64,
    #[serde(default)]
    pub svc_t1_sent: u64,
    #[serde(default)]
    pub svc_t1_dropped: u64,
    #[serde(default)]
    pub motion_enabled: bool,
    #[serde(default)]
    pub motion_max_displacement: u16,
    #[serde(default)]
    pub motion_frames_sent: u64,
    #[serde(default)]
    pub motion_fallback_frames_sent: u64,
    #[serde(default)]
    pub motion_saved_bytes: u64,
    #[serde(default)]
    pub tile_enabled: bool,
    #[serde(default)]
    pub tile_size: u16,
    #[serde(default)]
    pub tile_frames_sent: u64,
    #[serde(default)]
    pub dirty_tiles_sent: u64,
    #[serde(default)]
    pub skipped_tiles_count: u64,
    #[serde(default)]
    pub tile_saved_bytes: u64,
    #[serde(default)]
    pub tile_cache_enabled: bool,
    #[serde(default)]
    pub tile_cache_max_entries: u16,
    #[serde(default)]
    pub tile_cache_entries: u16,
    #[serde(default)]
    pub tile_cache_hits: u64,
    #[serde(default)]
    pub tile_cache_misses: u64,
    #[serde(default)]
    pub tile_cache_hit_ratio_pct: f32,
    #[serde(default)]
    pub tile_cache_saved_bytes: u64,
    #[serde(default)]
    pub focus_enabled: bool,
    #[serde(default)]
    pub focus_mode: String,
    #[serde(default)]
    pub focus_radius: u16,
    #[serde(default)]
    pub focus_peripheral_skip_ratio: u8,
    #[serde(default)]
    pub focus_box_x: u16,
    #[serde(default)]
    pub focus_box_y: u16,
    #[serde(default)]
    pub focus_box_w: u16,
    #[serde(default)]
    pub focus_box_h: u16,
    #[serde(default)]
    pub focus_tiles_sent: u64,
    #[serde(default)]
    pub peripheral_tiles_sent: u64,
    #[serde(default)]
    pub peripheral_tiles_skipped: u64,
    #[serde(default)]
    pub focus_saved_bandwidth_bytes: u64,
    #[serde(default)]
    pub tile_palette_enabled: bool,
    #[serde(default)]
    pub tile_palette_max_colors: u8,
    #[serde(default)]
    pub tile_palette_enable_rle: bool,
    #[serde(default)]
    pub tile_palette_solid_tiles: u64,
    #[serde(default)]
    pub tile_palette_bilevel_tiles: u64,
    #[serde(default)]
    pub tile_palette_palette_tiles: u64,
    #[serde(default)]
    pub tile_palette_truecolor_tiles: u64,
    #[serde(default)]
    pub tile_palette_saved_bytes: u64,
    #[serde(default)]
    pub tile_predict_enabled: bool,
    #[serde(default)]
    pub tile_predict_filter_mode: String,
    #[serde(default)]
    pub tile_predict_enable_rct: bool,
    #[serde(default)]
    pub tile_predict_tiles_sent: u64,
    #[serde(default)]
    pub tile_predict_saved_bytes: u64,
    #[serde(default)]
    pub tile_present_enabled: bool,
    #[serde(default)]
    pub tile_present_vsync_mode: String,
    #[serde(default)]
    pub tile_present_double_buffer: bool,
    #[serde(default)]
    pub tile_present_tear_prevention: bool,
    #[serde(default)]
    pub tile_present_committed_transactions: u64,
    #[serde(default)]
    pub tile_present_swapped_frames: u64,
    #[serde(default)]
    pub tile_present_torn_frames_prevented: u64,
    #[serde(default)]
    pub tile_present_avg_swap_latency_us: u64,
    #[serde(default)]
    pub tile_nack_enabled: bool,
    #[serde(default)]
    pub tile_nack_timeout_ms: u16,
    #[serde(default)]
    pub tile_nack_max_retries: u8,
    #[serde(default)]
    pub tile_nack_requests_sent: u64,
    #[serde(default)]
    pub tile_nack_tiles_retransmitted: u64,
    #[serde(default)]
    pub tile_nack_degraded_commits: u64,
    #[serde(default)]
    pub tile_saliency_enabled: bool,
    #[serde(default)]
    pub tile_saliency_high_threshold: u8,
    #[serde(default)]
    pub tile_saliency_low_threshold: u8,
    #[serde(default)]
    pub tile_saliency_drop_background_on_congestion: bool,
    #[serde(default)]
    pub tile_saliency_critical_tiles: u64,
    #[serde(default)]
    pub tile_saliency_normal_tiles: u64,
    #[serde(default)]
    pub tile_saliency_background_tiles: u64,
    #[serde(default)]
    pub tile_saliency_background_skipped: u64,
    #[serde(default)]
    pub tile_lod_enabled: bool,
    #[serde(default)]
    pub tile_lod_max_lod: u8,
    #[serde(default)]
    pub tile_lod_auto_lod_on_congestion: bool,
    #[serde(default)]
    pub tile_lod0_tiles: u64,
    #[serde(default)]
    pub tile_lod1_tiles: u64,
    #[serde(default)]
    pub tile_lod2_tiles: u64,
    #[serde(default)]
    pub tile_lod_bytes_saved: u64,
    #[serde(default)]
    pub tile_batch_enabled: bool,
    #[serde(default)]
    pub tile_batch_target_chunk_bytes: u16,
    #[serde(default)]
    pub tile_batch_pacing_interval_us: u16,
    #[serde(default)]
    pub tile_batches_sent: u64,
    #[serde(default)]
    pub tile_chunks_sent: u64,
    #[serde(default)]
    pub tile_batch_avg_chunk_bytes: u32,
    #[serde(default)]
    pub tile_batch_pacing_pauses: u64,
    #[serde(default)]
    pub tile_motion_enabled: bool,
    #[serde(default)]
    pub tile_motion_max_displacement: u16,
    #[serde(default)]
    pub tile_motion_min_match_ratio: u8,
    #[serde(default)]
    pub tile_motion_tiles_sent: u64,
    #[serde(default)]
    pub tile_motion_fallback_tiles_sent: u64,
    #[serde(default)]
    pub tile_motion_bytes_saved: u64,
}

/// Internal accumulation state for Screen Objective Visual Quality Metric evaluation (v2.24 / ADR-110).
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenQualityMetricState {
    pub evaluated_frames: u64,
    pub sum_mse: f64,
    pub sum_psnr_db: f64,
    pub sum_ssim: f64,
    pub last_psnr_db: f64,
    pub last_ssim: f64,
}

impl Default for ScreenQualityMetricState {
    fn default() -> Self {
        Self {
            evaluated_frames: 0,
            sum_mse: 0.0,
            sum_psnr_db: 0.0,
            sum_ssim: 0.0,
            last_psnr_db: 100.0,
            last_ssim: 1.0,
        }
    }
}

/// Update payload for screen security watermark configuration (v2.18 / ADR-104).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
pub struct ScreenWatermarkUpdate {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub host_enforced: Option<bool>,
    #[serde(default)]
    pub text_template: Option<String>,
    #[serde(default)]
    pub opacity: Option<u8>,
    #[serde(default)]
    pub color_rgb: Option<[u8; 3]>,
    #[serde(default)]
    pub scale: Option<u8>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub spacing_x: Option<u16>,
    #[serde(default)]
    pub spacing_y: Option<u16>,
}

/// Status report for the screen session recorder (v2.15 / ADR-101).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScreenRecordingReport {
    pub ok: bool,
    pub active: bool,
    pub path: String,
    pub frames_recorded: u64,
    pub bytes_recorded: u64,
    pub started_at_us: u64,
    pub duration_us: u64,
    pub last_frame_seq: u64,
    pub peer_recording: bool,
    pub width: u32,
    pub height: u32,
}

/// One replayed frame summary from a TREC container (v2.15 / ADR-101).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScreenRecordingReplayRecord {
    pub seq: u64,
    pub pts_us: u64,
    pub len: usize,
}

/// Replay inspection report for a recorded TREC container (v2.15 / ADR-101).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScreenRecordingReplayReport {
    pub ok: bool,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub format_tag: u8,
    pub codec_tag: u8,
    pub started_at_us: u64,
    pub records: Vec<ScreenRecordingReplayRecord>,
    pub records_returned: usize,
    pub truncated: bool,
}

/// Armed screen session recording: the TREC header is written lazily on the
/// first presented frame, when the actual rect/codec are known (v2.15 / ADR-101).
pub struct ScreenRecordingSession {
    pub path: std::path::PathBuf,
    pub started_at_us: u64,
    pub writer: Option<ScreenRecordingWriter<std::io::BufWriter<std::fs::File>>>,
    pub frames: u64,
    pub bytes: u64,
    pub last_seq: u64,
}

/// Armed screen session in-container playback session (v2.16 / ADR-102).
pub struct ScreenPlaybackSession {
    pub path: std::path::PathBuf,
    pub stop_tx: tokio::sync::watch::Sender<bool>,
    pub paused: Arc<AtomicBool>,
    pub speed: Arc<AtomicU32>,
    pub loop_enabled: Arc<AtomicBool>,
    pub report: Arc<Mutex<ScreenPlaybackReport>>,
}

impl ScreenPlaybackSession {
    pub fn state(&self) -> ScreenPlaybackState {
        let r = self.report.lock().expect("lock");
        r.state.parse().unwrap_or(ScreenPlaybackState::Idle)
    }

    pub fn current_report(&self) -> ScreenPlaybackReport {
        self.report.lock().expect("lock").clone()
    }
}

/// Status report for screen damage detection and static frame adaptive squelch (v2.03 / ADR-089).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ScreenSquelchReport {
    pub ok: bool,
    pub mode: String,
    pub is_squelched: bool,
    pub frames_evaluated: u64,
    pub frames_passed: u64,
    pub frames_squelched: u64,
    pub keepalive_frames: u64,
    pub last_dirty_ratio: f32,
    pub consecutive_static_frames: u64,
    pub timestamp_ms: u64,
}

/// Status report for hardware cursor decoupling (ADR-088).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ScreenCursorReport {
    pub ok: bool,
    pub mode: String,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    pub shape: String,
    pub updates_sent: u64,
    pub updates_received: u64,
    pub updates_dropped_dedup: u64,
    pub updates_dropped_rate: u64,
    pub timestamp_ms: u64,
}

/// Status report for receiver playout buffer and adaptive jitter dejitterizer (ADR-087).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ScreenPlayoutReport {
    pub ok: bool,
    pub mode: String,
    pub target_delay_ms: f64,
    pub current_jitter_ms: f64,
    pub target_delay_us: u64,
    pub current_jitter_us: u64,
    pub buffered_frames: usize,
    pub played_frames: u64,
    pub dropped_late_frames: u64,
    pub dropped_overflow_frames: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_delay_us: Option<u64>,
    pub timestamp_ms: u64,
}

/// Status report for 4-dimensional autonomous adaptation matrix coordinator (ADR-086).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenMatrixReport {
    pub ok: bool,
    pub mode: String,
    pub health: String,
    pub bitrate_bps: u32,
    pub fps_target: u32,
    pub scale_pct: u32,
    pub profile: String,
    pub qp_min: u8,
    pub qp_max: u8,
    pub auto_fps: bool,
    pub auto_scale: bool,
    pub consecutive_healthy_acks: u32,
    pub consecutive_drops: u32,
    pub in_flight: u64,
    pub rtt_us: Option<u64>,
    pub timestamp_ms: u64,
}

/// Status report for screen streaming content profile and QP quality bounds (ADR-085).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenProfileReport {
    pub ok: bool,
    pub profile: tetherly_core::ScreenContentProfile,
    pub min_qp: u8,
    pub max_qp: u8,
    pub is_custom_qp: bool,
    #[serde(default)]
    pub auto_profile: bool,
    pub timestamp_ms: u64,
}

/// Status report for screen streaming resolution downscaling (ADR-084).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenScaleReport {
    pub ok: bool,
    pub scale_pct: u32,
    pub min_scale_pct: u32,
    pub max_scale_pct: u32,
    #[serde(default)]
    pub auto_profile: bool,
    pub timestamp_ms: u64,
}

/// Status report for screen streaming frame rate / FPS pacing (ADR-083).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenFpsReport {
    pub ok: bool,
    pub fps: u32,
    pub min_fps: u32,
    pub max_fps: u32,
    pub interval_us: u64,
    #[serde(default)]
    pub auto_profile: bool,
    pub timestamp_ms: u64,
}

/// Status report for screen streaming bitrate (ADR-082).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenBitrateReport {
    pub ok: bool,
    pub bitrate_bps: u32,
    pub bitrate_kbps: u32,
    pub min_bps: u32,
    pub max_bps: u32,
    pub timestamp_ms: u64,
}

/// Control result report for remote screen streaming actions (allow, revoke, start, stop).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ScreenControlReport {
    pub ok: bool,
    pub action: String,
    pub state: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    pub timestamp_ms: u64,
}

/// Wall-clock p95 of host-side frame send intervals. Real timing lives here in
/// tetherly-node (core is deterministic and OS-free); production verifies the
/// frame budget (M5.1) against this. Microsecond resolution.
pub(super) struct ScreenIntervalTracker {
    pub(super) last: Option<std::time::Instant>,
    pub(super) intervals: Vec<u64>, // micros, newest first
    pub(super) max: usize,
}

impl ScreenIntervalTracker {
    pub(super) fn new() -> Self {
        Self {
            last: None,
            intervals: Vec::with_capacity(1024),
            max: 1024,
        }
    }

    /// Mark one frame send. The elapsed time since the previous send is the
    /// interval; the first send only seeds the clock.
    pub(super) fn record(&mut self) {
        let now = std::time::Instant::now();
        if let Some(prev) = self.last.replace(now) {
            self.intervals
                .push(now.duration_since(prev).as_micros() as u64);
        }
        if self.intervals.len() > self.max {
            let excess = self.intervals.len() - self.max;
            self.intervals.drain(..excess);
        }
    }

    /// 95th-percentile frame interval in microseconds, or `None` before two
    /// frames have shipped.
    pub(super) fn p95_us(&self) -> Option<u64> {
        if self.intervals.len() < 2 {
            return None;
        }
        let mut sorted = self.intervals.clone();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        Some(sorted[idx.min(sorted.len() - 1)])
    }
}

/// Active tile batch reassembly tracking on the receiver (v2.39 / ADR-125).
#[derive(Debug, Clone)]
pub struct TileBatchReceiverState {
    pub batch_id: u32,
    pub total_chunks: u16,
    pub chunks_received: std::collections::BTreeSet<u16>,
}
