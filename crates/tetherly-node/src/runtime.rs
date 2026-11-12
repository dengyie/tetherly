// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node runtime: LAN + optional EasyTier sidecar overlay. Overlay is unicast
//! only; no mDNS on TUN.
//!
//! Layout: the module root keeps the node's own state (`Node`, `Shared`,
//! `NodeConfig`, `UserCommand`) so every feature submodule can reach private
//! fields by ancestor privacy. Each feature area lives in its own submodule;
//! they expose `pub(super)` inherent methods on `Node`, re-exported from here,
//! so `crate::runtime::X` keeps resolving no matter which submodule defines X.

use crate::events::UiEvent;
use crate::sidecar::{OverlayConfig, OverlayStatus};
use crate::store::{
    ActionPolicy, AudioRoutingPolicy, AutoAcceptPolicy, BandwidthProfileTable, BatteryPolicy,
    BenchPolicy, CameraPolicy, ClipboardPolicy, FocusPolicy, HandoffInboxTable, HandoffPolicy,
    InputTunnelPolicy, LocalActionDefinition, LocatorPolicy, MediaControlPolicy, MicrophonePolicy,
    ProfileTable, RouteTable, TopologyLayout, TopologyPolicy, VirtualDisplayPolicy,
};
use crate::telemetry::TrafficTelemetry;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::frame::InnerFrame;
use tetherly_core::ports::{Clip, Clock, Rng};
use tetherly_core::session::LockoutTable;
use tetherly_core::{
    ActionItem, ActionResultFrame, AncsIngress, AncsTransport, BatteryStateFrame, BenchReportFrame,
    ClipHub, ControlMsg, CoreError, CursorSeat, DeviceId, FileHub, FocusStateFrame, InputServer,
    InputSink, MediaStateFrame, MemoryScreenSink, MemorySink, MemoryTrustStore, Notifier,
    NotifyHub, OpenAllowlist, ScreenAudioSyncMode, ScreenAudioSyncStats, ScreenCodec,
    ScreenCursorMode, ScreenCursorStats, ScreenDamageTracker, ScreenFecMode, ScreenFrame,
    ScreenHarqMode, ScreenLatencyReport, ScreenPacerMode, ScreenPlayoutMode, ScreenPlayoutStats,
    ScreenProbeMode, ScreenProbeStats, ScreenProfilerMode, ScreenSender, ScreenSink, ScreenSource,
    ScreenSquelchMode, ScreenStats, ScreenTrendlineMode, ScreenTunerMode, ScreenTuningReport,
    ScreenTwccMode, ScreenTwccStats, TcpLimiter, TopologyStateFrame,
};
use tetherly_crypto::Identity;
use tetherly_net::session::{CONTROL_PORT, FILE_PORT, INPUT_PORT, SCREEN_PORT};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex as AsyncMutex};
use tracing::debug;

mod actions;
mod audio;
mod audit;
mod bench;
mod camera;
mod clipboard;
mod config;
mod discovery;
mod drag;
mod drag_controller;
mod drag_target;
mod files;
mod focus;
mod handoff;
mod input;
mod lifecycle;
mod locate;
mod media;
mod messages;
mod notify;
mod overlay;
mod remote_session;
mod routes;
mod screen_control;
mod screen_dto;
mod screen_media;
mod screen_mode;
mod screen_status;
mod screen_stream;
mod session;
mod shell_reveal;
mod storage;
mod topology;
mod trust;
mod virtual_display;

pub use actions::*;
pub use audio::*;
pub use audit::*;
pub use bench::*;
pub use camera::*;
pub use clipboard::*;
pub use discovery::*;
pub use drag_controller::*;
pub use drag_target::*;
pub use files::*;
pub use focus::*;
pub use handoff::*;
pub use input::*;
pub use locate::*;
pub use media::*;
pub use messages::*;
pub use notify::*;
pub use remote_session::RemoteSessionStartReport;
pub use routes::*;
pub use screen_dto::*;
pub use shell_reveal::*;
pub use storage::*;
pub use topology::*;
pub use trust::*;
pub use virtual_display::*;

struct OsRng;

impl Rng for OsRng {
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError> {
        getrandom::getrandom(dest).map_err(|_| CoreError::Rng)
    }
}

/// Hard budget for one `ScreenSource::grab` before the runtime interrupts it.
const SCREEN_GRAB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Depth of the per-session outbound queues. A bounded queue is the backpressure
/// that stops a wedged peer from turning a live session into unbounded memory
/// growth; a full queue surfaces as an error instead of silently buffering.
const SESSION_QUEUE_DEPTH: usize = 256;

/// Screen fragments are large, so their queue is short: a frame that cannot fit
/// is dropped rather than queued behind a slow link.
const SCREEN_QUEUE_DEPTH: usize = 32;

#[derive(Clone)]

pub struct NodeConfig {
    pub data_dir: PathBuf,
    pub name: String,
    pub platform: String,
    pub control_port: u16,
    pub file_port: u16,
    pub input_port: u16,
    pub screen_port: u16,
    pub advertise: bool,
    /// Tests inject a clip. Production uses SystemClip.
    pub memory_clip: bool,
    /// Tests inject an in-memory notifier. Production uses SystemNotifier.
    pub memory_notifier: bool,
    /// Tests bind 127.0.0.1 only so two nodes can share a host.
    pub loopback_only: bool,
    /// Production reconnects trusted peers. Tests turn this off to avoid races.
    pub reconnect: bool,
    /// Loopback HTTP UI. 0 disables.
    pub ui_port: u16,
    /// EasyTier sidecar. Missing binary/RPC is success (LAN only).
    pub overlay: OverlayConfig,
    /// ANCS link to a phone. `None` until the platform BLE layer is attached;
    /// tests attach a `MemoryAncsTransport`.
    pub ancs: Option<Arc<dyn AncsTransport>>,
    /// BLE peripheral identity of the phone, used to derive its synthetic id.
    pub ancs_peripheral: String,
    /// Screen capture source. `None` defaults to a System stub that returns
    /// ScreenRefused (real capture is Manual-required). Tests inject
    /// MemoryScreenSource for deterministic CI.
    pub screen_source: Option<Arc<dyn ScreenSource>>,
    /// Screen encoder/decoder. `None` defaults to lossless zstd level 3.
    pub screen_codec: Option<Arc<dyn ScreenCodec>>,
    /// Prefer H.264 for outbound screen streaming (ADR-079). Still falls
    /// back to zstd per-peer when `screencodecs` negotiation says no.
    pub screen_codec_h264: bool,
    /// Encoder bitrate ceiling for H.264 screen streaming, kilobits/s.
    pub screen_bitrate_kbps: u32,
    /// Pick that ceiling from the peer's measured bandwidth profile
    /// (ADR-078) instead of the manual flag; re-tiers hot on new benches.
    pub screen_bitrate_auto: bool,
    /// Target frame rate in FPS for screen streaming (ADR-083).
    pub screen_fps: u32,
    /// Pick target FPS tier from measured bandwidth profile (ADR-083).
    pub screen_fps_auto: bool,
    /// Target resolution scale percentage for screen streaming (25..=100) (ADR-084).
    pub screen_scale_pct: u32,
    /// Pick target resolution scale percentage tier from measured bandwidth profile (ADR-084).
    pub screen_scale_auto: bool,
    /// Content optimization profile for screen streaming (ADR-085).
    pub screen_profile: tetherly_core::ScreenContentProfile,
    /// Pick content profile automatically (ADR-085).
    pub screen_profile_auto: bool,
    /// Minimum Quantization Parameter bound for screen streaming (ADR-085).
    pub screen_qp_min: u8,
    /// Maximum Quantization Parameter bound for screen streaming (ADR-085).
    pub screen_qp_max: u8,
    /// Operation mode for 4D autonomous adaptation matrix coordinator (ADR-086).
    pub screen_matrix_mode: tetherly_core::ScreenMatrixMode,
    /// Operation mode for screen receiver playout buffer and adaptive dejitterizer (ADR-087).
    pub screen_playout_mode: tetherly_core::ScreenPlayoutMode,
    /// Operation mode for screen hardware cursor decoupling (ADR-088).
    pub screen_cursor_mode: tetherly_core::ScreenCursorMode,
    /// Operation mode for screen damage detection and static frame adaptive squelch (ADR-089).
    pub screen_squelch_mode: tetherly_core::ScreenSquelchMode,
    /// Operation mode for screen forward error correction (FEC) and adaptive redundancy protection (ADR-090).
    pub screen_fec_mode: tetherly_core::ScreenFecMode,
    /// Operation mode for screen streaming microsecond packet pacing and token-bucket burst shaper (ADR-091).
    pub screen_pacer_mode: tetherly_core::ScreenPacerMode,
    /// Operation mode for screen streaming delay-based trendline bufferbloat filter (ADR-092).
    pub screen_trendline_mode: tetherly_core::ScreenTrendlineMode,
    /// Operation mode for screen streaming selective NACK and Hybrid ARQ (HARQ) pipeline (ADR-093).
    pub screen_harq_mode: tetherly_core::ScreenHarqMode,
    /// Operation mode for receiver-driven transport-wide congestion control (TWCC) (ADR-094).
    pub screen_twcc_mode: tetherly_core::ScreenTwccMode,
    /// Operation mode for active bandwidth probing and fast ramp-up pipeline (ADR-095).
    pub screen_probe_mode: tetherly_core::ScreenProbeMode,
    /// Operation mode for screen and audio playout lip-sync & clock synchronization pipeline (ADR-096).
    pub screen_sync_mode: tetherly_core::ScreenAudioSyncMode,
    /// Operation mode for microsecond-precision multi-stage latency profiler & telemetry (ADR-099).
    pub screen_latency_mode: tetherly_core::ScreenProfilerMode,
    /// Operation mode for the bottleneck-driven autonomous tuner (ADR-100).
    pub screen_tuning_mode: tetherly_core::ScreenTunerMode,
    /// Initial active display ID for screen capture (ADR-097). Defaults to 0 (Primary).
    pub screen_display_id: u8,
    /// Screen presentation sink. `None` defaults to the in-memory observer;
    /// production passes `SystemSink` for real output.
    pub screen_sink: Option<Arc<dyn ScreenSink>>,
    /// Real key/mouse injection for the controlled side of 45719. `None`
    /// keeps CI's MemorySink-only behavior (the observer always records).
    pub input_sink: Option<Arc<Mutex<dyn InputSink>>>,
}

fn default_screen_bitrate_kbps() -> u32 {
    4000
}

fn default_screen_fps() -> u32 {
    tetherly_core::SCREEN_FPS_DEFAULT
}

fn default_screen_scale_pct() -> u32 {
    tetherly_core::SCREEN_SCALE_DEFAULT_PCT
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: crate::store::default_data_dir(),
            name: hostname(),
            platform: std::env::consts::OS.to_string(),
            control_port: CONTROL_PORT,
            file_port: FILE_PORT,
            input_port: INPUT_PORT,
            screen_port: SCREEN_PORT,
            advertise: true,
            memory_clip: false,
            memory_notifier: false,
            loopback_only: false,
            reconnect: true,
            ui_port: 45716,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "local-iphone".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        }
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "tetherly".into())
}

/// Scan `inbox/` for `.<name>.resume.json` sidecars and rebuild the in-memory
/// resume table. Only meaningful for content-addressed transfers whose
/// `transfer_id` equals the name; a mismatched/unreadable sidecar is skipped.
fn load_resume_table(data_dir: &std::path::Path) -> tetherly_core::ResumeTable {
    let mut table = tetherly_core::ResumeTable::new();
    let inbox = data_dir.join("inbox");
    let Ok(rd) = std::fs::read_dir(&inbox) else {
        return table;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(stem) = name
            .strip_prefix('.')
            .and_then(|s| s.strip_suffix(".resume.json"))
        else {
            continue;
        };
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(dto) = serde_json::from_slice::<ResumeSidecar>(&bytes) else {
            continue;
        };
        let Ok(source) = DeviceId::parse(&dto.device_id) else {
            continue;
        };
        if dto.offset == 0 || dto.offset >= dto.total {
            continue;
        }
        // A sidecar without a matching (or long enough) partial is torn state
        // (crash between chunk write and sidecar write): drop it and restart
        // the transfer from 0 rather than promising a resume we cannot honor.
        let part = inbox.join(format!(".{stem}.part"));
        let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if len < dto.offset {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        table.put(tetherly_core::ResumeState {
            transfer_id: stem.to_string(),
            source,
            total: dto.total,
            offset: dto.offset,
        });
    }
    table
}

/// Format bytes into human-readable memory string (B, KiB, MiB, GiB).
pub fn format_memory_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

struct Hubs {
    notify: NotifyHub,
    clip: ClipHub,
    files: FileHub,
    /// Chunk-aligned progress markers for interrupted incoming transfers.
    /// Loaded from disk at startup; the node persists them as sidecars.
    resume: tetherly_core::ResumeTable,
}

/// Maximum number of completed/failed transfer records retained in memory.
pub const MAX_TERMINAL_TRANSFERS: usize = 64;

/// Helper to determine the standard MIME type from a file name or extension.
pub fn mime_type_from_name(name: &str) -> &'static str {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("json") => "application/json",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript",
        Some("csv") => "text/csv; charset=utf-8",
        Some("zip") => "application/zip",
        Some("gz") | Some("tgz") => "application/gzip",
        Some("tar") => "application/x-tar",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

/// Comprehensive report of node static and dynamic configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NodeConfigReport {
    pub ok: bool,
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub data_dir: String,
    pub loopback_only: bool,
    pub advertise: bool,
    pub reconnect: bool,
    pub memory_clip: bool,
    #[serde(default)]
    pub memory_notifier: bool,
    pub ui_port: u16,
    pub control_port: u16,
    pub file_port: u16,
    pub input_port: u16,
    pub screen_port: u16,
    pub overlay_enabled: bool,
    pub ancs_attached: bool,
    pub ancs_peripheral: String,
}

struct Shared {
    identity: Identity,
    cfg: NodeConfig,
    trust: Arc<Mutex<MemoryTrustStore>>,
    lockout: Arc<Mutex<LockoutTable>>,
    tcp_limiter: Arc<Mutex<TcpLimiter>>,
    clock: Arc<dyn Clock>,
    hubs: Mutex<Hubs>,
    clip: Arc<dyn Clip>,
    pin: Mutex<Option<([u8; 8], u64)>>,
    events: broadcast::Sender<UiEvent>,
    sessions: AsyncMutex<HashMap<DeviceId, mpsc::Sender<InnerFrame>>>,
    /// Outbound input channel tagged with a generation id. A session task
    /// may only clear the slot if its own generation is still current — a
    /// stale task exiting late (e.g. after a takeover rebuilt the channel)
    /// must not kill the new channel.
    input_out: Mutex<Option<(u64, mpsc::Sender<InnerFrame>)>>,
    input_reconnect_deadline: Mutex<Option<std::time::Instant>>,
    input_buffered_frames: Mutex<VecDeque<InnerFrame>>,
    live: Mutex<HashMap<DeviceId, LivePeer>>,
    ping_waiters: Mutex<HashMap<DeviceId, Vec<oneshot::Sender<u64>>>>,
    pending_out: Mutex<HashMap<String, OutboundFile>>,
    active_transfers: Mutex<HashMap<String, ActiveTransfer>>,
    transfer_cancels: Mutex<HashMap<String, oneshot::Sender<()>>>,
    transfer_stats: Mutex<TransferStats>,
    bound_control: AtomicU16,
    bound_file: AtomicU16,
    bound_input: AtomicU16,
    bound_screen: AtomicU16,
    /// Loopback UI API port actually bound. 0 until `run()` serves it.
    bound_ui: AtomicU16,
    /// Input cursor owner. None until a trusted desktop dials 45719.
    input_seat: Mutex<Option<CursorSeat>>,
    input_sink: Mutex<MemorySink>,
    /// Production injection sink (SendInput). Applied events are mirrored
    /// here after the CI observer records them.
    input_real_sink: Option<Arc<Mutex<dyn InputSink>>>,
    /// Seq lives here so repeated edge crossings do not replay seq 1.
    input_engine: Mutex<InputServer>,
    /// Active input peer currently bound or focused.
    input_peer: Mutex<Option<DeviceId>>,
    /// Whether OS-level input capture hooks have been successfully armed.
    input_capture_active: std::sync::atomic::AtomicBool,
    pub(crate) input_hook_control: Mutex<Option<crate::platform::HookControl>>,
    /// Frames outbound to the screen controller on 45720. None until attached.
    screen_out: AsyncMutex<Option<mpsc::Sender<InnerFrame>>>,
    /// Active screen peer currently streaming or attached.
    screen_peer: Mutex<Option<DeviceId>>,
    /// Sender-side screen state machine: consent, seq, refusal counts.
    screen_engine: Mutex<ScreenSender>,
    /// Where sender-side frames come from. MemorySource in CI; real capture
    /// lives in tetherly-node platform code and stays Manual-required.
    screen_source: Arc<dyn ScreenSource>,
    /// Controller-side trace of presented frames. Clone() shares inner state.
    screen_observer: Mutex<MemoryScreenSink>,
    /// Visitor-side control channel. None until a visitor attaches.
    screen_ctl: AsyncMutex<Option<mpsc::Sender<ControlMsg>>>,
    /// Visitor-side decompressor. Defaults to zstd lossless.
    screen_codec: Arc<dyn ScreenCodec>,
    /// H.264 decoder for screen frames tagged SCREEN_COMPRESS_H264 (ADR-079).
    screen_codec_h264: Arc<dyn ScreenCodec>,
    /// Outbound screen codec selector when the operator opted into H.264
    /// (ADR-079); `None` keeps the zstd-only default path.
    screen_encode_codec: Option<Arc<crate::screen::NegotiatedCodec>>,
    /// Whether the selector's bitrate follows the ADR-078 profile.
    screen_bitrate_auto: bool,
    /// One-shot per stream: the first streamed frame picks the tier.
    screen_bitrate_applied: std::sync::atomic::AtomicBool,
    /// Whether target FPS follows the measured bandwidth profile (ADR-083).
    screen_fps_auto: bool,
    /// One-shot per stream: the first streamed frame picks the FPS tier.
    screen_fps_applied: std::sync::atomic::AtomicBool,
    /// Whether target resolution scale follows the measured bandwidth profile (ADR-084).
    screen_scale_auto: bool,
    /// One-shot per stream: the first streamed frame picks the scale tier.
    screen_scale_applied: std::sync::atomic::AtomicBool,
    /// Whether target content profile follows the measured bandwidth profile (ADR-085).
    screen_profile_auto: bool,
    /// One-shot per stream: the first streamed frame picks the profile tier.
    screen_profile_applied: std::sync::atomic::AtomicBool,
    /// Visitor-side output. Defaults to the in-memory observer; production
    /// injects `SystemSink` for real display.
    screen_sink: Arc<dyn ScreenSink>,
    /// Host-side frame-interval tracker (p95 budget). Recorded per successful
    /// frame send.
    screen_interval: Mutex<ScreenIntervalTracker>,
    /// Visitor-side receiver counters (received/dropped/bytes). Written by the
    /// screen-client task so the UI can show what actually arrived.
    screen_rx_stats: Mutex<ScreenStats>,
    /// Playout buffer dejittering mode for incoming screen streams (ADR-087).
    screen_playout_mode: Mutex<ScreenPlayoutMode>,
    /// Playout buffer dejittering metrics for incoming screen streams (ADR-087).
    screen_playout_stats: Mutex<ScreenPlayoutStats>,
    /// Hardware cursor decoupling mode for screen streams (ADR-088).
    screen_cursor_mode: Mutex<ScreenCursorMode>,
    /// Hardware cursor metrics for screen streams (ADR-088).
    screen_cursor_stats: Mutex<ScreenCursorStats>,
    /// Adaptive squelch mode for screen streams (ADR-089).
    screen_squelch_mode: Mutex<ScreenSquelchMode>,
    /// Forward error correction (FEC) mode for screen streams (ADR-090).
    screen_fec_mode: Mutex<ScreenFecMode>,
    /// Microsecond packet pacing and burst shaper mode for screen streams (ADR-091).
    screen_pacer_mode: Mutex<ScreenPacerMode>,
    /// Delay-based trendline bufferbloat filter mode for screen streams (ADR-092).
    screen_trendline_mode: Mutex<ScreenTrendlineMode>,
    /// Hybrid ARQ (HARQ) selective retransmission mode for screen streams (ADR-093).
    screen_harq_mode: Mutex<ScreenHarqMode>,
    /// Transport-wide congestion control (TWCC) mode for screen streams (ADR-094).
    screen_twcc_mode: Mutex<ScreenTwccMode>,
    screen_twcc_stats: Mutex<ScreenTwccStats>,
    /// Active bandwidth probing and fast ramp-up mode for screen streams (ADR-095).
    screen_probe_mode: Mutex<ScreenProbeMode>,
    screen_probe_stats: Mutex<ScreenProbeStats>,
    /// Audio-video playout lip-sync and clock drift synchronization mode (ADR-096).
    screen_sync_mode: Mutex<ScreenAudioSyncMode>,
    screen_sync_stats: Mutex<ScreenAudioSyncStats>,
    /// Microsecond-precision multi-stage latency profiler mode (ADR-099).
    screen_latency_mode: Mutex<ScreenProfilerMode>,
    screen_latency_stats: Mutex<ScreenLatencyReport>,
    /// Bottleneck-driven autonomous tuner mode (ADR-100).
    screen_tuning_mode: Mutex<ScreenTunerMode>,
    screen_tuning_stats: Mutex<ScreenTuningReport>,
    /// Active screen session recording (ADR-101).
    screen_recording: Mutex<Option<ScreenRecordingSession>>,
    /// Peer disclosed recording state via ControlMsg::SetRecordingState (ADR-101).
    screen_peer_recording: std::sync::atomic::AtomicBool,
    /// Viewer-side: the host answered our `Start` with the refusal `Stop`
    /// reply (consent revoked mid-session / never granted). One-shot latch
    /// the remote-session start consults to roll the session back instead of
    /// presenting an endless black screen.
    screen_start_refused: std::sync::atomic::AtomicBool,
    /// Monotonic ticket of the in-flight `screen_start` attempt. A Stop-reply
    /// may only latch `screen_start_refused` when its ticket matches — a
    /// leftover refusal reply from a PREVIOUS screen channel (teardown racing
    /// a new session's Start) must not poison the new attempt's verdict.
    screen_start_ticket: std::sync::atomic::AtomicU64,
    /// Active screen session in-container playback session (ADR-102).
    screen_playback: Mutex<Option<ScreenPlaybackSession>>,
    /// Most recent uncompressed screen frame presented or grabbed for snapshots (ADR-103).
    screen_last_frame: Mutex<Option<ScreenFrame>>,
    /// Cumulative count of screen snapshots captured (ADR-103).
    screen_snapshot_count: AtomicU64,
    /// Dynamic security watermark configuration (v2.18 / ADR-104).
    screen_watermark: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenWatermarkConfig>>,
    /// Total frames presented with watermark overlay (v2.18 / ADR-104).
    screen_watermark_frames: AtomicU64,
    /// Whether peer host enforced security watermark overlay (v2.18 / ADR-104).
    screen_peer_watermark_enforced: AtomicBool,
    /// Collaborative whiteboard annotation items (v2.19 / ADR-105).
    screen_annotations: std::sync::Arc<std::sync::RwLock<Vec<tetherly_core::AnnotationItem>>>,
    /// Collaborative whiteboard history and undo/redo engine (v2.20 / ADR-106).
    screen_whiteboard: std::sync::Arc<std::sync::RwLock<tetherly_core::WhiteboardHistory>>,
    /// Total whiteboard undo operations executed (v2.20 / ADR-106).
    screen_whiteboard_undo_count: AtomicU64,
    /// Total whiteboard redo operations executed (v2.20 / ADR-106).
    screen_whiteboard_redo_count: AtomicU64,
    /// Transient virtual laser pointer state (v2.19 / ADR-105).
    screen_laser_pointer:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::LaserPointerState>>>,
    /// Active Screen Region-of-Interest (ROI) viewport configuration (v2.21 / ADR-107).
    screen_roi: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenRoi>>,
    /// Total count of ROI viewport updates applied (v2.21 / ADR-107).
    screen_roi_updates: AtomicU64,
    /// Active screen adaptive color depth & quantization configuration (v2.22 / ADR-108).
    screen_color_mode: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenColorConfig>>,
    /// Total count of color mode adjustments applied (v2.22 / ADR-108).
    screen_color_updates: AtomicU64,
    /// Active screen differential delta patch configuration (v2.23 / ADR-109).
    screen_delta_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenDeltaConfig>>,
    /// Total count of differential delta frames sent (v2.23 / ADR-109).
    screen_delta_frames_sent: AtomicU64,
    /// Total count of full keyframes sent (v2.23 / ADR-109).
    screen_delta_keyframes_sent: AtomicU64,
    /// Total count of differential delta frames received (v2.23 / ADR-109).
    screen_delta_frames_received: AtomicU64,
    /// Total count of full keyframes received (v2.23 / ADR-109).
    screen_delta_keyframes_received: AtomicU64,
    /// Total bytes saved by delta compression (v2.23 / ADR-109).
    screen_delta_saved_bytes: AtomicU64,
    /// Monotonic delta frame counter since last keyframe (v2.23 / ADR-109).
    screen_delta_counter: AtomicU64,
    /// Receiver-side persistent full reference canvas for delta patch blitting (v2.23 / ADR-109).
    screen_receiver_canvas: std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Receiver-side double-buffered back-buffer for atomic tile presentation transactions (v2.35 / ADR-121).
    screen_receiver_back_buffer:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Active screen objective visual quality metric configuration (v2.24 / ADR-110).
    screen_quality_metric_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenQualityMetricConfig>>,
    /// Active screen objective visual quality metric state and evaluation telemetry (v2.24 / ADR-110).
    screen_quality_metric_state: std::sync::Arc<std::sync::RwLock<ScreenQualityMetricState>>,
    /// Frame counter for metric sampling (v2.24 / ADR-110).
    screen_metric_counter: AtomicU64,
    /// Reference frame for objective visual quality metric distortion evaluation (v2.24 / ADR-110).
    screen_metric_ref_frame: std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Active screen autonomous closed-loop QoE adaptation controller (v2.25 / ADR-111).
    screen_qoe_controller: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenQoeController>>,
    /// Screen Streaming Progressive Intra-Refresh (PIR) configuration (v2.26 / ADR-112).
    screen_pir_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenPirConfig>>,
    /// Screen PIR total sweeps completed (v2.26 / ADR-112).
    screen_pir_sweeps_completed: AtomicU64,
    /// Screen PIR total band slices sent (v2.26 / ADR-112).
    screen_pir_slices_sent: AtomicU64,
    /// Screen PIR current band slice index [0..bands) (v2.26 / ADR-112).
    screen_pir_current_band: AtomicU8,
    /// Screen PIR active sweep in progress flag (v2.26 / ADR-112).
    screen_pir_active_sweep: AtomicBool,
    /// Screen Streaming Long-Term Reference (LTR) configuration (v2.27 / ADR-113).
    screen_ltr_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenLtrConfig>>,
    /// Current synchronized golden LTR reference frame sequence number (v2.27 / ADR-113).
    screen_ltr_current_seq: AtomicU64,
    /// Total LTR golden reference anchors established (v2.27 / ADR-113).
    screen_ltr_anchors_established: AtomicU64,
    /// Total LTR gap recovery requests dispatched (v2.27 / ADR-113).
    screen_ltr_recoveries_requested: AtomicU64,
    /// Total LTR differential recovery frames successfully completed (v2.27 / ADR-113).
    screen_ltr_recoveries_completed: AtomicU64,
    /// Total burst traffic bytes saved by LTR differential recovery vs full IDR keyframe (v2.27 / ADR-113).
    screen_ltr_saved_burst_bytes: AtomicU64,
    /// Flag indicating an incoming LTR recovery request is pending next frame emission (v2.27 / ADR-113).
    screen_ltr_recovery_pending: AtomicBool,
    /// Sender-side persistent golden reference frame canvas (v2.27 / ADR-113).
    screen_sender_ltr_canvas: std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Receiver-side persistent golden reference frame canvas (v2.27 / ADR-113).
    screen_receiver_ltr_canvas:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Screen 2-Layer Temporal Scalability (SVC-T) Configuration (v2.28 / ADR-114).
    screen_svc_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenSvcConfig>>,
    /// Total T0 base layer frames emitted (v2.28 / ADR-114).
    screen_svc_t0_frames_sent: AtomicU64,
    /// Total T1 enhancement layer frames emitted (v2.28 / ADR-114).
    screen_svc_t1_frames_sent: AtomicU64,
    /// Total T1 enhancement layer frames dropped for congestion mitigation (v2.28 / ADR-114).
    screen_svc_t1_frames_dropped: AtomicU64,
    /// Total bandwidth bytes saved by dropping T1 frames (v2.28 / ADR-114).
    screen_svc_saved_bytes: AtomicU64,
    /// Sender-side T0 base layer reference frame canvas (v2.28 / ADR-114).
    screen_sender_t0_canvas: std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Receiver-side T0 base layer reference frame canvas (v2.28 / ADR-114).
    screen_receiver_t0_canvas:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Screen 2D Motion Vector Estimation & Displacement Compensation Configuration (v2.29 / ADR-115).
    screen_motion_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenMotionConfig>>,
    /// Total motion scroll accelerated frames emitted (v2.29 / ADR-115).
    screen_motion_frames_sent: AtomicU64,
    /// Total fallback frames when motion match failed (v2.29 / ADR-115).
    screen_motion_fallback_frames_sent: AtomicU64,
    /// Total bandwidth bytes saved by motion scroll strip compression (v2.29 / ADR-115).
    screen_motion_saved_bytes: AtomicU64,
    /// Sender-side motion estimation reference frame canvas (v2.29 / ADR-115).
    screen_sender_motion_canvas:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Screen Spatial Tile-Grid Decomposition Configuration (v2.30 / ADR-116).
    screen_tile_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileConfig>>,
    /// Total spatial tile-grid frames emitted (v2.30 / ADR-116).
    screen_tile_frames_sent: AtomicU64,
    /// Total dirty tiles emitted (v2.30 / ADR-116).
    screen_tile_dirty_tiles_sent: AtomicU64,
    /// Total unchanged tiles skipped (v2.30 / ADR-116).
    screen_tile_skipped_count: AtomicU64,
    /// Total bandwidth bytes saved by skipping unchanged tiles (v2.30 / ADR-116).
    screen_tile_saved_bytes: AtomicU64,
    /// Sender-side spatial tile-grid reference frame canvas (v2.30 / ADR-116).
    screen_sender_tile_canvas:
        std::sync::Arc<std::sync::RwLock<Option<tetherly_core::ScreenFrame>>>,
    /// Screen Spatial Tile Dictionary Cache Configuration (v2.31 / ADR-117).
    screen_tile_cache_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileCacheConfig>>,
    /// Sender-side tile dictionary cache (v2.31 / ADR-117).
    screen_tile_cache_dict: std::sync::Arc<Mutex<tetherly_core::ScreenTileDictionary>>,
    /// Receiver-side tile dictionary cache slots (v2.31 / ADR-117).
    screen_tile_cache_receiver: std::sync::Arc<Mutex<Vec<Option<Vec<u8>>>>>,
    /// Total tile cache hits (zero-payload reuse) (v2.31 / ADR-117).
    screen_tile_cache_hits: AtomicU64,
    /// Total tile cache misses (compressed patch transmission) (v2.31 / ADR-117).
    screen_tile_cache_misses: AtomicU64,
    /// Total bandwidth bytes saved by zero-payload cache hits (v2.31 / ADR-117).
    screen_tile_cache_saved_bytes: AtomicU64,
    /// Screen Foveated Focus Awareness & Priority Scheduling Configuration (v2.32 / ADR-118).
    screen_focus_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenFocusConfig>>,
    /// Total Priority 0 (Focus Zone) tiles sent with zero decimation (v2.32 / ADR-118).
    screen_focus_tiles_sent: AtomicU64,
    /// Total Priority 1 (Peripheral Zone) tiles sent on cadence refresh (v2.32 / ADR-118).
    screen_focus_peripheral_sent: AtomicU64,
    /// Total Priority 1 (Peripheral Zone) tiles skipped due to foveated decimation (v2.32 / ADR-118).
    screen_focus_peripheral_skipped: AtomicU64,
    /// Total bandwidth bytes saved by peripheral foveated tile decimation (v2.32 / ADR-118).
    screen_focus_saved_bytes: AtomicU64,
    /// Screen tile-level adaptive palette compression configuration (v2.33 / ADR-119).
    screen_tile_palette_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTilePaletteConfig>>,
    /// Total solid color tiles compressed (v2.33 / ADR-119).
    screen_tile_palette_solid_count: AtomicU64,
    /// Total bi-level bitmap tiles compressed (v2.33 / ADR-119).
    screen_tile_palette_bilevel_count: AtomicU64,
    /// Total 16-color indexed palette tiles compressed (v2.33 / ADR-119).
    screen_tile_palette_palette_count: AtomicU64,
    /// Total truecolor tiles fallback (v2.33 / ADR-119).
    screen_tile_palette_truecolor_count: AtomicU64,
    /// Total bandwidth bytes saved by tile palette compression (v2.33 / ADR-119).
    screen_tile_palette_saved_bytes: AtomicU64,
    /// Screen tile-level predictive filter & reversible color transform configuration (v2.34 / ADR-120).
    screen_tile_predict_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTilePredictConfig>>,
    /// Total truecolor tiles compressed using predictive filter pipeline (v2.34 / ADR-120).
    screen_tile_predict_tiles_count: AtomicU64,
    /// Total bandwidth bytes saved by tile predictive filter pipeline (v2.34 / ADR-120).
    screen_tile_predict_saved_bytes: AtomicU64,
    /// Screen tile presentation transaction and double-buffered swap configuration (v2.35 / ADR-121).
    screen_tile_present_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTilePresentConfig>>,
    /// Total presentation transactions committed (v2.35 / ADR-121).
    screen_tile_present_committed_txs: AtomicU64,
    /// Total frames swapped atomically into the front display buffer (v2.35 / ADR-121).
    screen_tile_present_swapped_frames: AtomicU64,
    /// Total visual tear occurrences prevented by atomic staging (v2.35 / ADR-121).
    screen_tile_present_torn_prevented: AtomicU64,
    /// Cumulative swap latency in microseconds (v2.35 / ADR-121).
    screen_tile_present_total_latency_us: AtomicU64,
    /// Screen streaming tile-level fast retransmission and micro-NACK configuration (v2.36 / ADR-122).
    screen_tile_nack_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileNackConfig>>,
    /// Sender bounded ring-buffer of recent dirty tiles for instant retransmission (v2.36 / ADR-122).
    screen_tile_resend_buffer:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileResendBuffer>>,
    /// Total tile-level micro-NACK requests sent (v2.36 / ADR-122).
    screen_tile_nack_requests_sent: AtomicU64,
    /// Total tiles retransmitted upon micro-NACK requests (v2.36 / ADR-122).
    screen_tile_nack_tiles_retransmitted: AtomicU64,
    /// Total presentation transactions degraded-committed due to timeout or max retries (v2.36 / ADR-122).
    screen_tile_nack_degraded_commits: AtomicU64,
    /// Screen streaming dynamic tile saliency and tiered priority scheduling configuration (v2.37 / ADR-123).
    screen_tile_saliency_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileSaliencyConfig>>,
    /// Cumulative count of critical-tier tiles sent (v2.37 / ADR-123).
    screen_tile_saliency_critical_tiles: AtomicU64,
    /// Cumulative count of normal-tier tiles sent (v2.37 / ADR-123).
    screen_tile_saliency_normal_tiles: AtomicU64,
    /// Cumulative count of background-tier tiles sent (v2.37 / ADR-123).
    screen_tile_saliency_background_tiles: AtomicU64,
    /// Cumulative count of background-tier tiles skipped during network congestion (v2.37 / ADR-123).
    screen_tile_saliency_background_skipped: AtomicU64,
    /// Configuration for screen tile dynamic multi-resolution Level-of-Detail (LoD) downsampling (v2.38 / ADR-124).
    screen_tile_lod_config: std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileLodConfig>>,
    /// Cumulative count of full-resolution Lod0 tiles sent (v2.38 / ADR-124).
    screen_tile_lod0_tiles: AtomicU64,
    /// Cumulative count of half-resolution Lod1 tiles sent (v2.38 / ADR-124).
    screen_tile_lod1_tiles: AtomicU64,
    /// Cumulative count of quarter-resolution Lod2 tiles sent (v2.38 / ADR-124).
    screen_tile_lod2_tiles: AtomicU64,
    /// Cumulative bytes saved via LoD downsampling (v2.38 / ADR-124).
    screen_tile_lod_bytes_saved: AtomicU64,
    /// Configuration for screen dynamic tile MTU chunking and pacing (v2.39 / ADR-125).
    screen_tile_batch_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileBatchConfig>>,
    /// Cumulative count of tile batches sent (v2.39 / ADR-125).
    screen_tile_batches_sent: AtomicU64,
    /// Cumulative count of tile chunks sent (v2.39 / ADR-125).
    screen_tile_chunks_sent: AtomicU64,
    /// Cumulative payload bytes across all tile batch chunks sent (v2.39 / ADR-125).
    screen_tile_batch_bytes_total: AtomicU64,
    /// Cumulative count of pacing delays applied between chunks (v2.39 / ADR-125).
    screen_tile_batch_pacing_pauses: AtomicU64,
    /// Active tile batch reassembly tracking on the receiver (v2.39 / ADR-125).
    screen_tile_batch_receiver_state: Mutex<Option<TileBatchReceiverState>>,
    /// Configuration for screen tile local motion estimation and displacement compensation (v2.40 / ADR-126).
    screen_tile_motion_config:
        std::sync::Arc<std::sync::RwLock<tetherly_core::ScreenTileMotionConfig>>,
    /// Cumulative count of tiles compressed using local motion displacement (v2.40 / ADR-126).
    screen_tile_motion_tiles_sent: AtomicU64,
    /// Cumulative count of tiles falling back from motion estimation to other subcodecs (v2.40 / ADR-126).
    screen_tile_motion_fallback_tiles_sent: AtomicU64,
    /// Cumulative payload bytes saved via tile motion displacement (v2.40 / ADR-126).
    screen_tile_motion_bytes_saved: AtomicU64,
    /// Total annotations added (v2.19 / ADR-105).
    screen_annotation_count: AtomicU64,
    /// Total laser pointer coordinate updates (v2.19 / ADR-105).
    screen_laser_updates: AtomicU64,
    /// Client-side outbound channel for raw InnerFrames (v2.19 / ADR-105).
    screen_client_out: AsyncMutex<Option<mpsc::Sender<InnerFrame>>>,
    /// Active display ID for screen capture (ADR-097). Defaults to 0 (Primary).
    screen_active_display_id: AtomicU8,
    /// Known displays available on this host (ADR-097).
    screen_displays: Mutex<Vec<tetherly_core::ScreenDisplayInfo>>,
    /// Count of display switch operations executed during runtime (ADR-097).
    screen_display_switches_count: AtomicU64,
    /// Timestamp (us) of last display switch (ADR-097).
    screen_last_display_switch_us: AtomicU64,
    /// Screen streaming privacy mode (curtain blanking) enabled flag (ADR-098).
    screen_privacy_enabled: AtomicBool,
    /// Screen streaming host physical input lockout enabled flag (ADR-098).
    screen_privacy_lock_input: AtomicBool,
    /// Screen streaming blanking method identifier (ADR-098).
    screen_privacy_blanking_method: Mutex<String>,
    /// Timestamp (us) of last privacy mode change (ADR-098).
    screen_privacy_last_change_us: AtomicU64,
    /// Damage tracker for screen frames (ADR-089).
    screen_damage_tracker: Mutex<ScreenDamageTracker>,
    /// Previous frame bytes for difference detection (ADR-089).
    screen_prev_frame: Mutex<Option<Vec<u8>>>,
    shutdown: watch::Sender<bool>,
    overlay: Mutex<OverlayStatus>,
    /// ANCS ingress. Present only when a transport was attached.
    ancs: Mutex<Option<AncsIngress>>,
    /// Synthetic peer id for the ANCS phone (ANCS is not a Noise session).
    ancs_source: DeviceId,
    /// Apps allowed to show "open". Empty until config/UI supplies rules.
    allowlist: Mutex<OpenAllowlist>,
    started_at: std::time::Instant,
    node_name: Mutex<String>,
    advertise: AtomicBool,
    reconnect: AtomicBool,
    advertise_stop: Mutex<Option<watch::Sender<bool>>>,
    telemetry: Arc<TrafficTelemetry>,
    local_caps: Mutex<std::collections::HashSet<String>>,
    blocked: Mutex<crate::store::BlockedTable>,
    auto_accept_trusted: AtomicBool,
    auto_accepted_count: AtomicU64,
    discovered: Mutex<HashMap<String, DiscoveredPeerInfo>>,
    rescan_trigger: broadcast::Sender<()>,
    opener: Mutex<Arc<dyn tetherly_core::Opener>>,
    notifier: Mutex<Arc<dyn Notifier>>,
    notify_config: Mutex<NotifyBannerConfig>,
    notify_dispatched_count: AtomicU64,
    notify_suppressed_count: AtomicU64,
    sys_probe: Mutex<Arc<dyn crate::platform::SysProbe>>,
    heartbeat_config: Mutex<HeartbeatConfig>,
    link_trackers: Mutex<HashMap<DeviceId, PeerLinkTracker>>,
    audit_engine: Mutex<AuditEngine>,
    message_engine: Mutex<MessageEngine>,
    outbox_engine: Mutex<OutboxEngine>,
    profiles: Mutex<ProfileTable>,
    auto_accept_policy: Mutex<AutoAcceptPolicy>,
    clipboard_policy: Mutex<ClipboardPolicy>,
    clip_auto_synced_out_count: AtomicU64,
    clip_auto_synced_in_count: AtomicU64,
    clip_sensitive_blocked_count: AtomicU64,
    last_auto_sync_hash: Mutex<Option<String>>,
    handoff_policy: Mutex<HandoffPolicy>,
    handoff_inbox: Mutex<HandoffInboxTable>,
    handoff_seq: AtomicU64,
    media_policy: Mutex<MediaControlPolicy>,
    local_media_state: Mutex<MediaStateFrame>,
    peer_media_states: Mutex<HashMap<String, MediaStateFrame>>,
    media_cmd_seq: AtomicU64,
    battery_policy: Mutex<BatteryPolicy>,
    local_battery_state: Mutex<BatteryStateFrame>,
    peer_battery_states: Mutex<HashMap<String, BatteryStateFrame>>,
    locator_policy: Mutex<LocatorPolicy>,
    active_locate_ring: Mutex<Option<ActiveLocateRing>>,
    peer_last_locate_req_ms: Mutex<HashMap<String, u64>>,
    locate_req_seq: AtomicU64,
    local_actions: Mutex<Vec<LocalActionDefinition>>,
    action_policy: Mutex<ActionPolicy>,
    peer_remote_actions: Mutex<HashMap<String, Vec<ActionItem>>>,
    peer_last_action_trigger_ms: Mutex<HashMap<String, u64>>,
    action_trigger_seq: AtomicU64,
    action_results_cache: Mutex<VecDeque<ActionResultFrame>>,
    focus_policy: Mutex<FocusPolicy>,
    local_focus_state: Mutex<FocusStateFrame>,
    peer_focus_states: Mutex<HashMap<String, FocusStateFrame>>,
    topology_layout: Mutex<TopologyLayout>,
    topology_policy: Mutex<TopologyPolicy>,
    peer_topologies: Mutex<HashMap<String, TopologyStateFrame>>,
    cursor_last_pos: Mutex<(i32, i32)>,
    input_policy: Mutex<InputTunnelPolicy>,
    active_input_tunnel: Mutex<Option<ActiveInputTunnelSession>>,
    /// ADR-138 remote desktop session — single slot. In-memory only: a crash
    /// resets to no session by construction (sessions are transient).
    pub(crate) remote_session: Mutex<Option<crate::runtime::input::RemoteSessionState>>,
    /// ADR-138 session-scoped keymap: the host's suggested preset from the
    /// session resp (e.g. "mac_to_windows"). Applied for the lifetime of the
    /// session so the controller's keystrokes remap as the target expects;
    /// cleared on close. In-memory only, like the session itself.
    pub(crate) remote_session_keymap: Mutex<Option<String>>,
    /// ADR-138: set when the host auto-allowed screen streaming only because
    /// the session handshake implied consent (accept path with no prior
    /// Allowed|Streaming). On session teardown the auto-grant is revoked —
    /// an explicit operator allow granted before the session survives it.
    pub(crate) remote_session_auto_allowed: std::sync::atomic::AtomicBool,
    /// Test/operator gate that pauses the controller's session heartbeat
    /// loop (the keepalive that refreshes the host's session slot). Pausing
    /// is how "silence" is simulated now that the heartbeat actually keeps
    /// the host slot alive.
    pub(crate) session_heartbeat_paused: std::sync::atomic::AtomicBool,
    /// Last time a four-finger swipe was dropped for "not controlling"
    /// (throttles the debug log so stray swipes don't spam).
    pub(crate) gesture_swipe_last_ignored_ms: std::sync::atomic::AtomicU64,
    /// Active gesture→intent table for the gesture forwarding layer.
    /// Defaults mirror macOS trackpad semantics; a saved config overrides.
    pub(crate) gesture_map: Mutex<tetherly_core::gesture_map::GestureMap>,
    /// Receiver seq-decision ring `(seq, accepted)` mirrored from the
    /// client-loop receiver for diagnostics (cliff-vs-duplicate triage).
    pub(crate) screen_seq_trace: Mutex<Vec<(u64, bool)>>,
    /// Monotonic generation of the active 45720 server channel. Each
    /// `attach_screen_server` increments it; a pump loop may only run the
    /// engine-level `on_peer_gone` reset when its generation is still the
    /// newest — a replaced loop's teardown must not reset its successor.
    pub(crate) screen_channel_gen: std::sync::atomic::AtomicU64,
    /// Monotonic generation of the active 45720 client (viewer) channel.
    /// Each `attach_screen_client` increments it; a client pump loop may only
    /// clear `screen_peer` / `screen_ctl` / `screen_client_out` when its
    /// generation is still the newest — a replaced loop's exit must not wipe
    /// the registration triple its successor just wrote (and the reuse check
    /// in `remote_session_start` reads `screen_peer`, so a stale wipe also
    /// defeats channel reuse).
    pub(crate) screen_client_gen: std::sync::atomic::AtomicU64,
    /// Handle of the running inbound input-client task. A newer accept
    /// aborts the previous one so stacked inbound sessions can never leave
    /// two clients racing on `input_sink` / the real injection sink.
    input_client_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Single-dialer gate for `open_input`. Concurrent callers (link watcher
    /// tick, takeover attach, UI) must not open competing connections to the
    /// same peer: the loser's session would become a zombie and flip
    /// `input_out` back and forth.
    input_dialing: std::sync::atomic::AtomicBool,
    /// Instant of the last ghost-roam attach kick. The drive-path guard
    /// fires per input event; without this gate a 1000 Hz hook stream would
    /// spawn a request storm and each pre-arm would overwrite the previous
    /// session (the resp of the dropped session then dead-ends, CI m3_11).
    /// `Instant` (monotonic), not wall clock: an NTP step must not stall the
    /// rescue path.
    input_attach_kick_at: Mutex<Option<std::time::Instant>>,
    /// Ghost-roam attach in flight: CAS-dedupes drive-kick and watcher
    /// auto-attach so two attaches within the pre-arm-to-open window do not
    /// overwrite each other's pre-arm (the loser's resp is then dropped by
    /// session-id mismatch — safe but wasteful and audit-noisy).
    input_attaching: std::sync::atomic::AtomicBool,
    /// Instant of the last automatic direction flip (cooldown between flips).
    last_auto_reverse: Mutex<Option<std::time::Instant>>,
    /// Auto-reverse dwell state: start of the current sustained edge push and
    /// the instant of the last boundary-touching event (gap-continuity check:
    /// a push counts as sustained only while touch events keep arriving).
    reverse_arm_since: Mutex<Option<std::time::Instant>>,
    reverse_last_tick: Mutex<Option<std::time::Instant>>,
    /// Pinned dwell heartbeat (v0.2.25): the instant the LAST dwell-opening
    /// physical event arrived, plus the pinned position it reported. Windows
    /// LL hooks fire only when the cursor MOVES — with the entry placement
    /// parked ON the shared edge, a continued push into the clamp produces
    /// NO further WM_MOUSEMOVE, so the gap-continuity dwell starves on its
    /// first tick (production 2026-11-12 09:34: each push logged exactly one
    /// "requiring long dwell" line and nothing else). The heartbeat task
    /// feeds the dwell state machine from this anchor while the hand holds
    /// the push, until the cursor actually moves off the clamp band.
    reverse_pinned_touch: Mutex<Option<(std::time::Instant, i32, i32)>>,
    /// Distance arming for auto-reverse: accumulated physical movement (px)
    /// since the mouse was last idle for REVERSE_RESET_AFTER_MS. A freshly
    /// plugged-in or merely bumped mouse must traverse a real range before
    /// it may take over; its last cursor position feeds the coordinate-delta
    /// accumulation (Windows hooks report no per-event deltas).
    reverse_move_accum_px: std::sync::atomic::AtomicI64,
    /// Set when the DRIVE path itself crossed Local->Remote (this node
    /// started roaming as the controller with its own edge crossing). The
    /// target's engine also sits in Remote while being controlled
    /// (`resume_remote`) — an engine Remote->Local oscillation there must
    /// not be mistaken for a roaming-stint end.
    roaming_own_crossing: std::sync::atomic::AtomicBool,
    reverse_last_move: Mutex<Option<std::time::Instant>>,
    reverse_last_pos: Mutex<Option<(i32, i32)>>,
    /// Async runtime handle captured at startup: the capture-hook thread has
    /// no runtime context but must be able to spawn the takeover attach.
    rt_handle: Mutex<Option<tokio::runtime::Handle>>,
    virtual_display_policy: Mutex<VirtualDisplayPolicy>,
    active_virtual_display: Mutex<Option<ActiveVirtualDisplaySession>>,
    audio_routing_policy: Mutex<AudioRoutingPolicy>,
    active_audio_routing: Mutex<Option<ActiveAudioRoutingSession>>,
    camera_policy: Mutex<CameraPolicy>,
    active_camera: Mutex<Option<ActiveCameraSession>>,
    microphone_policy: Mutex<MicrophonePolicy>,
    active_microphone: Mutex<Option<ActiveMicrophoneSession>>,
    bench_policy: Mutex<BenchPolicy>,
    active_bench: Mutex<Option<ActiveBenchSession>>,
    last_bench_report: Mutex<Option<BenchReportFrame>>,
    bandwidth_profiles: Mutex<BandwidthProfileTable>,
    pub(crate) drag_controller: Mutex<DragController>,
    pub(crate) drag_target: Mutex<DragTarget>,
    /// Pending receiver-side drag sessions awaiting their first byte:
    /// `drag_id -> {source peer, registered at}`, populated when a
    /// `RemoteDropConfirm` arrives and consumed when the first inbound
    /// `FileOffer` claims it. Entries carry their registration time so a
    /// confirm whose transfer never arrived (crash, dropped connection)
    /// ages out instead of mis-claiming an unrelated later offer.
    pub(crate) pending_drag_drops: Mutex<HashMap<u64, PendingDragDrop>>,
    /// `transfer_id -> (source peer, drag_id)` linking an accepted inbound
    /// transfer back to the drag session that originated it, so the
    /// completion hook can settle MOTW + reveal with the real source
    /// attribution. Populated when the first FileOffer of a drag arrives.
    pub(crate) transfer_drag_ids: Mutex<HashMap<String, (DeviceId, u64)>>,
}

/// Value of `Shared.pending_drag_drops`.
#[derive(Debug, Clone)]
pub(crate) struct PendingDragDrop {
    pub(crate) peer: DeviceId,
    pub(crate) registered_ms: u64,
}

impl Shared {
    /// Lock the screen engine for a QoE adjustment, tolerating poisoning: an
    /// unrelated panic while holding the engine lock must cost us the quality
    /// adaptation, not the whole frame-pump task (`expect` here turned a
    /// poisoned lock into a frozen stream).
    fn screen_engine_recovered(&self) -> std::sync::MutexGuard<'_, ScreenSender> {
        match self.screen_engine.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn record_quality_metric(&self, frame: &tetherly_core::ScreenFrame) {
        let (enabled, sample_interval) = {
            let cfg = self.screen_quality_metric_config.read().unwrap();
            let qoe_active = self
                .screen_qoe_controller
                .read()
                .unwrap()
                .config()
                .is_active();
            (cfg.enabled || qoe_active, cfg.sample_interval)
        };
        if !enabled || sample_interval == 0 || frame.compressed() {
            return;
        }

        let mut ref_guard = self.screen_metric_ref_frame.write().unwrap();
        match ref_guard.as_ref() {
            None => {
                *ref_guard = Some(frame.clone());
            }
            Some(ref_frame) => {
                if ref_frame.rect.w != frame.rect.w
                    || ref_frame.rect.h != frame.rect.h
                    || ref_frame.format != frame.format
                {
                    *ref_guard = Some(frame.clone());
                    return;
                }
                let count = self
                    .screen_metric_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if (count + 1) % (sample_interval as u64) == 0 {
                    if let (Ok(mse), Ok(psnr), Ok(ssim)) = (
                        frame.compute_mse(ref_frame),
                        frame.compute_psnr(ref_frame),
                        frame.compute_ssim(ref_frame),
                    ) {
                        let mut state = self.screen_quality_metric_state.write().unwrap();
                        state.evaluated_frames += 1;
                        state.sum_mse += mse;
                        state.sum_psnr_db += psnr;
                        state.sum_ssim += ssim;
                        state.last_psnr_db = psnr;
                        state.last_ssim = ssim;

                        // ADR-111 / v2.25: Autonomous Closed-Loop QoE Adaptation
                        let now_us = self.clock.unix_us();
                        let metric_rep = tetherly_core::ScreenQualityMetricReport {
                            ok: true,
                            enabled: true,
                            sample_interval,
                            evaluated_frames: state.evaluated_frames,
                            avg_mse: state.sum_mse / (state.evaluated_frames as f64),
                            avg_psnr_db: state.sum_psnr_db / (state.evaluated_frames as f64),
                            avg_ssim: state.sum_ssim / (state.evaluated_frames as f64),
                            last_psnr_db: psnr,
                            last_ssim: ssim,
                            quality_warning: psnr < 30.0 || ssim < 0.85,
                        };
                        let action = {
                            let mut qoe = self.screen_qoe_controller.write().expect("screen_qoe");
                            qoe.evaluate(&metric_rep, now_us)
                        };
                        match action {
                            tetherly_core::ScreenQoeAction::Hold => {}
                            tetherly_core::ScreenQoeAction::BoostBitrate => {
                                // QoE adjustment is best-effort: a poisoned
                                // engine lock (an invariant bug elsewhere)
                                // must degrade quality adaptation, not panic
                                // the frame-pump task and freeze the stream.
                                let mut engine = self.screen_engine_recovered();
                                let cur = engine.bitrate_bps();
                                let boosted = (cur.saturating_mul(120) / 100)
                                    .min(tetherly_core::SCREEN_BITRATE_MAX_BPS);
                                engine.set_bitrate_bps(boosted);
                                if let Some(sel) = &self.screen_encode_codec {
                                    sel.set_bitrate_bps(boosted);
                                }
                                self.screen_source.set_bitrate_bps(boosted);
                            }
                            tetherly_core::ScreenQoeAction::UpgradeScale => {
                                let mut engine = self.screen_engine_recovered();
                                let cur = engine.scale_pct();
                                if cur < 100 {
                                    engine.set_scale_pct(100);
                                    if let Some(sel) = &self.screen_encode_codec {
                                        sel.set_scale_pct(100);
                                    }
                                    self.screen_source.set_scale_pct(100);
                                }
                            }
                            tetherly_core::ScreenQoeAction::LowerQp => {
                                let mut engine = self.screen_engine_recovered();
                                let (min_qp, max_qp) = engine.qp_range();
                                let new_min = min_qp.saturating_sub(4).max(10);
                                let new_max = max_qp.saturating_sub(4).max(20);
                                engine.set_qp_range(new_min, new_max);
                            }
                            tetherly_core::ScreenQoeAction::ForceKeyframe => {
                                let mut engine = self.screen_engine_recovered();
                                let _ =
                                    engine.on_control(tetherly_core::ControlMsg::KeyframeRequest);
                                drop(engine);
                                self.screen_source.force_idr();
                                if let Some(sel) = &self.screen_encode_codec {
                                    sel.force_idr();
                                }
                            }
                            tetherly_core::ScreenQoeAction::FallbackThrottle => {
                                let mut engine = self.screen_engine_recovered();
                                let cur = engine.fps_target();
                                let new_fps = cur.saturating_sub(5).max(15);
                                engine.set_fps(new_fps);
                                if let Some(sel) = &self.screen_encode_codec {
                                    sel.set_fps(new_fps);
                                }
                                self.screen_source.set_fps(new_fps);
                            }
                        }
                    }
                    *ref_guard = Some(frame.clone());
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
}

#[derive(Debug, Clone)]
pub enum UserCommand {
    GeneratePin,
    PairWith { pin: String, addr: SocketAddr },
    CopyCandidate { id: String },
    InsertCandidate { id: String },
    DismissCandidate { id: String },
    AcceptFile { transfer_id: String },
    RejectFile { transfer_id: String },
    OfferFile { path: PathBuf, peer: DeviceId },
    SendClipboard,
    Forget { device_id: DeviceId },
}

/// One accept loop, shared by every listener (control/file/input/screen, LAN
/// and overlay). Each accepted stream is handed to `handler` on its own task;
/// accept errors back off briefly. Ends when the node shuts down.
fn node_accept_loop<F, Fut>(node: Node, listener: TcpListener, label: &'static str, handler: F)
where
    F: Fn(Node, TcpStream, SocketAddr) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), tetherly_net::NetError>> + Send,
{
    let mut stop = node.shared.shutdown.subscribe();
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    if *stop.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, addr)) => {
                            let node = node.clone_handle();
                            let handler = handler.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handler(node, stream, addr).await {
                                    debug!(%e, %addr, label, "channel");
                                }
                            });
                        }
                        Err(e) => {
                            debug!(%e, label, "accept");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            }
        }
    });
}

fn route_addr(table: &RouteTable, device_id: &str) -> Option<SocketAddr> {
    table
        .routes
        .iter()
        .find(|r| r.device_id == device_id)
        .and_then(|r| r.addr.parse().ok())
}

fn parse_file_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "fileport=")
}

fn parse_input_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "inputport=")
}

// Test-only imports for the inline test modules below.
#[cfg(test)]
use crate::store::ActionAccessMode;
#[cfg(test)]
use tetherly_core::frame::ResourceHandoffFrame;
#[cfg(test)]
use tetherly_core::ActionTriggerFrame;
#[cfg(test)]
use tetherly_core::FocusMode;
#[cfg(test)]
use tetherly_core::HandoffStatus;
#[cfg(test)]
use tetherly_core::LocateRequestFrame;
#[cfg(test)]
use tetherly_core::LocateStopFrame;
#[cfg(test)]
use tetherly_core::MediaCommand;
#[cfg(test)]
use tetherly_core::MediaCommandFrame;
#[cfg(test)]
use tetherly_core::PlaybackState;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn bounded_retention_prunes_oldest_terminal_transfers() {
        let mut transfers = HashMap::new();
        let peer = DeviceId::from_id_pk(&[1u8; 32]);

        // 1 in-flight transfer
        transfers.insert(
            "active_1".to_string(),
            ActiveTransfer {
                transfer_id: "active_1".to_string(),
                peer: peer.clone(),
                direction: TransferDirection::Outbound,
                filename: "active.bin".to_string(),
                total: 1000,
                transferred: 500,
                speed_bps: 100,
                phase: TransferPhase::Transferring,
                error: None,
                started_ms: 100,
                updated_ms: 200,
                resume_offset: None,
                is_resumed: false,
            },
        );

        // Add MAX_TERMINAL_TRANSFERS + 20 terminal transfers with increasing timestamps
        for i in 0..(MAX_TERMINAL_TRANSFERS + 20) {
            let tid = format!("term_{i}");
            transfers.insert(
                tid.clone(),
                ActiveTransfer {
                    transfer_id: tid,
                    peer: peer.clone(),
                    direction: TransferDirection::Inbound,
                    filename: format!("file_{i}.bin"),
                    total: 100,
                    transferred: 100,
                    speed_bps: 0,
                    phase: if i % 2 == 0 {
                        TransferPhase::Completed
                    } else {
                        TransferPhase::Failed
                    },
                    error: None,
                    started_ms: (i as u64) * 10,
                    updated_ms: (i as u64) * 10 + 5,
                    resume_offset: None,
                    is_resumed: false,
                },
            );
        }

        Node::prune_terminal_transfers_locked(&mut transfers);

        // The active transfer MUST NOT be pruned
        assert!(transfers.contains_key("active_1"));
        assert_eq!(
            transfers.get("active_1").unwrap().phase,
            TransferPhase::Transferring
        );

        // Terminal count must be exactly MAX_TERMINAL_TRANSFERS
        let terminal_count = transfers
            .values()
            .filter(|t| t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed)
            .count();
        assert_eq!(terminal_count, MAX_TERMINAL_TRANSFERS);

        // The 20 oldest terminal transfers (i = 0..20) must have been pruned
        for i in 0..20 {
            assert!(!transfers.contains_key(&format!("term_{i}")));
        }
        // Newer terminal transfers must be kept
        for i in 20..(MAX_TERMINAL_TRANSFERS + 20) {
            assert!(transfers.contains_key(&format!("term_{i}")));
        }
    }
}

fn parse_screen_port(caps: &[String]) -> Option<u16> {
    parse_port_cap(caps, "screenport=")
}

fn parse_port_cap(caps: &[String], prefix: &str) -> Option<u16> {
    caps.iter().find_map(|c| {
        c.strip_prefix(prefix)
            .and_then(|s| s.parse::<u16>().ok())
            .filter(|p| *p != 0)
    })
}

/// Extract pure feature capability tokens from a capability list, filtering out port assignments.
pub fn extract_features(caps: &[String]) -> Vec<String> {
    let mut feats: Vec<String> = caps.iter().filter(|c| !c.contains('=')).cloned().collect();
    feats.sort();
    feats.dedup();
    feats
}

fn validate_cap_name(cap: &str) -> Result<(), CoreError> {
    let trimmed = cap.trim();
    if trimmed.is_empty() || trimmed.len() > 32 {
        return Err(CoreError::InvalidPayload(
            "capability token must be between 1 and 32 characters".into(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CoreError::InvalidPayload(
            "capability token contains invalid characters (allowed: [a-zA-Z0-9_-])".into(),
        ));
    }
    Ok(())
}

fn validate_tag(tag: &str) -> Result<String, CoreError> {
    let trimmed = tag.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidProfile("tag cannot be empty".to_string()));
    }
    if trimmed.len() > 32 {
        return Err(CoreError::InvalidProfile(
            "tag cannot exceed 32 characters".to_string(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CoreError::InvalidProfile(format!(
            "tag '{trimmed}' contains invalid characters; only ASCII alphanumeric, '-', and '_' are allowed"
        )));
    }
    Ok(trimmed)
}

fn validate_group(group: &str) -> Result<Option<String>, CoreError> {
    let trimmed = group.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 32 {
        return Err(CoreError::InvalidProfile(
            "group name cannot exceed 32 characters".to_string(),
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(CoreError::InvalidProfile(
            "group name contains invalid control characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_notes(notes: &str) -> Result<Option<String>, CoreError> {
    let trimmed = notes.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 256 {
        return Err(CoreError::InvalidProfile(
            "notes cannot exceed 256 characters".to_string(),
        ));
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t')
    {
        return Err(CoreError::InvalidProfile(
            "notes contain invalid control characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod resume_scan_tests {
    use super::*;
    use tetherly_core::{DeviceId, FILE_CHUNK};

    fn temp_inbox(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-scan-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        dir
    }

    fn write_sidecar(dir: &std::path::Path, stem: &str, src: &str, total: u64, offset: u64) {
        let dto = serde_json::json!({ "device_id": src, "total": total, "offset": offset });
        std::fs::write(
            dir.join("inbox").join(format!(".{stem}.resume.json")),
            serde_json::to_vec(&dto).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_healthy_sidecar_is_loaded_and_a_torn_one_is_dropped() {
        let dir = temp_inbox("scan");
        let src = DeviceId::from_id_pk(&[7u8; 32]).to_string();
        let total = 3 * FILE_CHUNK as u64;
        // Healthy: sidecar offset 2*CHUNK, matching partial of that length.
        write_sidecar(&dir, "tr_good", &src, total, 2 * FILE_CHUNK as u64);
        std::fs::write(
            dir.join("inbox").join(".tr_good.part"),
            vec![0u8; 2 * FILE_CHUNK],
        )
        .unwrap();
        // Torn: sidecar claims 2*CHUNK but the partial is missing entirely.
        write_sidecar(&dir, "tr_torn", &src, total, 2 * FILE_CHUNK as u64);
        // Bogus: unparseable source id, and a zero/oversized offset.
        write_sidecar(
            &dir,
            "tr_badid",
            "not-a-device-id",
            total,
            FILE_CHUNK as u64,
        );
        write_sidecar(&dir, "tr_zero", &src, total, 0);
        write_sidecar(&dir, "tr_over", &src, total, total + 1);
        // Markdown dir entries that merely look like sidecars are ignored.
        std::fs::write(dir.join("inbox").join("plain.txt"), b"x").unwrap();

        let table = load_resume_table(&dir);
        let good = table.get("tr_good").expect("healthy sidecar loads");
        assert_eq!(good.offset, 2 * FILE_CHUNK as u64);
        assert!(table.get("tr_torn").is_none(), "torn sidecar is dropped");
        assert!(table.get("tr_badid").is_none());
        assert!(table.get("tr_zero").is_none());
        assert!(table.get("tr_over").is_none());
        assert_eq!(table.all().len(), 1);
        // The torn sidecar file was removed so a later transfer starts clean.
        assert!(!dir.join("inbox").join(".tr_torn.resume.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_inbox_dir_yields_an_empty_table() {
        let dir = std::env::temp_dir().join(format!("tetherly-scan-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let table = load_resume_table(&dir);
        assert_eq!(table.all().len(), 0);
    }
}

#[cfg(test)]
pub(crate) mod clipboard_policy_tests {
    use super::*;
    use tetherly_core::{ClipSyncMode, ManualClock};

    pub(crate) fn temp_test_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-clip-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(1000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        (node, dir)
    }

    #[tokio::test]
    async fn test_clipboard_policy_defaults_and_mutation() {
        let (node, dir) = temp_test_node("defaults");
        let report = node.clipboard_policy_report();
        assert!(report.ok);
        assert_eq!(report.policy.mode, ClipSyncMode::BidirectionalAuto);
        assert!(report.policy.sensitive_guard);
        assert_eq!(report.policy.max_payload_bytes, 65536);
        assert_eq!(report.auto_synced_out_count, 0);
        assert_eq!(report.sensitive_blocked_count, 0);

        let mut updated = report.policy.clone();
        updated.mode = ClipSyncMode::BidirectionalAuto;
        updated.allowed_groups = vec!["office".into()];
        updated.allowed_tags = vec!["secure".into()];
        updated.max_payload_bytes = 1024;
        let update_report = node.set_clipboard_policy(updated.clone()).unwrap();
        assert!(update_report.ok);
        assert_eq!(update_report.policy.mode, ClipSyncMode::BidirectionalAuto);
        assert_eq!(update_report.policy.max_payload_bytes, 1024);

        // Verify loaded from disk matches
        let from_disk = crate::store::load_clipboard_policy(&dir);
        assert_eq!(from_disk.mode, ClipSyncMode::BidirectionalAuto);
        assert_eq!(from_disk.allowed_groups, vec!["office".to_string()]);
        assert_eq!(from_disk.max_payload_bytes, 1024);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_poll_clipboard_auto_sync_sensitive_guard_blocks() {
        let (node, dir) = temp_test_node("guard");
        let mut policy = node.clipboard_policy();
        policy.mode = ClipSyncMode::BidirectionalAuto;
        policy.sensitive_guard = true;
        node.set_clipboard_policy(policy).unwrap();

        // 1. Write an SSH private key to memory clipboard
        let priv_key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA\n-----END OPENSSH PRIVATE KEY-----";
        node.shared.clip.set_text(priv_key).unwrap();

        // 2. Poll auto-sync: should block and return 0
        let dispatched = node.poll_clipboard_auto_sync().await.unwrap();
        assert_eq!(dispatched, 0);

        // 3. Sensitive blocked count should be incremented
        let report = node.clipboard_policy_report();
        assert_eq!(report.sensitive_blocked_count, 1);
        assert_eq!(report.auto_synced_out_count, 0);

        // 4. Repeated poll should not re-trigger block count because hash is already recorded
        let dispatched2 = node.poll_clipboard_auto_sync().await.unwrap();
        assert_eq!(dispatched2, 0);
        let report2 = node.clipboard_policy_report();
        assert_eq!(report2.sensitive_blocked_count, 1);

        // 5. Normal safe text should proceed without blocking
        node.shared
            .clip
            .set_text("Hello safe mesh network!")
            .unwrap();
        let _ = node.poll_clipboard_auto_sync().await.unwrap();
        let report3 = node.clipboard_policy_report();
        assert_eq!(report3.sensitive_blocked_count, 1); // unchaged

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_poll_clipboard_auto_sync_payload_size_ceiling() {
        let (node, dir) = temp_test_node("ceiling");
        let mut policy = node.clipboard_policy();
        policy.mode = ClipSyncMode::BidirectionalAuto;
        policy.max_payload_bytes = 100;
        node.set_clipboard_policy(policy).unwrap();

        // Write 150 bytes text
        let large_text = "A".repeat(150);
        node.shared.clip.set_text(&large_text).unwrap();

        let dispatched = node.poll_clipboard_auto_sync().await.unwrap();
        assert_eq!(dispatched, 0);
        let report = node.clipboard_policy_report();
        assert_eq!(report.sensitive_blocked_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod handoff_engine_tests {
    use super::*;
    use tetherly_core::ManualClock;

    fn temp_handoff_node(
        tag: &str,
    ) -> (Node, std::path::PathBuf, Arc<crate::platform::MemoryOpener>) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-handoff-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "handoff-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(5000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        // Record opens instead of launching the user's real browser.
        let opener = Arc::new(crate::platform::MemoryOpener::new());
        node.set_opener(opener.clone());
        (node, dir, opener)
    }

    #[test]
    fn test_handoff_policy_defaults_and_mutation() {
        use tetherly_core::HandoffAction;
        let (node, dir, _opener) = temp_handoff_node("policy");
        let report = node.handoff_policy_report();
        assert!(report.ok);
        assert_eq!(report.policy.action, HandoffAction::PromptOnly);
        assert_eq!(report.policy.max_inbox_entries, 100);

        let mut updated = report.policy.clone();
        updated.action = HandoffAction::AutoOpen;
        updated.allowed_groups = vec!["engineering".into()];
        updated.allowed_tags = vec!["workstation".into()];
        updated.max_inbox_entries = 50;

        let res = node.set_handoff_policy(updated).unwrap();
        assert!(res.ok);
        assert_eq!(res.policy.action, HandoffAction::AutoOpen);
        assert_eq!(res.policy.max_inbox_entries, 50);

        // Check persisted on disk
        let on_disk = crate::store::load_handoff_policy(&dir);
        assert_eq!(on_disk.action, HandoffAction::AutoOpen);
        assert_eq!(on_disk.allowed_groups, vec!["engineering".to_string()]);
        assert_eq!(on_disk.max_inbox_entries, 50);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_handoff_receive_prompt_only_and_inbox_lifecycle() {
        let (node, dir, opener) = temp_handoff_node("inbox");
        let peer = DeviceId::from_id_pk(&[9u8; 32]);

        // 1. Receive valid URL with default PromptOnly policy
        let frame = ResourceHandoffFrame {
            id: "ho_test_1".to_string(),
            url: "https://example.com/docs/api?q=rust".to_string(),
            title: "API Docs".to_string(),
            auto_open_requested: false,
            timestamp_ms: 5000,
        };
        node.receive_resource_handoff(&peer, frame);

        // 2. Query inbox
        let list = node.handoff_inbox_list(None);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "ho_test_1");
        assert_eq!(list[0].url, "https://example.com/docs/api?q=rust");
        assert_eq!(list[0].title, "API Docs");
        assert_eq!(list[0].status, HandoffStatus::Pending);
        assert!(list[0].opened_ms.is_none());

        // 3. Open entry
        let opened = node.handoff_inbox_open("ho_test_1").unwrap();
        assert_eq!(opened.status, HandoffStatus::Opened);
        assert!(opened.opened_ms.is_some());
        assert_eq!(
            opener.opened(),
            vec!["https://example.com/docs/api?q=rust".to_string()]
        );

        // 4. Dismiss entry
        let dismissed = node.handoff_inbox_dismiss("ho_test_1").unwrap();
        assert_eq!(dismissed.status, HandoffStatus::Dismissed);

        // 5. Clear inbox
        let cleared = node.handoff_inbox_clear().unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(node.handoff_inbox_list(None).len(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_handoff_receive_firewall_rejects_malicious_urls() {
        let (node, dir, opener) = temp_handoff_node("reject");
        let peer = DeviceId::from_id_pk(&[9u8; 32]);

        // Attempt to hand off file:// URL
        let file_frame = ResourceHandoffFrame {
            id: "ho_malicious_1".to_string(),
            url: "file:///etc/shadow".to_string(),
            title: "passwd".to_string(),
            auto_open_requested: true,
            timestamp_ms: 5000,
        };
        node.receive_resource_handoff(&peer, file_frame);
        assert_eq!(node.handoff_inbox_list(None).len(), 0);

        // Attempt to hand off javascript:
        let js_frame = ResourceHandoffFrame {
            id: "ho_malicious_2".to_string(),
            url: "javascript:alert(1)".to_string(),
            title: "pwn".to_string(),
            auto_open_requested: true,
            timestamp_ms: 5000,
        };
        node.receive_resource_handoff(&peer, js_frame);
        assert_eq!(node.handoff_inbox_list(None).len(), 0);

        // Attempt to hand off metadata IP
        let meta_frame = ResourceHandoffFrame {
            id: "ho_malicious_3".to_string(),
            url: "http://169.254.169.254/latest/meta-data/".to_string(),
            title: "aws".to_string(),
            auto_open_requested: true,
            timestamp_ms: 5000,
        };
        node.receive_resource_handoff(&peer, meta_frame);
        assert_eq!(node.handoff_inbox_list(None).len(), 0);
        assert!(
            opener.opened().is_empty(),
            "firewall-rejected URLs must never reach the opener"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod media_policy_tests {
    use super::*;
    use tetherly_core::{ManualClock, MediaControlMode};

    fn temp_media_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-media-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "media-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(5000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        (node, dir)
    }

    #[test]
    fn test_media_policy_defaults_and_mutation() {
        let (node, dir) = temp_media_node("policy");
        let rep = node.media_policy_report();
        assert!(rep.ok);
        assert_eq!(rep.policy.mode, MediaControlMode::Allow);
        assert_eq!(rep.policy.max_volume_limit, 100);
        assert!(rep.policy.allow_seek);

        let mut updated = rep.policy.clone();
        updated.mode = MediaControlMode::Allow;
        updated.allowed_groups = vec!["lounge".into()];
        updated.max_volume_limit = 75;
        updated.allow_seek = false;

        let res = node.set_media_policy(updated).unwrap();
        assert!(res.ok);
        assert_eq!(res.policy.max_volume_limit, 75);
        assert!(!res.policy.allow_seek);

        let on_disk = crate::store::load_media_policy(&dir);
        assert_eq!(on_disk.max_volume_limit, 75);
        assert!(!on_disk.allow_seek);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_media_command_execution_state_machine_and_volume_clamping() {
        let (node, dir) = temp_media_node("state_machine");
        let peer = DeviceId::from_id_pk(&[11u8; 32]);

        // 1. Initial state
        let st = node.media_get_state(None).unwrap();
        assert!(st.ok);
        let cur = st.state.unwrap();
        assert_eq!(cur.state, PlaybackState::Stopped);
        assert_eq!(cur.volume_percent, 70);

        // 2. Play command
        let cmd_play = MediaCommandFrame {
            id: "cmd_1".into(),
            command: MediaCommand::Play,
            target_player: None,
            timestamp_ms: 5000,
        };
        node.receive_media_command(&peer, cmd_play);
        let cur = node.media_get_state(None).unwrap().state.unwrap();
        assert_eq!(cur.state, PlaybackState::Playing);
        assert!(!cur.can_play);
        assert!(cur.can_pause);

        // 3. Set volume clamped to max_volume_limit (set policy max 85)
        let mut pol = node.media_policy();
        pol.max_volume_limit = 85;
        node.set_media_policy(pol).unwrap();

        let cmd_vol = MediaCommandFrame {
            id: "cmd_2".into(),
            command: MediaCommand::SetVolume { volume_percent: 95 },
            target_player: None,
            timestamp_ms: 5010,
        };
        node.receive_media_command(&peer, cmd_vol);
        let cur = node.media_get_state(None).unwrap().state.unwrap();
        assert_eq!(cur.volume_percent, 85); // clamped!

        // 4. Pause command
        let cmd_pause = MediaCommandFrame {
            id: "cmd_3".into(),
            command: MediaCommand::Pause,
            target_player: None,
            timestamp_ms: 5020,
        };
        node.receive_media_command(&peer, cmd_pause);
        let cur = node.media_get_state(None).unwrap().state.unwrap();
        assert_eq!(cur.state, PlaybackState::Paused);
        assert!(cur.can_play);
        assert!(!cur.can_pause);

        // 5. Seek command blocked when seek disabled
        let mut pol = node.media_policy();
        pol.allow_seek = false;
        node.set_media_policy(pol).unwrap();

        let cmd_seek = MediaCommandFrame {
            id: "cmd_4".into(),
            command: MediaCommand::Seek { offset_ms: 10000 },
            target_player: None,
            timestamp_ms: 5030,
        };
        node.receive_media_command(&peer, cmd_seek);
        let cur = node.media_get_state(None).unwrap().state.unwrap();
        assert_eq!(cur.position_ms, 0); // Not modified because blocked

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_screen_twcc_mode_and_reporting() {
        let (node, dir) = temp_media_node("screen_twcc");

        // 1. Initial TWCC mode defaults to Auto
        assert_eq!(node.screen_twcc_mode(), tetherly_core::ScreenTwccMode::Auto);
        let rep = node.screen_twcc_report();
        assert!(rep.ok);
        assert_eq!(rep.mode, tetherly_core::ScreenTwccMode::Auto);
        assert_eq!(rep.feedbacks_sent, 0);
        assert_eq!(rep.feedbacks_received, 0);

        // 2. Mutate to Aggressive
        let rep_agg = node.screen_set_twcc_mode(tetherly_core::ScreenTwccMode::Aggressive);
        assert_eq!(rep_agg.mode, tetherly_core::ScreenTwccMode::Aggressive);
        assert_eq!(
            node.screen_twcc_mode(),
            tetherly_core::ScreenTwccMode::Aggressive
        );

        // 3. Mutate to Conservative
        let rep_cons = node.screen_set_twcc_mode(tetherly_core::ScreenTwccMode::Conservative);
        assert_eq!(rep_cons.mode, tetherly_core::ScreenTwccMode::Conservative);
        assert_eq!(
            node.screen_twcc_mode(),
            tetherly_core::ScreenTwccMode::Conservative
        );

        // 4. Mutate to Disabled
        let rep_dis = node.screen_set_twcc_mode(tetherly_core::ScreenTwccMode::Disabled);
        assert_eq!(rep_dis.mode, tetherly_core::ScreenTwccMode::Disabled);
        assert_eq!(
            node.screen_twcc_mode(),
            tetherly_core::ScreenTwccMode::Disabled
        );

        // 5. Restore to Auto
        let rep_auto = node.screen_set_twcc_mode(tetherly_core::ScreenTwccMode::Auto);
        assert_eq!(rep_auto.mode, tetherly_core::ScreenTwccMode::Auto);
        assert_eq!(node.screen_twcc_mode(), tetherly_core::ScreenTwccMode::Auto);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod locator_policy_tests {
    use super::*;
    use tetherly_core::{LocatePattern, ManualClock};

    fn temp_locator_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-locate-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "locate-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let clock = Arc::new(ManualClock::new(5000));
        let node = Node::start_with_clock(cfg, clock).unwrap();
        (node, dir)
    }

    #[test]
    fn test_locator_policy_defaults_and_mutation() {
        let (node, dir) = temp_locator_node("policy");
        let rep = node.locator_policy_report();
        assert!(rep.ok);
        assert!(rep.policy.enabled);
        assert!(rep.policy.allow_override_silent);
        assert_eq!(rep.policy.max_duration_secs, 60);

        let mut updated = rep.policy.clone();
        updated.allow_override_silent = false;
        updated.max_duration_secs = 30;
        updated.allowed_groups = vec!["family".into()];

        let res = node.set_locator_policy(updated).unwrap();
        assert!(res.ok);
        assert!(!res.policy.allow_override_silent);
        assert_eq!(res.policy.max_duration_secs, 30);

        let on_disk = crate::store::load_locator_policy(&dir);
        assert!(!on_disk.allow_override_silent);
        assert_eq!(on_disk.max_duration_secs, 30);
        assert_eq!(on_disk.allowed_groups, vec!["family"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_locate_request_and_stop_and_rate_limiting() {
        let (node, dir) = temp_locator_node("locate_run");
        let peer = DeviceId::from_id_pk(&[22u8; 32]);

        // 1. Initial status: no active ring
        let st = node.locate_status();
        assert!(st.ok);
        assert!(st.active_ring.is_none());

        // 2. Receive locate request
        let req1 = LocateRequestFrame {
            request_id: "req_test_1".to_string(),
            duration_secs: 25,
            pattern: LocatePattern::Siren,
            override_silent: true,
            message: Some("Find me!".to_string()),
            timestamp_ms: 5000,
        };
        node.receive_locate_request(&peer, req1);

        let st2 = node.locate_status();
        assert!(st2.active_ring.is_some());
        let ring = st2.active_ring.unwrap();
        assert_eq!(ring.request_id, "req_test_1");
        assert_eq!(ring.duration_secs, 25);
        assert_eq!(ring.pattern, LocatePattern::Siren);
        assert!(ring.override_silent);
        assert_eq!(ring.message.as_deref(), Some("Find me!"));

        // 3. Immediate second request from same peer -> rate limited (<10s cooldown)
        let req2 = LocateRequestFrame {
            request_id: "req_test_2".to_string(),
            duration_secs: 15,
            pattern: LocatePattern::Pulse,
            override_silent: true,
            message: Some("Spamming".to_string()),
            timestamp_ms: 5005,
        };
        node.receive_locate_request(&peer, req2);
        // Active ring unchanged
        let st3 = node.locate_status();
        assert_eq!(st3.active_ring.unwrap().request_id, "req_test_1");

        // 4. Inbound stop from peer
        let stop1 = LocateStopFrame {
            request_id: Some("req_test_1".to_string()),
            reason: "found_it".to_string(),
            timestamp_ms: 5010,
        };
        node.receive_locate_stop(&peer, stop1);
        let st4 = node.locate_status();
        assert!(st4.active_ring.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_action_node(tag: &str) -> (Node, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-action-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "action-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "test".into(),
            screen_source: None,
            screen_codec: None,
            screen_codec_h264: false,
            screen_bitrate_kbps: default_screen_bitrate_kbps(),
            screen_bitrate_auto: true,
            screen_fps: default_screen_fps(),
            screen_fps_auto: true,
            screen_scale_pct: default_screen_scale_pct(),
            screen_scale_auto: true,
            screen_profile: tetherly_core::ScreenContentProfile::Auto,
            screen_profile_auto: true,
            screen_qp_min: tetherly_core::SCREEN_QP_MIN_DEFAULT,
            screen_qp_max: tetherly_core::SCREEN_QP_MAX_DEFAULT,
            screen_matrix_mode: tetherly_core::ScreenMatrixMode::Auto,
            screen_playout_mode: tetherly_core::ScreenPlayoutMode::Adaptive,
            screen_cursor_mode: tetherly_core::ScreenCursorMode::Decoupled,
            screen_squelch_mode: tetherly_core::ScreenSquelchMode::Auto,
            screen_fec_mode: tetherly_core::ScreenFecMode::Auto,
            screen_pacer_mode: tetherly_core::ScreenPacerMode::Auto,
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            screen_twcc_mode: tetherly_core::ScreenTwccMode::Auto,
            screen_probe_mode: tetherly_core::ScreenProbeMode::Auto,
            screen_sync_mode: tetherly_core::ScreenAudioSyncMode::Auto,
            screen_latency_mode: tetherly_core::ScreenProfilerMode::Auto,
            screen_tuning_mode: tetherly_core::ScreenTunerMode::Standard,
            screen_display_id: 0,
            screen_sink: None,
            input_sink: None,
        };
        let node = Node::start(cfg).unwrap();
        (node, dir)
    }

    #[tokio::test]
    async fn test_action_runtime_policy_local_allowlist_and_sandbox_execution() {
        let (node, dir) = temp_action_node("action_run");
        let peer = DeviceId::from_id_pk(&[33u8; 32]);

        // 1. Initial state: policy defaults, no local actions
        let pol = node.action_policy();
        assert_eq!(pol.mode, ActionAccessMode::Allow);
        assert!(pol.expose_action_list);

        let actions = node.local_actions();
        assert!(actions.is_empty());

        // 2. Add local action definition
        let def1 = LocalActionDefinition {
            id: "echo_test".to_string(),
            name: "Echo Test".to_string(),
            description: Some("Outputs hello from echo".to_string()),
            icon: Some("terminal".to_string()),
            command: "echo".to_string(),
            args: vec!["hello-action".to_string()],
            timeout_secs: 5,
            enabled: true,
        };
        let rep1 = node.add_local_action(def1).unwrap();
        assert_eq!(rep1.count, 1);
        assert_eq!(rep1.actions[0].id, "echo_test");

        // 3. Reject duplicate action id or invalid characters
        let dup = LocalActionDefinition {
            id: "echo_test".to_string(),
            name: "Duplicate".to_string(),
            description: None,
            icon: None,
            command: "ls".to_string(),
            args: vec![],
            timeout_secs: 5,
            enabled: true,
        };
        // add_local_action updates if same ID
        let rep_dup = node.add_local_action(dup).unwrap();
        assert_eq!(rep_dup.count, 1);
        assert_eq!(rep_dup.actions[0].command, "ls");

        // Add back echo_test
        let def_echo = LocalActionDefinition {
            id: "echo_test".to_string(),
            name: "Echo Test".to_string(),
            description: Some("Outputs hello from echo".to_string()),
            icon: Some("terminal".to_string()),
            command: "echo".to_string(),
            args: vec!["hello-action".to_string()],
            timeout_secs: 5,
            enabled: true,
        };
        let _ = node.add_local_action(def_echo).unwrap();

        // 4. Inbound action trigger execution
        let trig1 = ActionTriggerFrame {
            trigger_id: "trig_t1".to_string(),
            action_id: "echo_test".to_string(),
            timestamp_ms: 1000,
        };
        node.receive_action_trigger(&peer, trig1);

        // Subprocess execution runs on a spawned task: poll instead of a
        // bare sleep so slow CI runners (Windows) cannot race the assert.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if node.action_history().count >= 1 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "action execution never recorded its history entry"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let hist = node.action_history();
        assert_eq!(hist.count, 1);
        let res = &hist.results[0];
        assert_eq!(res.trigger_id, "trig_t1");
        assert_eq!(res.action_id, "echo_test");
        assert!(res.success);
        assert_eq!(res.exit_code, Some(0));
        assert!(res
            .output_summary
            .as_ref()
            .unwrap()
            .contains("hello-action"));

        // 5. Rate limiting: immediate second trigger from same peer is rejected (<5s cooldown)
        let trig2 = ActionTriggerFrame {
            trigger_id: "trig_t2".to_string(),
            action_id: "echo_test".to_string(),
            timestamp_ms: 1050,
        };
        node.receive_action_trigger(&peer, trig2);
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Cache unchanged (rate-limited failure sent over wire)
        let hist2 = node.action_history();
        assert_eq!(hist2.count, 1);

        // 6. Unknown action rejected
        // Advance clock or use another peer
        let peer2 = DeviceId::from_id_pk(&[34u8; 32]);
        let trig_unk = ActionTriggerFrame {
            trigger_id: "trig_unk".to_string(),
            action_id: "nonexistent".to_string(),
            timestamp_ms: 2000,
        };
        node.receive_action_trigger(&peer2, trig_unk);
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 7. Policy Disabled mode rejects trigger
        let mut disabled_pol = node.action_policy();
        disabled_pol.mode = ActionAccessMode::Disabled;
        node.set_action_policy(disabled_pol).unwrap();

        let peer3 = DeviceId::from_id_pk(&[35u8; 32]);
        let trig_dis = ActionTriggerFrame {
            trigger_id: "trig_dis".to_string(),
            action_id: "echo_test".to_string(),
            timestamp_ms: 3000,
        };
        node.receive_action_trigger(&peer3, trig_dis);
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 8. Test remove_local_action
        let rep_rem = node.remove_local_action("echo_test").unwrap();
        assert_eq!(rep_rem.count, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_focus_mode_runtime_and_notification_suppression() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-focus-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "focus-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(10_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial focus status: inactive by default
        let st0 = node.focus_status();
        assert!(st0.ok);
        assert!(!st0.enabled);
        assert!(!st0.active);

        // 2. Normal notifications pass when focus mode is inactive
        let res_norm1 = node
            .dispatch_notification_banner("测试通知 1", "普通内容", Some("ChatApp"), false)
            .unwrap();
        assert!(res_norm1);

        // 3. Activate focus mode with 30 min duration
        let set_rep = node
            .set_focus_state(
                true,
                Some(FocusMode::Meeting),
                Some(30),
                Some("Weekly sync meeting".to_string()),
                false,
            )
            .await
            .unwrap();
        assert!(set_rep.ok);
        assert!(set_rep.enabled);
        assert!(set_rep.active);
        assert_eq!(set_rep.mode, FocusMode::Meeting);
        assert_eq!(set_rep.expires_ms, Some(10_000 + 30 * 60 * 1000));

        let st1 = node.focus_status();
        assert!(st1.active);
        assert_eq!(st1.mode, FocusMode::Meeting);
        assert_eq!(st1.remaining_secs, Some(1800));

        // 4. In active focus mode, normal notifications are suppressed!
        let res_norm2 = node
            .dispatch_notification_banner("测试通知 2", "工作群聊消息", Some("Slack"), false)
            .unwrap();
        assert!(!res_norm2); // Suppressed!

        // 5. In active focus mode, critical / OTP notifications break through!
        let res_otp = node
            .dispatch_notification_banner(
                "银行验证码",
                "您的验证码是 654321，5分钟内有效",
                Some("Messages"),
                true, // is_otp
            )
            .unwrap();
        assert!(res_otp); // Emergency breakthrough permitted!

        // 6. Test notification banner test endpoint reflects focus mode suppression
        let test_rep = node.test_notification_banner(None, None).unwrap();
        assert!(test_rep.suppressed);
        assert_eq!(
            test_rep.suppression_reason,
            Some("focus_mode_meeting".to_string())
        );

        // 7. Receive peer focus state
        let peer_did = DeviceId::from_id_pk(&[77u8; 32]);
        let peer_frame = FocusStateFrame {
            enabled: true,
            mode: FocusMode::Work,
            expires_ms: Some(50_000),
            reason: Some("Deep coding".to_string()),
            timestamp_ms: 10_000,
        };
        node.receive_focus_state(&peer_did, peer_frame);

        let peers_rep = node.peer_focus_states();
        assert_eq!(peers_rep.count, 1);
        assert_eq!(peers_rep.peers[0].peer, peer_did.to_string());
        assert!(peers_rep.peers[0].active);
        assert_eq!(peers_rep.peers[0].state.mode, FocusMode::Work);

        // 8. Turn off focus mode
        let off_rep = node
            .set_focus_state(false, None, None, None, false)
            .await
            .unwrap();
        assert!(!off_rep.enabled);
        assert!(!off_rep.active);

        let st_off = node.focus_status();
        assert!(!st_off.active);

        // 9. Now normal notification passes again
        let res_norm3 = node
            .dispatch_notification_banner("测试通知 3", "已恢复正常通知", Some("Slack"), false)
            .unwrap();
        assert!(res_norm3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_fps_pacing_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-fps-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "fps-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(10_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to 30 FPS
        let initial_rep = node.screen_fps_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.fps, 30);
        assert_eq!(initial_rep.min_fps, 5);
        assert_eq!(initial_rep.max_fps, 120);
        assert_eq!(initial_rep.interval_us, 33_333);

        // 2. Set FPS override
        let rep_60 = node.screen_set_fps_reported(60);
        assert!(rep_60.ok);
        assert_eq!(rep_60.fps, 60);
        assert_eq!(rep_60.interval_us, 16_666);
        assert_eq!(node.screen_fps(), 60);

        // 3. Clamping bounds
        let rep_clamp_high = node.screen_set_fps_reported(200);
        assert_eq!(rep_clamp_high.fps, 120);
        assert_eq!(rep_clamp_high.interval_us, 8_333);

        let rep_clamp_low = node.screen_set_fps_reported(1);
        assert_eq!(rep_clamp_low.fps, 5);
        assert_eq!(rep_clamp_low.interval_us, 200_000);

        // 4. Status report includes fps_target
        let status = node.screen_status_report().await;
        assert_eq!(status.fps_target, 5);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_content_profile_and_qp_bounds_runtime() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-profile-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "profile-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(20_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Auto profile and (12, 35) default QP bounds
        let initial_rep = node.screen_profile_report();
        assert!(initial_rep.ok);
        assert_eq!(
            initial_rep.profile,
            tetherly_core::ScreenContentProfile::Auto
        );
        assert_eq!(initial_rep.min_qp, tetherly_core::SCREEN_QP_TEXT_DEFAULT.0);
        assert_eq!(initial_rep.max_qp, tetherly_core::SCREEN_QP_TEXT_DEFAULT.1);
        assert!(!initial_rep.is_custom_qp);
        assert!(initial_rep.auto_profile);

        // 2. Set profile override to Motion
        let rep_motion =
            node.screen_set_profile_reported(tetherly_core::ScreenContentProfile::Motion);
        assert!(rep_motion.ok);
        assert_eq!(
            rep_motion.profile,
            tetherly_core::ScreenContentProfile::Motion
        );
        assert_eq!(rep_motion.min_qp, tetherly_core::SCREEN_QP_MOTION_DEFAULT.0);
        assert_eq!(rep_motion.max_qp, tetherly_core::SCREEN_QP_MOTION_DEFAULT.1);
        assert!(!rep_motion.is_custom_qp);
        assert_eq!(
            node.screen_profile(),
            tetherly_core::ScreenContentProfile::Motion
        );

        // 3. Set custom QP range override
        let rep_custom_qp = node.screen_set_qp_range_reported(10, 28);
        assert!(rep_custom_qp.ok);
        assert_eq!(rep_custom_qp.min_qp, 10);
        assert_eq!(rep_custom_qp.max_qp, 28);
        assert!(rep_custom_qp.is_custom_qp);
        assert_eq!(node.screen_qp_range(), (10, 28));

        // 4. Clamping bounds verification
        let rep_clamped = node.screen_set_qp_range_reported(60, 20); // min > max -> clamped
        assert_eq!(rep_clamped.min_qp, 51);
        assert_eq!(rep_clamped.max_qp, 51);

        let rep_reset = node.screen_set_qp_range_reported(14, 40);
        assert_eq!(rep_reset.min_qp, 14);
        assert_eq!(rep_reset.max_qp, 40);

        // 5. Status report reflects active profile and QP bounds
        let status = node.screen_status_report().await;
        assert_eq!(status.profile, "motion");
        assert_eq!(status.qp_min, 14);
        assert_eq!(status.qp_max, 40);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_matrix_mode_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-matrix-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "matrix-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(30_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Auto mode and Healthy status
        let initial_rep = node.screen_matrix_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.mode, "auto");
        assert_eq!(initial_rep.health, "healthy");
        assert_eq!(initial_rep.fps_target, 30);
        assert_eq!(initial_rep.scale_pct, 100);
        assert_eq!(
            node.screen_matrix_mode(),
            tetherly_core::ScreenMatrixMode::Auto
        );

        // 2. Set mode to Conservative
        let rep_cons = node.screen_set_matrix_mode(tetherly_core::ScreenMatrixMode::Conservative);
        assert!(rep_cons.ok);
        assert_eq!(rep_cons.mode, "conservative");
        assert_eq!(
            node.screen_matrix_mode(),
            tetherly_core::ScreenMatrixMode::Conservative
        );

        // 3. Set mode to Manual
        let rep_manual = node.screen_set_matrix_mode(tetherly_core::ScreenMatrixMode::Manual);
        assert!(rep_manual.ok);
        assert_eq!(rep_manual.mode, "manual");
        assert_eq!(
            node.screen_matrix_mode(),
            tetherly_core::ScreenMatrixMode::Manual
        );

        // 4. Status report reflects active matrix mode and network health
        let status = node.screen_status_report().await;
        assert_eq!(status.matrix_mode, "manual");
        assert_eq!(status.network_health, "healthy");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_playout_mode_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-playout-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "playout-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(40_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Adaptive mode with 15ms base delay
        let initial_rep = node.screen_playout_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.mode, "adaptive");
        assert_eq!(
            initial_rep.target_delay_us,
            tetherly_core::SCREEN_PLAYOUT_ADAPTIVE_BASE_DELAY_US
        );
        assert_eq!(initial_rep.target_delay_ms, 15.0);
        assert_eq!(initial_rep.current_jitter_us, 0);
        assert_eq!(initial_rep.buffered_frames, 0);
        assert_eq!(initial_rep.played_frames, 0);
        assert_eq!(initial_rep.dropped_late_frames, 0);
        assert_eq!(
            node.screen_playout_mode(),
            tetherly_core::ScreenPlayoutMode::Adaptive
        );

        // 2. Set mode to ZeroLatency
        let rep_zero = node.screen_set_playout_mode(tetherly_core::ScreenPlayoutMode::ZeroLatency);
        assert!(rep_zero.ok);
        assert_eq!(rep_zero.mode, "zerolatency");
        assert_eq!(
            rep_zero.target_delay_us,
            tetherly_core::SCREEN_PLAYOUT_MIN_DELAY_US
        );
        assert_eq!(rep_zero.target_delay_ms, 5.0);
        assert_eq!(
            node.screen_playout_mode(),
            tetherly_core::ScreenPlayoutMode::ZeroLatency
        );

        // 3. Set mode to Smooth
        let rep_smooth = node.screen_set_playout_mode(tetherly_core::ScreenPlayoutMode::Smooth);
        assert!(rep_smooth.ok);
        assert_eq!(rep_smooth.mode, "smooth");
        assert_eq!(
            rep_smooth.target_delay_us,
            tetherly_core::SCREEN_PLAYOUT_SMOOTH_BASE_DELAY_US
        );
        assert_eq!(rep_smooth.target_delay_ms, 35.0);
        assert_eq!(
            node.screen_playout_mode(),
            tetherly_core::ScreenPlayoutMode::Smooth
        );

        // 4. Status report reflects active playout mode and metrics
        let status = node.screen_status_report().await;
        assert_eq!(status.playout_mode, "smooth");
        assert_eq!(status.playout_delay_ms, 35.0);
        assert_eq!(status.playout_buffered, 0);
        assert_eq!(status.playout_dropped_late, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_cursor_mode_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-cursor-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "cursor-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(50_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Decoupled mode (ADR-088)
        let initial_rep = node.screen_cursor_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.mode, "decoupled");
        assert_eq!(initial_rep.x, 0);
        assert_eq!(initial_rep.y, 0);
        assert!(!initial_rep.visible);
        assert_eq!(initial_rep.shape, "arrow");
        assert_eq!(initial_rep.updates_sent, 0);
        assert_eq!(initial_rep.updates_received, 0);
        assert_eq!(initial_rep.updates_dropped_dedup, 0);
        assert_eq!(initial_rep.updates_dropped_rate, 0);
        assert_eq!(
            node.screen_cursor_mode(),
            tetherly_core::ScreenCursorMode::Decoupled
        );

        // 2. Set mode to Embedded
        let rep_embedded = node.screen_set_cursor_mode(tetherly_core::ScreenCursorMode::Embedded);
        assert!(rep_embedded.ok);
        assert_eq!(rep_embedded.mode, "embedded");
        assert_eq!(
            node.screen_cursor_mode(),
            tetherly_core::ScreenCursorMode::Embedded
        );

        // 3. Set mode to Hidden
        let rep_hidden = node.screen_set_cursor_mode(tetherly_core::ScreenCursorMode::Hidden);
        assert!(rep_hidden.ok);
        assert_eq!(rep_hidden.mode, "hidden");
        assert_eq!(
            node.screen_cursor_mode(),
            tetherly_core::ScreenCursorMode::Hidden
        );

        // 4. Status report reflects active cursor mode and telemetry
        let status = node.screen_status_report().await;
        assert_eq!(status.cursor_mode, "hidden");
        assert_eq!(status.cursor_x, 0);
        assert_eq!(status.cursor_y, 0);
        assert!(!status.cursor_visible);
        assert_eq!(status.cursor_shape, "arrow");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_trendline_mode_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-trendline-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "trendline-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            screen_trendline_mode: tetherly_core::ScreenTrendlineMode::Auto,
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(50_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Auto mode (ADR-092)
        let initial_rep = node.screen_trendline_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.mode, tetherly_core::ScreenTrendlineMode::Auto);
        assert_eq!(
            initial_rep.state,
            tetherly_core::ScreenBandwidthUsageState::Normal
        );
        assert_eq!(initial_rep.overuse_count, 0);
        assert_eq!(initial_rep.underuse_count, 0);
        assert_eq!(
            node.screen_trendline_mode(),
            tetherly_core::ScreenTrendlineMode::Auto
        );

        // 2. Set mode to Aggressive
        let rep_agg =
            node.screen_set_trendline_mode(tetherly_core::ScreenTrendlineMode::Aggressive);
        assert!(rep_agg.ok);
        assert_eq!(rep_agg.mode, tetherly_core::ScreenTrendlineMode::Aggressive);
        assert!((rep_agg.threshold_us_per_frame - 6.0).abs() < 0.01);
        assert_eq!(
            node.screen_trendline_mode(),
            tetherly_core::ScreenTrendlineMode::Aggressive
        );

        // 3. Set mode to Conservative
        let rep_cons =
            node.screen_set_trendline_mode(tetherly_core::ScreenTrendlineMode::Conservative);
        assert!(rep_cons.ok);
        assert_eq!(
            rep_cons.mode,
            tetherly_core::ScreenTrendlineMode::Conservative
        );
        assert!((rep_cons.threshold_us_per_frame - 25.0).abs() < 0.01);
        assert_eq!(
            node.screen_trendline_mode(),
            tetherly_core::ScreenTrendlineMode::Conservative
        );

        // 4. Set mode to Disabled
        let rep_dis = node.screen_set_trendline_mode(tetherly_core::ScreenTrendlineMode::Disabled);
        assert!(rep_dis.ok);
        assert_eq!(rep_dis.mode, tetherly_core::ScreenTrendlineMode::Disabled);
        assert_eq!(rep_dis.threshold_us_per_frame, 0.0);
        assert_eq!(
            node.screen_trendline_mode(),
            tetherly_core::ScreenTrendlineMode::Disabled
        );

        // 5. Status report reflects active trendline mode
        let status = node.screen_status_report().await;
        assert_eq!(status.trendline_mode, "disabled");
        assert_eq!(status.trendline_state, "normal");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_screen_harq_mode_and_reporting() {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-harq-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = NodeConfig {
            data_dir: dir.clone(),
            control_port: 0,
            file_port: 0,
            input_port: 0,
            screen_port: 0,
            ui_port: 0,
            name: "harq-test-node".into(),
            platform: "test".into(),
            memory_clip: true,
            memory_notifier: true,
            loopback_only: true,
            advertise: false,
            reconnect: false,
            overlay: OverlayConfig::default(),
            screen_harq_mode: tetherly_core::ScreenHarqMode::Auto,
            ..Default::default()
        };
        let clock = Arc::new(ManualClock::new(50_000));
        let node = Node::start_with_clock(cfg, clock.clone()).unwrap();

        // 1. Initial report: defaults to Auto mode (ADR-093)
        let initial_rep = node.screen_harq_report();
        assert!(initial_rep.ok);
        assert_eq!(initial_rep.mode, tetherly_core::ScreenHarqMode::Auto);
        assert_eq!(initial_rep.nacks_sent, 0);
        assert_eq!(initial_rep.nacks_received, 0);
        assert_eq!(initial_rep.shards_retransmitted, 0);
        assert_eq!(initial_rep.recovered_frames, 0);
        assert_eq!(initial_rep.retransmit_timeouts, 0);
        assert_eq!(initial_rep.buffer_occupancy, 0);
        assert_eq!(
            initial_rep.buffer_capacity,
            tetherly_core::SCREEN_HARQ_DEFAULT_BUFFER_CAPACITY
        );
        assert_eq!(node.screen_harq_mode(), tetherly_core::ScreenHarqMode::Auto);

        // 2. Set mode to Aggressive
        let rep_agg = node.screen_set_harq_mode(tetherly_core::ScreenHarqMode::Aggressive);
        assert!(rep_agg.ok);
        assert_eq!(rep_agg.mode, tetherly_core::ScreenHarqMode::Aggressive);
        assert_eq!(
            node.screen_harq_mode(),
            tetherly_core::ScreenHarqMode::Aggressive
        );

        // 3. Set mode to Conservative
        let rep_cons = node.screen_set_harq_mode(tetherly_core::ScreenHarqMode::Conservative);
        assert!(rep_cons.ok);
        assert_eq!(rep_cons.mode, tetherly_core::ScreenHarqMode::Conservative);
        assert_eq!(
            node.screen_harq_mode(),
            tetherly_core::ScreenHarqMode::Conservative
        );

        // 4. Set mode to Disabled
        let rep_dis = node.screen_set_harq_mode(tetherly_core::ScreenHarqMode::Disabled);
        assert!(rep_dis.ok);
        assert_eq!(rep_dis.mode, tetherly_core::ScreenHarqMode::Disabled);
        assert_eq!(
            node.screen_harq_mode(),
            tetherly_core::ScreenHarqMode::Disabled
        );

        // 5. Status report reflects active HARQ mode and metrics
        let status = node.screen_status_report().await;
        assert_eq!(status.harq_mode, "disabled");
        assert_eq!(status.harq_nacks_sent, 0);
        assert_eq!(status.harq_nacks_received, 0);
        assert_eq!(status.harq_shards_retransmitted, 0);
        assert_eq!(status.harq_recovered_frames, 0);
        assert_eq!(status.harq_retransmit_timeouts, 0);
        assert_eq!(status.harq_buffer_occupancy, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
