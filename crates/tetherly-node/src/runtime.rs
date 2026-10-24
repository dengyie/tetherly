// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN + optional EasyTier sidecar overlay. Overlay is unicast only; no mDNS on TUN.

use crate::ancs::AncsInbound;
use crate::events::{file_offered, peer_up, CandidateViewDto, UiEvent};
use crate::lan::{bind_many, bind_many_optional, bind_overlay, control_bind_addrs, lan_ips};
use crate::platform::{insert_foreground, MemoryClip, SystemClip};
use crate::sidecar::{self, OverlayConfig, OverlaySource, OverlayStatus};
use crate::store::{
    load_action_policy, load_audio_routing_policy, load_bandwidth_profiles, load_battery_policy,
    load_bench_policy, load_camera_policy, load_clipboard_policy, load_focus_policy,
    load_handoff_inbox, load_handoff_policy, load_input_policy, load_local_actions,
    load_locator_policy, load_media_policy, load_microphone_policy, load_topology_layout,
    load_topology_policy, load_trust, load_virtual_display_policy, remember_route,
    save_action_policy, save_audio_routing_policy, save_bandwidth_profiles, save_battery_policy,
    save_bench_policy, save_camera_policy, save_clipboard_policy, save_focus_policy,
    save_handoff_inbox, save_handoff_policy, save_input_policy, save_local_actions,
    save_locator_policy, save_media_policy, save_microphone_policy, save_topology_layout,
    save_topology_policy, save_trust, save_virtual_display_policy, ActionAccessMode, ActionPolicy,
    AudioRoutingPolicy, AutoAcceptPolicy, BandwidthProfileTable, BatteryPolicy, BenchPolicy,
    CameraPolicy, ClipboardPolicy, DeviceType, FocusPolicy, HandoffInboxTable, HandoffPolicy,
    InputTunnelPolicy, LocalActionDefinition, LocatorPolicy, MediaControlPolicy, MicrophonePolicy,
    PeerBandwidthProfile, PeerProfile, ProfileTable, RouteTable, TargetSelector, TopologyLayout,
    TopologyPolicy, VirtualDisplayPolicy,
};
use crate::telemetry::{TrafficReport, TrafficTelemetry};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::frame::{
    ping, pong, CapsUpdate, ClipSet, FileDone, FileOffer, InnerFrame, NotifyPush, NotifyReply,
    ResourceHandoffFrame, TextMessage,
};
use tetherly_core::liveness::Liveness;
use tetherly_core::pin::{format_pin, generate_pin};
use tetherly_core::ports::{Clip, Clock, Rng, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::session::{backoff_delay, PIN_TTL};
use tetherly_core::{
    ancs_actions, ancs_source_id, map_edge_exit_to_entry, path_kind, recommend_streaming_params,
    ActionItem, ActionListReqFrame, ActionListRespFrame, ActionResultFrame, ActionTriggerFrame,
    ActionValidator, AncsEvent, AncsIngress, AncsState, AncsTransport, AudioChannelLayout,
    AudioRoutingAnnounceFrame, AudioRoutingAttachFrame, AudioRoutingChunkFrame,
    AudioRoutingDetachFrame, AudioRoutingRole, AudioRoutingValidator, AudioSampleFormat,
    AudioSpatialPosition, AudioStreamCodec, BatteryStateFrame, BatteryValidator, BenchAcceptFrame,
    BenchDataFrame, BenchDirection, BenchReportFrame, BenchRequestFrame, BenchRole, BenchValidator,
    CameraAnnounceFrame, CameraAttachFrame, CameraCodec, CameraDetachFrame, CameraFrameChunk,
    CameraResolution, CameraRole, CameraValidator, ClipApply, ClipDirection, ClipHub,
    ClipMetaEntry, ConnectOutcome, ControlMsg, CoreError, CursorSeat, DeviceId, DisplayEdge,
    DisplayRect, EdgeCrossingConfig, EdgeNeighbor, FileHub, FocusMode, FocusStateFrame,
    FocusSyncReqFrame, FocusValidator, HandoffEntry, HandoffStatus, IngestOutcome, InputClient,
    InputEvent, InputServer, InputSink, InputTunnelHeartbeatFrame, InputTunnelReleaseFrame,
    InputTunnelRequestFrame, InputTunnelRespFrame, InputTunnelRole, InputTunnelValidator,
    LocatePattern, LocateRequestFrame, LocateStopFrame, LocateValidator, MediaCommand,
    MediaCommandFrame, MediaStateFrame, MediaValidator, MemoryScreenSink, MemorySink,
    MemoryTrustStore, MicAnnounceFrame, MicAttachFrame, MicAudioChunkFrame, MicCodec,
    MicDetachFrame, MicMode, MicRole, MicValidator, NotificationPayload, NotificationUrgency,
    Notifier, NotifyHub, OpenAllowlist, PathKind, PlaybackState, PowerSource, ReplyView,
    ResourceHandoffValidator, ScreenAssembler, ScreenCodec, ScreenFrame, ScreenReceiver,
    ScreenSender, ScreenSink, ScreenSource, ScreenState, ScreenStats, SensitiveContentGuard,
    TcpLimiter, TopologyStateFrame, TopologySyncReqFrame, TopologyValidator, TrustStore,
    VirtualDisplayAnnounceFrame, VirtualDisplayAttachFrame, VirtualDisplayCodec,
    VirtualDisplayDetachFrame, VirtualDisplayFrameAck, VirtualDisplayMode,
    VirtualDisplayPixelFormat, VirtualDisplayRole, VirtualDisplayValidator, CLIPBOARD_OTP_CLEAR_MS,
    DEFAULT_LOCATE_DURATION_SECS, OVERLAY_PROBE_MS,
};
use tetherly_crypto::Identity;
use tetherly_net::dispatch::SessionEvent;
use tetherly_net::filechan::read_header as read_file_header;
use tetherly_net::session::{
    accept_session, dial_session, ActiveSession, SessionConfig, CONTROL_PORT, FILE_PORT,
    HANDSHAKE_TIMEOUT, INPUT_PORT, SCREEN_PORT,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex as AsyncMutex};
use tracing::{debug, info, warn};

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

#[derive(Clone)]
pub struct LivePeer {
    pub device_id: DeviceId,
    pub name: String,
    pub platform: String,
    pub addr: SocketAddr,
    pub handshake_hash: [u8; 32],
    /// Advertised via caps.update `fileport=`. None until the peer tells us.
    pub file_port: Option<u16>,
    /// Advertised via caps.update `inputport=`. None until the peer tells us.
    pub input_port: Option<u16>,
    /// Advertised via caps.update `screenport=`. None until the peer tells us.
    pub screen_port: Option<u16>,
    /// Advertised capability tokens from the peer.
    pub caps: Vec<String>,
    pub path: PathKind,
    pub rtt_ms: Option<u64>,
    pub last_seen_ms: u64,
}

/// Report returned by local capabilities introspection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LocalCapsReport {
    pub ok: bool,
    pub device_id: String,
    pub caps: Vec<String>,
    pub features: Vec<String>,
    pub file_port: u16,
    pub input_port: u16,
    pub screen_port: u16,
}

/// Capability profile for a connected peer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerCapsDto {
    pub device_id: String,
    pub name: String,
    pub alias: Option<String>,
    pub caps: Vec<String>,
    pub features: Vec<String>,
    pub file_port: Option<u16>,
    pub input_port: Option<u16>,
    pub screen_port: Option<u16>,
    pub online: bool,
}

/// Discovered peer representation on local network via mDNS beacons.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveredPeerDto {
    pub device_id: String,
    pub name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub fp: Option<String>,
    pub caps: Vec<String>,
    pub is_trusted: bool,
    pub is_online: bool,
    pub last_seen_ms: u64,
}

/// Summary report of discovered LAN peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<DiscoveredPeerDto>,
}

/// Report returned when initiating a fresh LAN discovery rescan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiscoveryRescanReport {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeerInfo {
    pub device_id: String,
    pub name: String,
    pub addrs: Vec<String>,
    pub port: u16,
    pub fp: Option<String>,
    pub caps: Vec<String>,
    pub last_seen_ms: u64,
}

/// Report summarizing capabilities of peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeersCapsReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<PeerCapsDto>,
}

/// Report returned by an on-demand peer ping probe over the Noise session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerPingReport {
    pub ok: bool,
    pub device_id: String,
    pub rtt_ms: u64,
    pub path: String,
    pub addr: String,
}

/// Status report for the node's currently armed pairing PIN and its lifecycle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PinReport {
    pub ok: bool,
    pub active: bool,
    pub pin: Option<String>,
    pub expires_in_secs: u64,
    pub ttl_secs: u64,
    pub created_at_ms: u64,
}

/// Report returned when actively connecting to a trusted peer on demand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerConnectReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub addr: Option<String>,
    pub connected: bool,
    pub already_connected: bool,
}

/// Report returned when actively disconnecting a live peer session on demand.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerDisconnectReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub disconnected: bool,
    pub already_disconnected: bool,
}

/// Report returned when blocking a peer device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerBlockReport {
    pub ok: bool,
    pub device_id: String,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub reason: Option<String>,
    pub blocked: bool,
    pub already_blocked: bool,
    pub disconnected_active: bool,
}

/// Report returned when unblocking a peer device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PeerUnblockReport {
    pub ok: bool,
    pub device_id: String,
    pub unblocked: bool,
    pub was_blocked: bool,
}

/// Summary report of all blocked peer devices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BlockedPeersReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<crate::store::BlockedPeer>,
}

/// Report returned when dismissing a single notification candidate.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CandidateDismissReport {
    pub ok: bool,
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub uid: String,
    pub notified_remote: bool,
}

/// Report returned when performing batch dismissal of notification candidates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchDismissReport {
    pub ok: bool,
    pub count: usize,
    pub ids: Vec<String>,
    pub notified_remote_count: usize,
}

/// Filter options for candidate dismissal operations.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DismissFilter {
    pub id: Option<String>,
    pub peer: Option<String>,
    pub app_id: Option<String>,
    pub all: Option<bool>,
}

/// Do-Not-Disturb modes for native desktop notification banners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DndMode {
    #[default]
    Off,
    Silent,
    All,
}

/// Node configuration for desktop notification banners.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NotifyBannerConfig {
    pub banners_enabled: bool,
    pub dnd_mode: DndMode,
    pub sound_enabled: bool,
    pub mask_otp: bool,
    pub allow_otp_in_dnd: bool,
}

impl Default for NotifyBannerConfig {
    fn default() -> Self {
        Self {
            banners_enabled: true,
            dnd_mode: DndMode::Off,
            sound_enabled: true,
            mask_otp: true,
            allow_otp_in_dnd: true,
        }
    }
}

/// Status report for desktop notification banners and telemetry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NotifyConfigReport {
    pub ok: bool,
    pub config: NotifyBannerConfig,
    pub total_dispatched: u64,
    pub total_suppressed: u64,
}

/// Result report for testing desktop notification banners.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NotifyTestReport {
    pub ok: bool,
    pub sent: bool,
    pub suppressed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
    pub message: String,
    pub title: String,
}

/// Optional payload parameters for triggering a test notification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct NotifyTestRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
}

/// Detailed process resource consumption metrics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProcessMetrics {
    pub pid: u32,
    pub uptime_secs: u64,
    pub rss_bytes: u64,
    pub virt_bytes: u64,
    pub thread_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_fds: Option<u32>,
    pub cpu_time_ms: u64,
    pub memory_rss_formatted: String,
    pub memory_virt_formatted: String,
}

/// Host machine hardware and operating system profile.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HostMetrics {
    pub os: String,
    pub arch: String,
    pub num_cpus: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_formatted: Option<String>,
}

/// Comprehensive system health and resource telemetry report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SystemReport {
    pub ok: bool,
    pub node_name: String,
    pub device_id: String,
    pub process: ProcessMetrics,
    pub host: HostMetrics,
    pub timestamp_ms: u64,
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

/// Overall assessment grade of peer connection quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkGrade {
    Excellent,
    Good,
    Fair,
    Poor,
    Unreachable,
}

impl LinkGrade {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Fair => "fair",
            Self::Poor => "poor",
            Self::Unreachable => "unreachable",
        }
    }
}

/// Dynamic heartbeat keepalive configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HeartbeatConfig {
    pub interval_secs: u64,
    pub deadline_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval_secs: 15,
            deadline_secs: 45,
        }
    }
}

/// Link quality telemetry profile for a peer connection.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeerLinkQualityDto {
    pub device_id: String,
    pub name: String,
    pub addr: String,
    pub is_online: bool,
    pub last_seen_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    pub pings_sent: u64,
    pub pongs_recv: u64,
    pub packet_loss_rate: f64,
    pub grade: LinkGrade,
    pub samples_count: usize,
    pub connected_uptime_secs: u64,
}

/// Overall link quality and heartbeat status report.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinkQualityReport {
    pub ok: bool,
    pub heartbeat: HeartbeatConfig,
    pub total_peers: usize,
    pub online_peers: usize,
    pub links: Vec<PeerLinkQualityDto>,
}

/// Result of an on-demand multi-ping probe.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LinkProbeReport {
    pub ok: bool,
    pub peer: String,
    pub count: usize,
    pub sent: usize,
    pub received: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_rtt_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f64>,
    pub packet_loss_rate: f64,
    pub samples: Vec<u64>,
    pub grade: LinkGrade,
}

/// Sliding window metrics tracker for an active peer link.
#[derive(Debug, Clone)]
pub struct PeerLinkTracker {
    pub pings_sent: u64,
    pub pongs_recv: u64,
    pub recent_rtts: std::collections::VecDeque<u64>,
    pub max_samples: usize,
    pub connected_at_ms: u64,
    pub last_ping_sent_ms: Option<u64>,
}

impl PeerLinkTracker {
    pub fn new(now_ms: u64) -> Self {
        Self {
            pings_sent: 0,
            pongs_recv: 0,
            recent_rtts: std::collections::VecDeque::with_capacity(32),
            max_samples: 32,
            connected_at_ms: now_ms,
            last_ping_sent_ms: None,
        }
    }

    pub fn record_ping_sent(&mut self, now_ms: u64) {
        self.pings_sent = self.pings_sent.saturating_add(1);
        self.last_ping_sent_ms = Some(now_ms);
    }

    pub fn record_pong_recv(&mut self, rtt_ms: u64) {
        self.pongs_recv = self.pongs_recv.saturating_add(1);
        if self.recent_rtts.len() >= self.max_samples {
            self.recent_rtts.pop_front();
        }
        self.recent_rtts.push_back(rtt_ms);
    }

    pub fn min_rtt(&self) -> Option<u64> {
        self.recent_rtts.iter().copied().min()
    }

    pub fn max_rtt(&self) -> Option<u64> {
        self.recent_rtts.iter().copied().max()
    }

    pub fn avg_rtt(&self) -> Option<f64> {
        if self.recent_rtts.is_empty() {
            None
        } else {
            let sum: u64 = self.recent_rtts.iter().sum();
            Some((sum as f64 / self.recent_rtts.len() as f64 * 100.0).round() / 100.0)
        }
    }

    pub fn jitter_ms(&self) -> Option<f64> {
        if self.recent_rtts.len() < 2 {
            return None;
        }
        let total_diff: f64 = self
            .recent_rtts
            .iter()
            .zip(self.recent_rtts.iter().skip(1))
            .map(|(a, b)| (*b as f64 - *a as f64).abs())
            .sum();
        let count = (self.recent_rtts.len() - 1) as f64;
        Some(((total_diff / count) * 100.0).round() / 100.0)
    }

    pub fn packet_loss_rate(&self) -> f64 {
        if self.pings_sent == 0 || self.pongs_recv >= self.pings_sent {
            0.0
        } else {
            let lost = self.pings_sent.saturating_sub(self.pongs_recv);
            let rate = (lost as f64 / self.pings_sent as f64) * 100.0;
            (rate * 10.0).round() / 10.0
        }
    }

    pub fn evaluate_grade(&self, is_online: bool) -> LinkGrade {
        if !is_online {
            return LinkGrade::Unreachable;
        }
        let loss = self.packet_loss_rate();
        match self.avg_rtt() {
            Some(avg) if avg < 25.0 && loss < 1.0 => LinkGrade::Excellent,
            Some(avg) if avg < 100.0 && loss < 5.0 => LinkGrade::Good,
            Some(avg) if avg < 300.0 && loss < 15.0 => LinkGrade::Fair,
            Some(_) => LinkGrade::Poor,
            None if self.pings_sent > 0 && self.pongs_recv == 0 => LinkGrade::Poor,
            None => LinkGrade::Good,
        }
    }
}

/// Category of audit event for security classification and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditCategory {
    Security,
    Peer,
    File,
    Clipboard,
    Notify,
    Route,
    Config,
    Focus,
}

impl std::fmt::Display for AuditCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditCategory::Security => write!(f, "security"),
            AuditCategory::Peer => write!(f, "peer"),
            AuditCategory::File => write!(f, "file"),
            AuditCategory::Clipboard => write!(f, "clipboard"),
            AuditCategory::Notify => write!(f, "notify"),
            AuditCategory::Route => write!(f, "route"),
            AuditCategory::Config => write!(f, "config"),
            AuditCategory::Focus => write!(f, "focus"),
        }
    }
}

impl std::str::FromStr for AuditCategory {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "security" => Ok(AuditCategory::Security),
            "peer" | "peers" => Ok(AuditCategory::Peer),
            "file" | "files" => Ok(AuditCategory::File),
            "clipboard" | "clip" => Ok(AuditCategory::Clipboard),
            "notify" | "notification" => Ok(AuditCategory::Notify),
            "route" | "routes" => Ok(AuditCategory::Route),
            "config" | "configuration" => Ok(AuditCategory::Config),
            "focus" | "dnd" => Ok(AuditCategory::Focus),
            _ => Err(format!(
                "unknown audit category '{s}', expected: security, peer, file, clipboard, notify, route, config, focus"
            )),
        }
    }
}

/// Severity level of audit event.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Info,
    Notice,
    Warning,
    Alert,
}

impl std::fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditSeverity::Info => write!(f, "info"),
            AuditSeverity::Notice => write!(f, "notice"),
            AuditSeverity::Warning => write!(f, "warning"),
            AuditSeverity::Alert => write!(f, "alert"),
        }
    }
}

impl std::str::FromStr for AuditSeverity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "info" => Ok(AuditSeverity::Info),
            "notice" => Ok(AuditSeverity::Notice),
            "warning" | "warn" => Ok(AuditSeverity::Warning),
            "alert" | "critical" => Ok(AuditSeverity::Alert),
            _ => Err(format!(
                "unknown audit severity '{s}', expected: info, notice, warning, alert"
            )),
        }
    }
}

/// Structured security audit entry. Zero-plaintext compliant.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub id: u64,
    pub timestamp_ms: u64,
    pub category: AuditCategory,
    pub severity: AuditSeverity,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub summary: String,
    pub success: bool,
}

/// Paginated and filtered report of audit entries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditLogReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub capacity: usize,
    pub entries: Vec<AuditEntry>,
}

/// Confirmation report when clearing audit log.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuditClearReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub message: String,
}

/// In-memory bounded circular ring buffer for zero-plaintext audit logs.
#[derive(Debug)]
pub struct AuditEngine {
    max_capacity: usize,
    next_id: u64,
    entries: std::collections::VecDeque<AuditEntry>,
}

impl AuditEngine {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(10),
            next_id: 1,
            entries: std::collections::VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        now_ms: u64,
        category: AuditCategory,
        severity: AuditSeverity,
        action: impl Into<String>,
        target: Option<String>,
        summary: impl Into<String>,
        success: bool,
    ) -> AuditEntry {
        let entry = AuditEntry {
            id: self.next_id,
            timestamp_ms: now_ms,
            category,
            severity,
            action: action.into(),
            target,
            summary: summary.into(),
            success,
        };
        self.next_id += 1;
        if self.entries.len() >= self.max_capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry.clone());
        entry
    }

    pub fn query(
        &self,
        category: Option<AuditCategory>,
        severity: Option<AuditSeverity>,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> (usize, Vec<AuditEntry>) {
        let total = self.entries.len();
        let mut filtered: Vec<AuditEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| {
                if let Some(c) = category {
                    if e.category != c {
                        return false;
                    }
                }
                if let Some(s) = severity {
                    if e.severity < s {
                        return false;
                    }
                }
                if let Some(since) = since_ms {
                    if e.timestamp_ms < since {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        if let Some(lim) = limit {
            if filtered.len() > lim {
                filtered.truncate(lim);
            }
        }
        (total, filtered)
    }

    pub fn clear(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        count
    }
}

/// Direction of a peer direct secure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

impl std::fmt::Display for MessageDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageDirection::Inbound => write!(f, "inbound"),
            MessageDirection::Outbound => write!(f, "outbound"),
        }
    }
}

/// A direct message sent point-to-point to or received from a peer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessage {
    pub id: String,
    pub peer: String,
    pub direction: MessageDirection,
    pub text: String,
    pub timestamp_ms: u64,
    pub read: bool,
}

/// In-memory bounded ring buffer for peer direct messages (default 200 items).
#[derive(Debug)]
pub struct MessageEngine {
    max_capacity: usize,
    messages: std::collections::VecDeque<PeerMessage>,
}

impl MessageEngine {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(10),
            messages: std::collections::VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn push(&mut self, msg: PeerMessage) {
        if self.messages.len() >= self.max_capacity {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }

    pub fn query(
        &self,
        peer: Option<&str>,
        limit: Option<usize>,
        unread_only: bool,
    ) -> (usize, usize, Vec<PeerMessage>) {
        let total = self.messages.len();
        let unread = self.messages.iter().filter(|m| !m.read).count();
        let lim = limit.unwrap_or(50).clamp(1, 500);
        let mut list: Vec<PeerMessage> = self
            .messages
            .iter()
            .rev()
            .filter(|m| {
                if let Some(p) = peer {
                    if m.peer != p {
                        return false;
                    }
                }
                if unread_only && m.read {
                    return false;
                }
                true
            })
            .take(lim)
            .cloned()
            .collect();
        // Return in chronological order
        list.reverse();
        (total, unread, list)
    }

    pub fn mark_read(&mut self, peer: Option<&str>) -> usize {
        let mut count = 0;
        for m in self.messages.iter_mut() {
            if !m.read {
                if let Some(p) = peer {
                    if m.peer != p {
                        continue;
                    }
                }
                m.read = true;
                count += 1;
            }
        }
        count
    }

    pub fn clear(&mut self, peer: Option<&str>) -> usize {
        if let Some(p) = peer {
            let before = self.messages.len();
            self.messages.retain(|m| m.peer != p);
            before - self.messages.len()
        } else {
            let count = self.messages.len();
            self.messages.clear();
            count
        }
    }
}

/// Report after sending a peer direct message.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageSendReport {
    pub ok: bool,
    pub message: PeerMessage,
}

/// Paginated/filtered history of peer direct messages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageHistoryReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub unread: usize,
    pub messages: Vec<PeerMessage>,
}

/// Clear confirmation report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageClearReport {
    pub ok: bool,
    pub target: String,
    pub cleared_count: usize,
    pub message: String,
}

/// Status of an outbound file transfer in the node's outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    InFlight,
    Completed,
    Rejected,
    Failed,
    Cancelled,
}

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InFlight => "in_flight",
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for OutboxStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "in_flight" | "inflight" | "sending" => Ok(Self::InFlight),
            "completed" | "complete" | "done" => Ok(Self::Completed),
            "rejected" | "reject" => Ok(Self::Rejected),
            "failed" | "fail" | "error" => Ok(Self::Failed),
            "cancelled" | "cancel" | "canceled" => Ok(Self::Cancelled),
            other => Err(format!("unknown outbox status '{other}'")),
        }
    }
}

/// A structured outbox record tracking a sent file transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxEntry {
    pub id: String,
    pub transfer_id: String,
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub status: OutboxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_ms: Option<u64>,
    #[serde(default)]
    pub speed_bps: u64,
}

/// Bounded in-memory circular ring buffer for outbox records.
pub struct OutboxEngine {
    pub capacity: usize,
    pub entries: VecDeque<OutboxEntry>,
}

impl Default for OutboxEngine {
    fn default() -> Self {
        Self::new(200)
    }
}

impl OutboxEngine {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.clamp(10, 1000),
            entries: VecDeque::with_capacity(capacity.min(200)),
        }
    }

    pub fn push(&mut self, entry: OutboxEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn update_status(
        &mut self,
        transfer_id: &str,
        status: OutboxStatus,
        error: Option<String>,
        completed_ms: Option<u64>,
        speed_bps: Option<u64>,
    ) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| e.transfer_id == transfer_id)
        {
            entry.status = status;
            if let Some(err) = error {
                entry.error = Some(err);
            }
            if let Some(cm) = completed_ms {
                entry.completed_ms = Some(cm);
            }
            if let Some(spd) = speed_bps {
                entry.speed_bps = spd;
            }
            true
        } else {
            false
        }
    }

    pub fn query(
        &self,
        peer: Option<&str>,
        status: Option<OutboxStatus>,
        limit: Option<usize>,
    ) -> (usize, Vec<OutboxEntry>) {
        let total = self.entries.len();
        let lim = limit.unwrap_or(50).clamp(1, 500);
        let mut list: Vec<OutboxEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| {
                if let Some(p) = peer {
                    if e.peer != p && e.peer_alias.as_deref() != Some(p) {
                        return false;
                    }
                }
                if let Some(st) = status {
                    if e.status != st {
                        return false;
                    }
                }
                true
            })
            .take(lim)
            .cloned()
            .collect();
        list.reverse();
        (total, list)
    }

    pub fn clear(&mut self, peer: Option<&str>, status: Option<OutboxStatus>) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| {
            if let Some(p) = peer {
                if e.peer == p || e.peer_alias.as_deref() == Some(p) {
                    if let Some(st) = status {
                        return e.status != st;
                    }
                    return false;
                }
            } else if let Some(st) = status {
                return e.status != st;
            } else {
                return false;
            }
            true
        });
        before - self.entries.len()
    }

    pub fn get_by_id(&self, id_or_transfer_id: &str) -> Option<OutboxEntry> {
        let needle = id_or_transfer_id.trim();
        self.entries
            .iter()
            .rev()
            .find(|e| e.id == needle || e.transfer_id == needle || e.id.starts_with(needle))
            .cloned()
    }
}

/// Outbox query response report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub entries: Vec<OutboxEntry>,
}

/// Report after clearing outbox records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxClearReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub target_peer: Option<String>,
    pub target_status: Option<OutboxStatus>,
    pub message: String,
}

/// Report after re-offering a sent file from outbox.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxResendReport {
    pub ok: bool,
    pub original_id: String,
    pub new_transfer_id: String,
    pub message: String,
}

/// Query report for peer profiles.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerProfilesReport {
    pub ok: bool,
    pub total: usize,
    pub profiles: Vec<PeerProfile>,
}

/// Report after updating a peer profile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerProfileUpdateReport {
    pub ok: bool,
    pub device_id: String,
    pub profile: PeerProfile,
    pub message: String,
}

/// Report after modifying peer tags.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerTagsReport {
    pub ok: bool,
    pub device_id: String,
    pub tags: Vec<String>,
    pub message: String,
}

/// Report after setting or clearing a peer group.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerGroupReport {
    pub ok: bool,
    pub device_id: String,
    pub group: Option<String>,
    pub message: String,
}

/// Single target dispatch outcome for multi-peer message fan-out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for multi-peer targeted message dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiMessageSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerMessageSendItem>,
}

/// Single target dispatch outcome for multi-peer file offer fan-out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerFileSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbox_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for single-hash multi-peer file offer dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiFileSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerFileSendItem>,
}

/// Detailed status and configuration report for granular file auto-accept policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutoAcceptPolicyReport {
    pub ok: bool,
    pub policy: AutoAcceptPolicy,
    pub auto_accepted_count: u64,
    pub message: String,
}

/// Comprehensive status and configuration report for granular clipboard auto-sync policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardPolicyReport {
    pub ok: bool,
    pub policy: ClipboardPolicy,
    pub auto_synced_out_count: u64,
    pub auto_synced_in_count: u64,
    pub sensitive_blocked_count: u64,
    pub message: String,
}

/// Single target dispatch outcome for multi-peer resource handoff.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerHandoffSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for multi-peer targeted resource handoff dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiHandoffSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub url: String,
    pub title: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerHandoffSendItem>,
}

/// Single peer handoff send result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandoffSendResult {
    pub ok: bool,
    pub id: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub url: String,
    pub title: String,
    pub auto_open_requested: bool,
    pub timestamp_ms: u64,
}

/// Status and configuration report for resource handoff policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandoffPolicyReport {
    pub ok: bool,
    pub policy: HandoffPolicy,
    pub message: String,
}

/// Single target dispatch outcome for multi-peer media command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMediaCommandSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Single peer media command send result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaCommandSendResult {
    pub ok: bool,
    pub id: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_player: Option<String>,
    pub timestamp_ms: u64,
}

/// Comprehensive aggregated report for multi-peer targeted media command dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiMediaCommandReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub command: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerMediaCommandSendItem>,
}

/// Status and configuration report for media control policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaPolicyReport {
    pub ok: bool,
    pub policy: MediaControlPolicy,
    pub message: String,
}

/// Status report for current local or peer media playback state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaStatusReport {
    pub ok: bool,
    pub peer: Option<String>,
    pub state: Option<MediaStateFrame>,
}

/// Status report for current local or peer battery and power state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryStatusReport {
    pub ok: bool,
    pub peer: Option<String>,
    pub state: Option<BatteryStateFrame>,
}

/// Status and configuration report for battery and power policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryPolicyReport {
    pub ok: bool,
    pub policy: BatteryPolicy,
    pub message: String,
}

/// Status report for local battery state broadcast to online mesh peers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryBroadcastReport {
    pub ok: bool,
    pub state: BatteryStateFrame,
    #[serde(alias = "sent_count")]
    pub broadcast_count: usize,
    pub message: String,
}

/// Active acoustic ringing session on this local device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveLocateRing {
    pub request_id: String,
    pub initiator_peer: String,
    pub initiator_alias: String,
    pub duration_secs: u32,
    pub pattern: LocatePattern,
    pub override_silent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub started_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Status report for locator ringing on this node.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateStatusReport {
    pub ok: bool,
    pub is_ringing: bool,
    pub active_ring: Option<ActiveLocateRing>,
    pub policy: LocatorPolicy,
    pub message: String,
}

/// Report after requesting a target mesh device to start ringing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateRingReport {
    pub ok: bool,
    pub request_id: String,
    pub target_peer: String,
    pub duration_secs: u32,
    pub pattern: LocatePattern,
    pub override_silent: bool,
    pub message: String,
}

/// Report after cancelling/stopping a locator ringing session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateStopReport {
    pub ok: bool,
    pub request_id: Option<String>,
    pub target_peer: Option<String>,
    pub reason: String,
    pub message: String,
}

/// Status and configuration report for device locator policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocatorPolicyReport {
    pub ok: bool,
    pub policy: LocatorPolicy,
    pub message: String,
}

/// Status and configuration report for action execution policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionPolicyReport {
    pub ok: bool,
    pub policy: ActionPolicy,
    pub message: String,
}

/// Report for local registered actions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocalActionReport {
    pub ok: bool,
    pub count: usize,
    pub actions: Vec<LocalActionDefinition>,
    pub message: String,
}

/// Report for cached or discovered remote peer actions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteActionReport {
    pub ok: bool,
    pub peer: String,
    pub count: usize,
    pub actions: Vec<ActionItem>,
    pub message: String,
}

/// Report returned when triggering a remote action.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionTriggerReport {
    pub ok: bool,
    pub trigger_id: String,
    pub action_id: String,
    pub target_peer: String,
    pub status: String,
    pub message: String,
}

/// Report for recent action execution results.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionHistoryReport {
    pub ok: bool,
    pub count: usize,
    pub results: Vec<ActionResultFrame>,
    pub message: String,
}

/// Status report for the local device's focus and DND state, effective status, and active quiet hours.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusStatusReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: FocusMode,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_secs: Option<u64>,
    pub quiet_hours_active: bool,
    pub policy: FocusPolicy,
    pub message: String,
}

/// Report for a specific peer device's focus state and active countdown.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerFocusReport {
    pub ok: bool,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub state: FocusStateFrame,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_secs: Option<u64>,
}

/// Aggregated report of known mesh peers and their advertised focus states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeersFocusReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<PeerFocusReport>,
}

/// Report returned when activating or deactivating focus mode locally and broadcasting to mesh peers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusSetReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: FocusMode,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub broadcast_count: usize,
    pub message: String,
}

/// Status and governance report for focus and quiet hours policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusPolicyReport {
    pub ok: bool,
    pub policy: FocusPolicy,
    pub message: String,
}

/// Status and configuration report for local spatial display topology.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologyLayoutReport {
    pub ok: bool,
    pub display: DisplayRect,
    pub neighbors: HashMap<DisplayEdge, EdgeNeighbor>,
    pub config: EdgeCrossingConfig,
    pub peer_count: usize,
    pub message: String,
}

/// Summary entry for a peer's advertised display topology.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeerTopologyItem {
    pub peer_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub display: DisplayRect,
    pub neighbors: HashMap<DisplayEdge, EdgeNeighbor>,
    pub config: EdgeCrossingConfig,
    pub timestamp_ms: u64,
}

/// Report of all known remote peer display topologies.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologyPeersReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<PeerTopologyItem>,
    pub message: String,
}

/// Report returned when linking a peer to a display edge.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologyLinkReport {
    pub ok: bool,
    pub edge: DisplayEdge,
    pub peer_id: String,
    pub peer_edge: DisplayEdge,
    pub offset: i32,
    pub message: String,
}

/// Report returned when unlinking an edge neighbor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologyUnlinkReport {
    pub ok: bool,
    pub edge: DisplayEdge,
    pub message: String,
}

/// Report returned when updating display geometry or crossing parameters.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologyConfigReport {
    pub ok: bool,
    pub display: DisplayRect,
    pub config: EdgeCrossingConfig,
    pub message: String,
}

/// Report returned after broadcasting topology synchronization.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TopologySyncReport {
    pub ok: bool,
    pub broadcast_count: usize,
    pub message: String,
}

/// Status and governance report for topology coordination policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TopologyPolicyReport {
    pub ok: bool,
    pub policy: TopologyPolicy,
    pub message: String,
}

/// Result of an edge crossing decision on cursor movement.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorCrossingDecision {
    pub crossed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge: Option<DisplayEdge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_y: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

/// Active virtual input tunnel session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ActiveInputTunnelSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: InputTunnelRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_edge: Option<DisplayEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_height: Option<u32>,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Comprehensive status report of virtual peripheral sharing and input tunneling.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputTunnelStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveInputTunnelSession>,
    pub policy: InputTunnelPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when attaching or releasing an input tunnel session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputTunnelDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<InputTunnelRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual peripheral sharing and input tunneling.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputTunnelPolicyReport {
    pub ok: bool,
    pub policy: InputTunnelPolicy,
    pub message: String,
}

/// Partial update payload for input tunnel policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InputTunnelPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_control: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_keyboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_mouse: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey_release_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active virtual secondary display session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveVirtualDisplaySession {
    pub session_id: String,
    pub peer_id: String,
    pub role: VirtualDisplayRole,
    pub assigned_edge: DisplayEdge,
    pub mode: VirtualDisplayMode,
    pub codec: VirtualDisplayCodec,
    pub target_fps: u32,
    pub current_fps: u32,
    pub rtt_us: u64,
    pub dropped_frames: u32,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Comprehensive status report of virtual secondary display streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VirtualDisplayStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveVirtualDisplaySession>,
    pub policy: VirtualDisplayPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when attaching or detaching a virtual display session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VirtualDisplayDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<VirtualDisplayRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual secondary display.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VirtualDisplayPolicyReport {
    pub ok: bool,
    pub policy: VirtualDisplayPolicy,
    pub message: String,
}

/// Partial update payload for virtual display policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VirtualDisplayPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_hosting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_extend: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active spatial audio routing session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveAudioRoutingSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: AudioRoutingRole,
    pub layout: AudioChannelLayout,
    pub sample_rate: u32,
    pub channels: u8,
    pub format: AudioSampleFormat,
    pub codec: AudioStreamCodec,
    pub spatial_pos: Option<AudioSpatialPosition>,
    pub volume_gain: f32,
    pub chunks_sent: u64,
    pub chunks_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for spatial audio routing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AudioRoutingStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveAudioRoutingSession>,
    pub policy: AudioRoutingPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating an audio routing route.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AudioRoutingDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<AudioRoutingRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for spatial audio routing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AudioRoutingPolicyReport {
    pub ok: bool,
    pub policy: AudioRoutingPolicy,
    pub message: String,
}

/// Partial update payload for audio routing policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AudioRoutingPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_sink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_outbound_routing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_volume_limit: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_spatial_rendering: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active virtual camera streaming session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveCameraSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: CameraRole,
    pub lens_id: String,
    pub resolution: CameraResolution,
    pub target_fps: u32,
    pub current_fps: u32,
    pub codec: CameraCodec,
    pub flashlight_on: bool,
    pub zoom_ratio: f32,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for virtual camera streaming and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CameraStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveCameraSession>,
    pub policy: CameraPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating a camera session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CameraDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<CameraRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual camera streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CameraPolicyReport {
    pub ok: bool,
    pub policy: CameraPolicy,
    pub message: String,
}

/// Partial update payload for camera policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CameraPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_capture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_privacy_shutter_indicator: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active virtual microphone or voice intercom session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveMicrophoneSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: MicRole,
    pub mode: MicMode,
    pub mic_id: String,
    pub codec: MicCodec,
    pub sample_rate: u32,
    pub channels: u32,
    pub gain: f32,
    pub muted: bool,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    pub privacy_indicator_active: bool,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for virtual microphone, intercom streaming and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophoneStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveMicrophoneSession>,
    pub policy: MicrophonePolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating a microphone session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophoneDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<MicRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual microphone streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophonePolicyReport {
    pub ok: bool,
    pub policy: MicrophonePolicy,
    pub message: String,
}

/// Partial update payload for microphone policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MicrophonePolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_capture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_intercom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_session_duration_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_privacy_indicator: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active link throughput benchmark session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveBenchSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: BenchRole,
    pub direction: BenchDirection,
    pub duration_secs: u32,
    pub chunk_size: u32,
    pub chunks_processed: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for the link benchmark engine and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BenchStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveBenchSession>,
    pub policy: BenchPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<BenchReportFrame>,
    pub timestamp_ms: u64,
}

/// Decision report when starting or stopping a benchmark session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BenchDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for link benchmarking.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BenchPolicyReport {
    pub ok: bool,
    pub policy: BenchPolicy,
    pub message: String,
}

/// Streaming parameter recommendation report for a single peer (ADR-078).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StreamingRecommendationReport {
    pub ok: bool,
    pub peer_id: String,
    pub peer_alias: String,
    pub has_profile: bool,
    pub measured_mbps: f64,
    pub direction: String,
    pub measured_at_ms: u64,
    pub recommendation: tetherly_core::recommend::StreamingRecommendation,
    pub timestamp_ms: u64,
}

/// One peer's recommendation summary line for the aggregate status view.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PeerRecommendationSummary {
    pub peer_id: String,
    pub peer_alias: String,
    pub measured_mbps: f64,
    pub direction: String,
    pub measured_at_ms: u64,
    pub tier: String,
}

/// Aggregate recommendation status across all measured peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RecommendationStatusReport {
    pub ok: bool,
    pub peers: Vec<PeerRecommendationSummary>,
    pub timestamp_ms: u64,
}

/// Partial update payload for bench policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BenchPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_bench: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bandwidth_mbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Status report for the node's clipboard state and statistics without leaking plaintext.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipStatusReport {
    pub ok: bool,
    pub has_text: bool,
    pub bytes: usize,
    pub hash: Option<String>,
    pub last_seq: u64,
    pub inbound_count: u64,
    pub outbound_count: u64,
    pub history_count: usize,
    /// False on hosts with no display server (headless Linux): the OS
    /// clipboard port itself is absent, sync statistics remain observable.
    pub clipboard_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Bounded history report of clipboard synchronization events without leaking plaintext.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipHistoryReport {
    pub ok: bool,
    pub total: usize,
    pub entries: Vec<ClipMetaEntryDto>,
}

/// Metadata entry representing a single inbound or outbound clipboard synchronization.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipMetaEntryDto {
    pub seq: u64,
    pub source: String,
    pub direction: String,
    pub bytes: usize,
    pub hash: String,
    pub timestamp_ms: u64,
}

/// Report returned when clearing the system clipboard and resetting hub state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipClearReport {
    pub ok: bool,
    pub cleared_history_count: usize,
    pub timestamp_ms: u64,
    /// False when no OS clipboard exists (headless): history still resets,
    /// there is simply nothing on the display server to clear.
    pub clipboard_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

struct Hubs {
    notify: NotifyHub,
    clip: ClipHub,
    files: FileHub,
    /// Chunk-aligned progress markers for interrupted incoming transfers.
    /// Loaded from disk at startup; the node persists them as sidecars.
    resume: tetherly_core::ResumeTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransferPhase {
    Transferring,
    Verifying,
    Completed,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ActiveTransfer {
    pub transfer_id: String,
    pub peer: DeviceId,
    pub direction: TransferDirection,
    pub filename: String,
    pub total: u64,
    pub transferred: u64,
    pub speed_bps: u64,
    pub phase: TransferPhase,
    pub error: Option<String>,
    pub started_ms: u64,
    pub updated_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_offset: Option<u64>,
    #[serde(default)]
    pub is_resumed: bool,
}

/// Comprehensive lightweight report of active and retained file transfers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TransfersReport {
    pub ok: bool,
    pub active_count: usize,
    pub terminal_count: usize,
    pub total_count: usize,
    pub transfers: Vec<ActiveTransfer>,
    pub stats: TransferStats,
}

/// Maximum number of completed/failed transfer records retained in memory.
pub const MAX_TERMINAL_TRANSFERS: usize = 64;

/// Comprehensive report of pending incoming file transfer offers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileOffersReport {
    pub ok: bool,
    pub count: usize,
    pub auto_accept_trusted: bool,
    pub offers: Vec<FileOfferDto>,
}

/// Detailed metadata of a single pending file transfer offer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileOfferDto {
    pub transfer_id: String,
    pub filename: String,
    pub total_bytes: u64,
    pub resume_from: u64,
    pub source: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_name: Option<String>,
    pub size: u64,
    pub sha256: String,
    pub timestamp_ms: u64,
    pub offered_ms: u64,
}

/// Result of accepting or rejecting a file transfer offer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileDecisionReport {
    pub ok: bool,
    pub accepted: bool,
    pub action: String,
    pub transfer_id: String,
    pub peer: String,
    pub filename: String,
    pub total_bytes: u64,
    pub resume_from: u64,
    pub timestamp_ms: u64,
}

/// Result of batch accepting or rejecting file transfer offers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FileBatchDecisionReport {
    pub ok: bool,
    pub action: String,
    pub accepted_count: usize,
    pub rejected_count: usize,
    pub total_evaluated: usize,
    pub decisions: Vec<FileDecisionReport>,
}

/// Single item within a batch send request.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// Request payload for batch file sending.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BatchSendItem>,
}

/// Result report for an individual file within a batch send operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchTransferItemReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
    pub name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Aggregate report returned after submitting a batch of files for sending.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BatchSendReport {
    pub ok: bool,
    pub batch_id: String,
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub total_files: usize,
    pub successful_files: usize,
    pub failed_files: usize,
    pub total_bytes: u64,
    pub items: Vec<BatchTransferItemReport>,
}

/// Detailed inspection of an in-progress or interrupted transfer capable of being resumed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ResumeMarkerDto {
    pub transfer_id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_alias: Option<String>,
    pub total_bytes: u64,
    pub offset_bytes: u64,
    pub progress_percent: f64,
    pub chunk_aligned: bool,
    pub part_file_exists: bool,
    pub part_file_bytes: u64,
}

/// Comprehensive lightweight report of all stored resume markers in memory and disk sidecars.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ResumeMarkersReport {
    pub ok: bool,
    pub count: usize,
    pub total_partial_bytes: u64,
    pub markers: Vec<ResumeMarkerDto>,
}

/// Status and configuration of auto-accept policy for trusted peers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AutoAcceptReport {
    pub ok: bool,
    pub enabled: bool,
    pub auto_accepted_count: u64,
}

/// Session aggregate transfer metrics across all inbound and outbound files.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TransferStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub completed_count: u64,
    pub failed_count: u64,
}

/// Structured DTO representing one allowlist rule for ANCS notification action open.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OpenRuleDto {
    pub app_id: String,
    pub url: String,
}

/// Lightweight status report for the ANCS (Apple Notification Center Service) ingress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsReport {
    pub ok: bool,
    pub attached: bool,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peripheral: Option<String>,
    pub retry_at_ms: Option<u64>,
    pub rules: Vec<OpenRuleDto>,
    pub allowlist: Vec<String>,
}

/// Report returned by an ANCS connect attempt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsConnectReport {
    pub ok: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// Report returned by an ANCS disconnect or reset action.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsDisconnectReport {
    pub ok: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// Report returned when an allowlist rule is added or removed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsRuleReport {
    pub ok: bool,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<OpenRuleDto>,
    pub total_rules: usize,
    pub timestamp_ms: u64,
}

/// Report returned when all ANCS allowlist rules are cleared.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsClearRulesReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub total_rules: usize,
    pub timestamp_ms: u64,
}

/// Lightweight status report for node data directory and storage consumption.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StorageReport {
    pub ok: bool,
    pub data_dir: String,
    pub inbox_dir: String,
    pub inbox_total_bytes: u64,
    pub inbox_file_count: usize,
    pub partial_bytes: u64,
    pub partial_count: usize,
    pub orphaned_partial_count: usize,
    pub orphaned_partial_bytes: u64,
    pub meta_bytes: u64,
    #[serde(default)]
    pub resume_markers_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resume_markers: Vec<ResumeMarkerDto>,
}

/// Report returned by a storage cleanup operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StorageCleanReport {
    pub ok: bool,
    pub freed_bytes: u64,
    pub deleted_files_count: usize,
    pub cleared_resume_entries: usize,
}

/// Metadata entry for a completed received file in the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxFileDto {
    pub name: String,
    pub size: u64,
    pub modified_at_secs: u64,
    pub is_safe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

/// Comprehensive report of files residing in the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxReport {
    pub ok: bool,
    pub count: usize,
    pub total_bytes: u64,
    pub files: Vec<InboxFileDto>,
}

/// Result of deleting a specific file from the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxDeleteReport {
    pub ok: bool,
    pub name: String,
    pub freed_bytes: u64,
}

/// Result of clearing multiple or all files from the incoming inbox.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxClearReport {
    pub ok: bool,
    pub deleted_count: usize,
    pub freed_bytes: u64,
}

/// Result of opening a specific file or the inbox directory itself in the OS.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InboxOpenReport {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub path: String,
    pub message: String,
}

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

/// Port assignments bound by the running node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NodePorts {
    pub control: u16,
    pub file: u16,
    pub input: u16,
    pub screen: u16,
    pub ui: u16,
}

/// Lightweight health report for liveness probes, monitoring, and diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HealthReport {
    pub ok: bool,
    pub status: String,
    pub version: String,
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub pid: u32,
    pub uptime_secs: u64,
    pub ports: NodePorts,
    pub live_peers: usize,
    pub trusted_peers: usize,
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

/// Physical addressing route entry with enriched peer status metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RouteEntryDto {
    pub device_id: String,
    pub addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub online: bool,
    pub trusted: bool,
}

/// Comprehensive report of cached physical addressing routes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RoutesReport {
    pub ok: bool,
    pub count: usize,
    pub routes: Vec<RouteEntryDto>,
}

/// Result report from adding or removing a physical addressing route.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RouteMutateReport {
    pub ok: bool,
    pub device_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    pub action: String,
}

/// Comprehensive status report for remote screen streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
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

/// Comprehensive status report for input sharing (mouse & keyboard).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputStatusReport {
    pub ok: bool,
    pub channel_open: bool,
    pub seat: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_peer: Option<String>,
}

/// Control result report for input sharing actions (start, stop).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputControlReport {
    pub ok: bool,
    pub action: String,
    pub seat: String,
    pub channel_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    pub timestamp_ms: u64,
}

/// One local network interface classification report DTO.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct IfaceReportDto {
    pub iface: String,
    pub ip: String,
    pub listen: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Status report for EasyTier overlay network sidecar.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OverlayStatusReport {
    pub enabled: bool,
    pub present: bool,
    pub source: String,
    pub local_ips: Vec<String>,
    pub peer_count: usize,
    pub peers: Vec<String>,
}

/// Comprehensive network interfaces and overlay status report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NetworkReport {
    pub ok: bool,
    #[serde(alias = "ifaces")]
    pub interfaces: Vec<IfaceReportDto>,
    pub overlay: OverlayStatusReport,
}

/// Action report returned when triggering a dynamic network refresh and overlay probe.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NetworkRefreshReport {
    pub ok: bool,
    pub action: String,
    pub ifaces_scanned: usize,
    pub ifaces_listening: usize,
    pub overlay_present: bool,
    pub overlay_source: String,
    pub overlay_local_ips: Vec<String>,
    pub overlay_peers_count: usize,
    pub timestamp_ms: u64,
}

enum OutboundSource {
    Bytes(Vec<u8>),
    Path(std::path::PathBuf),
}

struct OutboundFile {
    peer: DeviceId,
    #[allow(dead_code)]
    name: String,
    source: OutboundSource,
    size: u64,
}

/// Sidecar persisted next to a `.part` file: lets an interrupted incoming
/// transfer resume after a process restart. No file content, only progress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ResumeSidecar {
    #[serde(alias = "source")]
    device_id: String,
    #[serde(alias = "total_bytes")]
    total: u64,
    offset: u64,
}

/// Wall-clock p95 of host-side frame send intervals. Real timing lives here in
/// tetherly-node (core is deterministic and OS-free); production verifies the
/// frame budget (M5.1) against this. Microsecond resolution.
struct ScreenIntervalTracker {
    last: Option<std::time::Instant>,
    intervals: Vec<u64>, // micros, newest first
    max: usize,
}

impl ScreenIntervalTracker {
    fn new() -> Self {
        Self {
            last: None,
            intervals: Vec::with_capacity(1024),
            max: 1024,
        }
    }

    /// Mark one frame send. The elapsed time since the previous send is the
    /// interval; the first send only seeds the clock.
    fn record(&mut self) {
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
    fn p95_us(&self) -> Option<u64> {
        if self.intervals.len() < 2 {
            return None;
        }
        let mut sorted = self.intervals.clone();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        Some(sorted[idx.min(sorted.len() - 1)])
    }
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
    input_out: AsyncMutex<Option<mpsc::Sender<InnerFrame>>>,
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
    /// Visitor-side output. Defaults to the in-memory observer; production
    /// injects `SystemSink` for real display.
    screen_sink: Arc<dyn ScreenSink>,
    /// Host-side frame-interval tracker (p95 budget). Recorded per successful
    /// frame send.
    screen_interval: Mutex<ScreenIntervalTracker>,
    /// Visitor-side receiver counters (received/dropped/bytes). Written by the
    /// screen-client task so the UI can show what actually arrived.
    screen_rx_stats: Mutex<ScreenStats>,
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
    cursor_edge_dwell: Mutex<HashMap<DisplayEdge, u64>>,
    cursor_edge_resistance_acc: Mutex<HashMap<DisplayEdge, u32>>,
    input_policy: Mutex<InputTunnelPolicy>,
    active_input_tunnel: Mutex<Option<ActiveInputTunnelSession>>,
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

impl Node {
    pub fn start(cfg: NodeConfig) -> Result<Self, crate::store::NodeStoreError> {
        Self::start_with_clock(cfg, Arc::new(SystemClock))
    }

    pub fn start_with_clock(
        cfg: NodeConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, crate::store::NodeStoreError> {
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
            input_out: AsyncMutex::new(None),
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
            screen_out: AsyncMutex::new(None),
            screen_peer: Mutex::new(None),
            screen_engine: Mutex::new({
                let mut s = ScreenSender::with_fps(screen_fps);
                s.set_scale_pct(screen_scale_pct);
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
            screen_sink,
            screen_interval: Mutex::new(ScreenIntervalTracker::new()),
            screen_rx_stats: Mutex::new(ScreenStats::default()),
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
            cursor_edge_dwell: Mutex::new(HashMap::new()),
            cursor_edge_resistance_acc: Mutex::new(HashMap::new()),
            input_policy: Mutex::new(initial_input_policy),
            active_input_tunnel: Mutex::new(None),
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
        });
        Ok(Self { shared })
    }

    pub fn set_sys_probe(&self, probe: Arc<dyn crate::platform::SysProbe>) {
        *self.shared.sys_probe.lock().expect("set_sys_probe lock") = probe;
    }

    pub fn sys_probe(&self) -> Arc<dyn crate::platform::SysProbe> {
        self.shared
            .sys_probe
            .lock()
            .expect("sys_probe lock")
            .clone()
    }

    pub fn system_report(&self) -> Result<SystemReport, CoreError> {
        let pid = std::process::id();
        let uptime_secs = self.shared.started_at.elapsed().as_secs();
        let probe = self.sys_probe();
        let proc_sample = probe.sample_process(pid)?;
        let host_sample = probe.sample_host()?;

        let process = ProcessMetrics {
            pid,
            uptime_secs,
            rss_bytes: proc_sample.rss_bytes,
            virt_bytes: proc_sample.virt_bytes,
            thread_count: proc_sample.thread_count,
            open_fds: proc_sample.open_fds,
            cpu_time_ms: proc_sample.cpu_time_ms,
            memory_rss_formatted: format_memory_bytes(proc_sample.rss_bytes),
            memory_virt_formatted: format_memory_bytes(proc_sample.virt_bytes),
        };

        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let host = HostMetrics {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            num_cpus,
            total_memory_bytes: host_sample.total_memory_bytes,
            total_memory_formatted: host_sample.total_memory_bytes.map(format_memory_bytes),
        };

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let node_name = self
            .shared
            .node_name
            .lock()
            .expect("node_name lock")
            .clone();
        let device_id = self.shared.identity.device_id().to_string();

        let report = SystemReport {
            ok: true,
            node_name,
            device_id,
            process: process.clone(),
            host: host.clone(),
            timestamp_ms,
        };

        let _ = self.shared.events.send(UiEvent::system_metrics_refreshed(
            process.pid,
            process.uptime_secs,
            process.rss_bytes,
            process.virt_bytes,
            process.thread_count,
            process.open_fds,
            process.cpu_time_ms,
            host.num_cpus,
            host.total_memory_bytes,
            timestamp_ms,
        ));

        Ok(report)
    }

    pub fn set_opener(&self, opener: Arc<dyn tetherly_core::Opener>) {
        *self.shared.opener.lock().expect("set_opener lock") = opener;
    }

    pub fn opener(&self) -> Arc<dyn tetherly_core::Opener> {
        self.shared.opener.lock().expect("opener lock").clone()
    }

    pub fn set_notifier(&self, notifier: Arc<dyn Notifier>) {
        *self.shared.notifier.lock().expect("set_notifier lock") = notifier;
    }

    pub fn notifier(&self) -> Arc<dyn Notifier> {
        self.shared.notifier.lock().expect("notifier lock").clone()
    }

    pub fn notify_config(&self) -> NotifyBannerConfig {
        self.shared
            .notify_config
            .lock()
            .expect("notify_config lock")
            .clone()
    }

    pub fn notify_config_report(&self) -> NotifyConfigReport {
        let config = self.notify_config();
        let total_dispatched = self.shared.notify_dispatched_count.load(Ordering::Relaxed);
        let total_suppressed = self.shared.notify_suppressed_count.load(Ordering::Relaxed);
        NotifyConfigReport {
            ok: true,
            config,
            total_dispatched,
            total_suppressed,
        }
    }

    pub fn update_notify_config(&self, new_cfg: NotifyBannerConfig) -> NotifyConfigReport {
        {
            let mut c = self
                .shared
                .notify_config
                .lock()
                .expect("notify_config lock");
            *c = new_cfg.clone();
        }
        let dnd_str = match new_cfg.dnd_mode {
            DndMode::Off => "off",
            DndMode::Silent => "silent",
            DndMode::All => "all",
        };
        let _ = self.shared.events.send(UiEvent::notify_config_changed(
            new_cfg.banners_enabled,
            dnd_str,
            new_cfg.sound_enabled,
            new_cfg.mask_otp,
            new_cfg.allow_otp_in_dnd,
        ));
        self.notify_config_report()
    }

    pub fn dispatch_notification_banner(
        &self,
        title: &str,
        body: &str,
        app_name: Option<&str>,
        is_otp: bool,
    ) -> Result<bool, CoreError> {
        let cfg = self.notify_config();
        if !cfg.banners_enabled {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        let should_suppress = match cfg.dnd_mode {
            DndMode::All => !(is_otp && cfg.allow_otp_in_dnd),
            DndMode::Silent | DndMode::Off => false,
        };

        if should_suppress {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        // ADR-070: Cross-device focus mode and quiet hours suppression
        let focus_status = self.focus_status();
        if focus_status.active {
            let breakthrough = is_otp && focus_status.policy.allow_emergency_breakthrough;
            if !breakthrough {
                self.shared
                    .notify_suppressed_count
                    .fetch_add(1, Ordering::Relaxed);
                self.record_audit(
                    AuditCategory::Focus,
                    AuditSeverity::Info,
                    "focus_notification_suppressed",
                    None::<String>,
                    format!(
                        "Notification '{}' suppressed due to active focus mode ({:?})",
                        title, focus_status.mode
                    ),
                    false,
                );
                return Ok(false);
            }
        }

        let play_sound = cfg.sound_enabled && cfg.dnd_mode != DndMode::Silent;

        let display_body = if is_otp && cfg.mask_otp {
            "收到短信验证码 (已在通知中心就绪)".to_string()
        } else {
            body.to_string()
        };

        let urgency = if is_otp {
            NotificationUrgency::Critical
        } else {
            NotificationUrgency::Normal
        };

        let mut payload = NotificationPayload::new(title, display_body)
            .with_sound(play_sound)
            .with_urgency(urgency);
        if let Some(app) = app_name {
            payload = payload.with_app_name(app);
        }

        let notifier = self.notifier();
        notifier.notify(&payload)?;
        self.shared
            .notify_dispatched_count
            .fetch_add(1, Ordering::Relaxed);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::notification_banner_sent(
            title, app_name, is_otp, now_ms,
        ));

        Ok(true)
    }

    pub fn test_notification_banner(
        &self,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<NotifyTestReport, CoreError> {
        let t = title.unwrap_or("Tetherly 桌面通知测试");
        let b = body.unwrap_or("这是一条来自 Tetherly 的测试通知横幅。跨设备互联已正常工作！");
        let cfg = self.notify_config();
        if !cfg.banners_enabled {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some("banners_disabled".into()),
                message: "notification suppressed: banners are disabled in config".into(),
                title: t.into(),
            });
        }

        let should_suppress = match cfg.dnd_mode {
            DndMode::All => true,
            DndMode::Silent | DndMode::Off => false,
        };

        if should_suppress {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some(format!("dnd_mode_{:?}", cfg.dnd_mode).to_lowercase()),
                message: format!(
                    "notification suppressed by Do-Not-Disturb mode: {:?}",
                    cfg.dnd_mode
                ),
                title: t.into(),
            });
        }

        // ADR-070: Cross-device focus mode and quiet hours suppression for test notifications
        let focus_status = self.focus_status();
        if focus_status.active {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some(
                    format!("focus_mode_{:?}", focus_status.mode).to_lowercase(),
                ),
                message: format!(
                    "notification suppressed by active focus mode: {:?} (quiet_hours={})",
                    focus_status.mode, focus_status.quiet_hours_active
                ),
                title: t.into(),
            });
        }

        let play_sound = cfg.sound_enabled && cfg.dnd_mode != DndMode::Silent;
        let payload = NotificationPayload::new(t, b)
            .with_app_name("Tetherly")
            .with_sound(play_sound)
            .with_urgency(NotificationUrgency::Normal);

        let notifier = self.notifier();
        notifier.notify(&payload)?;
        self.shared
            .notify_dispatched_count
            .fetch_add(1, Ordering::Relaxed);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::notification_banner_sent(
            t,
            Some("Tetherly"),
            false,
            now_ms,
        ));

        Ok(NotifyTestReport {
            ok: true,
            sent: true,
            suppressed: false,
            suppression_reason: None,
            message: "test desktop notification banner dispatched successfully".into(),
            title: t.into(),
        })
    }

    pub fn identity(&self) -> &Identity {
        &self.shared.identity
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UiEvent> {
        self.shared.events.subscribe()
    }

    pub fn session_config(&self) -> SessionConfig {
        self.session_config_inner(false)
    }

    fn session_config_inner(&self, resume_only: bool) -> SessionConfig {
        let pin = *self.shared.pin.lock().expect("pin");
        SessionConfig {
            identity: self.shared.identity.clone(),
            name: self.name(),
            platform: self.shared.cfg.platform.clone(),
            trust: self.shared.trust.clone(),
            lockout: self.shared.lockout.clone(),
            tcp_limiter: self.shared.tcp_limiter.clone(),
            pin: pin.map(|(p, _)| p),
            pin_created_ms: pin.map(|(_, t)| t).unwrap_or(0),
            clock: self.shared.clock.clone(),
            handshake_timeout: HANDSHAKE_TIMEOUT,
            hello_override: None,
            resume_only,
            screen_only: false,
            caps: None,
        }
    }

    pub fn persist_trust(&self) {
        let store = self.shared.trust.lock().expect("trust");
        if let Err(e) = save_trust(&self.shared.cfg.data_dir, &store) {
            warn!(%e, "trust persist failed");
        }
    }

    pub fn generate_pin(&self) -> Result<String, CoreError> {
        let pin = generate_pin(&OsRng)?;
        let now = self.shared.clock.unix_ms();
        *self.shared.pin.lock().expect("pin") = Some((pin, now));
        let _ = self
            .shared
            .events
            .send(UiEvent::PinStateChanged { active: true });
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "pin_generated",
            None::<String>,
            "new pairing PIN generated for incoming connection",
            true,
        );
        Ok(format_pin(&pin))
    }

    pub fn set_pin(&self, digits: [u8; 8]) {
        *self.shared.pin.lock().expect("pin") = Some((digits, self.shared.clock.unix_ms()));
        let _ = self
            .shared
            .events
            .send(UiEvent::PinStateChanged { active: true });
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "pin_set",
            None::<String>,
            "pairing PIN set by runtime",
            true,
        );
    }

    pub fn pin_if_fresh(&self) -> Option<String> {
        let guard = self.shared.pin.lock().expect("pin");
        let (pin, created) = guard.as_ref()?;
        let age = self.shared.clock.unix_ms().saturating_sub(*created);
        if age > PIN_TTL.as_millis() as u64 {
            return None;
        }
        Some(format_pin(pin))
    }

    pub fn pin_report(&self) -> PinReport {
        let ttl_secs = PIN_TTL.as_secs();
        let ttl_ms = PIN_TTL.as_millis() as u64;
        let guard = self.shared.pin.lock().expect("pin");
        match guard.as_ref() {
            Some((digits, created)) => {
                let age = self.shared.clock.unix_ms().saturating_sub(*created);
                if age <= ttl_ms {
                    let rem_ms = ttl_ms.saturating_sub(age);
                    let rem_secs = rem_ms / 1000;
                    PinReport {
                        ok: true,
                        active: true,
                        pin: Some(format_pin(digits)),
                        expires_in_secs: rem_secs,
                        ttl_secs,
                        created_at_ms: *created,
                    }
                } else {
                    PinReport {
                        ok: true,
                        active: false,
                        pin: None,
                        expires_in_secs: 0,
                        ttl_secs,
                        created_at_ms: *created,
                    }
                }
            }
            None => PinReport {
                ok: true,
                active: false,
                pin: None,
                expires_in_secs: 0,
                ttl_secs,
                created_at_ms: 0,
            },
        }
    }

    pub fn clear_pin(&self) -> bool {
        let mut guard = self.shared.pin.lock().expect("pin");
        let was_active = if let Some((_, created)) = guard.as_ref() {
            let age = self.shared.clock.unix_ms().saturating_sub(*created);
            age <= PIN_TTL.as_millis() as u64
        } else {
            false
        };
        *guard = None;
        drop(guard);
        if was_active {
            let _ = self
                .shared
                .events
                .send(UiEvent::PinStateChanged { active: false });
        }
        was_active
    }

    pub fn name(&self) -> String {
        self.shared.node_name.lock().expect("node_name").clone()
    }

    pub fn platform(&self) -> &str {
        &self.shared.cfg.platform
    }

    pub fn is_advertise_enabled(&self) -> bool {
        self.shared.advertise.load(Ordering::SeqCst)
    }

    pub fn set_advertise_enabled(&self, enabled: bool) -> bool {
        let prev = self.shared.advertise.swap(enabled, Ordering::SeqCst);
        if prev != enabled {
            if enabled {
                self.advertise_mdns();
            } else if let Some(tx) = self
                .shared
                .advertise_stop
                .lock()
                .expect("advertise_stop")
                .take()
            {
                let _ = tx.send(true);
            }
            self.emit_config_changed();
        }
        enabled
    }

    pub fn is_reconnect_enabled(&self) -> bool {
        self.shared.reconnect.load(Ordering::SeqCst)
    }

    pub fn set_reconnect_enabled(&self, enabled: bool) -> bool {
        let prev = self.shared.reconnect.swap(enabled, Ordering::SeqCst);
        if prev != enabled {
            self.emit_config_changed();
        }
        enabled
    }

    pub fn set_node_name(&self, name: &str) -> Result<String, CoreError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidConfig(
                "node name cannot be empty".to_string(),
            ));
        }
        if trimmed.len() > 64 {
            return Err(CoreError::InvalidConfig(
                "node name exceeds maximum length of 64 bytes".to_string(),
            ));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(CoreError::InvalidConfig(
                "node name contains disallowed control characters".to_string(),
            ));
        }
        {
            let mut n = self.shared.node_name.lock().expect("node_name");
            *n = trimmed.to_string();
        }
        if self.is_advertise_enabled() {
            if let Some(tx) = self
                .shared
                .advertise_stop
                .lock()
                .expect("advertise_stop")
                .take()
            {
                let _ = tx.send(true);
            }
            self.advertise_mdns();
        }
        self.emit_config_changed();
        Ok(trimmed.to_string())
    }

    pub fn update_config(
        &self,
        name: Option<&str>,
        advertise: Option<bool>,
        reconnect: Option<bool>,
    ) -> Result<NodeConfigReport, CoreError> {
        if let Some(n) = name {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return Err(CoreError::InvalidConfig(
                    "node name cannot be empty".to_string(),
                ));
            }
            if trimmed.len() > 64 {
                return Err(CoreError::InvalidConfig(
                    "node name exceeds maximum length of 64 bytes".to_string(),
                ));
            }
            if trimmed.chars().any(|c| c.is_control()) {
                return Err(CoreError::InvalidConfig(
                    "node name contains disallowed control characters".to_string(),
                ));
            }
            *self.shared.node_name.lock().expect("node_name") = trimmed.to_string();
        }
        if let Some(adv) = advertise {
            let prev = self.shared.advertise.swap(adv, Ordering::SeqCst);
            if prev != adv {
                if adv {
                    self.advertise_mdns();
                } else if let Some(tx) = self
                    .shared
                    .advertise_stop
                    .lock()
                    .expect("advertise_stop")
                    .take()
                {
                    let _ = tx.send(true);
                }
            } else if adv && name.is_some() {
                if let Some(tx) = self
                    .shared
                    .advertise_stop
                    .lock()
                    .expect("advertise_stop")
                    .take()
                {
                    let _ = tx.send(true);
                }
                self.advertise_mdns();
            }
        } else if name.is_some() && self.is_advertise_enabled() {
            if let Some(tx) = self
                .shared
                .advertise_stop
                .lock()
                .expect("advertise_stop")
                .take()
            {
                let _ = tx.send(true);
            }
            self.advertise_mdns();
        }
        if let Some(rec) = reconnect {
            self.shared.reconnect.store(rec, Ordering::SeqCst);
        }
        self.emit_config_changed();
        Ok(self.config_report())
    }

    pub fn config_report(&self) -> NodeConfigReport {
        let bound_ctrl = self.shared.bound_control.load(Ordering::SeqCst);
        let bound_file = self.shared.bound_file.load(Ordering::SeqCst);
        let bound_input = self.shared.bound_input.load(Ordering::SeqCst);
        let bound_screen = self.shared.bound_screen.load(Ordering::SeqCst);
        let bound_ui = self.shared.bound_ui.load(Ordering::SeqCst);

        let ctrl_port = if bound_ctrl != 0 {
            bound_ctrl
        } else {
            self.shared.cfg.control_port
        };
        let file_port = if bound_file != 0 {
            bound_file
        } else {
            self.shared.cfg.file_port
        };
        let input_port = if bound_input != 0 {
            bound_input
        } else {
            self.shared.cfg.input_port
        };
        let screen_port = if bound_screen != 0 {
            bound_screen
        } else {
            self.shared.cfg.screen_port
        };
        let ui_port = if bound_ui != 0 {
            bound_ui
        } else {
            self.shared.cfg.ui_port
        };

        let ancs_attached = self.shared.ancs.lock().expect("ancs").is_some();

        NodeConfigReport {
            ok: true,
            device_id: self.shared.identity.device_id().to_string(),
            name: self.name(),
            platform: self.platform().to_string(),
            data_dir: self.shared.cfg.data_dir.display().to_string(),
            loopback_only: self.shared.cfg.loopback_only,
            advertise: self.is_advertise_enabled(),
            reconnect: self.is_reconnect_enabled(),
            memory_clip: self.shared.cfg.memory_clip,
            memory_notifier: self.shared.cfg.memory_notifier,
            ui_port,
            control_port: ctrl_port,
            file_port,
            input_port,
            screen_port,
            overlay_enabled: self.shared.cfg.overlay.enabled,
            ancs_attached,
            ancs_peripheral: self.shared.cfg.ancs_peripheral.clone(),
        }
    }

    fn emit_config_changed(&self) {
        let name = self.name();
        let advertise = self.is_advertise_enabled();
        let reconnect = self.is_reconnect_enabled();
        self.emit(UiEvent::ConfigChanged {
            name,
            advertise,
            reconnect,
        });
    }

    pub fn candidates(&self) -> Vec<CandidateViewDto> {
        let mut hubs = self.shared.hubs.lock().expect("hubs");
        let expired = hubs.notify.evict_expired(&*self.shared.clock);
        drop(hubs);
        for id in expired {
            let _ = self.shared.events.send(UiEvent::CandidateExpired { id });
        }
        self.shared
            .hubs
            .lock()
            .expect("hubs")
            .notify
            .views(&*self.shared.clock)
            .into_iter()
            .map(CandidateViewDto::from)
            .collect()
    }

    /// Return all active candidates filtered by optional peer, app_id, or has_otp.
    pub fn candidates_filtered(
        &self,
        peer: Option<&str>,
        app_id: Option<&str>,
        has_otp: Option<bool>,
    ) -> Result<Vec<CandidateViewDto>, CoreError> {
        let mut list = self.candidates();
        if let Some(peer_str) = peer.filter(|s| !s.trim().is_empty()) {
            let target_id = self.resolve_peer(peer_str)?;
            list.retain(|c| c.source == target_id.as_str());
        }
        if let Some(app) = app_id.filter(|s| !s.trim().is_empty()) {
            list.retain(|c| c.app_id == app.trim());
        }
        if let Some(need_otp) = has_otp {
            list.retain(|c| c.has_otp == need_otp);
        }
        Ok(list)
    }

    pub fn live_peers(&self) -> Vec<LivePeer> {
        self.shared
            .live
            .lock()
            .expect("live")
            .values()
            .cloned()
            .collect()
    }

    pub fn trusted(&self) -> Vec<tetherly_core::TrustedPeer> {
        self.shared.trust.lock().expect("trust").all()
    }

    pub fn is_trusted(&self, id: &DeviceId) -> bool {
        let store = self.shared.trust.lock().expect("trust");
        store.peers.get(id).is_some_and(|p| !p.revoked)
    }

    /// Retrieve the local friendly alias for a trusted peer device, if set and not revoked.
    pub fn peer_alias(&self, device_id: &DeviceId) -> Option<String> {
        let store = self.shared.trust.lock().expect("trust");
        store
            .peers
            .get(device_id)
            .filter(|p| !p.revoked && !p.alias.is_empty())
            .map(|p| p.alias.clone())
    }

    /// Update the local friendly alias for a trusted peer device.
    /// Validates length and characters, persists to trusted.json, and emits `UiEvent::PeerAliasChanged`.
    pub fn set_peer_alias(&self, device_id: &DeviceId, alias: &str) -> Result<bool, CoreError> {
        let trimmed = alias.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidAlias("alias cannot be empty".to_string()));
        }
        if trimmed.len() > 64 {
            return Err(CoreError::InvalidAlias(
                "alias exceeds maximum length of 64 bytes".to_string(),
            ));
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(CoreError::InvalidAlias(
                "alias contains disallowed control characters".to_string(),
            ));
        }

        let updated = {
            let mut store = self.shared.trust.lock().expect("trust");
            store.update_alias(device_id, trimmed)?
        };

        if updated {
            self.persist_trust();
            self.emit(UiEvent::PeerAliasChanged {
                device_id: device_id.to_string(),
                alias: trimmed.to_string(),
            });
        }

        Ok(updated)
    }

    /// Resolve a user-supplied peer query string (device_id, alias, or peer name)
    /// into a concrete DeviceId.
    pub fn resolve_peer(&self, query: &str) -> Result<DeviceId, CoreError> {
        let q = query.trim();
        if q.is_empty() {
            return Err(CoreError::PeerNotFound("empty query".to_string()));
        }

        // 1. Direct exact DeviceId parse
        if let Ok(id) = DeviceId::parse(q) {
            return Ok(id);
        }

        let live = self.live_peers();
        let trusted = self.trusted();
        let q_lower = q.to_lowercase();
        let mut matches = Vec::new();

        // 2. Exact match on alias or name (case-insensitive)
        for t in &trusted {
            if t.alias.to_lowercase() == q_lower && !matches.contains(&t.device_id) {
                matches.push(t.device_id.clone());
            }
        }
        for l in &live {
            if l.name.to_lowercase() == q_lower && !matches.contains(&l.device_id) {
                matches.push(l.device_id.clone());
            }
        }

        // 3. Substring / prefix match if no exact match
        if matches.is_empty() {
            for t in &trusted {
                if (t.alias.to_lowercase().contains(&q_lower)
                    || t.device_id.as_str().to_lowercase().contains(&q_lower))
                    && !matches.contains(&t.device_id)
                {
                    matches.push(t.device_id.clone());
                }
            }
            for l in &live {
                if (l.name.to_lowercase().contains(&q_lower)
                    || l.device_id.as_str().to_lowercase().contains(&q_lower))
                    && !matches.contains(&l.device_id)
                {
                    matches.push(l.device_id.clone());
                }
            }
        }

        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(CoreError::PeerNotFound(format!(
                "no peer matching '{q}' found"
            ))),
            _ => {
                let candidate_list = matches
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(CoreError::PeerNotFound(format!(
                    "multiple peers match '{q}': {candidate_list}"
                )))
            }
        }
    }

    /// Retrieve the metadata profile for a device. Defaults to empty profile if unrecorded.
    pub fn peer_profile(&self, device_id: &DeviceId) -> PeerProfile {
        let id_str = device_id.to_string();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .find(|p| p.device_id == id_str)
            .cloned()
            .unwrap_or_else(|| PeerProfile {
                device_id: id_str,
                device_type: DeviceType::Other,
                tags: Vec::new(),
                group: None,
                notes: None,
                updated_ms: 0,
            })
    }

    /// Query and filter peer profiles by tag, group, or device type.
    pub fn peer_profiles(
        &self,
        tag: Option<&str>,
        group: Option<&str>,
        device_type: Option<DeviceType>,
    ) -> PeerProfilesReport {
        let table = self.shared.profiles.lock().expect("profiles");
        let mut profiles = table.profiles.clone();

        if let Some(t) = tag {
            let t_clean = t.trim().to_ascii_lowercase();
            profiles.retain(|p| {
                p.tags
                    .iter()
                    .any(|tag_item| tag_item.to_ascii_lowercase() == t_clean)
            });
        }
        if let Some(g) = group {
            let g_clean = g.trim().to_ascii_lowercase();
            profiles.retain(|p| {
                p.group
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref()
                    == Some(&g_clean)
            });
        }
        if let Some(dt) = device_type {
            profiles.retain(|p| p.device_type == dt);
        }

        PeerProfilesReport {
            ok: true,
            total: profiles.len(),
            profiles,
        }
    }

    fn persist_profiles(&self, table: &ProfileTable) -> Result<(), CoreError> {
        crate::store::save_profiles(&self.shared.cfg.data_dir, table)
            .map_err(|e| CoreError::File(format!("failed to save profiles.json: {e}")))
    }

    /// Update a peer's profile attributes (type, tags, group, notes).
    pub fn update_peer_profile(
        &self,
        target: &str,
        device_type: Option<DeviceType>,
        tags: Option<Vec<String>>,
        group: Option<Option<String>>,
        notes: Option<Option<String>>,
    ) -> Result<PeerProfileUpdateReport, CoreError> {
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();

        let validated_tags = if let Some(tag_list) = tags {
            if tag_list.len() > 16 {
                return Err(CoreError::InvalidProfile(
                    "a device can have at most 16 tags".to_string(),
                ));
            }
            let mut set = Vec::new();
            for t in tag_list {
                let val = validate_tag(&t)?;
                if !set.contains(&val) {
                    set.push(val);
                }
            }
            Some(set)
        } else {
            None
        };

        let validated_group = if let Some(g) = group {
            match g {
                Some(ref g_str) => Some(validate_group(g_str)?),
                None => Some(None),
            }
        } else {
            None
        };

        let validated_notes = if let Some(n) = notes {
            match n {
                Some(ref n_str) => Some(validate_notes(n_str)?),
                None => Some(None),
            }
        } else {
            None
        };

        let now = self.shared.clock.unix_ms();

        let updated_profile = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let profile = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                if let Some(dt) = device_type {
                    p.device_type = dt;
                }
                if let Some(t) = validated_tags {
                    p.tags = t;
                }
                if let Some(g) = validated_group {
                    p.group = g;
                }
                if let Some(n) = validated_notes {
                    p.notes = n;
                }
                p.updated_ms = now;
                p.clone()
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: device_type.unwrap_or_default(),
                    tags: validated_tags.unwrap_or_default(),
                    group: validated_group.flatten(),
                    notes: validated_notes.flatten(),
                    updated_ms: now,
                };
                table.profiles.push(p.clone());
                p
            };
            self.persist_profiles(&table)?;
            profile
        };

        // Zero-Plaintext event broadcast: strictly never broadcast notes!
        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            updated_profile.device_type.as_str(),
            updated_profile.tags.clone(),
            updated_profile.group.clone(),
            updated_profile.updated_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "profile_updated",
            Some(device_id_str.clone()),
            format!(
                "profile updated for peer '{}': type={}, tags_count={}, group={:?}",
                device_id_str,
                updated_profile.device_type.as_str(),
                updated_profile.tags.len(),
                updated_profile.group
            ),
            true,
        );

        Ok(PeerProfileUpdateReport {
            ok: true,
            device_id: device_id_str,
            profile: updated_profile,
            message: "peer profile updated successfully".to_string(),
        })
    }

    /// Add a tag to a peer's profile.
    pub fn add_peer_tag(&self, target: &str, tag: &str) -> Result<PeerTagsReport, CoreError> {
        let clean_tag = validate_tag(tag)?;
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (tags, dt_str, grp) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (tags, dt_str, grp) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                if !p.tags.iter().any(|t| t.eq_ignore_ascii_case(&clean_tag)) {
                    if p.tags.len() >= 16 {
                        return Err(CoreError::InvalidProfile(
                            "maximum 16 tags exceeded".to_string(),
                        ));
                    }
                    p.tags.push(clean_tag.clone());
                    p.updated_ms = now;
                }
                (p.tags.clone(), p.device_type.as_str(), p.group.clone())
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: DeviceType::Other,
                    tags: vec![clean_tag.clone()],
                    group: None,
                    notes: None,
                    updated_ms: now,
                };
                table.profiles.push(p);
                (vec![clean_tag.clone()], "other", None)
            };
            self.persist_profiles(&table)?;
            (tags, dt_str, grp)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags.clone(),
            grp,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "tag_added",
            Some(device_id_str.clone()),
            format!("added tag '{clean_tag}' to peer '{device_id_str}'"),
            true,
        );

        Ok(PeerTagsReport {
            ok: true,
            device_id: device_id_str,
            tags,
            message: format!("tag '{clean_tag}' added successfully"),
        })
    }

    /// Remove a tag from a peer's profile.
    pub fn remove_peer_tag(&self, target: &str, tag: &str) -> Result<PeerTagsReport, CoreError> {
        let clean_tag = tag.trim().to_ascii_lowercase();
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (tags, dt_str, grp) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (tags, dt_str, grp) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                p.tags.retain(|t| !t.eq_ignore_ascii_case(&clean_tag));
                p.updated_ms = now;
                (p.tags.clone(), p.device_type.as_str(), p.group.clone())
            } else {
                (Vec::new(), "other", None)
            };
            self.persist_profiles(&table)?;
            (tags, dt_str, grp)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags.clone(),
            grp,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "tag_removed",
            Some(device_id_str.clone()),
            format!("removed tag '{clean_tag}' from peer '{device_id_str}'"),
            true,
        );

        Ok(PeerTagsReport {
            ok: true,
            device_id: device_id_str,
            tags,
            message: format!("tag '{clean_tag}' removed successfully"),
        })
    }

    /// Assign or clear a peer's group.
    pub fn set_peer_group(
        &self,
        target: &str,
        group: Option<&str>,
    ) -> Result<PeerGroupReport, CoreError> {
        let validated_group = match group {
            Some(g) => validate_group(g)?,
            None => None,
        };
        let device_id = self.resolve_peer(target)?;
        let device_id_str = device_id.to_string();
        let now = self.shared.clock.unix_ms();

        let (group_res, dt_str, tags) = {
            let mut table = self.shared.profiles.lock().expect("profiles");
            let pos = table
                .profiles
                .iter()
                .position(|p| p.device_id == device_id_str);
            let (grp, dt_str, tags_res) = if let Some(idx) = pos {
                let p = &mut table.profiles[idx];
                p.group = validated_group;
                p.updated_ms = now;
                (p.group.clone(), p.device_type.as_str(), p.tags.clone())
            } else {
                let p = PeerProfile {
                    device_id: device_id_str.clone(),
                    device_type: DeviceType::Other,
                    tags: Vec::new(),
                    group: validated_group.clone(),
                    notes: None,
                    updated_ms: now,
                };
                table.profiles.push(p);
                (validated_group, "other", Vec::new())
            };
            self.persist_profiles(&table)?;
            (grp, dt_str, tags_res)
        };

        self.emit(UiEvent::peer_profile_updated(
            &device_id_str,
            dt_str,
            tags,
            group_res.clone(),
            now,
        ));

        let action_desc = match &group_res {
            Some(g) => format!("set group to '{g}' for peer '{device_id_str}'"),
            None => format!("cleared group for peer '{device_id_str}'"),
        };

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "group_changed",
            Some(device_id_str.clone()),
            action_desc,
            true,
        );

        Ok(PeerGroupReport {
            ok: true,
            device_id: device_id_str,
            group: group_res,
            message: "peer group updated successfully".to_string(),
        })
    }

    /// Resolve all device IDs that match a given tag.
    pub fn resolve_peers_by_tag(&self, tag: &str) -> Vec<DeviceId> {
        let t_clean = tag.trim().to_ascii_lowercase();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| {
                p.tags
                    .iter()
                    .any(|item| item.to_ascii_lowercase() == t_clean)
            })
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all device IDs that match a given group.
    pub fn resolve_peers_by_group(&self, group: &str) -> Vec<DeviceId> {
        let g_clean = group.trim().to_ascii_lowercase();
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| {
                p.group
                    .as_deref()
                    .map(|s| s.to_ascii_lowercase())
                    .as_deref()
                    == Some(&g_clean)
            })
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all device IDs that match a given device type.
    pub fn resolve_peers_by_type(&self, device_type: DeviceType) -> Vec<DeviceId> {
        let table = self.shared.profiles.lock().expect("profiles");
        table
            .profiles
            .iter()
            .filter(|p| p.device_type == device_type)
            .filter_map(|p| DeviceId::parse(&p.device_id).ok())
            .collect()
    }

    /// Resolve all candidate target peer device IDs matched by a `TargetSelector`.
    /// Preserves deduplication order and ensures at least one target candidate was found.
    pub fn resolve_dispatch_targets(
        &self,
        selector: &TargetSelector,
    ) -> Result<Vec<DeviceId>, CoreError> {
        let targets = match selector {
            TargetSelector::Peer(p) => {
                let id = self.resolve_peer(p)?;
                vec![id]
            }
            TargetSelector::Tag(t) => self.resolve_peers_by_tag(t),
            TargetSelector::Group(g) => self.resolve_peers_by_group(g),
            TargetSelector::Type(dt) => self.resolve_peers_by_type(*dt),
            TargetSelector::AllOnline => {
                let live = self.live_peers();
                live.into_iter().map(|p| p.device_id).collect()
            }
        };

        let mut unique = Vec::new();
        for id in targets {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }

        if unique.is_empty() {
            return Err(CoreError::NoTargetsMatched(selector.as_label()));
        }

        Ok(unique)
    }

    pub fn routes(&self) -> Vec<crate::store::Route> {
        crate::store::load_routes(&self.shared.cfg.data_dir).routes
    }

    pub fn routes_report(&self) -> RoutesReport {
        let routes = crate::store::load_routes(&self.shared.cfg.data_dir).routes;
        let live = self.live_peers();
        let trusted = self.trusted();

        let entries = routes
            .into_iter()
            .map(|r| {
                let matched_live = live.iter().find(|l| l.device_id.as_str() == r.device_id);
                let matched_trusted = trusted.iter().find(|t| t.device_id.as_str() == r.device_id);

                let online = matched_live.is_some();
                let is_trusted = matched_trusted.map(|t| !t.revoked).unwrap_or(false);
                let alias = matched_trusted.and_then(|t| {
                    if t.alias.is_empty() {
                        None
                    } else {
                        Some(t.alias.clone())
                    }
                });
                let name = matched_live.map(|l| l.name.clone());

                RouteEntryDto {
                    device_id: r.device_id,
                    addr: r.addr,
                    alias,
                    name,
                    online,
                    trusted: is_trusted,
                }
            })
            .collect::<Vec<_>>();

        RoutesReport {
            ok: true,
            count: entries.len(),
            routes: entries,
        }
    }

    pub fn add_route(&self, peer_spec: &str, addr: &str) -> Result<RouteMutateReport, CoreError> {
        let addr_trimmed = addr.trim();
        let socket_addr: SocketAddr = addr_trimmed.parse().map_err(|e| {
            CoreError::InvalidRoute(format!("invalid socket address '{addr_trimmed}': {e}"))
        })?;
        if socket_addr.port() == 0 {
            return Err(CoreError::InvalidRoute("port cannot be 0".to_string()));
        }

        let peer_trimmed = peer_spec.trim();
        if peer_trimmed.is_empty() {
            return Err(CoreError::InvalidRoute(
                "peer identifier cannot be empty".to_string(),
            ));
        }
        let device_id = self.resolve_peer(peer_trimmed)?;
        let is_update = crate::store::remember_route(
            &self.shared.cfg.data_dir,
            device_id.as_str(),
            &socket_addr.to_string(),
        )
        .map_err(|e| CoreError::File(e.to_string()))?;

        let action = if is_update { "updated" } else { "added" };
        self.emit(UiEvent::RouteChanged {
            device_id: device_id.to_string(),
            addr: socket_addr.to_string(),
            action: action.to_string(),
        });
        self.record_audit(
            AuditCategory::Route,
            AuditSeverity::Info,
            format!("route_{action}"),
            Some(device_id.to_string()),
            format!("route {action} for peer '{device_id}' -> {socket_addr}"),
            true,
        );
        Ok(RouteMutateReport {
            ok: true,
            device_id: device_id.to_string(),
            addr: Some(socket_addr.to_string()),
            action: action.to_string(),
        })
    }

    pub fn remove_route(&self, peer_spec: &str) -> Result<RouteMutateReport, CoreError> {
        let peer_trimmed = peer_spec.trim();
        if peer_trimmed.is_empty() {
            return Err(CoreError::InvalidRoute(
                "peer identifier cannot be empty".to_string(),
            ));
        }
        let target_device_id = match self.resolve_peer(peer_trimmed) {
            Ok(id) => id.to_string(),
            Err(_) => {
                let routes = crate::store::load_routes(&self.shared.cfg.data_dir).routes;
                if let Some(r) = routes.iter().find(|r| r.device_id == peer_trimmed) {
                    r.device_id.clone()
                } else {
                    return Err(CoreError::PeerNotFound(format!(
                        "no route matching peer '{peer_trimmed}' found"
                    )));
                }
            }
        };

        let removed = crate::store::forget_route(&self.shared.cfg.data_dir, &target_device_id)
            .map_err(|e| CoreError::File(e.to_string()))?;
        if !removed {
            return Err(CoreError::PeerNotFound(format!(
                "no route found for peer '{target_device_id}'"
            )));
        }
        self.emit(UiEvent::RouteChanged {
            device_id: target_device_id.clone(),
            addr: String::new(),
            action: "removed".to_string(),
        });
        self.record_audit(
            AuditCategory::Route,
            AuditSeverity::Notice,
            "route_removed",
            Some(target_device_id.clone()),
            format!("route removed for peer '{target_device_id}'"),
            true,
        );
        Ok(RouteMutateReport {
            ok: true,
            device_id: target_device_id,
            addr: None,
            action: "removed".to_string(),
        })
    }

    pub fn discovered_peers(&self) -> DiscoveryReport {
        let disc = self.shared.discovered.lock().expect("discovered").clone();
        let trusted = self.trusted();
        let live = self.live_peers();

        let mut peers: Vec<DiscoveredPeerDto> = disc
            .into_values()
            .map(|p| {
                let did = &p.device_id;
                let is_trusted = trusted
                    .iter()
                    .any(|t| t.device_id.as_str() == did && !t.revoked);
                let is_online = live.iter().any(|l| l.device_id.as_str() == did);
                DiscoveredPeerDto {
                    device_id: p.device_id,
                    name: p.name,
                    addrs: p.addrs,
                    port: p.port,
                    fp: p.fp,
                    caps: p.caps,
                    is_trusted,
                    is_online,
                    last_seen_ms: p.last_seen_ms,
                }
            })
            .collect();

        peers.sort_by_key(|a| std::cmp::Reverse(a.last_seen_ms));
        let count = peers.len();
        DiscoveryReport {
            ok: true,
            count,
            peers,
        }
    }

    pub fn rescan_discovery(&self) -> DiscoveryRescanReport {
        let _ = self.shared.rescan_trigger.send(());
        DiscoveryRescanReport {
            ok: true,
            message: "discovery rescan initiated".to_string(),
        }
    }

    pub fn record_discovered_peer(
        &self,
        device_id: &str,
        name: &str,
        addrs: &[&str],
        port: u16,
        fp: Option<&str>,
        caps: &[&str],
    ) {
        if device_id == self.shared.identity.device_id().as_str() {
            return;
        }
        let now = self.shared.clock.unix_ms();
        let is_new = !self
            .shared
            .discovered
            .lock()
            .expect("discovered")
            .contains_key(device_id);
        let info = DiscoveredPeerInfo {
            device_id: device_id.to_string(),
            name: name.to_string(),
            addrs: addrs.iter().map(|s| s.to_string()).collect(),
            port,
            fp: fp.map(ToString::to_string),
            caps: caps.iter().map(|s| s.to_string()).collect(),
            last_seen_ms: now,
        };
        self.shared
            .discovered
            .lock()
            .expect("discovered")
            .insert(device_id.to_string(), info);

        if is_new {
            let primary_addr = addrs
                .first()
                .map(|ip| format!("{}:{}", ip, port))
                .unwrap_or_else(|| format!("127.0.0.1:{}", port));
            let _ = self.shared.events.send(UiEvent::PeerDiscovered {
                device_id: device_id.to_string(),
                name: name.to_string(),
                addr: primary_addr,
                caps: caps.iter().map(|s| s.to_string()).collect(),
            });
        }
    }

    pub fn expire_discovered_peer(&self, device_id: &str) -> bool {
        let removed = self
            .shared
            .discovered
            .lock()
            .expect("discovered")
            .remove(device_id)
            .is_some();
        if removed {
            let _ = self.shared.events.send(UiEvent::PeerExpired {
                device_id: device_id.to_string(),
            });
        }
        removed
    }

    pub async fn dial_peer(&self, target: &str) -> Result<SocketAddr, tetherly_net::NetError> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err(tetherly_net::NetError::Discovery(
                "target address or peer identifier cannot be empty".to_string(),
            ));
        }

        // 1. Direct SocketAddr parse
        if let Ok(sa) = trimmed.parse::<SocketAddr>() {
            self.dial(sa).await?;
            return Ok(sa);
        }

        // 2. Discovered peers candidate lookup
        let discovered_candidate = {
            let disc = self.shared.discovered.lock().expect("discovered");
            disc.values()
                .find(|p| {
                    p.device_id.eq_ignore_ascii_case(trimmed)
                        || p.name.eq_ignore_ascii_case(trimmed)
                })
                .map(|p| (p.addrs.clone(), p.port))
        };

        if let Some((addrs, port)) = discovered_candidate {
            for ip_str in &addrs {
                if let Ok(ip) = ip_str.parse::<IpAddr>() {
                    let sa = SocketAddr::new(ip, port);
                    if let Ok(Ok(())) =
                        tokio::time::timeout(std::time::Duration::from_secs(2), self.dial(sa)).await
                    {
                        return Ok(sa);
                    }
                }
            }
        }

        // 3. Route table candidate lookup
        let route_candidate = {
            let routes = self.routes();
            routes
                .into_iter()
                .find(|r| r.device_id.eq_ignore_ascii_case(trimmed))
        };

        if let Some(r) = route_candidate {
            if let Ok(sa) = r.addr.parse::<SocketAddr>() {
                if let Ok(Ok(())) =
                    tokio::time::timeout(std::time::Duration::from_secs(2), self.dial(sa)).await
                {
                    return Ok(sa);
                }
            }
        }

        Err(tetherly_net::NetError::Discovery(format!(
            "unable to resolve or reach peer target '{trimmed}'"
        )))
    }

    fn spawn_discovery_browser(&self) {
        if self.shared.cfg.loopback_only {
            return;
        }
        let node = self.clone_handle();
        tokio::task::spawn_blocking(move || {
            let d = match tetherly_net::mdns::daemon() {
                Ok(d) => d,
                Err(e) => {
                    warn!(%e, "mdns daemon start for discovery failed");
                    return;
                }
            };
            let receiver = match tetherly_net::mdns::browse(&d) {
                Ok(r) => r,
                Err(e) => {
                    warn!(%e, "mdns browse start failed");
                    return;
                }
            };
            let handle = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => return,
            };
            handle.block_on(async move {
                let mut receiver = receiver;
                let mut stop = node.shared.shutdown.subscribe();
                let mut rescan_rx = node.shared.rescan_trigger.subscribe();

                loop {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                let _ = tetherly_net::mdns::stop_browse(&d);
                                break;
                            }
                        }
                        Ok(_) = rescan_rx.recv() => {
                            let _ = tetherly_net::mdns::stop_browse(&d);
                            if let Ok(new_r) = tetherly_net::mdns::browse(&d) {
                                receiver = new_r;
                            }
                        }
                        event_res = receiver.recv_async() => {
                            match event_res {
                                Ok(tetherly_net::mdns::ServiceEvent::ServiceResolved(info)) => {
                                    let port = info.get_port();
                                    let mut addrs: Vec<String> = info
                                        .get_addresses()
                                        .iter()
                                        .map(ToString::to_string)
                                        .collect();
                                    addrs.sort();

                                    let dev_id = info
                                        .get_property_val_str("id")
                                        .map(ToString::to_string)
                                        .unwrap_or_default();
                                    if dev_id.is_empty() || dev_id == node.shared.identity.device_id().as_str() {
                                        continue;
                                    }

                                    let name = info
                                        .get_property_val_str("name")
                                        .map(ToString::to_string)
                                        .unwrap_or_else(|| {
                                            info.get_hostname()
                                                .trim_end_matches('.')
                                                .trim_end_matches(".local")
                                                .to_string()
                                        });

                                    let fp = info.get_property_val_str("fp").map(ToString::to_string);
                                    let caps: Vec<String> = info
                                        .get_property_val_str("caps")
                                        .map(|c| c.split(',').filter(|s| !s.is_empty()).map(ToString::to_string).collect())
                                        .unwrap_or_default();

                                    let addr_refs: Vec<&str> = addrs.iter().map(AsRef::as_ref).collect();
                                    let caps_refs: Vec<&str> = caps.iter().map(AsRef::as_ref).collect();
                                    node.record_discovered_peer(
                                        &dev_id,
                                        &name,
                                        &addr_refs,
                                        port,
                                        fp.as_deref(),
                                        &caps_refs,
                                    );
                                }
                                Ok(tetherly_net::mdns::ServiceEvent::ServiceRemoved(_stype, fullname)) => {
                                    let mut to_remove = Vec::new();
                                    {
                                        let disc = node.shared.discovered.lock().expect("discovered");
                                        for (id, p) in disc.iter() {
                                            let inst = p.name.replace('.', "-");
                                            if fullname.contains(&inst) {
                                                to_remove.push(id.clone());
                                            }
                                        }
                                    }
                                    for id in to_remove {
                                        node.expire_discovered_peer(&id);
                                    }
                                }
                                Err(_) => break,
                                _ => {}
                            }
                        }
                    }
                }
            });
        });
    }

    pub fn traffic_report(&self) -> TrafficReport {
        let uptime = self.shared.started_at.elapsed().as_secs();
        self.shared.telemetry.snapshot(uptime)
    }

    pub fn reset_traffic_metrics(&self) {
        self.shared.telemetry.reset();
        self.emit(UiEvent::MetricsReset {
            timestamp_ms: self.shared.clock.unix_ms(),
        });
    }

    /// Introspect local capabilities and dynamic feature configuration.
    pub fn local_caps_report(&self) -> LocalCapsReport {
        let (caps, features) = {
            let local = self.shared.local_caps.lock().expect("local_caps");
            let mut feats: Vec<String> = local.iter().cloned().collect();
            feats.sort();
            let mut all_caps = feats.clone();
            all_caps.push(format!("fileport={}", self.file_port()));
            all_caps.push(format!("inputport={}", self.input_port()));
            all_caps.push(format!("screenport={}", self.screen_port()));
            all_caps.push("screencodecs=zstd,h264".to_string());
            (all_caps, feats)
        };
        LocalCapsReport {
            ok: true,
            device_id: self.shared.identity.device_id().to_string(),
            caps,
            features,
            file_port: self.file_port(),
            input_port: self.input_port(),
            screen_port: self.screen_port(),
        }
    }

    /// Broadcast the current local capabilities and port markers to all active sessions.
    pub async fn broadcast_caps(&self) {
        let rep = self.local_caps_report();
        let frame = match (CapsUpdate {
            caps: rep.caps.clone(),
        })
        .to_frame()
        {
            Ok(f) => f,
            Err(e) => {
                warn!(%e, "failed to serialize CapsUpdate frame");
                return;
            }
        };
        let sessions = self.shared.sessions.lock().await;
        for (peer, tx) in sessions.iter() {
            if let Err(e) = tx.try_send(frame.clone()) {
                debug!(%peer, %e, "failed to push CapsUpdate to session");
            }
        }
        self.emit(UiEvent::PeerCapsChanged {
            device_id: self.shared.identity.device_id().to_string(),
            caps: rep.caps,
            features: rep.features,
        });
    }

    /// Dynamically enable or disable local capabilities and broadcast to all connected peers.
    pub async fn update_local_caps(
        &self,
        enable: &[String],
        disable: &[String],
    ) -> Result<LocalCapsReport, CoreError> {
        for cap in enable.iter().chain(disable.iter()) {
            validate_cap_name(cap)?;
        }
        {
            let mut local = self.shared.local_caps.lock().expect("local_caps");
            for cap in enable {
                local.insert(cap.trim().to_string());
            }
            for cap in disable {
                local.remove(cap.trim());
            }
        }
        self.broadcast_caps().await;
        Ok(self.local_caps_report())
    }

    /// Introspect capabilities of peers. If target_peer is provided, resolves and returns
    /// that peer; otherwise returns all live peers.
    pub fn peers_caps_report(
        &self,
        target_peer: Option<&str>,
    ) -> Result<PeersCapsReport, CoreError> {
        let trusted = self.trusted();
        let mut reports = Vec::new();

        if let Some(query) = target_peer {
            let target_id = self.resolve_peer(query)?;
            let live_map = self.shared.live.lock().expect("live");
            if let Some(lp) = live_map.get(&target_id) {
                let alias = trusted
                    .iter()
                    .find(|t| t.device_id == target_id)
                    .and_then(|t| {
                        if t.alias.is_empty() {
                            None
                        } else {
                            Some(t.alias.clone())
                        }
                    });
                reports.push(PeerCapsDto {
                    device_id: target_id.to_string(),
                    name: lp.name.clone(),
                    alias,
                    caps: lp.caps.clone(),
                    features: extract_features(&lp.caps),
                    file_port: lp.file_port,
                    input_port: lp.input_port,
                    screen_port: lp.screen_port,
                    online: true,
                });
            } else {
                let matched_trusted = trusted.iter().find(|t| t.device_id == target_id);
                let alias = matched_trusted.and_then(|t| {
                    if t.alias.is_empty() {
                        None
                    } else {
                        Some(t.alias.clone())
                    }
                });
                let name = alias.clone().unwrap_or_else(|| "offline".to_string());
                reports.push(PeerCapsDto {
                    device_id: target_id.to_string(),
                    name,
                    alias,
                    caps: Vec::new(),
                    features: Vec::new(),
                    file_port: None,
                    input_port: None,
                    screen_port: None,
                    online: false,
                });
            }
        } else {
            let live_map = self.shared.live.lock().expect("live");
            for (id, lp) in live_map.iter() {
                let alias = trusted.iter().find(|t| t.device_id == *id).and_then(|t| {
                    if t.alias.is_empty() {
                        None
                    } else {
                        Some(t.alias.clone())
                    }
                });
                reports.push(PeerCapsDto {
                    device_id: id.to_string(),
                    name: lp.name.clone(),
                    alias,
                    caps: lp.caps.clone(),
                    features: extract_features(&lp.caps),
                    file_port: lp.file_port,
                    input_port: lp.input_port,
                    screen_port: lp.screen_port,
                    online: true,
                });
            }
        }

        reports.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        Ok(PeersCapsReport {
            ok: true,
            count: reports.len(),
            peers: reports,
        })
    }

    pub fn network_report(&self) -> (Vec<crate::lan::IfaceReport>, crate::sidecar::OverlayStatus) {
        (crate::lan::interface_report(), self.overlay_status())
    }

    pub fn network_report_dto(&self) -> NetworkReport {
        let (ifaces, ov) = self.network_report();
        let iface_dtos = ifaces
            .into_iter()
            .map(|i| IfaceReportDto {
                iface: i.iface,
                ip: i.ip.to_string(),
                listen: i.listen,
                reason: i.reason.map(|r| r.to_string()),
            })
            .collect();
        let overlay_dto = OverlayStatusReport {
            enabled: self.overlay_enabled(),
            present: ov.present,
            source: format!("{:?}", ov.source),
            local_ips: ov.local_ips.iter().map(|ip| ip.to_string()).collect(),
            peer_count: ov.peers.len(),
            peers: ov.peers.iter().map(|ip| ip.to_string()).collect(),
        };
        NetworkReport {
            ok: true,
            interfaces: iface_dtos,
            overlay: overlay_dto,
        }
    }

    pub async fn refresh_network(&self) -> NetworkRefreshReport {
        let ifaces = crate::lan::interface_report();
        let ifaces_scanned = ifaces.len();
        let ifaces_listening = ifaces.iter().filter(|i| i.listen).count();

        let ov = if self.overlay_enabled() {
            crate::sidecar::discover(&self.shared.cfg.overlay).await
        } else {
            crate::sidecar::OverlayStatus::lan_only()
        };

        let prev_ov = {
            let mut lock = self.shared.overlay.lock().expect("overlay");
            let old = lock.clone();
            *lock = ov.clone();
            old
        };

        let overlay_source_str = format!("{:?}", ov.source);
        self.emit(UiEvent::NetworkRefreshed {
            iface_count: ifaces_scanned,
            listen_count: ifaces_listening,
            overlay_present: ov.present,
            overlay_source: Some(overlay_source_str.clone()),
        });

        if prev_ov != ov {
            self.emit(UiEvent::OverlayStateChanged {
                present: ov.present,
                source: overlay_source_str.clone(),
                local_ips: ov.local_ips.iter().map(|ip| ip.to_string()).collect(),
                peer_count: ov.peers.len(),
            });
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        NetworkRefreshReport {
            ok: true,
            action: "refreshed".into(),
            ifaces_scanned,
            ifaces_listening,
            overlay_present: ov.present,
            overlay_source: overlay_source_str,
            overlay_local_ips: ov.local_ips.iter().map(|ip| ip.to_string()).collect(),
            overlay_peers_count: ov.peers.len(),
            timestamp_ms: now_ms,
        }
    }

    pub fn overlay_enabled(&self) -> bool {
        self.shared.cfg.overlay.enabled
    }

    pub fn emit(&self, ev: UiEvent) {
        let _ = self.shared.events.send(ev);
    }

    pub async fn copy_candidate(&self, id: &str) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        self.shared.clip.set_text(&code)?;
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.note_local_write(&code);
        }
        let clip = self.shared.clip.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(CLIPBOARD_OTP_CLEAR_MS)).await;
            if let Ok(Some(cur)) = clip.get_text() {
                if cur == code {
                    let _ = clip.set_text("");
                }
            }
        });
        Ok(())
    }

    pub fn insert_candidate(&self, id: &str) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        insert_foreground(&code)?;
        Ok(())
    }

    pub fn insert_candidate_with(
        &self,
        id: &str,
        insertor: &dyn tetherly_core::Insertor,
    ) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        insertor.insert(&code)
    }

    pub fn dismiss_candidate(&self, id: &str) -> bool {
        let removed = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.take_candidate(id)
        };
        if let Some(c) = removed {
            self.emit(UiEvent::CandidateDismissed {
                id: c.id.clone(),
                source: c.source.to_string(),
                app_id: c.app_id.clone(),
                uid: c.uid.clone(),
            });
            if let Ok(frame) = (tetherly_core::frame::NotifyDismiss {
                uid: c.uid,
                app_id: c.app_id,
            })
            .to_frame()
            {
                let this = self.clone();
                let source = c.source;
                tokio::spawn(async move {
                    let _ = this.send_to(&source, frame).await;
                });
            }
            true
        } else {
            false
        }
    }

    /// Dismiss a single candidate, synchronously awaiting remote wire notification.
    pub async fn dismiss_candidate_sync(
        &self,
        id: &str,
    ) -> Result<CandidateDismissReport, CoreError> {
        let removed = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.take_candidate(id)
        };
        let c = match removed {
            Some(c) => c,
            None => return Err(CoreError::UnknownCandidate),
        };

        let frame = tetherly_core::frame::NotifyDismiss {
            uid: c.uid.clone(),
            app_id: c.app_id.clone(),
        }
        .to_frame()?;
        let notified_remote = self.send_to(&c.source, frame).await.is_ok();

        self.emit(UiEvent::CandidateDismissed {
            id: c.id.clone(),
            source: c.source.to_string(),
            app_id: c.app_id.clone(),
            uid: c.uid.clone(),
        });

        Ok(CandidateDismissReport {
            ok: true,
            id: c.id,
            source: c.source.to_string(),
            app_id: c.app_id,
            uid: c.uid,
            notified_remote,
        })
    }

    /// Perform multi-criteria batch dismissal of candidates, forwarding NotifyDismiss
    /// to all affected connected peer sessions.
    pub async fn dismiss_candidates(
        &self,
        filter: &DismissFilter,
    ) -> Result<BatchDismissReport, CoreError> {
        let removed_candidates = if filter.all.unwrap_or(false) {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.clear_candidates()
        } else if let Some(peer_str) = filter.peer.as_deref().filter(|s| !s.trim().is_empty()) {
            let peer_id = self.resolve_peer(peer_str)?;
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.dismiss_by_source(&peer_id)
        } else if let Some(app_id) = filter.app_id.as_deref().filter(|s| !s.trim().is_empty()) {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.dismiss_by_app(app_id.trim())
        } else if let Some(id) = filter.id.as_deref().filter(|s| !s.trim().is_empty()) {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.take_candidate(id.trim()).into_iter().collect()
        } else {
            return Err(CoreError::InvalidPayload(
                "at least one filter (id, peer, app_id, all) is required".to_string(),
            ));
        };

        let count = removed_candidates.len();
        let mut ids = Vec::with_capacity(count);
        let mut notified_remote_count = 0;

        for c in removed_candidates {
            ids.push(c.id.clone());
            self.emit(UiEvent::CandidateDismissed {
                id: c.id.clone(),
                source: c.source.to_string(),
                app_id: c.app_id.clone(),
                uid: c.uid.clone(),
            });

            if let Ok(frame) = (tetherly_core::frame::NotifyDismiss {
                uid: c.uid,
                app_id: c.app_id,
            })
            .to_frame()
            {
                if self.send_to(&c.source, frame).await.is_ok() {
                    notified_remote_count += 1;
                }
            }
        }

        Ok(BatchDismissReport {
            ok: true,
            count,
            ids,
            notified_remote_count,
        })
    }

    /// Retrieve the OTP code associated with a candidate directly without
    /// copying to the system clipboard. Follows ADR-010 (on explicit user action)
    /// and never logs the returned code.
    pub fn get_candidate_otp(&self, id: &str) -> Result<String, CoreError> {
        let mut hubs = self.shared.hubs.lock().expect("hubs");
        hubs.notify.copy_otp(id, &*self.shared.clock)
    }

    /// Open the app that produced a candidate, on explicit user click. The url
    /// always comes from the allowlist; a url carried by the notification is
    /// never consulted (M4.2).
    pub fn open_candidate(
        &self,
        id: &str,
        opener: &dyn tetherly_core::Opener,
    ) -> Result<String, CoreError> {
        let app_id = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify
                .get(id)
                .map(|c| c.app_id.clone())
                .ok_or(CoreError::UnknownCandidate)?
        };
        let url = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .url_for(&app_id)
            .map(str::to_string)
            .ok_or_else(|| CoreError::OpenRefused(format!("{app_id} is not allowlisted")))?;
        opener.open(&url)?;
        Ok(url)
    }

    pub async fn send_notify(&self, peer: &DeviceId, push: NotifyPush) -> Result<(), CoreError> {
        self.send_to(peer, push.to_frame()?).await
    }

    pub async fn ingest_local_push(&self, source: DeviceId, push: NotifyPush) -> IngestOutcome {
        let title = push.title.clone();
        let body = push.body.clone();
        let app_name = push.app_name.clone();
        let outcome = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.ingest_push(source, push, &*self.shared.clock)
        };
        if let IngestOutcome::Candidate(v) = &outcome {
            self.emit(UiEvent::Candidate {
                candidate: CandidateViewDto::from(v.clone()),
            });
            let app_opt = if app_name.is_empty() {
                None
            } else {
                Some(app_name.as_str())
            };
            let _ = self.dispatch_notification_banner(&title, &body, app_opt, v.has_otp);
        }
        outcome
    }

    /// Reply to the notification behind a candidate, addressed to the phone it
    /// came from. Empty text is refused; the text is trimmed and capped by
    /// `NotifyReply::truncate` on the wire. Never executed, only delivered.
    pub async fn reply_to_candidate(&self, id: &str, text: &str) -> Result<(), CoreError> {
        let (source, app_id, uid) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let c = hubs.notify.get(id).ok_or(CoreError::UnknownCandidate)?;
            (c.source.clone(), c.app_id.clone(), c.uid.clone())
        };
        if text.trim().is_empty() {
            return Err(CoreError::EmptyReply);
        }
        let reply = NotifyReply {
            uid,
            app_id,
            text: text.to_string(),
        }
        .truncate();
        self.send_to(&source, reply.to_frame()?).await
    }

    /// Take a queued inbound reply exactly once by its stable id. Returns the
    /// addressing view and the reply text; the caller that owns the phone path
    /// acts on it (fires the notification's reply action). Single-use so a
    /// quick-reply is never fired twice.
    pub fn take_reply(&self, id: &str) -> Option<(ReplyView, String)> {
        self.shared
            .hubs
            .lock()
            .expect("hubs")
            .notify
            .take_reply_id(id)
    }

    /// Queued `notify.reply` entries awaiting delivery to their phone.
    pub fn reply_views(&self) -> Vec<ReplyView> {
        self.shared.hubs.lock().expect("hubs").notify.reply_views()
    }

    /// Install the app allowlist that gates the ANCS "open" action (M4.2).
    pub fn set_open_allowlist(&self, list: OpenAllowlist) {
        *self.shared.allowlist.lock().expect("allowlist") = list;
    }

    /// Add one allowlist rule (validated). Used by the pairing wizard.
    pub fn add_open_rule(&self, rule: tetherly_core::OpenRule) -> Result<(), CoreError> {
        self.shared.allowlist.lock().expect("allowlist").push(rule)
    }

    pub fn open_allowlist(&self) -> OpenAllowlist {
        self.shared.allowlist.lock().expect("allowlist").clone()
    }

    pub fn ancs_state(&self) -> Option<AncsState> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_ref()
            .map(|i| i.state())
    }

    pub fn ancs_report(&self) -> AncsReport {
        let (attached, state, retry_at_ms) = {
            let guard = self.shared.ancs.lock().expect("ancs");
            match guard.as_ref() {
                None => (false, "off".to_string(), None),
                Some(ing) => match ing.state() {
                    tetherly_core::AncsState::Idle => (true, "idle".to_string(), None),
                    tetherly_core::AncsState::Ready => (true, "ready".to_string(), None),
                    tetherly_core::AncsState::Backoff { retry_at_ms } => {
                        (true, "backoff".to_string(), Some(retry_at_ms))
                    }
                },
            }
        };
        let rules: Vec<OpenRuleDto> = self
            .open_allowlist()
            .rules()
            .iter()
            .map(|r| OpenRuleDto {
                app_id: r.app_id.clone(),
                url: r.url.clone(),
            })
            .collect();
        let allowlist = rules.iter().map(|r| r.app_id.clone()).collect();
        let peripheral = if self.shared.cfg.ancs_peripheral.is_empty() {
            None
        } else {
            Some(self.shared.cfg.ancs_peripheral.clone())
        };
        AncsReport {
            ok: true,
            attached,
            state,
            peripheral,
            retry_at_ms,
            rules,
            allowlist,
        }
    }

    /// Trigger ANCS connection and return a structured report, broadcasting state changes.
    pub fn ancs_connect_reported(&self) -> Result<AncsConnectReport, CoreError> {
        match self.ancs_connect()? {
            Some(ConnectOutcome::Subscribed) => {
                self.emit(UiEvent::ancs_state_changed("ready", true, None));
                Ok(AncsConnectReport {
                    ok: true,
                    state: "ready".to_string(),
                    retry_at_ms: None,
                    message: "ANCS subscribed".to_string(),
                })
            }
            Some(ConnectOutcome::BackingOff { retry_at_ms }) => {
                self.emit(UiEvent::ancs_state_changed(
                    "backoff",
                    true,
                    Some(retry_at_ms),
                ));
                Ok(AncsConnectReport {
                    ok: true,
                    state: "backoff".to_string(),
                    retry_at_ms: Some(retry_at_ms),
                    message: "ANCS backing off".to_string(),
                })
            }
            None => Err(CoreError::NoAncsTransport),
        }
    }

    /// Trigger ANCS disconnection or backoff reset, broadcasting state changes.
    pub fn ancs_disconnect_reported(&self) -> AncsDisconnectReport {
        let retry_at_ms = self.ancs_disconnected();
        let state = if retry_at_ms.is_some() {
            "backoff".to_string()
        } else {
            "idle".to_string()
        };
        self.emit(UiEvent::ancs_state_changed(&state, true, retry_at_ms));
        AncsDisconnectReport {
            ok: true,
            state,
            retry_at_ms,
            message: "ANCS disconnected".to_string(),
        }
    }

    /// Add one allowlist rule (validated), returning a structured report and broadcasting to event bus.
    pub fn add_open_rule_reported(
        &self,
        rule: tetherly_core::OpenRule,
    ) -> Result<AncsRuleReport, CoreError> {
        let app_id = rule.app_id.clone();
        let url = rule.url.clone();
        self.add_open_rule(rule)?;
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        self.emit(UiEvent::ancs_rule_changed(
            "added",
            Some(app_id.clone()),
            Some(url.clone()),
            total_rules,
        ));
        Ok(AncsRuleReport {
            ok: true,
            action: "added".to_string(),
            rule: Some(OpenRuleDto { app_id, url }),
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
    }

    /// Remove one allowlist rule by `app_id`, returning a structured report and broadcasting to event bus.
    pub fn remove_open_rule_reported(&self, app_id: &str) -> AncsRuleReport {
        let removed = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .remove(app_id);
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        let action = if removed.is_some() {
            "removed".to_string()
        } else {
            "not_found".to_string()
        };
        let rule_dto = removed.as_ref().map(|r| OpenRuleDto {
            app_id: r.app_id.clone(),
            url: r.url.clone(),
        });
        if removed.is_some() {
            self.emit(UiEvent::ancs_rule_changed(
                "removed",
                Some(app_id.to_string()),
                removed.as_ref().map(|r| r.url.clone()),
                total_rules,
            ));
        }
        AncsRuleReport {
            ok: true,
            action,
            rule: rule_dto,
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Clear all allowlist rules, returning a structured report and broadcasting to event bus.
    pub fn clear_open_rules_reported(&self) -> AncsClearRulesReport {
        let cleared_count = self.shared.allowlist.lock().expect("allowlist").clear();
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        self.emit(UiEvent::ancs_rule_changed(
            "cleared",
            None,
            None,
            total_rules,
        ));
        AncsClearRulesReport {
            ok: true,
            cleared_count,
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Lightweight health report for liveness probes, monitoring, and diagnostics.
    pub fn health_report(&self) -> HealthReport {
        let ports = NodePorts {
            control: self.control_port(),
            file: self.file_port(),
            input: self.input_port(),
            screen: self.screen_port(),
            ui: self.ui_port(),
        };
        HealthReport {
            ok: true,
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            device_id: self.shared.identity.device_id().to_string(),
            name: self.name(),
            platform: self.platform().to_string(),
            pid: std::process::id(),
            uptime_secs: self.shared.started_at.elapsed().as_secs(),
            ports,
            live_peers: self.live_peers().len(),
            trusted_peers: self.trusted().len(),
        }
    }

    /// Subscribe to the shutdown signal.
    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shared.shutdown.subscribe()
    }

    /// Inspect local storage, inbox files, in-flight/orphaned partial transfers and metadata.
    pub fn storage_report(&self) -> StorageReport {
        let data_dir = self.shared.cfg.data_dir.clone();
        let inbox_dir = data_dir.join("inbox");
        let mut inbox_total_bytes: u64 = 0;
        let mut inbox_file_count: usize = 0;
        let mut partial_bytes: u64 = 0;
        let mut partial_count: usize = 0;
        let mut orphaned_partial_count: usize = 0;
        let mut orphaned_partial_bytes: u64 = 0;
        let mut meta_bytes: u64 = 0;

        {
            let active = self.shared.active_transfers.lock().expect("transfers");
            let hubs = self.shared.hubs.lock().expect("hubs");

            if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let file_len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.starts_with('.') && name.ends_with(".part") {
                        let transfer_id = name
                            .strip_prefix('.')
                            .and_then(|s| s.strip_suffix(".part"))
                            .unwrap_or("");
                        partial_count += 1;
                        partial_bytes += file_len;

                        let is_active = active.get(transfer_id).is_some_and(|t| {
                            t.phase == TransferPhase::Transferring
                                || t.phase == TransferPhase::Verifying
                        });
                        let is_resumable = hubs.resume.get(transfer_id).is_some();
                        if !is_active && !is_resumable {
                            orphaned_partial_count += 1;
                            orphaned_partial_bytes += file_len;
                        }
                    } else if name.starts_with('.') && name.ends_with(".resume.json") {
                        meta_bytes += file_len;
                    } else if !name.starts_with('.') {
                        inbox_file_count += 1;
                        inbox_total_bytes += file_len;
                    }
                }
            }
        }

        for meta_file in [
            "identity.bin",
            "identity.ref",
            "trusted.json",
            "routes.json",
        ] {
            let p = data_dir.join(meta_file);
            if let Ok(m) = std::fs::metadata(&p) {
                meta_bytes += m.len();
            }
        }

        let markers_rep = self.resume_markers_report();

        StorageReport {
            ok: true,
            data_dir: data_dir.to_string_lossy().to_string(),
            inbox_dir: inbox_dir.to_string_lossy().to_string(),
            inbox_total_bytes,
            inbox_file_count,
            partial_bytes,
            partial_count,
            orphaned_partial_count,
            orphaned_partial_bytes,
            meta_bytes,
            resume_markers_count: markers_rep.count,
            resume_markers: markers_rep.markers,
        }
    }

    /// Inspect all active resume markers from memory (ResumeTable) and disk (.resume.json).
    pub fn resume_markers_report(&self) -> ResumeMarkersReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let states = self.shared.hubs.lock().expect("hubs").resume.all();
        let mut markers = Vec::new();
        let mut seen_tids = std::collections::HashSet::new();
        let mut total_partial_bytes: u64 = 0;

        for st in states {
            seen_tids.insert(st.transfer_id.clone());
            let part_path = inbox_dir.join(format!(".{}.part", st.transfer_id));
            let fallback_path = inbox_dir.join(format!("{}.part", st.transfer_id));
            let part_meta = std::fs::metadata(&part_path)
                .or_else(|_| std::fs::metadata(&fallback_path))
                .ok();
            let part_file_exists = part_meta.is_some();
            let part_file_bytes = part_meta.map(|m| m.len()).unwrap_or(0);
            total_partial_bytes += part_file_bytes;

            let progress_percent = if st.total > 0 {
                (st.offset as f64 / st.total as f64) * 100.0
            } else {
                0.0
            };
            let chunk_aligned = st.offset % tetherly_core::FILE_CHUNK as u64 == 0;
            let source_alias = self.peer_alias(&st.source);

            markers.push(ResumeMarkerDto {
                transfer_id: st.transfer_id,
                source: st.source.to_string(),
                source_alias,
                total_bytes: st.total,
                offset_bytes: st.offset,
                progress_percent,
                chunk_aligned,
                part_file_exists,
                part_file_bytes,
            });
        }

        // Also discover any on-disk resume sidecars (.resume.json) not currently cached in memory
        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let stem = if name.starts_with('.') && name.ends_with(".resume.json") {
                    name.strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".resume.json"))
                        .map(|s| s.to_string())
                } else if name.ends_with(".resume.json") {
                    name.strip_suffix(".resume.json").map(|s| s.to_string())
                } else {
                    None
                };

                let Some(transfer_id) = stem else { continue };
                if seen_tids.contains(&transfer_id) {
                    continue;
                }

                if let Ok(bytes) = std::fs::read(entry.path()) {
                    if let Ok(dto) = serde_json::from_slice::<ResumeSidecar>(&bytes) {
                        seen_tids.insert(transfer_id.clone());
                        let part_path = inbox_dir.join(format!(".{}.part", transfer_id));
                        let fallback_path = inbox_dir.join(format!("{}.part", transfer_id));
                        let part_meta = std::fs::metadata(&part_path)
                            .or_else(|_| std::fs::metadata(&fallback_path))
                            .ok();
                        let part_file_exists = part_meta.is_some();
                        let part_file_bytes = part_meta.map(|m| m.len()).unwrap_or(0);
                        total_partial_bytes += part_file_bytes;

                        let progress_percent = if dto.total > 0 {
                            (dto.offset as f64 / dto.total as f64) * 100.0
                        } else {
                            0.0
                        };
                        let chunk_aligned = dto.offset % tetherly_core::FILE_CHUNK as u64 == 0;
                        let source_alias = DeviceId::parse(&dto.device_id)
                            .ok()
                            .and_then(|dev| self.peer_alias(&dev));

                        markers.push(ResumeMarkerDto {
                            transfer_id,
                            source: dto.device_id,
                            source_alias,
                            total_bytes: dto.total,
                            offset_bytes: dto.offset,
                            progress_percent,
                            chunk_aligned,
                            part_file_exists,
                            part_file_bytes,
                        });
                    }
                }
            }
        }

        markers.sort_by(|a, b| a.transfer_id.cmp(&b.transfer_id));

        ResumeMarkersReport {
            ok: true,
            count: markers.len(),
            total_partial_bytes,
            markers,
        }
    }

    /// Garbage-collect incomplete or orphaned partial transfers in the inbox.
    pub fn clean_storage(&self, orphaned_only: bool) -> StorageCleanReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut freed_bytes: u64 = 0;
        let mut deleted_files_count: usize = 0;
        let mut cleared_resume_entries: usize = 0;

        let active = self.shared.active_transfers.lock().expect("transfers");
        let mut hubs = self.shared.hubs.lock().expect("hubs");

        let active_ids: std::collections::HashSet<String> = active
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying
            })
            .map(|(id, _)| id.clone())
            .collect();

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let file_len = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                if name.starts_with('.') && name.ends_with(".part") {
                    let transfer_id = name
                        .strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".part"))
                        .unwrap_or("");
                    if active_ids.contains(transfer_id) {
                        continue;
                    }
                    let is_resumable = hubs.resume.get(transfer_id).is_some();
                    if orphaned_only {
                        if !is_resumable && std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                            let sidecar = inbox_dir.join(format!(".{transfer_id}.resume.json"));
                            if sidecar.exists() {
                                if let Ok(sm) = std::fs::metadata(&sidecar) {
                                    freed_bytes += sm.len();
                                }
                                if std::fs::remove_file(&sidecar).is_ok() {
                                    deleted_files_count += 1;
                                }
                            }
                        }
                    } else {
                        if std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                        }
                        if hubs.resume.get(transfer_id).is_some() {
                            hubs.resume.remove(transfer_id);
                            cleared_resume_entries += 1;
                        }
                        let sidecar = inbox_dir.join(format!(".{transfer_id}.resume.json"));
                        if sidecar.exists() {
                            if let Ok(sm) = std::fs::metadata(&sidecar) {
                                freed_bytes += sm.len();
                            }
                            if std::fs::remove_file(&sidecar).is_ok() {
                                deleted_files_count += 1;
                            }
                        }
                    }
                } else if name.starts_with('.') && name.ends_with(".resume.json") {
                    let transfer_id = name
                        .strip_prefix('.')
                        .and_then(|s| s.strip_suffix(".resume.json"))
                        .unwrap_or("");
                    if active_ids.contains(transfer_id) {
                        continue;
                    }
                    let matching_part = inbox_dir.join(format!(".{transfer_id}.part"));
                    if !matching_part.exists() {
                        if hubs.resume.get(transfer_id).is_some() {
                            hubs.resume.remove(transfer_id);
                            cleared_resume_entries += 1;
                        }
                        if std::fs::remove_file(&path).is_ok() {
                            freed_bytes += file_len;
                            deleted_files_count += 1;
                        }
                    }
                }
            }
        }

        StorageCleanReport {
            ok: true,
            freed_bytes,
            deleted_files_count,
            cleared_resume_entries,
        }
    }

    /// Inspect files in the incoming inbox, optionally filtering by search substring or limiting output count.
    pub fn inbox_report(&self, search: Option<&str>, limit: Option<usize>) -> InboxReport {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n.to_string(),
                    _ => continue,
                };

                let (file_len, modified_at_secs) = entry
                    .metadata()
                    .map(|m| {
                        let len = m.len();
                        let mod_time = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        (len, mod_time)
                    })
                    .unwrap_or((0, 0));

                if let Some(q) = search {
                    if !q.is_empty() && !name.to_lowercase().contains(&q.to_lowercase()) {
                        continue;
                    }
                }

                let is_safe = tetherly_core::sanitize_file_name(&name).is_ok();
                let extension = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_string());

                files.push(InboxFileDto {
                    name,
                    size: file_len,
                    modified_at_secs,
                    is_safe,
                    extension,
                });
            }
        }

        // Sort descending by modified_at_secs, then ascending by name
        files.sort_by(|a, b| {
            b.modified_at_secs
                .cmp(&a.modified_at_secs)
                .then_with(|| a.name.cmp(&b.name))
        });

        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        let count = files.len();

        if let Some(lim) = limit {
            if files.len() > lim {
                files.truncate(lim);
            }
        }

        InboxReport {
            ok: true,
            count,
            total_bytes,
            files,
        }
    }

    /// Safely delete a file from the incoming inbox, guarding against path traversal attacks.
    pub fn delete_inbox_file(&self, name: &str) -> Result<InboxDeleteReport, CoreError> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name.contains('\0')
            || name.starts_with('.')
        {
            return Err(CoreError::UnsafeFileName);
        }

        let clean_name = tetherly_core::sanitize_file_name(name)?;
        if clean_name != name || clean_name.starts_with('.') {
            return Err(CoreError::UnsafeFileName);
        }

        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let file_path = inbox_dir.join(&clean_name);

        if !file_path.exists() || !file_path.is_file() {
            return Err(CoreError::File("file not found in inbox".into()));
        }

        let freed_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

        std::fs::remove_file(&file_path)?;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::inbox_file_deleted(
            &clean_name,
            freed_bytes,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "inbox_file_deleted",
            Some(clean_name.clone()),
            format!("inbox file '{clean_name}' deleted ({freed_bytes} bytes freed)"),
            true,
        );

        Ok(InboxDeleteReport {
            ok: true,
            name: clean_name,
            freed_bytes,
        })
    }

    /// Clear completed received files from the incoming inbox, optionally filtering by modified cutoff time.
    pub fn clear_inbox(&self, before_secs: Option<u64>) -> Result<InboxClearReport, CoreError> {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let mut deleted_count: usize = 0;
        let mut freed_bytes: u64 = 0;

        if let Ok(entries) = std::fs::read_dir(&inbox_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) if !n.starts_with('.') => n,
                    _ => continue,
                };
                let _ = name;

                let (file_len, mod_time) = entry
                    .metadata()
                    .map(|m| {
                        let len = m.len();
                        let t = m
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        (len, t)
                    })
                    .unwrap_or((0, 0));

                if let Some(cutoff) = before_secs {
                    if mod_time >= cutoff {
                        continue;
                    }
                }

                if std::fs::remove_file(&path).is_ok() {
                    deleted_count += 1;
                    freed_bytes += file_len;
                }
            }
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self
            .shared
            .events
            .send(UiEvent::inbox_cleared(deleted_count, freed_bytes, now_ms));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "inbox_cleared",
            None::<String>,
            format!("inbox cleared: {deleted_count} files removed ({freed_bytes} bytes freed)"),
            true,
        );

        Ok(InboxClearReport {
            ok: true,
            deleted_count,
            freed_bytes,
        })
    }

    /// Open a specific file in the inbox with the system default application,
    /// or open the inbox directory itself if no name is specified.
    pub fn open_inbox_file(
        &self,
        name: Option<&str>,
        opener: &dyn tetherly_core::Opener,
    ) -> Result<InboxOpenReport, CoreError> {
        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        match name.filter(|s| !s.trim().is_empty()) {
            Some(n) => {
                if n.is_empty()
                    || n.contains('/')
                    || n.contains('\\')
                    || n.contains("..")
                    || n.contains('\0')
                    || n.starts_with('.')
                {
                    return Err(CoreError::UnsafeFileName);
                }

                let clean_name = tetherly_core::sanitize_file_name(n)?;
                if clean_name != n || clean_name.starts_with('.') {
                    return Err(CoreError::UnsafeFileName);
                }

                let file_path = inbox_dir.join(&clean_name);
                if !file_path.exists() || !file_path.is_file() {
                    return Err(CoreError::File("file not found in inbox".into()));
                }

                let path_str = file_path.to_str().ok_or(CoreError::UnsafeFileName)?;
                opener.open(path_str)?;

                let _ = self
                    .shared
                    .events
                    .send(UiEvent::inbox_file_opened(Some(&clean_name), now_ms));

                self.record_audit(
                    AuditCategory::File,
                    AuditSeverity::Info,
                    "inbox_file_opened",
                    Some(clean_name.clone()),
                    format!("inbox file '{clean_name}' opened in system default application"),
                    true,
                );

                Ok(InboxOpenReport {
                    ok: true,
                    name: Some(clean_name),
                    path: file_path.display().to_string(),
                    message: "file opened in system default handler".into(),
                })
            }
            None => {
                std::fs::create_dir_all(&inbox_dir)?;
                let path_str = inbox_dir.to_str().ok_or(CoreError::UnsafeFileName)?;
                opener.open(path_str)?;

                let _ = self
                    .shared
                    .events
                    .send(UiEvent::inbox_file_opened(None::<String>, now_ms));

                self.record_audit(
                    AuditCategory::File,
                    AuditSeverity::Info,
                    "inbox_directory_opened",
                    None::<String>,
                    "inbox directory opened in system file manager",
                    true,
                );

                Ok(InboxOpenReport {
                    ok: true,
                    name: None,
                    path: inbox_dir.display().to_string(),
                    message: "inbox directory opened in system file manager".into(),
                })
            }
        }
    }

    /// Read the bytes and MIME type of a completed file from the inbox,
    /// enforcing strict path traversal guards and FILE_MAX limits.
    pub fn read_inbox_file(
        &self,
        name: &str,
    ) -> Result<(String, &'static str, Vec<u8>), CoreError> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || name.contains('\0')
            || name.starts_with('.')
        {
            return Err(CoreError::UnsafeFileName);
        }

        let clean_name = tetherly_core::sanitize_file_name(name)?;
        if clean_name != name || clean_name.starts_with('.') {
            return Err(CoreError::UnsafeFileName);
        }

        let inbox_dir = self.shared.cfg.data_dir.join("inbox");
        let file_path = inbox_dir.join(&clean_name);

        if !file_path.exists() || !file_path.is_file() {
            return Err(CoreError::File("file not found in inbox".into()));
        }

        let meta = std::fs::metadata(&file_path)?;
        if meta.len() > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(meta.len()));
        }

        let bytes = std::fs::read(&file_path)?;
        let mime = mime_type_from_name(&clean_name);
        Ok((clean_name, mime, bytes))
    }

    /// Bring the ANCS link up: Data Source first, then Notification Source.
    pub fn ancs_connect(&self) -> Result<Option<ConnectOutcome>, CoreError> {
        let mut guard = self.shared.ancs.lock().expect("ancs");
        match guard.as_mut() {
            Some(ing) => ing.connect().map(Some),
            None => Ok(None),
        }
    }

    /// Phone disconnected: schedule the backoff reconnect.
    pub fn ancs_disconnected(&self) -> Option<u64> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_mut()
            .map(|i| i.on_disconnected())
    }

    /// Notification Source value-changed. Never writes the Control Point.
    pub async fn ancs_notification_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_notification_source(value)).await
    }

    /// Data Source value-changed (fragments included).
    pub async fn ancs_data_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_data_source(value)).await
    }

    /// Drive the serial Control Point queue. Call from the BLE task.
    pub async fn ancs_tick(&self) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.tick()).await
    }

    async fn ancs_feed<F>(&self, f: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut AncsIngress) -> Result<(), CoreError>,
    {
        let events = {
            let mut guard = self.shared.ancs.lock().expect("ancs");
            let Some(ing) = guard.as_mut() else {
                return Ok(());
            };
            f(ing)?;
            ing.drain_events()
        };
        for ev in events {
            self.apply_ancs_event(ev).await;
        }
        Ok(())
    }

    async fn apply_ancs_event(&self, ev: AncsEvent) {
        let source = self.shared.ancs_source.clone();
        match ev {
            AncsEvent::Added(n) | AncsEvent::Modified(n) => {
                let actions = {
                    let list = self.shared.allowlist.lock().expect("allowlist");
                    ancs_actions(&n.app_id, &list)
                };
                let push = NotifyPush {
                    uid: n.uid.to_string(),
                    app_id: n.app_id,
                    app_name: n.app_name,
                    title: n.title,
                    body: n.body,
                    ts: self.shared.clock.unix_ms(),
                    actions,
                };
                let _ = self.ingest_local_push(source, push).await;
            }
            AncsEvent::Removed { uid } => {
                let ids = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.dismiss_uid(&source, &uid.to_string())
                };
                for id in ids {
                    self.emit(UiEvent::CandidateDismissed {
                        id,
                        source: source.to_string(),
                        app_id: String::new(),
                        uid: uid.to_string(),
                    });
                }
            }
        }
    }

    /// Own the ANCS link end to end: drive the ingress from the stack's inbound
    /// channel and tick it so the serial Control Point queue drains.
    ///
    /// Runs on its own thread with a current-thread runtime. The transport's
    /// subscribe/write calls block until the BLE stack answers (bounded by the
    /// stack's own ack timeout), and an ANCS link is far too low-rate to justify
    /// letting that land on the node's async workers.
    pub fn spawn_ancs_pump(&self, inbound: tokio::sync::mpsc::UnboundedReceiver<AncsInbound>) {
        let node = self.clone_handle();
        let spawned = std::thread::Builder::new()
            .name("tetherly-ancs-pump".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        warn!(%e, "ancs: pump runtime unavailable");
                        return;
                    }
                };
                runtime.block_on(node.ancs_pump(inbound));
            });
        if let Err(e) = spawned {
            warn!(%e, "ancs: pump thread unavailable");
        }
    }

    async fn ancs_pump(&self, mut inbound: tokio::sync::mpsc::UnboundedReceiver<AncsInbound>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = inbound.recv() => match maybe {
                    None => break,
                    // The session is up: subscribe Data Source then Notification
                    // Source (the ingress fixes that order; §9.4).
                    Some(AncsInbound::Ready) => {
                        if let Err(e) = self.ancs_connect() {
                            warn!(%e, "ancs: subscribing to the phone failed");
                        }
                    }
                    Some(AncsInbound::NotificationSource(value)) => {
                        if let Err(e) = self.ancs_notification_source(&value).await {
                            warn!(%e, "ancs: notification source rejected");
                        }
                    }
                    Some(AncsInbound::DataSource(value)) => {
                        if let Err(e) = self.ancs_data_source(&value).await {
                            warn!(%e, "ancs: data source rejected");
                        }
                    }
                    Some(AncsInbound::Disconnected) => {
                        if let Some(retry_at) = self.ancs_disconnected() {
                            info!(retry_at, "ancs: phone gone; backing off");
                        }
                    }
                },
                _ = tick.tick() => {
                    if let Err(e) = self.ancs_tick().await {
                        warn!(%e, "ancs: control point tick failed");
                    }
                }
            }
        }
    }

    pub async fn send_to(&self, peer: &DeviceId, frame: InnerFrame) -> Result<(), CoreError> {
        let tx = {
            let map = self.shared.sessions.lock().await;
            map.get(peer).cloned()
        };
        match tx {
            Some(tx) => match tx.try_send(frame) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("peer busy; send queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("peer gone".into()))
                }
            },
            None => Err(CoreError::Json("not connected".into())),
        }
    }

    /// Actively send a ping frame to a connected peer over the Noise session,
    /// await its pong reply, and measure the round-trip latency (RTT) in milliseconds.
    pub async fn ping_peer(&self, peer_id: &DeviceId) -> Result<PeerPingReport, CoreError> {
        let (path, addr) = {
            let live = self.shared.live.lock().expect("live");
            let peer = live.get(peer_id).ok_or(CoreError::NoConnectedPeer)?;
            let path_str = match peer.path {
                PathKind::Lan => "lan",
                PathKind::Overlay => "overlay",
            };
            (path_str.to_string(), peer.addr.to_string())
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut pw = self.shared.ping_waiters.lock().expect("ping_waiters");
            pw.entry(peer_id.clone()).or_default().push(tx);
        }

        let now_ms = self.shared.clock.unix_ms();
        {
            let mut trackers = self.shared.link_trackers.lock().expect("link_trackers");
            trackers
                .entry(peer_id.clone())
                .or_insert_with(|| PeerLinkTracker::new(now_ms))
                .record_ping_sent(now_ms);
        }
        if let Err(e) = self.send_to(peer_id, ping(now_ms)).await {
            let mut pw = self.shared.ping_waiters.lock().expect("ping_waiters");
            if let Some(list) = pw.get_mut(peer_id) {
                list.retain(|s| !s.is_closed());
            }
            return Err(e);
        }

        match tokio::time::timeout(std::time::Duration::from_secs(5), rx).await {
            Ok(Ok(rtt_ms)) => Ok(PeerPingReport {
                ok: true,
                device_id: peer_id.to_string(),
                rtt_ms,
                path,
                addr,
            }),
            Ok(Err(_)) => Err(CoreError::NoConnectedPeer),
            Err(_) => {
                let mut pw = self.shared.ping_waiters.lock().expect("ping_waiters");
                if let Some(list) = pw.get_mut(peer_id) {
                    list.retain(|s| !s.is_closed());
                }
                Err(CoreError::PingTimeout)
            }
        }
    }

    /// Read the current heartbeat keepalive configuration.
    pub fn heartbeat_config(&self) -> HeartbeatConfig {
        self.shared
            .heartbeat_config
            .lock()
            .expect("heartbeat_config")
            .clone()
    }

    /// Dynamically update the heartbeat keepalive interval and deadline.
    pub fn update_heartbeat_config(
        &self,
        interval_secs: Option<u64>,
        deadline_secs: Option<u64>,
    ) -> Result<HeartbeatConfig, CoreError> {
        let mut cfg = self
            .shared
            .heartbeat_config
            .lock()
            .expect("heartbeat_config");
        if let Some(i) = interval_secs {
            if !(1..=3600).contains(&i) {
                return Err(CoreError::Json(
                    "heartbeat interval must be between 1 and 3600 seconds".into(),
                ));
            }
            cfg.interval_secs = i;
        }
        if let Some(d) = deadline_secs {
            if d <= cfg.interval_secs || d > 86400 {
                return Err(CoreError::Json(
                    "deadline must be strictly greater than heartbeat interval and <= 86400 seconds".into(),
                ));
            }
            cfg.deadline_secs = d;
        } else if cfg.deadline_secs <= cfg.interval_secs {
            cfg.deadline_secs = cfg.interval_secs * 3;
        }

        let updated = cfg.clone();
        drop(cfg);

        self.emit(UiEvent::HeartbeatConfigChanged {
            interval_secs: updated.interval_secs,
            deadline_secs: updated.deadline_secs,
            timestamp_ms: self.shared.clock.unix_ms(),
        });

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "heartbeat_config_updated",
            None::<String>,
            format!(
                "heartbeat config updated: interval={}s, deadline={}s",
                updated.interval_secs, updated.deadline_secs
            ),
            true,
        );

        info!(
            interval = updated.interval_secs,
            deadline = updated.deadline_secs,
            "updated heartbeat configuration"
        );
        Ok(updated)
    }

    /// Generate comprehensive link quality and heartbeat status report for all peers.
    pub fn link_quality_report(&self) -> LinkQualityReport {
        let heartbeat = self.heartbeat_config();
        let now_ms = self.shared.clock.unix_ms();

        let live_map = self.shared.live.lock().expect("live");
        let trackers_map = self.shared.link_trackers.lock().expect("link_trackers");
        let trusted_list = self.trusted();

        let mut links = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 1. Process active connected peers
        for (peer_id, live) in live_map.iter() {
            seen.insert(peer_id.clone());
            let (tracker, connected_uptime) = if let Some(t) = trackers_map.get(peer_id) {
                let uptime = now_ms.saturating_sub(t.connected_at_ms) / 1000;
                (t.clone(), uptime)
            } else {
                (PeerLinkTracker::new(now_ms), 0)
            };

            links.push(PeerLinkQualityDto {
                device_id: peer_id.to_string(),
                name: live.name.clone(),
                addr: live.addr.to_string(),
                is_online: true,
                last_seen_ms: live.last_seen_ms,
                rtt_ms: live.rtt_ms,
                min_rtt_ms: tracker.min_rtt(),
                max_rtt_ms: tracker.max_rtt(),
                avg_rtt_ms: tracker.avg_rtt(),
                jitter_ms: tracker.jitter_ms(),
                pings_sent: tracker.pings_sent,
                pongs_recv: tracker.pongs_recv,
                packet_loss_rate: tracker.packet_loss_rate(),
                grade: tracker.evaluate_grade(true),
                samples_count: tracker.recent_rtts.len(),
                connected_uptime_secs: connected_uptime,
            });
        }

        // 2. Retain historical quality for trusted peers currently offline
        for trusted in &trusted_list {
            if !seen.contains(&trusted.device_id) {
                if let Some(t) = trackers_map.get(&trusted.device_id) {
                    seen.insert(trusted.device_id.clone());
                    links.push(PeerLinkQualityDto {
                        device_id: trusted.device_id.to_string(),
                        name: trusted.alias.clone(),
                        addr: "disconnected".to_string(),
                        is_online: false,
                        last_seen_ms: t.last_ping_sent_ms.unwrap_or(0),
                        rtt_ms: None,
                        min_rtt_ms: t.min_rtt(),
                        max_rtt_ms: t.max_rtt(),
                        avg_rtt_ms: t.avg_rtt(),
                        jitter_ms: t.jitter_ms(),
                        pings_sent: t.pings_sent,
                        pongs_recv: t.pongs_recv,
                        packet_loss_rate: 100.0,
                        grade: LinkGrade::Unreachable,
                        samples_count: t.recent_rtts.len(),
                        connected_uptime_secs: 0,
                    });
                }
            }
        }

        links.sort_by(|a, b| {
            b.is_online
                .cmp(&a.is_online)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.device_id.cmp(&b.device_id))
        });

        let total_peers = links.len();
        let online_peers = links.iter().filter(|l| l.is_online).count();

        LinkQualityReport {
            ok: true,
            heartbeat,
            total_peers,
            online_peers,
            links,
        }
    }

    /// Perform an on-demand multi-packet ping burst to accurately measure link quality.
    pub async fn probe_peer_link(
        &self,
        peer_target: &str,
        count: usize,
        interval_ms: u64,
    ) -> Result<LinkProbeReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let count = count.clamp(1, 20);
        let interval_ms = interval_ms.clamp(10, 1000);

        // Verify peer is actually online before probing
        {
            let live = self.shared.live.lock().expect("live");
            if !live.contains_key(&peer_id) {
                return Err(CoreError::NoConnectedPeer);
            }
        }

        let mut samples = Vec::new();
        let mut sent = 0;
        let mut received = 0;

        for _ in 0..count {
            sent += 1;
            if let Ok(rep) = self.ping_peer(&peer_id).await {
                received += 1;
                samples.push(rep.rtt_ms);
            }
            if sent < count {
                tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
            }
        }

        let min_rtt_ms = samples.iter().copied().min();
        let max_rtt_ms = samples.iter().copied().max();
        let avg_rtt_ms = if samples.is_empty() {
            None
        } else {
            let sum: u64 = samples.iter().sum();
            Some((sum as f64 / samples.len() as f64 * 100.0).round() / 100.0)
        };
        let jitter_ms = if samples.len() >= 2 {
            let diff_sum: f64 = samples
                .iter()
                .zip(samples.iter().skip(1))
                .map(|(a, b)| (*b as f64 - *a as f64).abs())
                .sum();
            Some(((diff_sum / (samples.len() - 1) as f64) * 100.0).round() / 100.0)
        } else {
            None
        };

        let packet_loss_rate = if sent == 0 {
            0.0
        } else {
            let lost = sent.saturating_sub(received);
            let rate = (lost as f64 / sent as f64) * 100.0;
            (rate * 10.0).round() / 10.0
        };

        let grade = match avg_rtt_ms {
            Some(avg) if avg < 25.0 && packet_loss_rate < 1.0 => LinkGrade::Excellent,
            Some(avg) if avg < 100.0 && packet_loss_rate < 5.0 => LinkGrade::Good,
            Some(avg) if avg < 300.0 && packet_loss_rate < 15.0 => LinkGrade::Fair,
            Some(_) => LinkGrade::Poor,
            None if sent > 0 && received == 0 => LinkGrade::Unreachable,
            None => LinkGrade::Poor,
        };

        Ok(LinkProbeReport {
            ok: true,
            peer: peer_id.to_string(),
            count,
            sent,
            received,
            min_rtt_ms,
            max_rtt_ms,
            avg_rtt_ms,
            jitter_ms,
            packet_loss_rate,
            samples,
            grade,
        })
    }

    /// Record a structured, zero-plaintext audit entry into the circular buffer and emit over SSE.
    pub fn record_audit(
        &self,
        category: AuditCategory,
        severity: AuditSeverity,
        action: impl Into<String>,
        target: Option<impl Into<String>>,
        summary: impl Into<String>,
        success: bool,
    ) -> AuditEntry {
        let now_ms = self.shared.clock.unix_ms();
        let target_str = target.map(|t| t.into());
        let entry = {
            let mut engine = self.shared.audit_engine.lock().expect("audit_engine lock");
            engine.push(
                now_ms, category, severity, action, target_str, summary, success,
            )
        };
        // Broadcast over SSE event bus (Zero-Plaintext)
        self.emit(UiEvent::audit_event_recorded(
            entry.id,
            entry.category.to_string(),
            entry.severity.to_string(),
            &entry.action,
            entry.target.as_deref(),
            &entry.summary,
            entry.success,
            entry.timestamp_ms,
        ));
        entry
    }

    /// Query and filter security audit log entries.
    pub fn audit_report(
        &self,
        category: Option<AuditCategory>,
        severity: Option<AuditSeverity>,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> AuditLogReport {
        let engine = self.shared.audit_engine.lock().expect("audit_engine lock");
        let capacity = engine.capacity();
        let (total, entries) = engine.query(category, severity, limit, since_ms);
        let returned = entries.len();
        AuditLogReport {
            ok: true,
            total,
            returned,
            capacity,
            entries,
        }
    }

    /// Purge audit log entries and log a security audit event for the clear operation.
    pub fn clear_audit_log(&self) -> AuditClearReport {
        let count = {
            let mut engine = self.shared.audit_engine.lock().expect("audit_engine lock");
            engine.clear()
        };
        // Record a Notice audit event for the clear action itself
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audit_log_cleared",
            None::<String>,
            format!("security audit log cleared ({count} entries purged)"),
            true,
        );
        AuditClearReport {
            ok: true,
            cleared_count: count,
            message: format!("successfully cleared {count} audit log entries"),
        }
    }

    /// Send a direct secure text or note message to a connected peer.
    pub async fn send_message_to_peer(
        &self,
        peer_target: &str,
        text: &str,
    ) -> Result<PeerMessageSendReport, CoreError> {
        let peer_id = if peer_target.trim().is_empty() {
            let live = self.live_peers();
            match live.len() {
                0 => return Err(CoreError::PeerNotFound("no connected peer".to_string())),
                1 => live[0].device_id.clone(),
                _ => {
                    return Err(CoreError::PeerNotFound(
                        "multiple peers connected; specify target peer".to_string(),
                    ))
                }
            }
        } else {
            self.resolve_peer(peer_target)?
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Json("message text cannot be empty".to_string()));
        }
        if trimmed.len() > tetherly_core::frame::TEXT_MESSAGE_MAX {
            return Err(CoreError::PayloadTooLarge);
        }

        static MSG_OUT_COUNTER: AtomicU64 = AtomicU64::new(1);
        let now_ms = self.shared.clock.unix_ms();
        let msg_id = format!(
            "msg_{}_{}",
            now_ms,
            MSG_OUT_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let wire_msg = TextMessage::new(&msg_id, trimmed, now_ms)?;
        let frame = wire_msg.to_frame()?;
        self.send_to(&peer_id, frame).await?;

        let peer_str = peer_id.to_string();
        let msg = PeerMessage {
            id: msg_id.clone(),
            peer: peer_str.clone(),
            direction: MessageDirection::Outbound,
            text: trimmed.to_string(),
            timestamp_ms: now_ms,
            read: true,
        };

        {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.push(msg.clone());
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_sent",
            Some(peer_str.clone()),
            format!("Sent direct text message (length: {} bytes)", trimmed.len()),
            true,
        );

        self.emit(UiEvent::MessageSent {
            id: msg_id,
            peer: peer_str,
            len: trimmed.len(),
            timestamp_ms: now_ms,
        });

        Ok(PeerMessageSendReport {
            ok: true,
            message: msg,
        })
    }

    /// Send a direct secure text or note message to multiple targets specified by a TargetSelector.
    /// Fans out message delivery to each online peer, records Outbound messages in MessageEngine,
    /// logs audit trails and emits UiEvent::MultiDispatchCompleted without leaking plaintext.
    pub async fn send_message_multi(
        &self,
        selector: &TargetSelector,
        text: &str,
    ) -> Result<MultiMessageSendReport, CoreError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Json("message text cannot be empty".to_string()));
        }
        if trimmed.len() > tetherly_core::frame::TEXT_MESSAGE_MAX {
            return Err(CoreError::PayloadTooLarge);
        }

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        static MSG_OUT_COUNTER: AtomicU64 = AtomicU64::new(1);
        let now_ms = self.shared.clock.unix_ms();

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerMessageSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    message_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let msg_id = format!(
                "msg_{}_{}",
                now_ms,
                MSG_OUT_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let wire_msg = TextMessage::new(&msg_id, trimmed, now_ms)?;
            let frame = wire_msg.to_frame()?;

            match self.send_to(peer_id, frame).await {
                Ok(_) => {
                    let msg = PeerMessage {
                        id: msg_id.clone(),
                        peer: peer_str.clone(),
                        direction: MessageDirection::Outbound,
                        text: trimmed.to_string(),
                        timestamp_ms: now_ms,
                        read: true,
                    };
                    {
                        let mut engine = self
                            .shared
                            .message_engine
                            .lock()
                            .expect("message_engine lock");
                        engine.push(msg);
                    }
                    self.emit(UiEvent::MessageSent {
                        id: msg_id.clone(),
                        peer: peer_str.clone(),
                        len: trimmed.len(),
                        timestamp_ms: now_ms,
                    });
                    success_count += 1;
                    dispatches.push(PeerMessageSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        message_id: Some(msg_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerMessageSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        message_id: Some(msg_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_multi_sent",
            None::<String>,
            format!(
                "Multicast message (length: {} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                trimmed.len(),
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "message",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiMessageSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest a direct message received over the Noise connection from a peer.
    pub fn receive_message(&self, peer_str: &str, wire_msg: TextMessage) {
        let now_ms = self.shared.clock.unix_ms();
        let len = wire_msg.text.len();
        let msg = PeerMessage {
            id: wire_msg.id.clone(),
            peer: peer_str.to_string(),
            direction: MessageDirection::Inbound,
            text: wire_msg.text,
            timestamp_ms: if wire_msg.timestamp_ms > 0 {
                wire_msg.timestamp_ms
            } else {
                now_ms
            },
            read: false,
        };

        {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.push(msg);
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_received",
            Some(peer_str.to_string()),
            format!("Received direct text message (length: {} bytes)", len),
            true,
        );

        self.emit(UiEvent::MessageReceived {
            id: wire_msg.id,
            peer: peer_str.to_string(),
            len,
            timestamp_ms: now_ms,
        });
    }

    /// Query and filter direct message history, with optional unread-only filtering and mark_read flag.
    pub fn message_history(
        &self,
        peer_target: Option<&str>,
        limit: Option<usize>,
        unread_only: bool,
        mark_read: bool,
    ) -> Result<PeerMessageHistoryReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let mut engine = self
            .shared
            .message_engine
            .lock()
            .expect("message_engine lock");
        let (total, unread_before, mut messages) =
            engine.query(resolved_peer.as_deref(), limit, unread_only);

        let unread = if mark_read {
            engine.mark_read(resolved_peer.as_deref());
            for m in messages.iter_mut() {
                if resolved_peer.is_none() || resolved_peer.as_deref() == Some(&m.peer) {
                    m.read = true;
                }
            }
            engine.messages.iter().filter(|m| !m.read).count()
        } else {
            unread_before
        };

        Ok(PeerMessageHistoryReport {
            ok: true,
            total,
            returned: messages.len(),
            unread,
            messages,
        })
    }

    /// Clear direct messages, optionally filtered by peer.
    pub fn clear_messages(
        &self,
        peer_target: Option<&str>,
    ) -> Result<PeerMessageClearReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let cleared_count = {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.clear(resolved_peer.as_deref())
        };

        let target_label = resolved_peer.clone().unwrap_or_else(|| "all".to_string());
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Notice,
            "message_cleared",
            resolved_peer,
            format!("Cleared {cleared_count} direct messages for target: {target_label}"),
            true,
        );

        Ok(PeerMessageClearReport {
            ok: true,
            target: target_label,
            cleared_count,
            message: format!("successfully cleared {cleared_count} messages"),
        })
    }

    /// Query and filter outbox transmission history, with optional peer, status and limit filtering.
    pub fn outbox_history(
        &self,
        peer_target: Option<&str>,
        status: Option<OutboxStatus>,
        limit: Option<usize>,
    ) -> Result<OutboxReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let engine = self
            .shared
            .outbox_engine
            .lock()
            .expect("outbox_engine lock");
        let (total, entries) = engine.query(resolved_peer.as_deref(), status, limit);

        Ok(OutboxReport {
            ok: true,
            total,
            returned: entries.len(),
            entries,
        })
    }

    /// Clear outbox transmission records, optionally filtered by peer and status.
    pub fn clear_outbox(
        &self,
        peer_target: Option<&str>,
        status: Option<OutboxStatus>,
    ) -> Result<OutboxClearReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let cleared_count = {
            let mut engine = self
                .shared
                .outbox_engine
                .lock()
                .expect("outbox_engine lock");
            engine.clear(resolved_peer.as_deref(), status)
        };

        let target_label = resolved_peer.clone().unwrap_or_else(|| "all".to_string());
        let status_label = status
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "all".to_string());
        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "outbox_cleared",
            resolved_peer.clone(),
            format!("Cleared {cleared_count} outbox entries for peer: {target_label}, status: {status_label}"),
            true,
        );

        self.emit(UiEvent::outbox_cleared(
            cleared_count,
            resolved_peer.clone(),
            status.map(|s| s.as_str()),
            self.shared.clock.unix_ms(),
        ));

        Ok(OutboxClearReport {
            ok: true,
            cleared_count,
            target_peer: resolved_peer,
            target_status: status,
            message: format!("successfully cleared {cleared_count} outbox entries"),
        })
    }

    /// Resend an outbox file by original entry id or transfer id.
    pub async fn resend_outbox_file(
        &self,
        id_or_transfer_id: &str,
    ) -> Result<OutboxResendReport, CoreError> {
        let entry = {
            let engine = self
                .shared
                .outbox_engine
                .lock()
                .expect("outbox_engine lock");
            engine
                .get_by_id(id_or_transfer_id)
                .ok_or(CoreError::UnknownTransfer)?
        };

        let peer_id = self.resolve_peer(&entry.peer)?;

        let new_transfer_id = if let Some(ref path_str) = entry.source_path {
            let p = std::path::Path::new(path_str);
            if !p.exists() || !p.is_file() {
                return Err(CoreError::File(format!(
                    "source file '{path_str}' no longer exists on disk"
                )));
            }
            self.offer_and_send_file(&peer_id, p, Some(&entry.filename))
                .await?
        } else {
            return Err(CoreError::File(
                "cannot resend memory-only file transfer without local source file path".into(),
            ));
        };

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "outbox_resent",
            Some(peer_id.to_string()),
            format!(
                "Resent outbox file '{}' (original: {}, new: {})",
                entry.filename, entry.id, new_transfer_id
            ),
            true,
        );

        Ok(OutboxResendReport {
            ok: true,
            original_id: entry.id,
            new_transfer_id: new_transfer_id.clone(),
            message: format!(
                "successfully re-offered file '{}' to peer {}",
                entry.filename, entry.peer
            ),
        })
    }

    pub fn clone_handle(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }

    pub fn overlay_status(&self) -> OverlayStatus {
        self.shared.overlay.lock().expect("overlay").clone()
    }

    pub fn lan_only(&self) -> bool {
        !self.overlay_status().present
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

    pub fn forget(&self, device_id: &DeviceId) {
        let mut store = self.shared.trust.lock().expect("trust");
        let _ = store.forget(device_id);
        drop(store);
        self.persist_trust();
        self.shared
            .ping_waiters
            .lock()
            .expect("ping_waiters")
            .remove(device_id);
        // A forgotten device must not keep its live session, its capability
        // state, or (if it held input) the remote seat. Dropping the outbound
        // sender closes the channel, which ends the session task and its TCP.
        let had_session = match self.shared.sessions.try_lock() {
            Ok(mut sessions) => sessions.remove(device_id).is_some(),
            Err(_) => false,
        };
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(device_id)
            .is_some();
        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(device_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(device_id) {
                *ip = None;
            }
        }
        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: device_id.to_string(),
            });
        }
    }

    /// Revoke trust and actively sever any connection with `device_id`.
    ///
    /// Purges the peer from the in-memory `TrustStore`, atomically re-persists
    /// `trusted.json`, forgets remembered routes in `routes.json`, terminates
    /// active session streaming by dropping its outbound sender, evicts the peer
    /// from live state, and emits `PeerDown`. Returns `true` if the peer was known
    /// and removed, `false` otherwise.
    pub async fn unpair(&self, device_id: &DeviceId) -> bool {
        let had_trust = {
            let mut store = self.shared.trust.lock().expect("trust");
            store.peers.remove(device_id).is_some()
        };
        self.persist_trust();
        let _ = crate::store::forget_route(&self.shared.cfg.data_dir, device_id.as_str());

        let had_session = self
            .shared
            .sessions
            .lock()
            .await
            .remove(device_id)
            .is_some();
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(device_id)
            .is_some();
        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(device_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(device_id) {
                *ip = None;
            }
        }
        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: device_id.to_string(),
            });
        }
        if had_trust || had_session || had_live {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "peer_unpaired",
                Some(device_id.to_string()),
                format!("peer '{device_id}' trust revoked and connection terminated"),
                true,
            );
        }
        had_trust || had_session || had_live
    }

    /// Actively connect to a trusted peer (specified by device ID, alias, or name).
    ///
    /// If `direct_addr` is provided, parses and validates the socket address, remembers it
    /// in `routes.json`, and attempts to dial it. Otherwise looks up remembered routes.
    /// If the peer is already live, returns immediately with `already_connected: true`.
    pub async fn connect_peer(
        &self,
        query: &str,
        direct_addr: Option<&str>,
    ) -> Result<PeerConnectReport, CoreError> {
        let target_id = self.resolve_peer(query)?;
        let parsed_direct: Option<SocketAddr> = if let Some(raw_addr) = direct_addr {
            let parsed: SocketAddr = raw_addr.trim().parse().map_err(|e| {
                CoreError::InvalidRoute(format!("invalid socket address '{raw_addr}': {e}"))
            })?;
            if parsed.port() == 0 {
                return Err(CoreError::InvalidRoute(
                    "socket port must be non-zero".to_string(),
                ));
            }
            Some(parsed)
        } else {
            None
        };

        if self.is_blocked(target_id.as_str()) {
            return Err(CoreError::PeerBlocked(target_id.to_string()));
        }

        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });

        // Check if already live
        {
            let live = self.shared.live.lock().expect("live");
            if let Some(lp) = live.get(&target_id) {
                return Ok(PeerConnectReport {
                    ok: true,
                    device_id: target_id.to_string(),
                    name: Some(lp.name.clone()),
                    alias,
                    addr: Some(lp.addr.to_string()),
                    connected: true,
                    already_connected: true,
                });
            }
        }

        let addr = if let Some(parsed) = parsed_direct {
            let _ = crate::store::remember_route(
                &self.shared.cfg.data_dir,
                target_id.as_str(),
                &parsed.to_string(),
            );
            parsed
        } else {
            let routes = crate::store::load_routes(&self.shared.cfg.data_dir);
            route_addr(&routes, target_id.as_str()).ok_or_else(|| {
                CoreError::InvalidRoute(format!("no known route for peer {}", target_id))
            })?
        };

        match self.dial(addr).await {
            Ok(()) => {
                let live_name = {
                    let live = self.shared.live.lock().expect("live");
                    live.get(&target_id).map(|lp| lp.name.clone())
                };
                let name = live_name.or_else(|| alias.clone());
                Ok(PeerConnectReport {
                    ok: true,
                    device_id: target_id.to_string(),
                    name,
                    alias,
                    addr: Some(addr.to_string()),
                    connected: true,
                    already_connected: false,
                })
            }
            Err(e) => Err(CoreError::InvalidRoute(format!(
                "failed to dial {} at {}: {}",
                target_id, addr, e
            ))),
        }
    }

    /// Actively disconnect an active session with the specified peer (by id, alias, or name)
    /// without revoking trust or forgetting its route.
    ///
    /// Drops the outbound session channel, evicts the peer from live state,
    /// emits `UiEvent::PeerDown`, and returns a `PeerDisconnectReport`.
    pub async fn disconnect_peer(&self, query: &str) -> Result<PeerDisconnectReport, CoreError> {
        let target_id = self.resolve_peer(query)?;
        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });
        let live_name = {
            let live = self.shared.live.lock().expect("live");
            live.get(&target_id).map(|lp| lp.name.clone())
        };
        let name = live_name.or_else(|| alias.clone());

        let had_session = self
            .shared
            .sessions
            .lock()
            .await
            .remove(&target_id)
            .is_some();
        let had_live = self
            .shared
            .live
            .lock()
            .expect("live")
            .remove(&target_id)
            .is_some();

        if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        }
        {
            let mut sp = self.shared.screen_peer.lock().expect("screen_peer");
            if sp.as_ref() == Some(&target_id) {
                *sp = None;
            }
        }
        {
            let mut ip = self.shared.input_peer.lock().expect("input_peer");
            if ip.as_ref() == Some(&target_id) {
                *ip = None;
            }
        }

        if had_session || had_live {
            self.emit(UiEvent::PeerDown {
                device_id: target_id.to_string(),
            });
            Ok(PeerDisconnectReport {
                ok: true,
                device_id: target_id.to_string(),
                name,
                alias,
                disconnected: true,
                already_disconnected: false,
            })
        } else {
            Ok(PeerDisconnectReport {
                ok: true,
                device_id: target_id.to_string(),
                name,
                alias,
                disconnected: false,
                already_disconnected: true,
            })
        }
    }

    /// Check if a peer device ID is currently in the local blocklist.
    pub fn is_blocked(&self, device_id: &str) -> bool {
        let blocked = self.shared.blocked.lock().expect("blocked");
        blocked.peers.iter().any(|p| p.device_id == device_id)
    }

    /// Return a summary report of all currently blocked peers.
    pub fn blocked_peers_report(&self) -> BlockedPeersReport {
        let blocked = self.shared.blocked.lock().expect("blocked");
        BlockedPeersReport {
            ok: true,
            count: blocked.peers.len(),
            peers: blocked.peers.clone(),
        }
    }

    /// Block a peer device by device ID, friendly alias, or broadcast name.
    ///
    /// Persists the block entry into `blocked.json`, and if the peer has an active
    /// connection, forcibly terminates it, evicts from live state, resets input seat,
    /// and emits `UiEvent::PeerDown`. Finally emits `UiEvent::PeerBlocked`.
    pub async fn block_peer(
        &self,
        query: &str,
        reason: Option<&str>,
    ) -> Result<PeerBlockReport, CoreError> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(CoreError::PeerNotFound("empty peer identifier".to_string()));
        }
        let target_id = match self.resolve_peer(trimmed_query) {
            Ok(id) => id,
            Err(e) => {
                if let Ok(id) = DeviceId::parse(trimmed_query) {
                    id
                } else {
                    return Err(e);
                }
            }
        };

        if target_id == *self.shared.identity.device_id() {
            return Err(CoreError::InvalidPayload(
                "cannot block local device".to_string(),
            ));
        }

        let sanitized_reason = if let Some(r) = reason {
            let r_trimmed = r.trim();
            if r_trimmed.is_empty() {
                None
            } else {
                if r_trimmed.len() > 128 {
                    return Err(CoreError::InvalidPayload(
                        "reason exceeds maximum length of 128 bytes".to_string(),
                    ));
                }
                if r_trimmed.chars().any(|c| c.is_control()) {
                    return Err(CoreError::InvalidPayload(
                        "reason cannot contain control characters".to_string(),
                    ));
                }
                Some(r_trimmed.to_string())
            }
        } else {
            None
        };

        let now_ms = self.shared.clock.unix_ms();
        let already_blocked = {
            let mut blocked = self.shared.blocked.lock().expect("blocked");
            let is_already = blocked
                .peers
                .iter()
                .any(|p| p.device_id == target_id.as_str());
            if let Some(existing) = blocked
                .peers
                .iter_mut()
                .find(|p| p.device_id == target_id.as_str())
            {
                existing.reason = sanitized_reason.clone();
                existing.blocked_at = now_ms;
            } else {
                blocked.peers.push(crate::store::BlockedPeer {
                    device_id: target_id.to_string(),
                    reason: sanitized_reason.clone(),
                    blocked_at: now_ms,
                });
            }
            let _ = crate::store::save_blocked(&self.shared.cfg.data_dir, &blocked);
            is_already
        };

        let disconnected_active = {
            let mut sessions = self.shared.sessions.lock().await;
            let had_session = sessions.remove(&target_id).is_some();
            let had_live = {
                let mut live = self.shared.live.lock().expect("live");
                live.remove(&target_id).is_some()
            };
            if had_session || had_live {
                if *self.shared.input_seat.lock().expect("seat") == Some(CursorSeat::Remote) {
                    *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                }
                self.emit(UiEvent::PeerDown {
                    device_id: target_id.to_string(),
                });
                true
            } else {
                false
            }
        };

        let trusted = self.trusted();
        let matched = trusted.iter().find(|t| t.device_id == target_id);
        let alias = matched.and_then(|t| {
            if t.alias.is_empty() {
                None
            } else {
                Some(t.alias.clone())
            }
        });
        let name = alias.clone();

        self.emit(UiEvent::PeerBlocked {
            device_id: target_id.to_string(),
            reason: sanitized_reason.clone(),
        });

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Warning,
            "peer_blocked",
            Some(target_id.to_string()),
            format!(
                "peer '{target_id}' added to blacklist{}",
                sanitized_reason
                    .as_deref()
                    .map(|r| format!(" (reason: {r})"))
                    .unwrap_or_default()
            ),
            true,
        );

        Ok(PeerBlockReport {
            ok: true,
            device_id: target_id.to_string(),
            name,
            alias,
            reason: sanitized_reason,
            blocked: true,
            already_blocked,
            disconnected_active,
        })
    }

    /// Unblock a previously blocked peer device.
    ///
    /// Removes the peer from `blocked.json` and emits `UiEvent::PeerUnblocked`.
    pub async fn unblock_peer(&self, query: &str) -> Result<PeerUnblockReport, CoreError> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err(CoreError::PeerNotFound("empty peer identifier".to_string()));
        }
        let target_id = match self.resolve_peer(trimmed_query) {
            Ok(id) => id,
            Err(e) => {
                if let Ok(id) = DeviceId::parse(trimmed_query) {
                    id
                } else {
                    return Err(e);
                }
            }
        };

        if target_id == *self.shared.identity.device_id() {
            return Err(CoreError::InvalidPayload(
                "cannot unblock local device".to_string(),
            ));
        }

        let was_blocked = {
            let mut blocked = self.shared.blocked.lock().expect("blocked");
            let initial_len = blocked.peers.len();
            blocked.peers.retain(|p| p.device_id != target_id.as_str());
            let removed = blocked.peers.len() < initial_len;
            if removed {
                let _ = crate::store::save_blocked(&self.shared.cfg.data_dir, &blocked);
            }
            removed
        };

        if was_blocked {
            self.emit(UiEvent::PeerUnblocked {
                device_id: target_id.to_string(),
            });
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "peer_unblocked",
                Some(target_id.to_string()),
                format!("peer '{target_id}' removed from blacklist"),
                true,
            );
        }

        Ok(PeerUnblockReport {
            ok: true,
            device_id: target_id.to_string(),
            unblocked: was_blocked,
            was_blocked,
        })
    }

    fn already_live_on_ip(&self, addr: SocketAddr) -> bool {
        self.shared
            .live
            .lock()
            .expect("live")
            .values()
            .any(|p| p.addr.ip() == addr.ip() && p.addr.port() == addr.port())
    }

    fn spawn_overlay(&self) {
        if !self.shared.cfg.loopback_only {
            let overlay_addrs =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.control_port);
            let overlay_file =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.file_port);
            let overlay_input =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.input_port);
            let overlay_screen =
                sidecar::overlay_bind_addrs(&self.shared.cfg.overlay, self.shared.cfg.screen_port);
            let node = self.clone_handle();
            tokio::spawn(async move {
                for l in bind_overlay(&overlay_addrs).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay control",
                        |n, s, a| async move { n.accept_one(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_file).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay file",
                        |n, s, a| async move { n.accept_file(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_input).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay input",
                        |n, s, a| async move { n.accept_input(s, a).await },
                    );
                }
                for l in bind_overlay(&overlay_screen).await {
                    node_accept_loop(
                        node.clone_handle(),
                        l,
                        "overlay screen",
                        |n, s, a| async move { n.accept_screen(s, a).await },
                    );
                }
            });
        }
        let node = self.clone_handle();
        tokio::spawn(async move {
            node.overlay_scan_loop().await;
        });
    }

    async fn overlay_scan_loop(&self) {
        let mut stop = self.shared.shutdown.subscribe();
        let mut last_present = false;
        loop {
            if *stop.borrow() {
                break;
            }
            let st = sidecar::discover(&self.shared.cfg.overlay).await;
            let present = st.present;
            {
                *self.shared.overlay.lock().expect("overlay") = st.clone();
            }
            if present != last_present {
                last_present = present;
                self.emit(UiEvent::Overlay {
                    present,
                    lan_only: !present,
                });
                self.emit(UiEvent::OverlayStateChanged {
                    present,
                    source: format!("{:?}", st.source),
                    local_ips: st.local_ips.iter().map(|ip| ip.to_string()).collect(),
                    peer_count: st.peers.len(),
                });
                if !present {
                    info!("overlay down; LAN only");
                } else {
                    info!(peers = st.peers.len(), "overlay peers");
                }
            }
            let live = self.live_peers();
            let live_ips: Vec<IpAddr> = live.iter().map(|p| p.addr.ip()).collect();
            let port = self.control_port();
            let mut targets: Vec<SocketAddr> = self.shared.cfg.overlay.extra_peers.clone();
            for ip in &st.peers {
                if live_ips.contains(&IpAddr::V4(*ip)) {
                    continue;
                }
                targets.push(SocketAddr::from((*ip, port)));
            }
            for addr in targets {
                if self.already_live_on_ip(addr) {
                    continue;
                }
                if let Err(e) = self.dial(addr).await {
                    debug!(%e, %addr, "overlay dial");
                }
            }
            tokio::select! {
                _ = stop.changed() => {
                    if *stop.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(OVERLAY_PROBE_MS.max(5_000))) => {}
            }
        }
    }

    fn advertise_mdns(&self) {
        let id = self.shared.identity.device_id().clone();
        let fp = DeviceId::fingerprint12(self.shared.identity.id_pk());
        let name = self.name();
        let port = self.shared.cfg.control_port;
        let addrs = lan_ips();
        let mut stop = self.shared.shutdown.subscribe();
        let (adv_tx, mut adv_rx) = watch::channel(false);
        *self.shared.advertise_stop.lock().expect("advertise_stop") = Some(adv_tx);
        tokio::task::spawn_blocking(move || {
            let d = match tetherly_net::mdns::daemon() {
                Ok(d) => d,
                Err(e) => {
                    warn!(%e, "mdns daemon");
                    return;
                }
            };
            if let Err(e) =
                tetherly_net::mdns::advertise(&d, &name, port, &id, &fp, "notify,clip,file", &addrs)
            {
                warn!(%e, "mdns advertise");
                return;
            }
            // Hold the registration for the node's lifetime, then drop the
            // daemon so the service is withdrawn instead of lingering.
            let handle = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => {
                    std::thread::park();
                    return;
                }
            };
            handle.block_on(async {
                loop {
                    if *stop.borrow() || *adv_rx.borrow() {
                        break;
                    }
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        _ = adv_rx.changed() => {
                            if *adv_rx.borrow() {
                                break;
                            }
                        }
                    }
                }
            });
        });
    }

    fn spawn_reconnect(&self) {
        let node = self.clone_handle();
        tokio::spawn(async move {
            let mut attempt = 0u32;
            let mut stop = node.shared.shutdown.subscribe();
            loop {
                if *stop.borrow() {
                    break;
                }
                if !node.is_reconnect_enabled() {
                    tokio::select! {
                        _ = stop.changed() => {
                            if *stop.borrow() {
                                break;
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    }
                    continue;
                }
                let trusted = node.trusted();
                let live = node.live_peers();
                let routes = crate::store::load_routes(&node.shared.cfg.data_dir);
                for peer in trusted {
                    if peer.revoked || node.is_blocked(peer.device_id.as_str()) {
                        continue;
                    }
                    if live.iter().any(|l| l.device_id == peer.device_id) {
                        continue;
                    }
                    if let Some(addr) = route_addr(&routes, peer.device_id.as_str()) {
                        match node.dial(addr).await {
                            Ok(()) => attempt = 0,
                            Err(e) => {
                                debug!(%e, "reconnect");
                                attempt = attempt.saturating_add(1);
                            }
                        }
                    }
                }
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(backoff_delay(attempt)) => {}
                }
            }
        });
    }

    pub async fn dial(&self, addr: SocketAddr) -> Result<(), tetherly_net::NetError> {
        if self.already_live_on_ip(addr) {
            return Ok(());
        }
        let stream = TcpStream::connect(addr).await?;
        let cfg = self.session_config();
        let sess = match dial_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        if self.is_blocked(sess.peer_id.as_str()) {
            warn!(peer = %sess.peer_id, "outbound dial aborted: peer is blocked");
            return Err(CoreError::PeerBlocked(sess.peer_id.to_string()).into());
        }
        self.persist_trust();
        let _ = remember_route(
            &self.shared.cfg.data_dir,
            sess.peer_id.as_str(),
            &addr.to_string(),
        );
        self.attach(sess).await;
        Ok(())
    }

    async fn accept_one(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let cfg = self.session_config();
        let sess = match accept_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        if self.is_blocked(sess.peer_id.as_str()) {
            warn!(peer = %sess.peer_id, "incoming session rejected: peer is blocked");
            self.shared.telemetry.record_handshake_failed();
            return Err(CoreError::PeerBlocked(sess.peer_id.to_string()).into());
        }
        self.persist_trust();
        let _ = remember_route(
            &self.shared.cfg.data_dir,
            sess.peer_id.as_str(),
            &addr.to_string(),
        );
        self.attach(sess).await;
        Ok(())
    }

    async fn attach(&self, mut sess: ActiveSession) {
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
                node.emit(UiEvent::PeerDown {
                    device_id: peer_id.to_string(),
                });
            }
        });
    }

    async fn handle_event(
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
                    caps: update.caps,
                    features,
                });
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
            SessionEvent::Unknown(_) => {}
        }
        Ok(())
    }

    fn apply_clip(&self, peer: &DeviceId, clip: ClipSet) -> Result<(), tetherly_net::NetError> {
        let policy = self.clipboard_policy();
        if !policy.mode.can_receive() {
            debug!(peer = %peer, "clipboard sync ignored due to receive_only/manual mode policy");
            return Ok(());
        }
        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "clipboard sync rejected: sender profile not eligible under clipboard policy");
            self.record_audit(
                AuditCategory::Clipboard,
                AuditSeverity::Warning,
                "clipboard_inbound_ineligible",
                Some(peer.to_string()),
                format!(
                    "Ignored inbound clipboard sync from {} due to profile scope policy",
                    peer
                ),
                false,
            );
            return Ok(());
        }

        let now = self.shared.clock.unix_ms();
        let bytes = clip.text.as_ref().map_or(0, |s| s.len());
        let hash = clip.hash.clone();
        let seq = clip.clip_seq;
        let apply = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.apply_remote(peer, clip, now)?
        };
        match apply {
            ClipApply::Apply(text) => {
                self.shared.clip.set_text(&text)?;
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                self.shared
                    .clip_auto_synced_in_count
                    .fetch_add(1, Ordering::Relaxed);
                {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.clip.record_entry(ClipMetaEntry {
                        seq,
                        source: Some(peer.clone()),
                        direction: ClipDirection::Inbound,
                        bytes,
                        hash: hash.clone(),
                        timestamp_ms: now,
                    });
                }
                self.emit(UiEvent::ClipboardSynced {
                    seq,
                    source: peer.to_string(),
                    direction: "inbound".to_string(),
                    bytes,
                    hash,
                    timestamp_ms: now,
                    sent_peers: 0,
                });
                if let Some(secs) = policy.auto_clear_timeout_secs {
                    if secs > 0 {
                        let clip = self.shared.clip.clone();
                        let expected_text = text.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(secs)).await;
                            if let Ok(Some(cur)) = clip.get_text() {
                                if cur == expected_text {
                                    let _ = clip.set_text("");
                                }
                            }
                        });
                    }
                }
            }
            ClipApply::Echo | ClipApply::RateLimited => {}
        }
        Ok(())
    }

    pub async fn accept_file(
        &self,
        mut stream: TcpStream,
        _addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        // Header is unauthenticated (id/total/start). It only names the
        // transfer; the token (HKDF over the Noise handshake + transfer_id)
        // gates every chunk, so a forged id simply fails to decrypt.
        let (transfer_id, total, start) = read_file_header(&mut stream).await?;
        let (accepted, source, size, resume_hint, name, expected) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(&transfer_id)
                .ok_or(tetherly_net::NetError::File("unknown".into()))?;
            (
                t.status == tetherly_core::TransferStatus::Accepted,
                t.source.clone(),
                t.files.iter().map(|f| f.size).sum::<u64>(),
                t.resume_from.unwrap_or(0),
                t.files
                    .first()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "file.bin".into()),
                t.files
                    .first()
                    .map(|f| f.sha256.clone())
                    .unwrap_or_default(),
            )
        };
        if !accepted {
            return Err(tetherly_net::NetError::File("not accepted".into()));
        }
        if total != size || start != resume_hint {
            return Err(tetherly_net::NetError::Handshake);
        }
        // The offer's size is authoritative (already capped by the hub), but the
        // channel header is unauthenticated, so re-assert the cap here and hand
        // the channel the real bound instead of `u64::MAX`: the whole body is
        // buffered in memory below, so an unbounded `total` would be an
        // unbounded allocation.
        if total > tetherly_core::FILE_MAX {
            return Err(tetherly_net::NetError::TooLarge);
        }
        let token = self
            .file_token_for(&source, &transfer_id)
            .ok_or(tetherly_net::NetError::File("no session token".into()))?;
        let dest_dir = self.shared.cfg.data_dir.join("inbox");
        std::fs::create_dir_all(&dest_dir)?;
        let part = dest_dir.join(format!(".{}.part", transfer_id));

        let mut hasher = tetherly_core::IncrementalSha256::new();
        // If resuming, verify that the on-disk partial exists and matches start,
        // feeding the prefix [0..start] into the incremental hasher.
        if start > 0 {
            let meta = std::fs::metadata(&part)?;
            let aligned = meta.len() - (meta.len() % tetherly_core::FILE_CHUNK as u64);
            if aligned < start {
                return Err(tetherly_net::NetError::Handshake);
            }
            use std::io::Read;
            let mut f = std::fs::File::open(&part)?;
            let mut buf = [0u8; tetherly_core::FILE_CHUNK];
            let mut read_total = 0u64;
            while read_total < start {
                let want = ((start - read_total) as usize).min(buf.len());
                f.read_exact(&mut buf[..want])?;
                hasher.update(&buf[..want]);
                read_total += want as u64;
            }
        }
        let mut written = start;
        let now = self.shared.clock.unix_ms();
        let mut cancel_rx = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            if let Some(t) = transfers.get_mut(&transfer_id) {
                if t.phase == TransferPhase::Failed {
                    return Err(tetherly_net::NetError::File("cancelled by user".into()));
                }
                t.transferred = start;
                if start > 0 {
                    t.resume_offset = Some(start);
                    t.is_resumed = true;
                }
                t.phase = TransferPhase::Transferring;
                t.updated_ms = now;
            } else {
                transfers.insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: source.clone(),
                        direction: TransferDirection::Inbound,
                        filename: name.clone(),
                        total,
                        transferred: start,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: if start > 0 { Some(start) } else { None },
                        is_resumed: start > 0,
                    },
                );
            }
            let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
            cancels.insert(transfer_id.clone(), cancel_tx);
            cancel_rx
        };
        // Open the .part file for writing chunks at offset `start`.
        let mut part_file = {
            let f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&part)?;
            f.set_len(start)?;
            f.sync_all()?;
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(&part)
                .await?
        };
        use tokio::io::AsyncSeekExt;
        part_file.seek(std::io::SeekFrom::Start(start)).await?;

        let node = self.clone_handle();
        let tid = transfer_id.clone();
        let recv_fut = tetherly_net::filechan::recv_chunks_to_sink(
            &mut stream,
            &mut part_file,
            &token,
            &transfer_id,
            total,
            start,
            tetherly_core::FILE_MAX,
            hasher,
            |chan_offset, chunk_len| {
                written = chan_offset + chunk_len as u64;
                node.update_transfer_progress(&tid, written);
                node.shared.telemetry.record_file_rx(chunk_len as u64);
                Ok(())
            },
        );

        let res = tokio::select! {
            _ = &mut cancel_rx => {
                self.shared.transfer_cancels.lock().expect("cancels").remove(&transfer_id);
                self.note_resume(&source, &transfer_id, total, written);
                self.set_transfer_phase(
                    &transfer_id,
                    TransferPhase::Failed,
                    Some("cancelled by user".into()),
                );
                return Err(tetherly_net::NetError::File("cancelled by user".into()));
            }
            r = recv_fut => {
                self.shared.transfer_cancels.lock().expect("cancels").remove(&transfer_id);
                r
            }
        };

        if let Err(e) = res {
            // Persist a chunk-aligned resume marker (in memory + sidecar) so the
            // next offer resumes instead of restarting.
            self.note_resume(&source, &transfer_id, total, written);
            self.set_transfer_phase(&transfer_id, TransferPhase::Failed, Some(e.to_string()));
            return Err(e);
        }

        self.set_transfer_phase(&transfer_id, TransferPhase::Verifying, None);

        let computed_digest = tetherly_core::hex_lower(&res.unwrap());
        if computed_digest != expected {
            let _ = std::fs::remove_file(&part);
            self.clear_resume(&transfer_id);
            self.set_transfer_phase(
                &transfer_id,
                TransferPhase::Failed,
                Some("sha256 mismatch".into()),
            );
            return Err(tetherly_net::NetError::File("sha256".into()));
        }

        // Drop part_file so the file handle is released before rename.
        drop(part_file);

        let path = dest_dir.join(&name);
        let _ = std::fs::remove_file(&path);
        std::fs::rename(&part, &path)?;
        self.clear_resume(&transfer_id);
        let done = FileDone {
            transfer_id: transfer_id.clone(),
            sha256: computed_digest,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let _ = hubs.files.mark_done(&done);
        }
        let _ = self.send_to(&source, done.to_frame()?).await;
        self.set_transfer_phase(&transfer_id, TransferPhase::Completed, None);
        self.emit(UiEvent::FileDone { transfer_id });
        let banner_title = "已接收文件";
        let banner_body = format!("文件 {name} 接收完成");
        let _ = self.dispatch_notification_banner(
            banner_title,
            &banner_body,
            Some("Tetherly File"),
            false,
        );
        Ok(())
    }

    /// Record a chunk-aligned resume marker in the hub and on disk (sidecar).
    fn note_resume(&self, source: &DeviceId, transfer_id: &str, total: u64, offset: u64) {
        let st = tetherly_core::ResumeState {
            transfer_id: transfer_id.to_string(),
            source: source.clone(),
            total,
            offset,
        };
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.put(st.clone());
        }
        // Persist the chunk-aligned offset the table actually settled on.
        let aligned = self
            .shared
            .hubs
            .lock()
            .expect("hubs")
            .resume
            .get(transfer_id)
            .map(|s| s.offset)
            .unwrap_or(0);
        if aligned == 0 {
            return;
        }
        let side = self.resume_sidecar(transfer_id);
        let dto = ResumeSidecar {
            device_id: source.as_str().to_string(),
            total,
            offset: aligned,
        };
        if let Ok(bytes) = serde_json::to_vec(&dto) {
            let _ = crate::store::atomic_write(&side, &bytes);
        }
    }

    fn clear_resume(&self, transfer_id: &str) {
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.resume.remove(transfer_id);
        }
        let _ = std::fs::remove_file(self.resume_sidecar(transfer_id));
    }

    fn resume_sidecar(&self, transfer_id: &str) -> std::path::PathBuf {
        self.shared
            .cfg
            .data_dir
            .join("inbox")
            .join(format!(".{}.resume.json", transfer_id))
    }

    /// Accepts an incoming file transfer, records the active transfer, dispatches
    /// the accept frame to the peer, and returns a structured decision report.
    pub async fn user_accept_file_reported(
        &self,
        transfer_id: &str,
    ) -> Result<FileDecisionReport, CoreError> {
        let (decision, source, name, total, resume_from) = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let decision = hubs
                .files
                .accept(transfer_id, self.shared.clock.unix_ms())?;
            let t = hubs
                .files
                .get(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            (
                decision,
                t.source.clone(),
                t.files
                    .first()
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "file.bin".into()),
                t.files.iter().map(|f| f.size).sum::<u64>(),
                t.resume_from.unwrap_or(0),
            )
        };
        let now = self.shared.clock.unix_ms();
        {
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            transfers.insert(
                transfer_id.to_string(),
                ActiveTransfer {
                    transfer_id: transfer_id.to_string(),
                    peer: source.clone(),
                    direction: TransferDirection::Inbound,
                    filename: name.clone(),
                    total,
                    transferred: resume_from,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: if resume_from > 0 {
                        Some(resume_from)
                    } else {
                        None
                    },
                    is_resumed: resume_from > 0,
                },
            );
        }
        // Token is derived from the live session handshake hash.
        self.send_to(&source, decision.accept_frame()?).await?;
        Ok(FileDecisionReport {
            ok: true,
            accepted: true,
            action: "accepted".to_string(),
            transfer_id: transfer_id.to_string(),
            peer: source.to_string(),
            filename: name,
            total_bytes: total,
            resume_from,
            timestamp_ms: now,
        })
    }

    pub async fn user_accept_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        self.user_accept_file_reported(transfer_id)
            .await
            .map(|_| ())
    }

    pub fn file_token_for(&self, peer: &DeviceId, transfer_id: &str) -> Option<[u8; 32]> {
        let live = self.shared.live.lock().expect("live");
        live.get(peer)
            .map(|p| tetherly_crypto::file_token(&p.handshake_hash, transfer_id.as_bytes()))
    }

    pub fn clip(&self) -> Arc<dyn Clip> {
        self.shared.clip.clone()
    }

    /// Rejects an incoming file transfer, emits a rejection event, dispatches
    /// the reject frame to the peer, and returns a structured decision report.
    pub async fn user_reject_file_reported(
        &self,
        transfer_id: &str,
    ) -> Result<FileDecisionReport, CoreError> {
        let (decision, source, name, total, resume_from) = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            let t = hubs
                .files
                .get(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            let source = t.source.clone();
            let name = t
                .files
                .first()
                .map(|f| f.name.clone())
                .unwrap_or_else(|| "file.bin".into());
            let total = t.files.iter().map(|f| f.size).sum::<u64>();
            let resume_from = t.resume_from.unwrap_or(0);
            let decision = hubs.files.reject(transfer_id)?;
            (decision, source, name, total, resume_from)
        };
        self.set_transfer_phase(
            transfer_id,
            TransferPhase::Failed,
            Some("rejected locally".into()),
        );
        self.emit(UiEvent::FileRejected {
            transfer_id: transfer_id.to_string(),
            source: source.to_string(),
            reason: Some("rejected locally".into()),
        });
        self.send_to(&source, decision.reject_frame()?).await?;
        Ok(FileDecisionReport {
            ok: true,
            accepted: false,
            action: "rejected".to_string(),
            transfer_id: transfer_id.to_string(),
            peer: source.to_string(),
            filename: name,
            total_bytes: total,
            resume_from,
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    pub async fn user_reject_file(&self, transfer_id: &str) -> Result<(), CoreError> {
        self.user_reject_file_reported(transfer_id)
            .await
            .map(|_| ())
    }

    /// Batch accepts pending incoming file offers, optionally filtered by peer.
    pub async fn batch_accept_files(
        &self,
        peer_filter: Option<&str>,
    ) -> Result<FileBatchDecisionReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileBatchDecisionReport {
                        ok: true,
                        action: "accepted".to_string(),
                        accepted_count: 0,
                        rejected_count: 0,
                        total_evaluated: 0,
                        decisions: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let candidates: Vec<String> = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .offered()
                .into_iter()
                .filter(|t| match &target_peer {
                    Some(expected) => &t.source == expected,
                    None => true,
                })
                .map(|t| t.transfer_id)
                .collect()
        };
        let mut reports = Vec::with_capacity(candidates.len());
        for tid in &candidates {
            if let Ok(rep) = self.user_accept_file_reported(tid).await {
                reports.push(rep);
            }
        }
        let accepted_count = reports.len();
        Ok(FileBatchDecisionReport {
            ok: true,
            action: "accepted".to_string(),
            accepted_count,
            rejected_count: 0,
            total_evaluated: candidates.len(),
            decisions: reports,
        })
    }

    /// Batch rejects pending incoming file offers, optionally filtered by peer.
    pub async fn batch_reject_files(
        &self,
        peer_filter: Option<&str>,
    ) -> Result<FileBatchDecisionReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileBatchDecisionReport {
                        ok: true,
                        action: "rejected".to_string(),
                        accepted_count: 0,
                        rejected_count: 0,
                        total_evaluated: 0,
                        decisions: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let candidates: Vec<String> = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.files
                .offered()
                .into_iter()
                .filter(|t| match &target_peer {
                    Some(expected) => &t.source == expected,
                    None => true,
                })
                .map(|t| t.transfer_id)
                .collect()
        };
        let mut reports = Vec::with_capacity(candidates.len());
        for tid in &candidates {
            if let Ok(rep) = self.user_reject_file_reported(tid).await {
                reports.push(rep);
            }
        }
        let rejected_count = reports.len();
        Ok(FileBatchDecisionReport {
            ok: true,
            action: "rejected".to_string(),
            accepted_count: 0,
            rejected_count,
            total_evaluated: candidates.len(),
            decisions: reports,
        })
    }

    /// Queries pending incoming file offers with optional transfer ID or peer filtering.
    pub fn file_offers(
        &self,
        id_filter: Option<&str>,
        peer_filter: Option<&str>,
    ) -> Result<FileOffersReport, CoreError> {
        let target_peer = if let Some(p) = peer_filter {
            match self.resolve_peer(p) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Ok(FileOffersReport {
                        ok: true,
                        count: 0,
                        auto_accept_trusted: self
                            .shared
                            .auto_accept_trusted
                            .load(Ordering::Relaxed),
                        offers: Vec::new(),
                    });
                }
            }
        } else {
            None
        };
        let raw_offers = self.offered_files();
        let trusted_map: HashMap<DeviceId, Option<String>> = {
            let trust = self.shared.trust.lock().expect("trust");
            trust
                .all()
                .into_iter()
                .map(|p| {
                    let alias = if p.alias.is_empty() {
                        None
                    } else {
                        Some(p.alias)
                    };
                    (p.device_id, alias)
                })
                .collect()
        };
        let mut filtered = Vec::new();
        for t in raw_offers {
            if let Some(expected_id) = id_filter {
                if t.transfer_id != expected_id {
                    continue;
                }
            }
            if let Some(ref expected_peer) = target_peer {
                if &t.source != expected_peer {
                    continue;
                }
            }
            let peer_alias = trusted_map.get(&t.source).cloned().flatten();
            let total_bytes: u64 = t.files.iter().map(|f| f.size).sum();
            let (name, sha256) = t
                .files
                .first()
                .map(|f| (f.name.clone(), f.sha256.clone()))
                .unwrap_or_default();
            filtered.push(FileOfferDto {
                transfer_id: t.transfer_id,
                filename: name,
                total_bytes,
                resume_from: t.resume_from.unwrap_or(0),
                source: t.source.to_string(),
                peer: t.source.to_string(),
                peer_alias,
                peer_name: None,
                size: total_bytes,
                sha256,
                timestamp_ms: t.offered_ms,
                offered_ms: t.offered_ms,
            });
        }
        if id_filter.is_some() && filtered.is_empty() {
            return Err(CoreError::UnknownTransfer);
        }
        Ok(FileOffersReport {
            ok: true,
            count: filtered.len(),
            auto_accept_trusted: self.shared.auto_accept_trusted.load(Ordering::Relaxed),
            offers: filtered,
        })
    }

    /// Status and configuration of auto-accept policy for trusted peers.
    pub fn auto_accept_status(&self) -> AutoAcceptReport {
        AutoAcceptReport {
            ok: true,
            enabled: self.shared.auto_accept_trusted.load(Ordering::Relaxed),
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
        }
    }

    /// Read the current granular auto-accept policy.
    pub fn auto_accept_policy(&self) -> AutoAcceptPolicy {
        self.shared
            .auto_accept_policy
            .lock()
            .expect("auto_accept_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for granular file auto-accept policy.
    pub fn auto_accept_policy_report(&self) -> AutoAcceptPolicyReport {
        AutoAcceptPolicyReport {
            ok: true,
            policy: self.auto_accept_policy(),
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
            message: "auto-accept policy report".to_string(),
        }
    }

    /// Update the granular auto-accept policy and persist to disk atomically.
    pub fn set_auto_accept_policy(
        &self,
        policy: AutoAcceptPolicy,
    ) -> Result<AutoAcceptPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        crate::store::save_auto_accept_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save auto_accept.json: {e}")))?;
        self.shared
            .auto_accept_trusted
            .store(policy.enabled, Ordering::Relaxed);
        *self
            .shared
            .auto_accept_policy
            .lock()
            .expect("auto_accept_policy") = policy.clone();

        self.emit(UiEvent::auto_accept_policy_changed(
            policy.enabled,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_file_size,
            now,
        ));

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "auto_accept_policy_changed",
            None::<String>,
            format!(
                "Updated file auto-accept policy: enabled={}, groups={:?}, tags={:?}, types={:?}, max_size={:?}",
                policy.enabled,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_file_size
            ),
            true,
        );

        Ok(AutoAcceptPolicyReport {
            ok: true,
            policy,
            auto_accepted_count: self.shared.auto_accepted_count.load(Ordering::Relaxed),
            message: "auto-accept policy updated successfully".to_string(),
        })
    }

    /// Configures the auto-accept policy for trusted peers and emits `UiEvent::FileAutoAcceptChanged`.
    pub fn set_auto_accept(&self, enabled: bool) -> AutoAcceptReport {
        let mut policy = self.auto_accept_policy();
        policy.enabled = enabled;
        let _ = self.set_auto_accept_policy(policy);
        self.emit(UiEvent::FileAutoAcceptChanged { enabled });
        self.auto_accept_status()
    }

    pub fn clipboard_policy(&self) -> ClipboardPolicy {
        self.shared
            .clipboard_policy
            .lock()
            .expect("clipboard_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for granular clipboard auto-sync policy.
    pub fn clipboard_policy_report(&self) -> ClipboardPolicyReport {
        ClipboardPolicyReport {
            ok: true,
            policy: self.clipboard_policy(),
            auto_synced_out_count: self
                .shared
                .clip_auto_synced_out_count
                .load(Ordering::Relaxed),
            auto_synced_in_count: self
                .shared
                .clip_auto_synced_in_count
                .load(Ordering::Relaxed),
            sensitive_blocked_count: self
                .shared
                .clip_sensitive_blocked_count
                .load(Ordering::Relaxed),
            message: "clipboard policy report".to_string(),
        }
    }

    /// Update the granular clipboard policy and persist to disk atomically.
    pub fn set_clipboard_policy(
        &self,
        policy: ClipboardPolicy,
    ) -> Result<ClipboardPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_clipboard_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save clipboard_policy.json: {e}")))?;
        *self
            .shared
            .clipboard_policy
            .lock()
            .expect("clipboard_policy") = policy.clone();

        self.emit(UiEvent::clipboard_policy_changed(
            policy.mode.as_str(),
            policy.sensitive_guard,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_payload_bytes,
            policy.auto_clear_timeout_secs,
            now,
        ));

        self.record_audit(
            AuditCategory::Clipboard,
            AuditSeverity::Info,
            "clipboard_policy_changed",
            None::<String>,
            format!(
                "Updated clipboard policy: mode={}, sensitive_guard={}, groups={:?}, tags={:?}, types={:?}, max_bytes={}, auto_clear={:?}",
                policy.mode.as_str(),
                policy.sensitive_guard,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_payload_bytes,
                policy.auto_clear_timeout_secs
            ),
            true,
        );

        Ok(ClipboardPolicyReport {
            ok: true,
            policy,
            auto_synced_out_count: self
                .shared
                .clip_auto_synced_out_count
                .load(Ordering::Relaxed),
            auto_synced_in_count: self
                .shared
                .clip_auto_synced_in_count
                .load(Ordering::Relaxed),
            sensitive_blocked_count: self
                .shared
                .clip_sensitive_blocked_count
                .load(Ordering::Relaxed),
            message: "clipboard policy updated successfully".to_string(),
        })
    }

    /// Read the current inbound resource handoff policy.
    pub fn handoff_policy(&self) -> HandoffPolicy {
        self.shared
            .handoff_policy
            .lock()
            .expect("handoff_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for resource handoff policy.
    pub fn handoff_policy_report(&self) -> HandoffPolicyReport {
        HandoffPolicyReport {
            ok: true,
            policy: self.handoff_policy(),
            message: "handoff policy retrieved successfully".to_string(),
        }
    }

    /// Update the resource handoff policy and persist to disk atomically.
    pub fn set_handoff_policy(
        &self,
        policy: HandoffPolicy,
    ) -> Result<HandoffPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_handoff_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save handoff_policy.json: {e}")))?;
        *self.shared.handoff_policy.lock().expect("handoff_policy") = policy.clone();

        self.emit(UiEvent::handoff_policy_changed(
            policy.action.as_str(),
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_inbox_entries,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_policy_changed",
            None::<String>,
            format!(
                "Updated handoff policy: action={}, groups={:?}, tags={:?}, types={:?}, max_inbox_entries={}",
                policy.action.as_str(),
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_inbox_entries,
            ),
            true,
        );

        Ok(HandoffPolicyReport {
            ok: true,
            policy,
            message: "handoff policy updated successfully".to_string(),
        })
    }

    /// Dispatch a resource or URL handoff to a specific peer over the Noise session.
    pub async fn handoff_send(
        &self,
        peer_target: &str,
        raw_url: &str,
        title: Option<&str>,
        auto_open: bool,
    ) -> Result<HandoffSendResult, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            ResourceHandoffValidator::validate(raw_url, Some(&allowlist))?
        };
        let now_ms = self.shared.clock.unix_ms();
        let seq = self.shared.handoff_seq.fetch_add(1, Ordering::Relaxed);
        let handoff_id = format!("ho_{}_{}", now_ms, seq);
        let title_str = title.unwrap_or("").trim().to_string();

        let frame = ResourceHandoffFrame {
            id: handoff_id.clone(),
            url: sanitized.url.clone(),
            title: title_str.clone(),
            auto_open_requested: auto_open,
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self.peer_alias(&peer_id);

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_sent",
            Some(peer_str.clone()),
            format!(
                "Sent resource handoff '{}' (host: {}) to peer '{}'",
                if title_str.is_empty() {
                    &sanitized.host
                } else {
                    &title_str
                },
                sanitized.host,
                peer_str
            ),
            true,
        );

        self.emit(UiEvent::resource_handoff_dispatched(
            &handoff_id,
            1,
            1,
            &sanitized.host,
            now_ms,
        ));

        Ok(HandoffSendResult {
            ok: true,
            id: handoff_id,
            peer: peer_str,
            peer_alias: alias,
            url: sanitized.url,
            title: title_str,
            auto_open_requested: auto_open,
            timestamp_ms: now_ms,
        })
    }

    /// Fan out a resource or URL handoff to multiple peers matching a target selector.
    pub async fn handoff_send_multi(
        &self,
        selector: &TargetSelector,
        raw_url: &str,
        title: Option<&str>,
        auto_open: bool,
    ) -> Result<MultiHandoffSendReport, CoreError> {
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            ResourceHandoffValidator::validate(raw_url, Some(&allowlist))?
        };
        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now_ms = self.shared.clock.unix_ms();
        let title_str = title.unwrap_or("").trim().to_string();
        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerHandoffSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    handoff_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = self.shared.handoff_seq.fetch_add(1, Ordering::Relaxed);
            let handoff_id = format!("ho_{}_{}", now_ms, seq);
            let frame = ResourceHandoffFrame {
                id: handoff_id.clone(),
                url: sanitized.url.clone(),
                title: title_str.clone(),
                auto_open_requested: auto_open,
                timestamp_ms: now_ms,
            };

            match frame.to_frame() {
                Ok(inner_frame) => match self.send_to(peer_id, inner_frame).await {
                    Ok(_) => {
                        success_count += 1;
                        dispatches.push(PeerHandoffSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            handoff_id: Some(handoff_id),
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        failed_count += 1;
                        dispatches.push(PeerHandoffSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            handoff_id: Some(handoff_id),
                            success: false,
                            error: Some(e.to_string()),
                        });
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerHandoffSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        handoff_id: Some(handoff_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_multi_sent",
            None::<String>,
            format!(
                "Multicast resource handoff '{}' (host: {}) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                if title_str.is_empty() { &sanitized.host } else { &title_str },
                sanitized.host,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        let batch_id = format!("ho_multi_{now_ms}");
        self.emit(UiEvent::resource_handoff_dispatched(
            &batch_id,
            targets.len(),
            success_count,
            &sanitized.host,
            now_ms,
        ));

        self.emit(UiEvent::multi_dispatch_completed(
            "handoff",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiHandoffSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            url: sanitized.url,
            title: title_str,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest and process an inbound resource handoff frame from a connected peer.
    pub fn receive_resource_handoff(&self, peer: &DeviceId, frame: ResourceHandoffFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        // 1. Strict URL validator firewall check
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            match ResourceHandoffValidator::validate(&frame.url, Some(&allowlist)) {
                Ok(s) => s,
                Err(e) => {
                    warn!(peer = %peer, error = %e, "rejected invalid inbound resource handoff URL");
                    self.record_audit(
                        AuditCategory::Peer,
                        AuditSeverity::Warning,
                        "handoff_rejected_invalid_url",
                        Some(peer_str.clone()),
                        format!("Blocked inbound handoff URL from {peer}: {e}"),
                        false,
                    );
                    return;
                }
            }
        };

        // 2. Evaluate ingress policy
        let policy = self.handoff_policy();
        if !policy.action.is_enabled() {
            debug!(peer = %peer, "inbound handoff ignored: policy is disabled");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "handoff_disabled_rejected",
                Some(peer_str.clone()),
                format!("Rejected inbound handoff from {peer} due to disabled policy"),
                false,
            );
            return;
        }

        // 3. Check auto-open eligibility
        let profile = self.peer_profile(peer);
        let auto_open_eligible = policy.is_peer_eligible_for_auto_open(&profile);
        let should_auto_open =
            frame.auto_open_requested && policy.action.allows_auto_open() && auto_open_eligible;

        let (status, opened_ms) = if should_auto_open {
            let opener = self.shared.opener.lock().expect("opener");
            match opener.open(&sanitized.url) {
                Ok(()) => {
                    info!(peer = %peer, host = %sanitized.host, "auto-opened inbound resource handoff");
                    (HandoffStatus::Opened, Some(now_ms))
                }
                Err(e) => {
                    warn!(peer = %peer, host = %sanitized.host, error = %e, "failed to auto-open URL, saving as pending");
                    (HandoffStatus::Pending, None)
                }
            }
        } else {
            (HandoffStatus::Pending, None)
        };

        let title = if frame.title.trim().is_empty() {
            sanitized.host.clone()
        } else {
            frame.title.trim().to_string()
        };

        let entry = HandoffEntry {
            id: frame.id.clone(),
            sender_id: peer_str.clone(),
            sender_alias: alias.clone(),
            url: sanitized.url.clone(),
            title: title.clone(),
            status,
            received_ms: now_ms,
            opened_ms,
        };

        // 4. Atomic persistence in bounded inbox
        let total_count = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            inbox.add(entry.clone(), policy.max_inbox_entries);
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            inbox.entries.len()
        };

        // 5. Zero-plaintext observability emission
        self.emit(UiEvent::resource_handoff_received(
            &entry.id,
            &peer_str,
            &alias,
            &sanitized.host,
            &title,
            status.as_str(),
            should_auto_open,
            now_ms,
        ));

        self.emit(UiEvent::handoff_inbox_updated(
            "received",
            Some(&entry.id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            if status == HandoffStatus::Opened {
                "handoff_received_auto_opened"
            } else {
                "handoff_received"
            },
            Some(peer_str),
            format!(
                "Received resource handoff '{}' (host: {}) from {} [status={}]",
                title,
                sanitized.host,
                alias,
                status.as_str()
            ),
            true,
        );
    }

    /// Query items from the resource handoff inbox, with optional status filter.
    pub fn handoff_inbox_list(&self, status_filter: Option<HandoffStatus>) -> Vec<HandoffEntry> {
        let inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
        match status_filter {
            Some(st) => inbox
                .entries
                .iter()
                .filter(|e| e.status == st)
                .cloned()
                .collect(),
            None => inbox.entries.clone(),
        }
    }

    /// Explicitly open a pending resource handoff from the inbox using the system opener.
    pub fn handoff_inbox_open(&self, id: &str) -> Result<HandoffEntry, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let (entry_to_open, total_count) = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let entry = inbox
                .find_mut(id)
                .ok_or_else(|| CoreError::Json(format!("handoff entry '{id}' not found")))?;
            entry.status = HandoffStatus::Opened;
            entry.opened_ms = Some(now_ms);
            let cloned = entry.clone();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            (cloned, inbox.entries.len())
        };

        let opener = self.shared.opener.lock().expect("opener");
        opener.open(&entry_to_open.url)?;

        self.emit(UiEvent::handoff_inbox_updated(
            "opened",
            Some(id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_opened",
            Some(entry_to_open.sender_id.clone()),
            format!("Opened resource handoff '{}' ({})", entry_to_open.title, id),
            true,
        );

        Ok(entry_to_open)
    }

    /// Mark a pending resource handoff entry as dismissed/ignored.
    pub fn handoff_inbox_dismiss(&self, id: &str) -> Result<HandoffEntry, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let (entry, total_count) = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let entry = inbox
                .find_mut(id)
                .ok_or_else(|| CoreError::Json(format!("handoff entry '{id}' not found")))?;
            entry.status = HandoffStatus::Dismissed;
            let cloned = entry.clone();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            (cloned, inbox.entries.len())
        };

        self.emit(UiEvent::handoff_inbox_updated(
            "dismissed",
            Some(id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_dismissed",
            Some(entry.sender_id.clone()),
            format!("Dismissed resource handoff '{}' ({})", entry.title, id),
            true,
        );

        Ok(entry)
    }

    /// Clear all records in the resource handoff inbox.
    pub fn handoff_inbox_clear(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let cleared = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let count = inbox.clear();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            count
        };

        self.emit(UiEvent::handoff_inbox_updated(
            "cleared",
            None::<String>,
            0,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_inbox_cleared",
            None::<String>,
            format!("Cleared {cleared} entries from handoff inbox"),
            true,
        );

        Ok(cleared)
    }

    /// Read the current inbound media control policy.
    pub fn media_policy(&self) -> MediaControlPolicy {
        self.shared
            .media_policy
            .lock()
            .expect("media_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for media control policy.
    pub fn media_policy_report(&self) -> MediaPolicyReport {
        MediaPolicyReport {
            ok: true,
            policy: self.media_policy(),
            message: "media policy retrieved successfully".to_string(),
        }
    }

    /// Update the media control policy and persist to disk atomically with 0o600 permissions.
    pub fn set_media_policy(
        &self,
        policy: MediaControlPolicy,
    ) -> Result<MediaPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_media_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save media_policy.json: {e}")))?;
        *self.shared.media_policy.lock().expect("media_policy") = policy.clone();

        self.emit(UiEvent::media_policy_changed(
            policy.mode.as_str(),
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_volume_limit,
            policy.allow_seek,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "media_policy_changed",
            None::<String>,
            format!(
                "Updated media control policy: mode={}, max_volume_limit={}, allow_seek={}, groups={:?}, tags={:?}, types={:?}",
                policy.mode.as_str(),
                policy.max_volume_limit,
                policy.allow_seek,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(MediaPolicyReport {
            ok: true,
            policy,
            message: "media control policy updated successfully".to_string(),
        })
    }

    /// Query current playback state for local device or a remote peer.
    pub fn media_get_state(
        &self,
        peer_target: Option<&str>,
    ) -> Result<MediaStatusReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?.to_string())
            }
        } else {
            None
        };

        let state = match resolved_peer.as_deref() {
            Some(peer_id) => {
                let states = self
                    .shared
                    .peer_media_states
                    .lock()
                    .expect("peer_media_states");
                states.get(peer_id).cloned()
            }
            None => {
                let st = self
                    .shared
                    .local_media_state
                    .lock()
                    .expect("local_media_state");
                Some(st.clone())
            }
        };

        Ok(MediaStatusReport {
            ok: true,
            peer: resolved_peer,
            state,
        })
    }

    /// Update local media playback state, sanitize metadata, enforce bounds, and broadcast to peers.
    pub fn media_update_local_state(
        &self,
        mut state: MediaStateFrame,
    ) -> Result<MediaStateFrame, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        state.title =
            MediaValidator::sanitize_text(&state.title, tetherly_core::MAX_MEDIA_TITLE_LEN);
        state.artist =
            MediaValidator::sanitize_text(&state.artist, tetherly_core::MAX_MEDIA_ARTIST_LEN);
        state.album =
            MediaValidator::sanitize_text(&state.album, tetherly_core::MAX_MEDIA_ALBUM_LEN);
        state.player_id =
            MediaValidator::sanitize_text(&state.player_id, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN);
        if state.player_id.is_empty() {
            state.player_id = "default".to_string();
        }
        state.volume_percent = MediaValidator::clamp_volume(state.volume_percent);
        state.timestamp_ms = now_ms;

        {
            let mut local = self
                .shared
                .local_media_state
                .lock()
                .expect("local_media_state");
            *local = state.clone();
        }

        let local_id = self.shared.identity.device_id().to_string();
        let name = self.name();

        self.emit(UiEvent::media_state_changed(
            &local_id,
            &name,
            &state.player_id,
            state.state.as_str(),
            &state.title,
            &state.artist,
            state.volume_percent,
            state.muted,
            now_ms,
        ));

        // Broadcast to all live sessions so remote peers have immediate synchronized playback state
        let frame_opt = state.to_frame().ok();
        if let Some(frame) = frame_opt {
            let node = self.clone_handle();
            tokio::spawn(async move {
                let live = node.live_peers();
                for peer in live {
                    let _ = node.send_to(&peer.device_id, frame.clone()).await;
                }
            });
        }

        Ok(state)
    }

    /// Dispatch a media playback command to a specific peer over the Noise session.
    pub async fn media_send_command(
        &self,
        peer_target: &str,
        command: MediaCommand,
        target_player: Option<String>,
    ) -> Result<MediaCommandSendResult, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let now_ms = self.shared.clock.unix_ms();
        let seq = self.shared.media_cmd_seq.fetch_add(1, Ordering::Relaxed);
        let cmd_id = format!("cmd_{}_{}", now_ms, seq);
        let cmd_label = command.as_label().to_string();

        let clean_player = target_player
            .map(|p| MediaValidator::sanitize_text(&p, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN));

        let frame = MediaCommandFrame {
            id: cmd_id.clone(),
            command: command.clone(),
            target_player: clean_player.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self.peer_alias(&peer_id);

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_command_sent",
            Some(peer_str.clone()),
            format!(
                "Sent media command '{}' to peer '{}' (id: {})",
                cmd_label, peer_str, cmd_id
            ),
            true,
        );

        self.emit(UiEvent::media_command_dispatched(
            &cmd_id, &peer_str, &cmd_label, true, now_ms,
        ));

        Ok(MediaCommandSendResult {
            ok: true,
            id: cmd_id,
            peer: peer_str,
            peer_alias: alias,
            command: cmd_label,
            target_player: clean_player,
            timestamp_ms: now_ms,
        })
    }

    /// Fan out a media playback command to multiple peers matching a target selector.
    pub async fn media_send_command_multi(
        &self,
        selector: &TargetSelector,
        command: MediaCommand,
        target_player: Option<String>,
    ) -> Result<MultiMediaCommandReport, CoreError> {
        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now_ms = self.shared.clock.unix_ms();
        let cmd_label = command.as_label().to_string();
        let clean_player = target_player
            .map(|p| MediaValidator::sanitize_text(&p, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN));

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerMediaCommandSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    command_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = self.shared.media_cmd_seq.fetch_add(1, Ordering::Relaxed);
            let cmd_id = format!("cmd_{}_{}", now_ms, seq);
            let frame = MediaCommandFrame {
                id: cmd_id.clone(),
                command: command.clone(),
                target_player: clean_player.clone(),
                timestamp_ms: now_ms,
            };

            match frame.to_frame() {
                Ok(inner_frame) => match self.send_to(peer_id, inner_frame).await {
                    Ok(_) => {
                        success_count += 1;
                        self.emit(UiEvent::media_command_dispatched(
                            &cmd_id, &peer_str, &cmd_label, true, now_ms,
                        ));
                        dispatches.push(PeerMediaCommandSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            command_id: Some(cmd_id),
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        failed_count += 1;
                        dispatches.push(PeerMediaCommandSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            command_id: Some(cmd_id),
                            success: false,
                            error: Some(e.to_string()),
                        });
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerMediaCommandSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        command_id: Some(cmd_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_multi_sent",
            None::<String>,
            format!(
                "Multicast media command '{}' via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                cmd_label,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "media",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiMediaCommandReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            command: cmd_label,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest and process an inbound media control command frame from a connected peer.
    pub fn receive_media_command(&self, peer: &DeviceId, mut frame: MediaCommandFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        let policy = self.media_policy();

        // 1. Ingress mode gate
        if !policy.mode.is_enabled() {
            debug!(peer = %peer, "inbound media command ignored: policy is disabled");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "media_command_blocked",
                Some(peer_str.clone()),
                format!("Blocked media command from {peer} due to disabled policy"),
                false,
            );
            return;
        }

        // 2. Profile eligibility check
        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound media command blocked: sender profile not eligible");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Warning,
                "media_command_blocked",
                Some(peer_str.clone()),
                format!(
                    "Blocked media command from {peer}: sender profile not eligible under policy"
                ),
                false,
            );
            return;
        }

        // 3. Command specific guards (seek disallowed, volume clamping)
        match &mut frame.command {
            MediaCommand::Seek { .. } => {
                if !policy.allow_seek {
                    debug!(peer = %peer, "inbound media seek command blocked: seek is disabled by policy");
                    self.record_audit(
                        AuditCategory::Peer,
                        AuditSeverity::Notice,
                        "media_command_blocked",
                        Some(peer_str.clone()),
                        format!("Blocked media seek command from {peer}: seek disabled by policy"),
                        false,
                    );
                    return;
                }
            }
            MediaCommand::SetVolume {
                ref mut volume_percent,
            } => {
                let clamped = policy.clamp_volume(*volume_percent);
                *volume_percent = clamped;
            }
            _ => {}
        }

        let cmd_label = frame.command.as_label().to_string();

        // 4. Update local player state machine
        let updated_state = {
            let mut state = self
                .shared
                .local_media_state
                .lock()
                .expect("local_media_state");
            match &frame.command {
                MediaCommand::Play => {
                    state.state = PlaybackState::Playing;
                    state.can_play = false;
                    state.can_pause = true;
                }
                MediaCommand::Pause => {
                    state.state = PlaybackState::Paused;
                    state.can_play = true;
                    state.can_pause = false;
                }
                MediaCommand::PlayPause => {
                    if state.state == PlaybackState::Playing {
                        state.state = PlaybackState::Paused;
                        state.can_play = true;
                        state.can_pause = false;
                    } else {
                        state.state = PlaybackState::Playing;
                        state.can_play = false;
                        state.can_pause = true;
                    }
                }
                MediaCommand::Stop => {
                    state.state = PlaybackState::Stopped;
                    state.position_ms = 0;
                    state.can_play = true;
                    state.can_pause = false;
                }
                MediaCommand::Next => {
                    state.position_ms = 0;
                }
                MediaCommand::Previous => {
                    state.position_ms = 0;
                }
                MediaCommand::Seek { offset_ms } => {
                    let new_pos = (state.position_ms as i64 + offset_ms).max(0) as u64;
                    state.position_ms = if state.duration_ms > 0 {
                        new_pos.min(state.duration_ms)
                    } else {
                        new_pos
                    };
                }
                MediaCommand::SetVolume { volume_percent } => {
                    state.volume_percent = *volume_percent;
                    if *volume_percent > 0 && state.muted {
                        state.muted = false;
                    }
                }
                MediaCommand::ToggleMute => {
                    state.muted = !state.muted;
                }
            }
            state.timestamp_ms = now_ms;
            state.clone()
        };

        // 5. Zero-Plaintext observability events
        self.emit(UiEvent::media_command_received(
            &frame.id,
            &peer_str,
            &alias,
            &cmd_label,
            frame.target_player.as_deref(),
            now_ms,
        ));

        let local_id = self.shared.identity.device_id().to_string();
        let local_name = self.name();

        self.emit(UiEvent::media_state_changed(
            &local_id,
            &local_name,
            &updated_state.player_id,
            updated_state.state.as_str(),
            &updated_state.title,
            &updated_state.artist,
            updated_state.volume_percent,
            updated_state.muted,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_command_executed",
            Some(peer_str),
            format!(
                "Executed media command '{}' from {} (state: {})",
                cmd_label,
                alias,
                updated_state.state.as_str()
            ),
            true,
        );
    }

    /// Ingest and cache an inbound media playback state frame from a connected peer.
    pub fn receive_media_state(&self, peer: &DeviceId, mut frame: MediaStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.title =
            MediaValidator::sanitize_text(&frame.title, tetherly_core::MAX_MEDIA_TITLE_LEN);
        frame.artist =
            MediaValidator::sanitize_text(&frame.artist, tetherly_core::MAX_MEDIA_ARTIST_LEN);
        frame.album =
            MediaValidator::sanitize_text(&frame.album, tetherly_core::MAX_MEDIA_ALBUM_LEN);
        frame.player_id =
            MediaValidator::sanitize_text(&frame.player_id, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN);
        if frame.player_id.is_empty() {
            frame.player_id = "default".to_string();
        }
        frame.volume_percent = MediaValidator::clamp_volume(frame.volume_percent);
        frame.timestamp_ms = now_ms;

        {
            let mut states = self
                .shared
                .peer_media_states
                .lock()
                .expect("peer_media_states");
            states.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::media_state_changed(
            &peer_str,
            &alias,
            &frame.player_id,
            frame.state.as_str(),
            &frame.title,
            &frame.artist,
            frame.volume_percent,
            frame.muted,
            now_ms,
        ));
    }

    /// Read the currently active battery and power management policy.
    pub fn battery_policy(&self) -> BatteryPolicy {
        self.shared
            .battery_policy
            .lock()
            .expect("battery_policy")
            .clone()
    }

    /// Status and configuration report for battery policy.
    pub fn battery_policy_report(&self) -> BatteryPolicyReport {
        BatteryPolicyReport {
            ok: true,
            policy: self.battery_policy(),
            message: "current battery policy".to_string(),
        }
    }

    /// Update and persist battery management policy with POSIX 0o600 permissions and atomic consistency.
    pub fn set_battery_policy(
        &self,
        mut policy: BatteryPolicy,
    ) -> Result<BatteryPolicyReport, CoreError> {
        policy.sanitize();
        save_battery_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save battery_policy.json: {e}")))?;
        {
            let mut p = self.shared.battery_policy.lock().expect("battery_policy");
            *p = policy.clone();
        }

        let now = self.shared.clock.unix_ms();
        self.emit(UiEvent::battery_policy_changed(
            policy.low_battery_threshold,
            policy.critical_battery_threshold,
            policy.alert_on_low_battery,
            policy.auto_throttle_on_battery_saver,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "battery_policy_changed",
            None::<String>,
            format!(
                "Updated battery policy: low_threshold={}, critical_threshold={}, alert={}, throttle={}, groups={:?}, tags={:?}, types={:?}",
                policy.low_battery_threshold,
                policy.critical_battery_threshold,
                policy.alert_on_low_battery,
                policy.auto_throttle_on_battery_saver,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(BatteryPolicyReport {
            ok: true,
            policy,
            message: "battery policy updated successfully".to_string(),
        })
    }

    /// Query current battery and power state for local device or a remote peer.
    pub fn battery_get_state(
        &self,
        peer_target: Option<&str>,
    ) -> Result<BatteryStatusReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?.to_string())
            }
        } else {
            None
        };

        let state = match resolved_peer.as_deref() {
            Some(peer_id) => {
                let states = self
                    .shared
                    .peer_battery_states
                    .lock()
                    .expect("peer_battery_states");
                states.get(peer_id).cloned()
            }
            None => {
                let st = self
                    .shared
                    .local_battery_state
                    .lock()
                    .expect("local_battery_state");
                Some(st.clone())
            }
        };

        Ok(BatteryStatusReport {
            ok: true,
            peer: resolved_peer,
            state,
        })
    }

    /// Update local battery state, clamp bounds, and broadcast TYPE_BATTERY_STATE to all online mesh peers.
    pub async fn battery_update_local_state(
        &self,
        mut state: BatteryStateFrame,
    ) -> Result<BatteryBroadcastReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        state.battery_level = BatteryValidator::clamp_level(state.battery_level);
        state.health_percent = BatteryValidator::clamp_health(state.health_percent);
        state.timestamp_ms = now_ms;

        {
            let mut local = self
                .shared
                .local_battery_state
                .lock()
                .expect("local_battery_state");
            *local = state.clone();
        }

        let local_id = self.shared.identity.device_id().to_string();
        let name = self.name();

        self.emit(UiEvent::battery_state_changed(
            &local_id,
            &name,
            state.battery_level,
            state.is_charging,
            state.power_source.as_str(),
            state.battery_saver,
            state.health_percent,
            now_ms,
        ));

        let frame = state.to_inner_frame()?;
        let live = self.live_peers();
        let mut sent_count = 0;
        for peer in live {
            if self.send_to(&peer.device_id, frame.clone()).await.is_ok() {
                sent_count += 1;
            }
        }

        Ok(BatteryBroadcastReport {
            ok: true,
            state,
            broadcast_count: sent_count,
            message: format!("battery state updated and broadcasted to {sent_count} peers"),
        })
    }

    /// Ingest and cache an inbound battery state frame from a connected peer, triggering low/critical battery alerts.
    pub fn receive_battery_state(&self, peer: &DeviceId, mut frame: BatteryStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.battery_level = BatteryValidator::clamp_level(frame.battery_level);
        frame.health_percent = BatteryValidator::clamp_health(frame.health_percent);
        frame.timestamp_ms = now_ms;

        let policy = self.battery_policy();
        let profile = self.peer_profile(peer);

        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound battery update ignored: sender profile not eligible under battery policy");
            return;
        }

        {
            let mut states = self
                .shared
                .peer_battery_states
                .lock()
                .expect("peer_battery_states");
            states.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::battery_state_changed(
            &peer_str,
            &alias,
            frame.battery_level,
            frame.is_charging,
            frame.power_source.as_str(),
            frame.battery_saver,
            frame.health_percent,
            now_ms,
        ));

        // Low / Critical battery alert checking when discharging
        if policy.alert_on_low_battery && !frame.is_charging {
            if frame.battery_level <= policy.critical_battery_threshold {
                self.emit(UiEvent::battery_low_alert(
                    &peer_str,
                    &alias,
                    frame.battery_level,
                    true,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Alert,
                    "battery_critical_alert",
                    Some(peer_str.clone()),
                    format!(
                        "CRITICAL battery alert for {}: level at {}% (threshold: {}%)",
                        alias, frame.battery_level, policy.critical_battery_threshold
                    ),
                    true,
                );
            } else if frame.battery_level <= policy.low_battery_threshold {
                self.emit(UiEvent::battery_low_alert(
                    &peer_str,
                    &alias,
                    frame.battery_level,
                    false,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Warning,
                    "battery_low_alert",
                    Some(peer_str.clone()),
                    format!(
                        "Low battery alert for {}: level at {}% (threshold: {}%)",
                        alias, frame.battery_level, policy.low_battery_threshold
                    ),
                    true,
                );
            }
        }
    }

    /// Read the current device locator policy.
    pub fn locator_policy(&self) -> LocatorPolicy {
        self.shared
            .locator_policy
            .lock()
            .expect("locator_policy")
            .clone()
    }

    /// Status and configuration report for device locator policy.
    pub fn locator_policy_report(&self) -> LocatorPolicyReport {
        LocatorPolicyReport {
            ok: true,
            policy: self.locator_policy(),
            message: "locator policy report".to_string(),
        }
    }

    /// Update the device locator policy and persist to disk atomically with 0o600 permissions.
    pub fn set_locator_policy(
        &self,
        mut policy: LocatorPolicy,
    ) -> Result<LocatorPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_locator_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save locator_policy.json: {e}")))?;
        *self.shared.locator_policy.lock().expect("locator_policy") = policy.clone();

        self.emit(UiEvent::locate_policy_changed(
            policy.enabled,
            policy.allow_override_silent,
            policy.max_duration_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "locate_policy_changed",
            None::<String>,
            format!(
                "Updated locator policy: enabled={}, allow_override_silent={}, max_duration_secs={}, groups={:?}, tags={:?}, types={:?}",
                policy.enabled,
                policy.allow_override_silent,
                policy.max_duration_secs,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(LocatorPolicyReport {
            ok: true,
            policy,
            message: "locator policy updated successfully".to_string(),
        })
    }

    /// Query current device locator status and active ringing state.
    pub fn locate_status(&self) -> LocateStatusReport {
        let active = self
            .shared
            .active_locate_ring
            .lock()
            .expect("active_locate_ring")
            .clone();
        let is_ringing = active.is_some();
        let message = if is_ringing {
            "device locator active ring in progress".to_string()
        } else {
            "device locator idle".to_string()
        };
        LocateStatusReport {
            ok: true,
            is_ringing,
            active_ring: active,
            policy: self.locator_policy(),
            message,
        }
    }

    /// Trigger acoustic ringing on a remote peer to locate it.
    pub async fn locate_ring(
        &self,
        peer_target: &str,
        duration_secs: Option<u32>,
        pattern: Option<LocatePattern>,
        override_silent: Option<bool>,
        message: Option<String>,
    ) -> Result<LocateRingReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let live = self.live_peers();
        if !live.iter().any(|p| p.device_id == peer_id) {
            return Err(CoreError::PeerNotFound(format!(
                "peer {} is not connected or offline",
                peer_id
            )));
        }

        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer_id.to_string();

        // Anti-harassment rate limiting: 10s cooldown per peer target
        {
            let mut last_map = self
                .shared
                .peer_last_locate_req_ms
                .lock()
                .expect("peer_last_locate_req_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 10_000 {
                    let wait_secs = (10_000 - now_ms.saturating_sub(last_ts)).div_ceil(1000);
                    return Err(CoreError::InvalidPayload(format!(
                        "anti-harassment rate limit: please wait at least {}s before ringing peer '{}' again",
                        wait_secs, peer_str
                    )));
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let seq = self.shared.locate_req_seq.fetch_add(1, Ordering::Relaxed);
        let request_id = format!("loc_{}_{}", now_ms, seq);

        let duration =
            LocateValidator::clamp_duration(duration_secs.unwrap_or(DEFAULT_LOCATE_DURATION_SECS));
        let pattern_val = pattern.unwrap_or_default();
        let override_val = override_silent.unwrap_or(true);
        let clean_msg = message.as_deref().map(LocateValidator::sanitize_message);

        let frame = LocateRequestFrame {
            request_id: request_id.clone(),
            duration_secs: duration,
            pattern: pattern_val,
            override_silent: override_val,
            message: clean_msg.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let peer_str = peer_id.to_string();

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "locate_ring_sent",
            Some(peer_str.clone()),
            format!(
                "Sent acoustic locate request {} (pattern: {}, duration: {}s, override_silent: {}) to peer '{}'",
                request_id,
                pattern_val.as_str(),
                duration,
                override_val,
                peer_str
            ),
            true,
        );

        Ok(LocateRingReport {
            ok: true,
            request_id,
            target_peer: peer_str,
            duration_secs: duration,
            pattern: pattern_val,
            override_silent: override_val,
            message: clean_msg.unwrap_or_default(),
        })
    }

    /// Stop active acoustic ringing either locally or by sending TYPE_LOCATE_STOP to a remote peer.
    pub async fn locate_stop(
        &self,
        peer_target: Option<&str>,
        request_id: Option<String>,
        reason: Option<String>,
    ) -> Result<LocateStopReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let clean_reason = reason
            .as_deref()
            .map(LocateValidator::sanitize_reason)
            .unwrap_or_else(|| "manual_stop".to_string());

        let target_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?)
            }
        } else {
            None
        };

        if let Some(peer_id) = target_peer {
            let clean_req_id = request_id
                .as_deref()
                .map(LocateValidator::sanitize_request_id);
            let frame = LocateStopFrame {
                request_id: clean_req_id.clone(),
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            self.send_to(&peer_id, frame.to_inner_frame()?).await?;

            let peer_str = peer_id.to_string();

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "locate_stop_sent",
                Some(peer_str.clone()),
                format!(
                    "Sent locate stop to peer '{}' (request_id: {:?}, reason: '{}')",
                    peer_str, clean_req_id, clean_reason
                ),
                true,
            );

            Ok(LocateStopReport {
                ok: true,
                request_id: clean_req_id,
                target_peer: Some(peer_str),
                reason: clean_reason,
                message: "locate stop request dispatched to peer".to_string(),
            })
        } else {
            // Local stop
            let stopped_ring = {
                let mut active = self
                    .shared
                    .active_locate_ring
                    .lock()
                    .expect("active_locate_ring");
                if let Some(ring) = active.as_ref() {
                    if request_id.is_none() || request_id.as_deref() == Some(&ring.request_id) {
                        active.take()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let (req_id, peer_str) = if let Some(ring) = stopped_ring {
                self.emit(UiEvent::locate_ring_stopped(
                    Some(&ring.request_id),
                    &ring.initiator_peer,
                    &ring.initiator_alias,
                    &clean_reason,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Info,
                    "locate_ring_stopped_locally",
                    Some(ring.initiator_peer.clone()),
                    format!(
                        "Locally stopped active acoustic ring {} from {} (reason: '{}')",
                        ring.request_id, ring.initiator_alias, clean_reason
                    ),
                    true,
                );
                (Some(ring.request_id), Some(ring.initiator_peer))
            } else {
                (request_id, None)
            };

            let message = if peer_str.is_some() {
                "local acoustic ring stopped".to_string()
            } else {
                "no active acoustic ring to stop".to_string()
            };

            Ok(LocateStopReport {
                ok: true,
                request_id: req_id,
                target_peer: peer_str,
                reason: clean_reason,
                message,
            })
        }
    }

    /// Ingest and process an inbound acoustic locate request from a connected peer.
    pub fn receive_locate_request(&self, peer: &DeviceId, frame: LocateRequestFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.locator_policy();
        if !policy.enabled {
            debug!(peer = %peer, "inbound locate request ignored: locator service disabled by policy");
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "locate_request_disabled",
                Some(peer_str.clone()),
                format!(
                    "Ignored locate request from {} because locator policy is disabled",
                    peer_str
                ),
                false,
            );
            return;
        }

        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound locate request rejected: peer profile not eligible under locator policy");
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "locate_request_ineligible",
                Some(peer_str.clone()),
                format!(
                    "Rejected locate request from {} due to profile scoping rules",
                    peer_str
                ),
                false,
            );
            return;
        }

        // Anti-harassment rate limiting: 10s cooldown per peer
        {
            let mut last_map = self
                .shared
                .peer_last_locate_req_ms
                .lock()
                .expect("peer_last_locate_req_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 10_000 {
                    warn!(peer = %peer, "inbound locate request rate-limited (<10s cooldown)");
                    self.record_audit(
                        AuditCategory::Security,
                        AuditSeverity::Warning,
                        "locate_request_rate_limited",
                        Some(peer_str.clone()),
                        format!(
                            "Rate-limited locate request from {} (<10s cooldown)",
                            peer_str
                        ),
                        false,
                    );
                    return;
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let clamped_duration =
            LocateValidator::clamp_duration(frame.duration_secs).min(policy.max_duration_secs);
        let override_silent = frame.override_silent && policy.allow_override_silent;
        let clean_msg = frame
            .message
            .as_deref()
            .map(LocateValidator::sanitize_message);

        let ring = ActiveLocateRing {
            request_id: frame.request_id.clone(),
            initiator_peer: peer_str.clone(),
            initiator_alias: alias.clone(),
            duration_secs: clamped_duration,
            pattern: frame.pattern,
            override_silent,
            message: clean_msg.clone(),
            started_at_ms: now_ms,
            expires_at_ms: now_ms + (clamped_duration as u64) * 1000,
        };

        {
            let mut active = self
                .shared
                .active_locate_ring
                .lock()
                .expect("active_locate_ring");
            *active = Some(ring);
        }

        self.emit(UiEvent::locate_ring_triggered(
            &frame.request_id,
            &peer_str,
            &alias,
            clamped_duration,
            frame.pattern.as_str(),
            override_silent,
            clean_msg.as_deref(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Alert,
            "locate_ring_triggered",
            Some(peer_str.clone()),
            format!(
                "Acoustic locate ring triggered by {} (pattern: {}, duration: {}s, override_silent: {})",
                alias,
                frame.pattern.as_str(),
                clamped_duration,
                override_silent
            ),
            true,
        );

        // Auto expiration task
        let shared = Arc::clone(&self.shared);
        let req_id = frame.request_id.clone();
        let wait_secs = clamped_duration;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                tokio::time::sleep(Duration::from_secs(wait_secs as u64)).await;
                let expired_ring = {
                    let mut active = shared
                        .active_locate_ring
                        .lock()
                        .expect("active_locate_ring");
                    if let Some(r) = active.as_ref() {
                        if r.request_id == req_id {
                            active.take()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(r) = expired_ring {
                    let exp_now = shared.clock.unix_ms();
                    let _ = shared.events.send(UiEvent::locate_ring_stopped(
                        Some(&r.request_id),
                        &r.initiator_peer,
                        &r.initiator_alias,
                        "duration_expired",
                        exp_now,
                    ));
                }
            });
        }
    }

    /// Ingest and process an inbound locate stop frame from a connected peer.
    pub fn receive_locate_stop(&self, peer: &DeviceId, frame: LocateStopFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        let clean_reason = LocateValidator::sanitize_reason(&frame.reason);

        let stopped_ring = {
            let mut active = self
                .shared
                .active_locate_ring
                .lock()
                .expect("active_locate_ring");
            if let Some(ring) = active.as_ref() {
                if frame.request_id.is_none()
                    || frame.request_id.as_deref() == Some(&ring.request_id)
                {
                    active.take()
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(ring) = stopped_ring {
            self.emit(UiEvent::locate_ring_stopped(
                Some(&ring.request_id),
                &ring.initiator_peer,
                &ring.initiator_alias,
                &clean_reason,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "locate_ring_stopped_remote",
                Some(peer_str),
                format!(
                    "Acoustic locate ring {} stopped by remote peer {} (reason: '{}')",
                    ring.request_id, alias, clean_reason
                ),
                true,
            );
        }
    }

    /// Returns the currently active action policy.
    pub fn action_policy(&self) -> ActionPolicy {
        self.shared
            .action_policy
            .lock()
            .expect("action_policy")
            .clone()
    }

    /// Status and configuration report for action execution policy.
    pub fn action_policy_report(&self) -> ActionPolicyReport {
        ActionPolicyReport {
            ok: true,
            policy: self.action_policy(),
            message: "action policy report".to_string(),
        }
    }

    /// Update the action policy and persist to disk atomically with 0o600 permissions.
    pub fn set_action_policy(
        &self,
        mut policy: ActionPolicy,
    ) -> Result<ActionPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_action_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save action_policy.json: {e}")))?;
        *self.shared.action_policy.lock().expect("action_policy") = policy.clone();

        self.emit(UiEvent::action_policy_changed(
            policy.mode.as_str(),
            policy.max_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.expose_action_list,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "action_policy_changed",
            None::<String>,
            format!(
                "Updated action policy: mode={}, max_timeout_secs={}, expose_action_list={}, groups={:?}, tags={:?}, types={:?}",
                policy.mode.as_str(),
                policy.max_timeout_secs,
                policy.expose_action_list,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(ActionPolicyReport {
            ok: true,
            policy,
            message: "action policy updated successfully".to_string(),
        })
    }

    /// Returns all registered local action definitions.
    pub fn local_actions(&self) -> Vec<LocalActionDefinition> {
        self.shared
            .local_actions
            .lock()
            .expect("local_actions")
            .clone()
    }

    /// Report for local registered actions.
    pub fn local_action_report(&self) -> LocalActionReport {
        let actions = self.local_actions();
        LocalActionReport {
            ok: true,
            count: actions.len(),
            actions,
            message: "local actions report".to_string(),
        }
    }

    /// Set all local actions atomically, persisting to actions.json with 0o600 permissions.
    pub fn set_local_actions(
        &self,
        mut actions: Vec<LocalActionDefinition>,
    ) -> Result<LocalActionReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        let mut seen = std::collections::HashSet::new();
        for action in &mut actions {
            action.id = ActionValidator::validate_action_id(&action.id)?;
            if !seen.insert(action.id.clone()) {
                return Err(CoreError::InvalidPayload(format!(
                    "duplicate action id: {}",
                    action.id
                )));
            }
            action.name = ActionValidator::sanitize_text(&action.name, 64);
            if action.name.is_empty() {
                action.name = action.id.clone();
            }
            action.description = action
                .description
                .as_deref()
                .map(|d| ActionValidator::sanitize_text(d, 256));
            action.icon = action
                .icon
                .as_deref()
                .map(|i| ActionValidator::sanitize_text(i, 64));
            action.timeout_secs = ActionValidator::clamp_timeout(action.timeout_secs);
            if action.command.trim().is_empty() {
                return Err(CoreError::InvalidPayload(
                    "action command cannot be empty".to_string(),
                ));
            }
        }

        save_local_actions(&self.shared.cfg.data_dir, &actions)
            .map_err(|e| CoreError::File(format!("failed to save actions.json: {e}")))?;
        *self.shared.local_actions.lock().expect("local_actions") = actions.clone();

        self.emit(UiEvent::local_actions_changed(actions.len(), now));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "local_actions_updated",
            None::<String>,
            format!(
                "Updated local action table ({} actions registered)",
                actions.len()
            ),
            true,
        );

        Ok(LocalActionReport {
            ok: true,
            count: actions.len(),
            actions,
            message: "local actions updated successfully".to_string(),
        })
    }

    /// Register or update a single local action definition.
    pub fn add_local_action(
        &self,
        mut action: LocalActionDefinition,
    ) -> Result<LocalActionReport, CoreError> {
        action.id = ActionValidator::validate_action_id(&action.id)?;
        action.name = ActionValidator::sanitize_text(&action.name, 64);
        if action.name.is_empty() {
            action.name = action.id.clone();
        }
        action.description = action
            .description
            .as_deref()
            .map(|d| ActionValidator::sanitize_text(d, 256));
        action.icon = action
            .icon
            .as_deref()
            .map(|i| ActionValidator::sanitize_text(i, 64));
        action.timeout_secs = ActionValidator::clamp_timeout(action.timeout_secs);
        if action.command.trim().is_empty() {
            return Err(CoreError::InvalidPayload(
                "action command cannot be empty".to_string(),
            ));
        }

        let mut current = self.local_actions();
        if let Some(pos) = current.iter().position(|a| a.id == action.id) {
            current[pos] = action;
        } else {
            current.push(action);
        }
        self.set_local_actions(current)
    }

    /// Remove a registered local action by id.
    pub fn remove_local_action(&self, action_id: &str) -> Result<LocalActionReport, CoreError> {
        let clean_id = ActionValidator::validate_action_id(action_id)?;
        let mut current = self.local_actions();
        let initial_len = current.len();
        current.retain(|a| a.id != clean_id);
        if current.len() == initial_len {
            return Err(CoreError::InvalidPayload(format!(
                "action '{}' not found",
                clean_id
            )));
        }
        self.set_local_actions(current)
    }

    /// Get cached action items from a remote peer.
    pub fn remote_actions(&self, peer_target: &str) -> Result<RemoteActionReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let peer_str = peer_id.to_string();
        let cache = self
            .shared
            .peer_remote_actions
            .lock()
            .expect("peer_remote_actions");
        let actions = cache.get(&peer_str).cloned().unwrap_or_default();
        Ok(RemoteActionReport {
            ok: true,
            peer: peer_str,
            count: actions.len(),
            actions,
            message: "remote actions report".to_string(),
        })
    }

    /// Query actions from a remote peer over wire protocol.
    pub async fn query_remote_actions(
        &self,
        peer_target: &str,
    ) -> Result<RemoteActionReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let now_ms = self.shared.clock.unix_ms();
        let seq = self
            .shared
            .action_trigger_seq
            .fetch_add(1, Ordering::Relaxed);
        let request_id = format!("act_req_{}_{}", now_ms, seq);

        let frame = ActionListReqFrame {
            request_id,
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::action_list_requested(&peer_str, &alias, now_ms));

        self.remote_actions(&peer_str)
    }

    /// Trigger execution of a predefined quick action on a remote peer.
    pub async fn trigger_remote_action(
        &self,
        peer_target: &str,
        action_id: &str,
    ) -> Result<ActionTriggerReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let clean_action_id = ActionValidator::validate_action_id(action_id)?;
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer_id.to_string();

        // 5-second anti-harassment rate limiting per peer
        {
            let mut last_map = self
                .shared
                .peer_last_action_trigger_ms
                .lock()
                .expect("peer_last_action_trigger_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 5_000 {
                    let wait_secs = (5_000 - now_ms.saturating_sub(last_ts)).div_ceil(1000);
                    return Err(CoreError::InvalidPayload(format!(
                        "rate limit: please wait at least {}s before triggering action on peer '{}' again",
                        wait_secs, peer_str
                    )));
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let seq = self
            .shared
            .action_trigger_seq
            .fetch_add(1, Ordering::Relaxed);
        let trigger_id = format!("trig_{}_{}", now_ms, seq);

        let frame = ActionTriggerFrame {
            trigger_id: trigger_id.clone(),
            action_id: clean_action_id.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::action_triggered(
            &trigger_id,
            &clean_action_id,
            &peer_str,
            &alias,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_trigger_sent",
            Some(peer_str.clone()),
            format!(
                "Sent action trigger request {} (action_id: '{}') to peer '{}'",
                trigger_id, clean_action_id, peer_str
            ),
            true,
        );

        Ok(ActionTriggerReport {
            ok: true,
            trigger_id,
            action_id: clean_action_id,
            target_peer: peer_str,
            status: "triggered".to_string(),
            message: "action trigger sent to peer".to_string(),
        })
    }

    /// Returns bounded history of recent action execution results.
    pub fn action_history(&self) -> ActionHistoryReport {
        let cache = self
            .shared
            .action_results_cache
            .lock()
            .expect("action_results_cache");
        let results = cache.iter().cloned().collect::<Vec<_>>();
        ActionHistoryReport {
            ok: true,
            count: results.len(),
            results,
            message: "action results history report".to_string(),
        }
    }

    /// Ingest and process an inbound ActionListReqFrame from a connected peer.
    pub fn receive_action_list_req(&self, peer: &DeviceId, frame: ActionListReqFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.action_policy();
        let profile = self.peer_profile(peer);

        let actions = if policy.mode == ActionAccessMode::Disabled
            || !policy.expose_action_list
            || !policy.is_peer_eligible(&profile)
        {
            Vec::new()
        } else {
            let local = self.local_actions();
            local
                .into_iter()
                .filter(|a| a.enabled)
                .map(|a| a.to_item())
                .collect()
        };

        self.emit(UiEvent::action_list_requested(&peer_str, &alias, now_ms));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_list_requested",
            Some(peer_str.clone()),
            format!(
                "Peer {} requested action list (advertised {} actions)",
                peer_str,
                actions.len()
            ),
            true,
        );

        let resp = ActionListRespFrame {
            request_id: frame.request_id,
            actions,
            timestamp_ms: now_ms,
        };

        let node = self.clone_handle();
        let target = peer.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(inner) = resp.to_inner_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Ingest and process an inbound ActionListRespFrame from a connected peer.
    pub fn receive_action_list_resp(&self, peer: &DeviceId, mut frame: ActionListRespFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        // Sanitize incoming action items
        for item in &mut frame.actions {
            item.id = ActionValidator::clean_identifier(&item.id, 64);
            item.name = ActionValidator::sanitize_text(&item.name, 64);
            item.description = item
                .description
                .as_deref()
                .map(|d| ActionValidator::sanitize_text(d, 256));
            item.icon = item
                .icon
                .as_deref()
                .map(|i| ActionValidator::sanitize_text(i, 64));
        }

        let count = frame.actions.len();
        {
            let mut cache = self
                .shared
                .peer_remote_actions
                .lock()
                .expect("peer_remote_actions");
            cache.insert(peer_str.clone(), frame.actions);
        }

        self.emit(UiEvent::action_list_received(
            &peer_str, &alias, count, now_ms,
        ));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_list_received",
            Some(peer_str),
            format!("Received {} advertised actions from peer {}", count, alias),
            true,
        );
    }

    /// Ingest and process an inbound ActionTriggerFrame from a connected peer.
    pub fn receive_action_trigger(&self, peer: &DeviceId, frame: ActionTriggerFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.action_policy();
        let profile = self.peer_profile(peer);

        // Anti-harassment rate limit check (5s cooldown per peer)
        {
            let mut last_map = self
                .shared
                .peer_last_action_trigger_ms
                .lock()
                .expect("peer_last_action_trigger_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 5_000 {
                    warn!(peer = %peer, "inbound action trigger rate-limited (<5s cooldown)");
                    self.record_audit(
                        AuditCategory::Security,
                        AuditSeverity::Warning,
                        "action_trigger_rate_limited",
                        Some(peer_str.clone()),
                        format!(
                            "Rate-limited action trigger from {} (<5s cooldown)",
                            peer_str
                        ),
                        false,
                    );
                    let result_frame = ActionResultFrame {
                        trigger_id: frame.trigger_id,
                        action_id: frame.action_id,
                        success: false,
                        exit_code: None,
                        output_summary: None,
                        error_message: Some("rate limit exceeded (5s cooldown)".to_string()),
                        duration_ms: 0,
                        timestamp_ms: now_ms,
                    };
                    let node = self.clone_handle();
                    let target = peer.clone();
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            if let Ok(inner) = result_frame.to_inner_frame() {
                                let _ = node.send_to(&target, inner).await;
                            }
                        });
                    }
                    return;
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        // Policy admission check
        if policy.mode == ActionAccessMode::Disabled {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_disabled",
                Some(peer_str.clone()),
                format!(
                    "Ignored action trigger from {} because action policy is disabled",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some("action policy is disabled".to_string()),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        if !policy.is_peer_eligible(&profile) {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_ineligible",
                Some(peer_str.clone()),
                format!(
                    "Rejected action trigger from {} due to profile scoping rules",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some("peer not eligible under action policy".to_string()),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        if policy.mode == ActionAccessMode::PromptOnly {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_prompt_required",
                Some(peer_str.clone()),
                format!(
                    "Action trigger from {} rejected: prompt confirmation required",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some(
                    "action execution requires interactive prompt confirmation".to_string(),
                ),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        // Local action lookup
        let local_actions = self.local_actions();
        let target_action = match local_actions
            .iter()
            .find(|a| a.id == frame.action_id && a.enabled)
        {
            Some(a) => a.clone(),
            None => {
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Warning,
                    "action_trigger_not_found",
                    Some(peer_str.clone()),
                    format!(
                        "Action '{}' requested by {} not found in local allowlist",
                        frame.action_id, peer_str
                    ),
                    false,
                );
                let result_frame = ActionResultFrame {
                    trigger_id: frame.trigger_id,
                    action_id: frame.action_id,
                    success: false,
                    exit_code: None,
                    output_summary: None,
                    error_message: Some("action not found in local allowlist".to_string()),
                    duration_ms: 0,
                    timestamp_ms: now_ms,
                };
                let node = self.clone_handle();
                let target = peer.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        if let Ok(inner) = result_frame.to_inner_frame() {
                            let _ = node.send_to(&target, inner).await;
                        }
                    });
                }
                return;
            }
        };

        self.emit(UiEvent::action_triggered(
            &frame.trigger_id,
            &frame.action_id,
            &peer_str,
            &alias,
            now_ms,
        ));

        // ADR-008: Execute locally predefined command and args under timeout sandbox
        let node = self.clone_handle();
        let target = peer.clone();
        let timeout_secs = target_action
            .timeout_secs
            .min(policy.max_timeout_secs)
            .clamp(1, 30);
        let trigger_id = frame.trigger_id;
        let action_id = frame.action_id;

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let start_time = std::time::Instant::now();
                let cmd_fut = tokio::process::Command::new(&target_action.command)
                    .args(&target_action.args)
                    .output();

                let (success, exit_code, output_summary, error_message) =
                    match tokio::time::timeout(Duration::from_secs(timeout_secs as u64), cmd_fut)
                        .await
                    {
                        Ok(Ok(output)) => {
                            let code = output.status.code();
                            let is_success = output.status.success();
                            let mut out_bytes = output.stdout;
                            out_bytes.extend_from_slice(&output.stderr);
                            let summary = String::from_utf8_lossy(&out_bytes);
                            let clean_summary = ActionValidator::sanitize_output(&summary);
                            (
                                is_success,
                                code,
                                if clean_summary.is_empty() {
                                    None
                                } else {
                                    Some(clean_summary)
                                },
                                if is_success {
                                    None
                                } else {
                                    Some(format!("exit code: {:?}", code))
                                },
                            )
                        }
                        Ok(Err(e)) => (false, None, None, Some(format!("execution failed: {}", e))),
                        Err(_) => (
                            false,
                            None,
                            None,
                            Some(format!("execution timed out after {}s", timeout_secs)),
                        ),
                    };

                let duration_ms = start_time.elapsed().as_millis() as u64;
                let exec_now = node.shared.clock.unix_ms();

                let result_frame = ActionResultFrame {
                    trigger_id: trigger_id.clone(),
                    action_id: action_id.clone(),
                    success,
                    exit_code,
                    output_summary,
                    error_message: error_message.clone(),
                    duration_ms,
                    timestamp_ms: exec_now,
                };

                node.record_audit(
                    AuditCategory::Security,
                    if success {
                        AuditSeverity::Info
                    } else {
                        AuditSeverity::Warning
                    },
                    "action_executed_locally",
                    Some(peer_str.clone()),
                    format!(
                        "Locally executed action '{}' for peer {} (success={}, duration={}ms, error={:?})",
                        action_id, peer_str, success, duration_ms, error_message
                    ),
                    success,
                );

                // Save to local history cache
                {
                    let mut cache = node
                        .shared
                        .action_results_cache
                        .lock()
                        .expect("action_results_cache");
                    if cache.len() >= 100 {
                        cache.pop_front();
                    }
                    cache.push_back(result_frame.clone());
                }

                if let Ok(inner) = result_frame.to_inner_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Ingest and process an inbound ActionResultFrame from a connected peer.
    pub fn receive_action_result(&self, peer: &DeviceId, frame: ActionResultFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        self.emit(UiEvent::action_result_received(
            &frame.trigger_id,
            &frame.action_id,
            &peer_str,
            &alias,
            frame.success,
            frame.exit_code,
            frame.duration_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            if frame.success {
                AuditSeverity::Info
            } else {
                AuditSeverity::Warning
            },
            "action_result_received",
            Some(peer_str),
            format!(
                "Action result from {} for '{}': success={}, duration={}ms, error={:?}",
                alias, frame.action_id, frame.success, frame.duration_ms, frame.error_message
            ),
            frame.success,
        );

        {
            let mut cache = self
                .shared
                .action_results_cache
                .lock()
                .expect("action_results_cache");
            if cache.len() >= 100 {
                cache.pop_front();
            }
            cache.push_back(frame);
        }
    }

    /// Returns the currently active focus and quiet hours policy.
    pub fn focus_policy(&self) -> FocusPolicy {
        self.shared
            .focus_policy
            .lock()
            .expect("focus_policy")
            .clone()
    }

    /// Status and configuration report for focus and quiet hours policy.
    pub fn focus_policy_report(&self) -> FocusPolicyReport {
        FocusPolicyReport {
            ok: true,
            policy: self.focus_policy(),
            message: "focus policy report".to_string(),
        }
    }

    /// Update the focus and quiet hours policy and persist to disk atomically with 0o600 permissions.
    pub fn set_focus_policy(
        &self,
        mut policy: FocusPolicy,
    ) -> Result<FocusPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_focus_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save focus_policy.json: {e}")))?;
        *self.shared.focus_policy.lock().expect("focus_policy") = policy.clone();

        self.emit(UiEvent::focus_policy_changed(
            policy.enabled,
            policy.default_mode.as_str(),
            policy.mirror_inbound_focus,
            policy.allow_emergency_breakthrough,
            policy.quiet_hours_enabled,
            policy.quiet_hours_start_minute,
            policy.quiet_hours_end_minute,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Focus,
            AuditSeverity::Info,
            "focus_policy_updated",
            None::<String>,
            format!(
                "Updated focus policy: enabled={}, default_mode={}, mirror={}, breakthrough={}, quiet_hours={}",
                policy.enabled,
                policy.default_mode.as_str(),
                policy.mirror_inbound_focus,
                policy.allow_emergency_breakthrough,
                policy.quiet_hours_enabled,
            ),
            true,
        );

        Ok(FocusPolicyReport {
            ok: true,
            policy,
            message: "focus policy updated successfully".to_string(),
        })
    }

    /// Returns the current focus status, combining local state, expiry, and quiet hours.
    pub fn focus_status(&self) -> FocusStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.focus_policy();
        let minute_of_day = ((now_ms / 1000) % 86400 / 60) as u16;
        let quiet_hours_active = policy.is_quiet_hours_active(minute_of_day);

        let local_state = self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state")
            .clone();

        let manual_active = local_state.is_active_at(now_ms);
        let enabled = local_state.enabled;
        let mode = local_state.mode;
        let reason = local_state.reason.clone();
        let expires_ms = local_state.expires_ms;

        let overall_active = policy.enabled && (manual_active || quiet_hours_active);
        let effective_mode = if manual_active {
            mode
        } else if quiet_hours_active {
            FocusMode::Sleep
        } else {
            mode
        };

        let remaining_secs = expires_ms.and_then(|exp| {
            if exp > now_ms {
                Some((exp - now_ms) / 1000)
            } else {
                None
            }
        });

        FocusStatusReport {
            ok: true,
            enabled,
            mode: effective_mode,
            active: overall_active,
            reason,
            expires_ms,
            remaining_secs,
            quiet_hours_active,
            policy,
            message: if overall_active {
                format!("focus mode active ({:?})", effective_mode)
            } else {
                "focus mode inactive".to_string()
            },
        }
    }

    /// Set local focus state (on/off, mode, duration, reason) and optionally broadcast to mesh peers.
    pub async fn set_focus_state(
        &self,
        enabled: bool,
        mode: Option<FocusMode>,
        duration_mins: Option<u32>,
        reason: Option<String>,
        broadcast: bool,
    ) -> Result<FocusSetReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let clean_reason = reason.as_deref().map(FocusValidator::sanitize_reason);
        let policy = self.focus_policy();
        let chosen_mode = mode.unwrap_or(policy.default_mode);

        let (expires_ms, frame) = if enabled {
            let duration = duration_mins.map(FocusValidator::clamp_duration_mins);
            let exp = duration.map(|d| now_ms + (d as u64) * 60 * 1000);
            let f = FocusStateFrame {
                enabled: true,
                mode: chosen_mode,
                expires_ms: exp,
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            (exp, f)
        } else {
            let f = FocusStateFrame {
                enabled: false,
                mode: chosen_mode,
                expires_ms: None,
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            (None, f)
        };

        *self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state") = frame.clone();

        let active = frame.is_active_at(now_ms);
        self.emit(UiEvent::focus_state_changed(
            frame.enabled,
            frame.mode.as_str(),
            active,
            frame.reason.as_deref(),
            frame.expires_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Focus,
            AuditSeverity::Info,
            if enabled {
                "focus_mode_activated"
            } else {
                "focus_mode_deactivated"
            },
            None::<String>,
            format!(
                "Focus mode {}: mode={:?}, duration={:?}, reason={:?}",
                if enabled { "activated" } else { "deactivated" },
                frame.mode,
                duration_mins,
                frame.reason
            ),
            true,
        );

        let mut broadcast_count = 0;
        if broadcast {
            if let Ok(inner) = frame.to_frame() {
                let live = self.live_peers();
                for p in live {
                    let profile = self.peer_profile(&p.device_id);
                    if policy.is_peer_eligible(&profile)
                        && self.send_to(&p.device_id, inner.clone()).await.is_ok()
                    {
                        broadcast_count += 1;
                    }
                }
            }
        }

        Ok(FocusSetReport {
            ok: true,
            enabled: frame.enabled,
            mode: frame.mode,
            active,
            expires_ms,
            reason: frame.reason,
            broadcast_count,
            message: if enabled {
                "focus mode activated".to_string()
            } else {
                "focus mode deactivated".to_string()
            },
        })
    }

    /// Returns focus state of all connected/known peers.
    pub fn peer_focus_states(&self) -> PeersFocusReport {
        let now_ms = self.shared.clock.unix_ms();
        let map = self
            .shared
            .peer_focus_states
            .lock()
            .expect("peer_focus_states");
        let mut peers = Vec::new();
        for (peer_id_str, frame) in map.iter() {
            let alias = self
                .live_peers()
                .iter()
                .find(|p| p.device_id.to_string() == *peer_id_str)
                .map(|p| p.name.clone());

            let active = frame.is_active_at(now_ms);
            let remaining_secs = frame.expires_ms.and_then(|exp| {
                if exp > now_ms {
                    Some((exp - now_ms) / 1000)
                } else {
                    None
                }
            });
            peers.push(PeerFocusReport {
                ok: true,
                peer: peer_id_str.clone(),
                peer_alias: alias,
                state: frame.clone(),
                active,
                remaining_secs,
            });
        }
        peers.sort_by(|a, b| a.peer.cmp(&b.peer));
        PeersFocusReport {
            ok: true,
            count: peers.len(),
            peers,
        }
    }

    /// Ingest and process an inbound FocusStateFrame from a connected peer.
    pub fn receive_focus_state(&self, peer: &DeviceId, frame: FocusStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.focus_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "focus state ignored: policy disabled or peer not eligible");
            return;
        }

        let active = frame.is_active_at(now_ms);
        {
            let mut map = self
                .shared
                .peer_focus_states
                .lock()
                .expect("peer_focus_states");
            map.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::peer_focus_state_changed(
            &peer_str,
            &alias,
            frame.enabled,
            frame.mode.as_str(),
            active,
            frame.reason.as_deref(),
            frame.expires_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Focus,
            AuditSeverity::Info,
            "peer_focus_state_updated",
            Some(peer_str.clone()),
            format!(
                "Peer {} ({}) updated focus state: enabled={}, mode={:?}, active={}",
                alias, peer_str, frame.enabled, frame.mode, active
            ),
            true,
        );

        if policy.mirror_inbound_focus && active {
            let mut local = self
                .shared
                .local_focus_state
                .lock()
                .expect("local_focus_state");
            let should_mirror = !local.is_active_at(now_ms);
            if should_mirror {
                let mirrored_frame = FocusStateFrame {
                    enabled: true,
                    mode: frame.mode,
                    expires_ms: frame.expires_ms,
                    reason: Some(format!("mirrored from {}", alias)),
                    timestamp_ms: now_ms,
                };
                *local = mirrored_frame;
                self.emit(UiEvent::focus_state_changed(
                    true,
                    frame.mode.as_str(),
                    true,
                    Some(&format!("mirrored from {}", alias)),
                    frame.expires_ms,
                    now_ms,
                ));
            }
        }
    }

    /// Ingest and process an inbound FocusSyncReqFrame from a connected peer.
    pub fn receive_focus_sync_req(&self, peer: &DeviceId, _frame: FocusSyncReqFrame) {
        let policy = self.focus_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            return;
        }

        let frame = self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state")
            .clone();

        let node = self.clone_handle();
        let target = peer.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(inner) = frame.to_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Request focus state sync from all eligible peers.
    pub async fn request_peers_focus_sync(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let req_frame = FocusSyncReqFrame {
            request_id: format!("req_{}", now_ms),
            timestamp_ms: now_ms,
        };
        let inner = req_frame.to_frame()?;
        let live = self.live_peers();
        let policy = self.focus_policy();
        let mut sent = 0;
        for p in live {
            let profile = self.peer_profile(&p.device_id);
            if policy.is_peer_eligible(&profile)
                && self.send_to(&p.device_id, inner.clone()).await.is_ok()
            {
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Returns the current local spatial display topology, configured neighbors, and crossing settings.
    pub fn topology_layout(&self) -> TopologyLayoutReport {
        let layout = self
            .shared
            .topology_layout
            .lock()
            .expect("topology_layout")
            .clone();
        let peer_count = self
            .shared
            .peer_topologies
            .lock()
            .expect("peer_topologies")
            .len();
        TopologyLayoutReport {
            ok: true,
            display: layout.display,
            neighbors: layout.neighbors,
            config: layout.config,
            peer_count,
            message: "local topology layout".to_string(),
        }
    }

    /// Returns all known remote peer display topologies with aliases.
    pub fn topology_peers(&self) -> TopologyPeersReport {
        let map = self.shared.peer_topologies.lock().expect("peer_topologies");
        let live = self.live_peers();
        let mut peers = Vec::new();
        for (peer_id_str, frame) in map.iter() {
            let alias = live
                .iter()
                .find(|p| p.device_id.to_string() == *peer_id_str)
                .map(|p| p.name.clone());
            peers.push(PeerTopologyItem {
                peer_id: peer_id_str.clone(),
                peer_alias: alias,
                display: frame.display,
                neighbors: frame.neighbors.clone(),
                config: frame.config,
                timestamp_ms: frame.timestamp_ms,
            });
        }
        peers.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        TopologyPeersReport {
            ok: true,
            count: peers.len(),
            peers,
            message: "known peer topologies".to_string(),
        }
    }

    /// Returns the current spatial topology coordination and inbound control policy.
    pub fn topology_policy(&self) -> TopologyPolicy {
        self.shared
            .topology_policy
            .lock()
            .expect("topology_policy")
            .clone()
    }

    /// Status and configuration report for topology policy.
    pub fn topology_policy_report(&self) -> TopologyPolicyReport {
        TopologyPolicyReport {
            ok: true,
            policy: self.topology_policy(),
            message: "topology policy report".to_string(),
        }
    }

    /// Update spatial topology coordination policy and persist to disk atomically with 0o600 permissions.
    pub fn set_topology_policy(
        &self,
        mut policy: TopologyPolicy,
    ) -> Result<TopologyPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_topology_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save topology_policy.json: {e}")))?;
        *self.shared.topology_policy.lock().expect("topology_policy") = policy.clone();

        self.emit(UiEvent::topology_policy_changed(
            policy.enabled,
            policy.allow_inbound_control,
            policy.auto_link_paired_peers,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "topology_policy_changed",
            None::<String>,
            format!(
                "Updated topology policy: enabled={}, allow_inbound_control={}, auto_link_paired_peers={}, groups={:?}, tags={:?}, types={:?}",
                policy.enabled,
                policy.allow_inbound_control,
                policy.auto_link_paired_peers,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(TopologyPolicyReport {
            ok: true,
            policy,
            message: "topology policy updated successfully".to_string(),
        })
    }

    /// Links a remote peer's display edge to the specified local edge.
    pub async fn topology_link_neighbor(
        &self,
        edge: DisplayEdge,
        peer_target: &str,
        peer_edge: Option<DisplayEdge>,
        offset: Option<i32>,
    ) -> Result<TopologyLinkReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let peer_str = peer_id.to_string();
        let target_edge = peer_edge.unwrap_or_else(|| edge.opposite());
        let target_offset = offset.unwrap_or(0).clamp(-4096, 4096);

        let now = self.shared.clock.unix_ms();
        let neighbor = EdgeNeighbor {
            peer_id: peer_str.clone(),
            peer_edge: target_edge,
            offset: target_offset,
        };

        let neighbors_count = {
            let mut layout = self.shared.topology_layout.lock().expect("topology_layout");
            layout.neighbors.insert(edge, neighbor);
            save_topology_layout(&self.shared.cfg.data_dir, &layout).map_err(|e| {
                CoreError::File(format!("failed to save topology_layout.json: {e}"))
            })?;
            layout.neighbors.len()
        };

        let policy = self.topology_policy();
        self.emit(UiEvent::topology_changed(
            policy.enabled,
            neighbors_count,
            now,
        ));

        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());
        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "topology_neighbor_linked",
            Some(peer_str.clone()),
            format!(
                "Linked local edge {:?} to peer {} ({}) edge {:?} with offset {}",
                edge, alias, peer_str, target_edge, target_offset
            ),
            true,
        );

        // Broadcast updated state in the background
        let node = self.clone_handle();
        tokio::spawn(async move {
            let _ = node.topology_broadcast_state().await;
        });

        Ok(TopologyLinkReport {
            ok: true,
            edge,
            peer_id: peer_str,
            peer_edge: target_edge,
            offset: target_offset,
            message: format!("neighbor linked at {:?}", edge),
        })
    }

    /// Unlinks any peer attached to the specified local display edge.
    pub async fn topology_unlink_neighbor(
        &self,
        edge: DisplayEdge,
    ) -> Result<TopologyUnlinkReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        let (removed, neighbors_count) = {
            let mut layout = self.shared.topology_layout.lock().expect("topology_layout");
            let removed = layout.neighbors.remove(&edge);
            save_topology_layout(&self.shared.cfg.data_dir, &layout).map_err(|e| {
                CoreError::File(format!("failed to save topology_layout.json: {e}"))
            })?;
            (removed, layout.neighbors.len())
        };

        let policy = self.topology_policy();
        self.emit(UiEvent::topology_changed(
            policy.enabled,
            neighbors_count,
            now,
        ));

        let removed_peer = removed.as_ref().map(|n| n.peer_id.clone());
        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "topology_neighbor_unlinked",
            removed_peer,
            format!("Unlinked neighbor from local edge {:?}", edge),
            true,
        );

        // Broadcast updated state in the background
        let node = self.clone_handle();
        tokio::spawn(async move {
            let _ = node.topology_broadcast_state().await;
        });

        Ok(TopologyUnlinkReport {
            ok: true,
            edge,
            message: format!("neighbor unlinked from {:?}", edge),
        })
    }

    /// Updates display geometry and edge crossing parameters.
    #[allow(clippy::too_many_arguments)]
    pub async fn topology_update_config(
        &self,
        resistance_px: Option<u32>,
        delay_ms: Option<u32>,
        corner_lock_size_px: Option<u32>,
        wrap_around: Option<bool>,
        width: Option<u32>,
        height: Option<u32>,
        scale_factor: Option<f32>,
    ) -> Result<TopologyConfigReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        let (display, config, neighbors_count) = {
            let mut layout = self.shared.topology_layout.lock().expect("topology_layout");
            if let Some(res) = resistance_px {
                layout.config.resistance_px = TopologyValidator::clamp_resistance(res);
            }
            if let Some(del) = delay_ms {
                layout.config.delay_ms = TopologyValidator::clamp_delay(del);
            }
            if let Some(cls) = corner_lock_size_px {
                layout.config.corner_lock_size_px = TopologyValidator::clamp_corner_lock(cls);
            }
            if let Some(wrap) = wrap_around {
                layout.config.wrap_around = wrap;
            }
            if let Some(w) = width {
                layout.display.width = TopologyValidator::clamp_width(w);
            }
            if let Some(h) = height {
                layout.display.height = TopologyValidator::clamp_height(h);
            }
            if let Some(scale) = scale_factor {
                layout.display.scale_factor = TopologyValidator::clamp_scale(scale);
            }
            save_topology_layout(&self.shared.cfg.data_dir, &layout).map_err(|e| {
                CoreError::File(format!("failed to save topology_layout.json: {e}"))
            })?;
            (layout.display, layout.config, layout.neighbors.len())
        };

        let policy = self.topology_policy();
        self.emit(UiEvent::topology_changed(
            policy.enabled,
            neighbors_count,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "topology_config_updated",
            None::<String>,
            format!(
                "Updated topology config: display={}x{}@{:.2}, resistance={}px, delay={}ms, corner_lock={}px, wrap={}",
                display.width, display.height, display.scale_factor,
                config.resistance_px, config.delay_ms, config.corner_lock_size_px, config.wrap_around
            ),
            true,
        );

        // Broadcast updated state in the background
        let node = self.clone_handle();
        tokio::spawn(async move {
            let _ = node.topology_broadcast_state().await;
        });

        Ok(TopologyConfigReport {
            ok: true,
            display,
            config,
            message: "topology configuration updated".to_string(),
        })
    }

    /// Broadcast current spatial display topology to all connected and eligible peers.
    pub async fn topology_broadcast_state(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let my_id = self.shared.identity.device_id().to_string();
        let (display, neighbors, config) = {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            (layout.display, layout.neighbors.clone(), layout.config)
        };

        let frame = TopologyStateFrame {
            device_id: my_id,
            display,
            neighbors,
            config,
            timestamp_ms: now_ms,
        };

        let inner = frame.to_frame()?;
        let live = self.live_peers();
        let policy = self.topology_policy();
        let mut broadcast_count = 0;

        for p in live {
            let profile = self.peer_profile(&p.device_id);
            if policy.is_peer_eligible(&profile)
                && self.send_to(&p.device_id, inner.clone()).await.is_ok()
            {
                broadcast_count += 1;
            }
        }

        Ok(broadcast_count)
    }

    /// Request topology state synchronization from all eligible connected peers.
    pub async fn topology_request_sync(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let req_frame = TopologySyncReqFrame {
            request_id: format!("topo_req_{}", now_ms),
            timestamp_ms: now_ms,
        };
        let inner = req_frame.to_frame()?;
        let live = self.live_peers();
        let policy = self.topology_policy();
        let mut sent = 0;

        for p in live {
            let profile = self.peer_profile(&p.device_id);
            if policy.is_peer_eligible(&profile)
                && self.send_to(&p.device_id, inner.clone()).await.is_ok()
            {
                sent += 1;
            }
        }

        Ok(sent)
    }

    /// Ingest and process an inbound TopologyStateFrame from a connected peer.
    pub fn receive_topology_state(&self, peer: &DeviceId, mut frame: TopologyStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.topology_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "topology state ignored: policy disabled or peer not eligible");
            return;
        }

        frame.sanitize();
        frame.device_id = peer_str.clone();

        let width = frame.display.width;
        let height = frame.display.height;
        let scale_factor = frame.display.scale_factor;

        {
            let mut cache = self.shared.peer_topologies.lock().expect("peer_topologies");
            cache.insert(peer_str.clone(), frame);
        }

        self.emit(UiEvent::peer_topology_updated(
            &peer_str,
            &alias,
            width,
            height,
            scale_factor,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "peer_topology_updated",
            Some(peer_str),
            format!(
                "Updated topology for peer {} ({}): display={}x{}",
                alias, peer, width, height
            ),
            true,
        );
    }

    /// Ingest and process an inbound TopologySyncReqFrame from a connected peer.
    pub fn receive_topology_sync_req(&self, peer: &DeviceId, _frame: TopologySyncReqFrame) {
        let policy = self.topology_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            return;
        }

        let my_id = self.shared.identity.device_id().to_string();
        let now_ms = self.shared.clock.unix_ms();
        let (display, neighbors, config) = {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            (layout.display, layout.neighbors.clone(), layout.config)
        };

        let frame = TopologyStateFrame {
            device_id: my_id,
            display,
            neighbors,
            config,
            timestamp_ms: now_ms,
        };

        let node = self.clone_handle();
        let target = peer.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(inner) = frame.to_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Evaluates cursor position against 2D spatial display topology, anti-misclick corner zones,
    /// edge resistance, and dwell delay. Computes proportional remote coordinates if crossing criteria are satisfied.
    pub fn evaluate_cursor_crossing(&self, x: i32, y: i32) -> CursorCrossingDecision {
        let policy = self.topology_policy();
        if !policy.enabled {
            return CursorCrossingDecision {
                crossed: false,
                target_peer: None,
                edge: None,
                entry_x: None,
                entry_y: None,
                blocked_reason: Some("topology coordination disabled".to_string()),
            };
        }

        let now = self.shared.clock.unix_ms();
        let (display, neighbors, config) = {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            (layout.display, layout.neighbors.clone(), layout.config)
        };

        let width = display.width as i32;
        let height = display.height as i32;

        // 1. Anti-misclick corner lock exclusion zone check
        if TopologyValidator::is_in_corner_zone(
            x,
            y,
            display.width,
            display.height,
            config.corner_lock_size_px,
        ) {
            return CursorCrossingDecision {
                crossed: false,
                target_peer: None,
                edge: None,
                entry_x: None,
                entry_y: None,
                blocked_reason: Some("corner anti-misclick exclusion zone".to_string()),
            };
        }

        // 2. Identify active edge boundary touch
        let active_edge = if x <= 0 {
            Some(DisplayEdge::Left)
        } else if x >= width - 1 {
            Some(DisplayEdge::Right)
        } else if y <= 0 {
            Some(DisplayEdge::Top)
        } else if y >= height - 1 {
            Some(DisplayEdge::Bottom)
        } else {
            None
        };

        let Some(edge) = active_edge else {
            // Not touching any edge: reset dwell and resistance
            self.shared.cursor_edge_dwell.lock().expect("dwell").clear();
            self.shared
                .cursor_edge_resistance_acc
                .lock()
                .expect("res")
                .clear();
            *self.shared.cursor_last_pos.lock().expect("pos") = (x, y);
            return CursorCrossingDecision {
                crossed: false,
                target_peer: None,
                edge: None,
                entry_x: None,
                entry_y: None,
                blocked_reason: None,
            };
        };

        // 3. Check if neighbor is configured at this edge
        let Some(neighbor) = neighbors.get(&edge) else {
            return CursorCrossingDecision {
                crossed: false,
                target_peer: None,
                edge: Some(edge),
                entry_x: None,
                entry_y: None,
                blocked_reason: Some(format!("no neighbor configured at {:?}", edge)),
            };
        };

        // 4. Check dwell delay
        {
            let mut dwell_map = self.shared.cursor_edge_dwell.lock().expect("dwell");
            let dwell_start = dwell_map.entry(edge).or_insert(now);
            if now.saturating_sub(*dwell_start) < config.delay_ms as u64 {
                return CursorCrossingDecision {
                    crossed: false,
                    target_peer: Some(neighbor.peer_id.clone()),
                    edge: Some(edge),
                    entry_x: None,
                    entry_y: None,
                    blocked_reason: Some(format!(
                        "dwell delay not reached ({}ms / {}ms)",
                        now.saturating_sub(*dwell_start),
                        config.delay_ms
                    )),
                };
            }
        }

        // 5. Check edge resistance
        let (last_x, last_y) = *self.shared.cursor_last_pos.lock().expect("pos");
        let push_distance = match edge {
            DisplayEdge::Left => (last_x - x).max(1) as u32,
            DisplayEdge::Right => (x - last_x).max(1) as u32,
            DisplayEdge::Top => (last_y - y).max(1) as u32,
            DisplayEdge::Bottom => (y - last_y).max(1) as u32,
        };

        {
            let mut res_map = self.shared.cursor_edge_resistance_acc.lock().expect("res");
            let accumulated = res_map.entry(edge).or_insert(0);
            *accumulated = accumulated.saturating_add(push_distance);
            if *accumulated < config.resistance_px {
                return CursorCrossingDecision {
                    crossed: false,
                    target_peer: Some(neighbor.peer_id.clone()),
                    edge: Some(edge),
                    entry_x: None,
                    entry_y: None,
                    blocked_reason: Some(format!(
                        "edge resistance not overcome ({}px / {}px)",
                        *accumulated, config.resistance_px
                    )),
                };
            }
        }

        // 6. Resolve target peer display geometry
        let remote_display = {
            let peers = self.shared.peer_topologies.lock().expect("peer_topologies");
            peers
                .get(&neighbor.peer_id)
                .map(|p| p.display)
                .unwrap_or_else(|| {
                    // Default fallback: 1920x1080@1.0
                    DisplayRect::new(1920, 1080, 1.0)
                })
        };

        // 7. Calculate proportional entrance coordinate
        let (entry_x, entry_y) = map_edge_exit_to_entry(
            &display,
            edge,
            (x, y),
            &remote_display,
            neighbor.peer_edge,
            neighbor.offset,
        );

        // Reset resistance and dwell after successful crossing trigger
        self.shared
            .cursor_edge_dwell
            .lock()
            .expect("dwell")
            .remove(&edge);
        self.shared
            .cursor_edge_resistance_acc
            .lock()
            .expect("res")
            .remove(&edge);
        *self.shared.cursor_last_pos.lock().expect("pos") = (x, y);

        let my_id = self.shared.identity.device_id().to_string();
        self.emit(UiEvent::cursor_edge_crossed(
            &my_id,
            &neighbor.peer_id,
            edge.as_str(),
            entry_x,
            entry_y,
            now,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "topology_edge_crossed",
            Some(neighbor.peer_id.clone()),
            format!(
                "Cursor crossed edge {:?} to peer {} at ({}, {})",
                edge, neighbor.peer_id, entry_x, entry_y
            ),
            true,
        );

        let decision = CursorCrossingDecision {
            crossed: true,
            target_peer: Some(neighbor.peer_id.clone()),
            edge: Some(edge),
            entry_x: Some(entry_x),
            entry_y: Some(entry_y),
            blocked_reason: None,
        };

        if policy.auto_link_paired_peers && self.input_policy().enabled {
            let node = self.clone_handle();
            let target_peer = neighbor.peer_id.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = node
                        .input_tunnel_attach(&target_peer, Some(edge), Some(entry_x), Some(entry_y))
                        .await;
                });
            }
        }

        decision
    }

    /// Ingest and process an inbound InputTunnelRequestFrame from a controller peer.
    pub async fn receive_input_tunnel_request(
        &self,
        peer: &DeviceId,
        mut frame: InputTunnelRequestFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if InputTunnelValidator::validate_session_id(&frame.session_id).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid input tunnel session_id rejected");
            return;
        }

        let policy = self.input_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual peripheral sharing disabled".to_string())
        } else if !policy.allow_inbound_control {
            Some("inbound input control rejected by target policy".to_string())
        } else if !policy.is_peer_eligible(&profile) {
            Some("peer ineligible under input policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let resp = InputTunnelRespFrame {
                session_id: frame.session_id.clone(),
                accepted: false,
                reject_reason: Some(reason.clone()),
                capture_width: None,
                capture_height: None,
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = resp.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "input_tunnel_rejected",
                Some(peer_str.clone()),
                format!(
                    "Rejected inbound input tunnel request from {}: {}",
                    peer_str, reason
                ),
                false,
            );
            return;
        }

        let (width, height) = {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            (layout.display.width, layout.display.height)
        };

        let (clamped_entry_x, clamped_entry_y) = match (frame.entry_x, frame.entry_y) {
            (Some(x), Some(y)) => {
                let (cx, cy) = InputTunnelValidator::clamp_coords(x, y, width, height);
                (Some(cx), Some(cy))
            }
            (Some(x), None) => {
                let (cx, _) = InputTunnelValidator::clamp_coords(x, 0, width, height);
                (Some(cx), None)
            }
            (None, Some(y)) => {
                let (_, cy) = InputTunnelValidator::clamp_coords(0, y, width, height);
                (None, Some(cy))
            }
            (None, None) => (None, None),
        };

        let session = ActiveInputTunnelSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: InputTunnelRole::Target,
            requested_edge: frame.requested_edge,
            entry_x: clamped_entry_x,
            entry_y: clamped_entry_y,
            capture_width: Some(width),
            capture_height: Some(height),
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel") = Some(session);

        let resp = InputTunnelRespFrame {
            session_id: frame.session_id.clone(),
            accepted: true,
            reject_reason: None,
            capture_width: Some(width),
            capture_height: Some(height),
            timestamp_ms: now_ms,
        };

        if let Ok(inner) = resp.to_frame() {
            let _ = self.send_to(peer, inner).await;
        }

        self.emit(UiEvent::input_tunnel_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "target",
            Some(width),
            Some(height),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "input_tunnel_started",
            Some(peer_str.clone()),
            format!(
                "Inbound input tunnel established with peer {} (target role)",
                peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound InputTunnelRespFrame from a target peer.
    pub async fn receive_input_tunnel_resp(
        &self,
        peer: &DeviceId,
        mut frame: InputTunnelRespFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if InputTunnelValidator::validate_session_id(&frame.session_id).is_err() {
            return;
        }

        // `input_tunnel_attach` pre-arms the controller session before the
        // request is answered, so a resp that matches no live session is
        // stale: the tunnel was released or timed out while the answer was in
        // flight. Release is terminal — a late resp must not resurrect the
        // tunnel or hijack the seat.
        {
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            let Some(session) = guard.as_mut() else {
                return;
            };
            if session.session_id != frame.session_id {
                return;
            }
            if frame.accepted {
                session.capture_width = frame.capture_width;
                session.capture_height = frame.capture_height;
                session.last_activity_ms = now_ms;
            } else {
                *guard = None;
            }
        }

        if frame.accepted {
            if !self.input_channel_open().await {
                let _ = self.open_input(peer).await;
            }

            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Remote);
            self.emit(UiEvent::input_seat_changed(
                "remote",
                true,
                Some(peer_str.clone()),
            ));

            self.emit(UiEvent::input_tunnel_session_started(
                &frame.session_id,
                &peer_str,
                &alias,
                "controller",
                frame.capture_width,
                frame.capture_height,
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "input_tunnel_started",
                Some(peer_str.clone()),
                format!(
                    "Outbound input tunnel accepted by peer {} (controller role)",
                    peer_str
                ),
                true,
            );
        } else {
            let reason = frame
                .reject_reason
                .unwrap_or_else(|| "rejected by peer".to_string());
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            self.emit(UiEvent::input_seat_changed(
                "local",
                false,
                Some(peer_str.clone()),
            ));

            self.emit(UiEvent::input_tunnel_session_closed(
                &frame.session_id,
                &peer_str,
                &alias,
                &reason,
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "input_tunnel_rejected",
                Some(peer_str.clone()),
                format!(
                    "Input tunnel request rejected by peer {}: {}",
                    peer_str, reason
                ),
                false,
            );
        }
    }

    /// Ingest and process an inbound InputTunnelReleaseFrame from either end of the tunnel.
    pub async fn receive_input_tunnel_release(
        &self,
        peer: &DeviceId,
        mut frame: InputTunnelReleaseFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            self.emit(UiEvent::input_seat_changed(
                "local",
                false,
                Some(peer_str.clone()),
            ));

            self.emit(UiEvent::input_tunnel_session_closed(
                &frame.session_id,
                &peer_str,
                &alias,
                &frame.reason,
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "input_tunnel_closed",
                Some(peer_str.clone()),
                format!(
                    "Input tunnel released by peer {}: {}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest and process keepalive heartbeat for an active input tunnel.
    pub fn receive_input_tunnel_heartbeat(
        &self,
        peer: &DeviceId,
        frame: InputTunnelHeartbeatFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        let mut guard = self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
            }
        }
    }

    /// Query status of active virtual input tunnel session and governance policy.
    pub fn input_tunnel_status(&self) -> InputTunnelStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.input_policy();
        let mut guard = self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel");

        let is_timed_out = if let Some(ref session) = *guard {
            let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::input_tunnel_session_closed(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    "idle timeout exceeded",
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "input_tunnel_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Input tunnel session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return InputTunnelStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        InputTunnelStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request virtual peripheral input tunnel attachment to a target peer.
    pub async fn input_tunnel_attach(
        &self,
        peer_query: &str,
        edge: Option<DisplayEdge>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<InputTunnelDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();

        let policy = self.input_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "input tunnel sharing disabled".to_string(),
            ));
        }
        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile) {
            return Err(CoreError::InvalidPayload(
                "peer not eligible under input policy".to_string(),
            ));
        }

        let session_id = format!("tun_{}", now_ms);
        let my_id = self.shared.identity.device_id().to_string();

        let req_frame = InputTunnelRequestFrame {
            session_id: session_id.clone(),
            client_device_id: my_id,
            requested_edge: edge,
            entry_x: x,
            entry_y: y,
            timestamp_ms: now_ms,
        };

        let inner = req_frame.to_frame()?;
        self.send_to(&peer_id, inner).await.map_err(|e| {
            CoreError::InvalidPayload(format!("failed to send input tunnel request to peer: {e}"))
        })?;

        {
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            *guard = Some(ActiveInputTunnelSession {
                session_id: session_id.clone(),
                peer_id: peer_str.clone(),
                role: InputTunnelRole::Controller,
                requested_edge: edge,
                entry_x: x,
                entry_y: y,
                capture_width: None,
                capture_height: None,
                started_at_ms: now_ms,
                last_activity_ms: now_ms,
            });
        }

        Ok(InputTunnelDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(InputTunnelRole::Controller),
            message: "input tunnel request sent to peer".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Release the active virtual input tunnel session.
    pub async fn input_tunnel_release(
        &self,
        reason: Option<&str>,
    ) -> Result<InputTunnelDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested release");
        let sanitized_reason = InputTunnelValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();

            let rel_frame = InputTunnelReleaseFrame {
                session_id: session.session_id.clone(),
                exit_edge: session.requested_edge,
                return_x: session.entry_x,
                return_y: session.entry_y,
                reason: sanitized_reason.clone(),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = rel_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::input_seat_changed(
                "local",
                false,
                Some(session.peer_id.clone()),
            ));
            self.emit(UiEvent::input_tunnel_session_closed(
                &session.session_id,
                &session.peer_id,
                &alias,
                &sanitized_reason,
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "input_tunnel_closed",
                Some(session.peer_id.clone()),
                format!(
                    "Input tunnel session {} released: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(InputTunnelDecisionReport {
                ok: true,
                action: "release".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("input tunnel session released: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(InputTunnelDecisionReport {
                ok: true,
                action: "release".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active input tunnel session to release".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current input tunneling and virtual peripheral sharing policy.
    pub fn input_policy(&self) -> InputTunnelPolicy {
        self.shared
            .input_policy
            .lock()
            .expect("input_policy")
            .clone()
    }

    /// Status and configuration report for input policy.
    pub fn input_policy_report(&self) -> InputTunnelPolicyReport {
        InputTunnelPolicyReport {
            ok: true,
            policy: self.input_policy(),
            message: "input policy report".to_string(),
        }
    }

    /// Update input tunneling policy and persist to disk atomically with 0o600 permissions.
    pub fn set_input_policy(
        &self,
        mut policy: InputTunnelPolicy,
    ) -> Result<InputTunnelPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_input_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save input_policy.json: {e}")))?;
        *self.shared.input_policy.lock().expect("input_policy") = policy.clone();

        self.emit(UiEvent::input_tunnel_policy_changed(
            policy.enabled,
            policy.allow_inbound_control,
            policy.allow_keyboard,
            policy.allow_mouse,
            policy.hotkey_release_enabled,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "input_tunnel_policy_changed",
            None::<String>,
            format!(
                "Updated input policy: enabled={}, allow_inbound={}, keyboard={}, mouse={}, timeout={}s",
                policy.enabled,
                policy.allow_inbound_control,
                policy.allow_keyboard,
                policy.allow_mouse,
                policy.idle_timeout_secs
            ),
            true,
        );

        Ok(InputTunnelPolicyReport {
            ok: true,
            policy,
            message: "input policy updated".to_string(),
        })
    }

    /// Partially update input tunneling policy and persist to disk.
    pub fn update_input_policy(
        &self,
        update: InputTunnelPolicyUpdate,
    ) -> Result<InputTunnelPolicyReport, CoreError> {
        let mut policy = self.input_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_control {
            policy.allow_inbound_control = v;
        }
        if let Some(v) = update.allow_keyboard {
            policy.allow_keyboard = v;
        }
        if let Some(v) = update.allow_mouse {
            policy.allow_mouse = v;
        }
        if let Some(v) = update.hotkey_release_enabled {
            policy.hotkey_release_enabled = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_input_policy(policy)
    }

    /// Ingest and process an inbound VirtualDisplayAnnounceFrame from a peer.
    pub fn receive_virtual_display_announce(
        &self,
        peer: &DeviceId,
        frame: VirtualDisplayAnnounceFrame,
    ) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            display = %frame.display_name,
            mode = ?frame.preferred_mode,
            "received virtual display announcement"
        );
    }

    /// Ingest and process an inbound VirtualDisplayAttachFrame to act as virtual display sink.
    pub async fn receive_virtual_display_attach(
        &self,
        peer: &DeviceId,
        mut frame: VirtualDisplayAttachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if VirtualDisplayValidator::validate_session_id(&frame.session_id).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid virtual display session_id rejected");
            return;
        }

        let policy = self.virtual_display_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual secondary display disabled".to_string())
        } else if !policy.allow_hosting {
            Some("hosting virtual secondary display disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, true) {
            Some("peer ineligible under virtual display policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = VirtualDisplayDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "virtual_display_rejected",
                Some(peer_str.clone()),
                format!(
                    "Rejected inbound virtual secondary display attach from {}: {}",
                    peer_str, reason
                ),
                false,
            );
            return;
        }

        let validated_mode = match VirtualDisplayValidator::validate_mode(&frame.mode) {
            Ok(m) => m,
            Err(_) => {
                let detach = VirtualDisplayDetachFrame {
                    session_id: frame.session_id.clone(),
                    reason: Some("invalid mode specifications".to_string()),
                    timestamp_ms: now_ms,
                };
                if let Ok(inner) = detach.to_frame() {
                    let _ = self.send_to(peer, inner).await;
                }
                return;
            }
        };

        let target_fps = frame.target_fps.clamp(10, policy.max_fps);
        let session = ActiveVirtualDisplaySession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: VirtualDisplayRole::Sink,
            assigned_edge: frame.assigned_edge,
            mode: validated_mode.clone(),
            codec: frame.codec,
            target_fps,
            current_fps: target_fps,
            rtt_us: 0,
            dropped_frames: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display") = Some(session);

        self.emit(UiEvent::virtual_display_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "sink",
            validated_mode.width,
            validated_mode.height,
            target_fps,
            frame.assigned_edge.as_str(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "virtual_display_sink_started",
            Some(peer_str.clone()),
            format!(
                "Virtual secondary display session {} attached as sink for host {}",
                frame.session_id, peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound VirtualDisplayDetachFrame from the other end.
    pub async fn receive_virtual_display_detach(
        &self,
        peer: &DeviceId,
        mut frame: VirtualDisplayDetachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_virtual_display
                .lock()
                .expect("active_virtual_display");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::virtual_display_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "virtual_display_detached",
                Some(peer_str.clone()),
                format!(
                    "Virtual secondary display detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest and process frame ACK feedback for adaptive frame rate and backpressure tuning.
    pub fn receive_virtual_display_frame_ack(&self, peer: &DeviceId, ack: VirtualDisplayFrameAck) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        let mut guard = self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display");
        if let Some(ref mut session) = *guard {
            if session.session_id == ack.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.rtt_us = ack.render_delay_us;
                session.dropped_frames = session.dropped_frames.saturating_add(ack.dropped_frames);

                // Adaptive frame rate tuning based on backpressure and render latency:
                if ack.dropped_frames > 2 || ack.render_delay_us > 50_000 {
                    session.current_fps = session.current_fps.saturating_sub(5).max(10);
                } else if ack.dropped_frames == 0 && ack.render_delay_us < 20_000 {
                    session.current_fps = (session.current_fps + 2).min(session.target_fps);
                }
            }
        }
    }

    /// Query status of active virtual secondary display session and governance policy.
    pub fn virtual_display_status(&self) -> VirtualDisplayStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.virtual_display_policy();
        let mut guard = self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display");

        let is_timed_out = if let Some(ref session) = *guard {
            let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::virtual_display_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "virtual_display_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Virtual display session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return VirtualDisplayStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        VirtualDisplayStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a peer as a virtual secondary display (extending desktop).
    pub async fn virtual_display_attach(
        &self,
        peer_query: &str,
        edge: Option<DisplayEdge>,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<u32>,
    ) -> Result<VirtualDisplayDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.virtual_display_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual secondary display disabled".to_string(),
            ));
        }
        if !policy.allow_remote_extend {
            return Err(CoreError::InvalidPayload(
                "extending desktop to remote secondary display disabled by policy".to_string(),
            ));
        }
        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, false) {
            return Err(CoreError::InvalidPayload(
                "peer not eligible under virtual display policy".to_string(),
            ));
        }

        let req_width = width
            .unwrap_or(1920)
            .clamp(320, policy.max_resolution_width.min(7680));
        let req_height = height
            .unwrap_or(1080)
            .clamp(240, policy.max_resolution_height.min(4320));
        let req_fps = fps.unwrap_or(60).clamp(10, policy.max_fps.min(120));
        let assigned_edge = edge.unwrap_or(DisplayEdge::Right);

        let mode = VirtualDisplayMode {
            width: req_width,
            height: req_height,
            refresh_rate: req_fps,
            scale_factor: 1.0,
            pixel_format: VirtualDisplayPixelFormat::Bgra8,
        };

        let session_id = format!("vdisp_{}", now_ms);
        let my_id = self.shared.identity.device_id().to_string();

        let attach_frame = VirtualDisplayAttachFrame {
            session_id: session_id.clone(),
            peer_id: my_id,
            assigned_edge,
            mode: mode.clone(),
            codec: VirtualDisplayCodec::ZstdBgra,
            target_fps: req_fps,
            timestamp_ms: now_ms,
        };

        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await.map_err(|e| {
            CoreError::InvalidPayload(format!(
                "failed to send virtual display attach frame to peer: {e}"
            ))
        })?;

        let session = ActiveVirtualDisplaySession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: VirtualDisplayRole::Host,
            assigned_edge,
            mode,
            codec: VirtualDisplayCodec::ZstdBgra,
            target_fps: req_fps,
            current_fps: req_fps,
            rtt_us: 0,
            dropped_frames: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display") = Some(session);

        self.emit(UiEvent::virtual_display_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            req_width,
            req_height,
            req_fps,
            assigned_edge.as_str(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "virtual_display_attached",
            Some(peer_str.clone()),
            format!(
                "Extended desktop to peer {} ({}x{}@{}fps on edge {:?})",
                peer_str, req_width, req_height, req_fps, assigned_edge
            ),
            true,
        );

        Ok(VirtualDisplayDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(VirtualDisplayRole::Host),
            message: "virtual secondary display attached to peer".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active virtual secondary display session.
    pub async fn virtual_display_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<VirtualDisplayDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = VirtualDisplayValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_virtual_display
                .lock()
                .expect("active_virtual_display");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = VirtualDisplayDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::virtual_display_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "virtual_display_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Virtual display session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(VirtualDisplayDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("virtual display session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(VirtualDisplayDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active virtual display session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current virtual display governance policy.
    pub fn virtual_display_policy(&self) -> VirtualDisplayPolicy {
        self.shared
            .virtual_display_policy
            .lock()
            .expect("virtual_display_policy")
            .clone()
    }

    /// Status and configuration report for virtual display policy.
    pub fn virtual_display_policy_report(&self) -> VirtualDisplayPolicyReport {
        VirtualDisplayPolicyReport {
            ok: true,
            policy: self.virtual_display_policy(),
            message: "virtual display policy report".to_string(),
        }
    }

    /// Update virtual display policy and persist to disk atomically with 0o600 permissions.
    pub fn set_virtual_display_policy(
        &self,
        mut policy: VirtualDisplayPolicy,
    ) -> Result<VirtualDisplayPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_virtual_display_policy(&self.shared.cfg.data_dir, &policy).map_err(|e| {
            CoreError::File(format!("failed to save virtual_display_policy.json: {e}"))
        })?;
        *self
            .shared
            .virtual_display_policy
            .lock()
            .expect("virtual_display_policy") = policy.clone();

        self.emit(UiEvent::virtual_display_policy_changed(
            policy.enabled,
            policy.allow_hosting,
            policy.allow_remote_extend,
            policy.max_resolution_width,
            policy.max_resolution_height,
            policy.max_fps,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "virtual_display_policy_changed",
            None::<String>,
            format!(
                "Updated virtual display policy: enabled={}, allow_hosting={}, allow_remote_extend={}, max_res={}x{}, max_fps={}",
                policy.enabled,
                policy.allow_hosting,
                policy.allow_remote_extend,
                policy.max_resolution_width,
                policy.max_resolution_height,
                policy.max_fps
            ),
            true,
        );

        Ok(VirtualDisplayPolicyReport {
            ok: true,
            policy,
            message: "virtual display policy updated".to_string(),
        })
    }

    /// Partially update virtual display policy and persist to disk.
    pub fn update_virtual_display_policy(
        &self,
        update: VirtualDisplayPolicyUpdate,
    ) -> Result<VirtualDisplayPolicyReport, CoreError> {
        let mut policy = self.virtual_display_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_hosting {
            policy.allow_hosting = v;
        }
        if let Some(v) = update.allow_remote_extend {
            policy.allow_remote_extend = v;
        }
        if let Some(v) = update.max_resolution_width {
            policy.max_resolution_width = v;
        }
        if let Some(v) = update.max_resolution_height {
            policy.max_resolution_height = v;
        }
        if let Some(v) = update.max_fps {
            policy.max_fps = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_virtual_display_policy(policy)
    }

    /// Ingest and process an inbound AudioRoutingAnnounceFrame from a peer.
    pub fn receive_audio_routing_announce(
        &self,
        peer: &DeviceId,
        frame: AudioRoutingAnnounceFrame,
    ) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            capabilities = ?frame.capabilities,
            "received audio routing announcement"
        );
    }

    /// Ingest and process an inbound AudioRoutingAttachFrame to act as an audio sink.
    pub async fn receive_audio_routing_attach(
        &self,
        peer: &DeviceId,
        mut frame: AudioRoutingAttachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if AudioRoutingValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid audio routing attach frame rejected");
            return;
        }

        let policy = self.audio_routing_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("spatial audio routing disabled".to_string())
        } else if !policy.allow_inbound_sink {
            Some("sink audio playback disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, true) {
            Some("peer ineligible under audio routing policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = AudioRoutingDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "audio_routing_attach_rejected",
                Some(peer_str.clone()),
                format!(
                    "Rejected audio routing attach from {}: {}",
                    peer_str, reason
                ),
                false,
            );
            return;
        }

        let max_gain = (policy.max_volume_limit as f32) / 100.0;
        let effective_gain = frame.volume_gain.min(max_gain);

        let session = ActiveAudioRoutingSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: AudioRoutingRole::Sink,
            layout: frame.layout,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            format: frame.format,
            codec: frame.codec,
            spatial_pos: frame.spatial_pos,
            volume_gain: effective_gain,
            chunks_sent: 0,
            chunks_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing") = Some(session);

        self.emit(UiEvent::audio_routing_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "sink",
            frame.layout.as_str(),
            frame.sample_rate,
            frame.channels,
            frame.format.as_str(),
            frame.codec.as_str(),
            effective_gain,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audio_routing_sink_started",
            Some(peer_str.clone()),
            format!(
                "Audio routing session {} established as sink for host {}",
                frame.session_id, peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound AudioRoutingDetachFrame.
    pub async fn receive_audio_routing_detach(
        &self,
        peer: &DeviceId,
        mut frame: AudioRoutingDetachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_audio_routing
                .lock()
                .expect("active_audio_routing");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::audio_routing_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "audio_routing_detached",
                Some(peer_str.clone()),
                format!(
                    "Audio routing detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest streaming audio chunk.
    pub fn receive_audio_routing_chunk(&self, peer: &DeviceId, frame: AudioRoutingChunkFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if AudioRoutingValidator::validate_chunk(&frame).is_err() {
            return;
        }

        let mut guard = self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.chunks_received = session.chunks_received.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.payload_bytes.len() as u64);
            }
        }
    }

    /// Query status of active spatial audio routing session and governance policy.
    pub fn audio_routing_status(&self) -> AudioRoutingStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.audio_routing_policy();
        let mut guard = self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing");

        let is_timed_out = if let Some(ref session) = *guard {
            let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::audio_routing_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "audio_routing_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Audio routing session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return AudioRoutingStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        AudioRoutingStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a peer as an audio sink (routing local audio to peer).
    #[allow(clippy::too_many_arguments)]
    pub async fn audio_routing_attach(
        &self,
        peer_query: &str,
        layout: Option<AudioChannelLayout>,
        sample_rate: Option<u32>,
        channels: Option<u8>,
        spatial_x: Option<f32>,
        spatial_y: Option<f32>,
        spatial_z: Option<f32>,
        volume_gain: Option<f32>,
    ) -> Result<AudioRoutingDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.audio_routing_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "spatial audio routing disabled".to_string(),
            ));
        }
        if !policy.allow_outbound_routing {
            return Err(CoreError::InvalidPayload(
                "routing audio to remote peers disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, false) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under audio routing policy filters".to_string(),
            ));
        }

        let req_layout = layout.unwrap_or(AudioChannelLayout::Stereo);
        let req_sample_rate = sample_rate.unwrap_or(48000).clamp(8000, 192000);
        let req_channels = channels
            .unwrap_or_else(|| req_layout.default_channels())
            .clamp(1, 8);
        let req_format = AudioSampleFormat::S16Le;
        let req_codec = AudioStreamCodec::ZstdPcm;
        let spatial_pos = if spatial_x.is_some() || spatial_y.is_some() || spatial_z.is_some() {
            Some(AudioSpatialPosition {
                x: spatial_x.unwrap_or(0.0).clamp(-10.0, 10.0),
                y: spatial_y.unwrap_or(0.0).clamp(-10.0, 10.0),
                z: spatial_z.unwrap_or(0.0).clamp(-10.0, 10.0),
            })
        } else {
            None
        };
        let max_gain = (policy.max_volume_limit as f32) / 100.0;
        let req_gain = volume_gain.unwrap_or(1.0).clamp(0.0, 2.0).min(max_gain);

        let session_id = format!("ar-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = AudioRoutingAttachFrame {
            session_id: session_id.clone(),
            source_id: self.identity().device_id().to_string(),
            sink_id: peer_str.clone(),
            layout: req_layout,
            sample_rate: req_sample_rate,
            channels: req_channels,
            format: req_format,
            codec: req_codec,
            spatial_pos,
            volume_gain: req_gain,
            timestamp_ms: now_ms,
        };

        AudioRoutingValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveAudioRoutingSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: AudioRoutingRole::Source,
            layout: req_layout,
            sample_rate: req_sample_rate,
            channels: req_channels,
            format: req_format,
            codec: req_codec,
            spatial_pos,
            volume_gain: req_gain,
            chunks_sent: 0,
            chunks_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing") = Some(session);

        self.emit(UiEvent::audio_routing_session_started(
            &session_id,
            &peer_str,
            &alias,
            "source",
            req_layout.as_str(),
            req_sample_rate,
            req_channels,
            req_format.as_str(),
            req_codec.as_str(),
            req_gain,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audio_routing_attached",
            Some(peer_str.clone()),
            format!(
                "Routed audio to peer {} ({:?} @ {}Hz, {} ch, gain={:.2})",
                peer_str, req_layout, req_sample_rate, req_channels, req_gain
            ),
            true,
        );

        Ok(AudioRoutingDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(AudioRoutingRole::Source),
            message: "audio routing established to peer".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active audio routing session.
    pub async fn audio_routing_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<AudioRoutingDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = AudioRoutingValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_audio_routing
                .lock()
                .expect("active_audio_routing");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = AudioRoutingDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::audio_routing_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "audio_routing_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Audio routing session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(AudioRoutingDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("audio routing session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(AudioRoutingDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active audio routing session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current audio routing governance policy.
    pub fn audio_routing_policy(&self) -> AudioRoutingPolicy {
        self.shared
            .audio_routing_policy
            .lock()
            .expect("audio_routing_policy")
            .clone()
    }

    /// Status and configuration report for audio routing policy.
    pub fn audio_routing_policy_report(&self) -> AudioRoutingPolicyReport {
        AudioRoutingPolicyReport {
            ok: true,
            policy: self.audio_routing_policy(),
            message: "audio routing policy report".to_string(),
        }
    }

    /// Update audio routing policy and persist to disk atomically with 0o600 permissions.
    pub fn set_audio_routing_policy(
        &self,
        mut policy: AudioRoutingPolicy,
    ) -> Result<AudioRoutingPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_audio_routing_policy(&self.shared.cfg.data_dir, &policy).map_err(|e| {
            CoreError::File(format!("failed to save audio_routing_policy.json: {e}"))
        })?;
        *self
            .shared
            .audio_routing_policy
            .lock()
            .expect("audio_routing_policy") = policy.clone();

        self.emit(UiEvent::audio_routing_policy_changed(
            policy.enabled,
            policy.allow_inbound_sink,
            policy.allow_outbound_routing,
            policy.max_volume_limit,
            policy.allow_spatial_rendering,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "audio_routing_policy_changed",
            None::<String>,
            format!(
                "Updated audio routing policy: enabled={}, allow_sink={}, allow_outbound={}, max_vol={}, spatial={}",
                policy.enabled,
                policy.allow_inbound_sink,
                policy.allow_outbound_routing,
                policy.max_volume_limit,
                policy.allow_spatial_rendering
            ),
            true,
        );

        Ok(AudioRoutingPolicyReport {
            ok: true,
            policy,
            message: "audio routing policy updated".to_string(),
        })
    }

    /// Partially update audio routing policy and persist to disk.
    pub fn update_audio_routing_policy(
        &self,
        update: AudioRoutingPolicyUpdate,
    ) -> Result<AudioRoutingPolicyReport, CoreError> {
        let mut policy = self.audio_routing_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_sink {
            policy.allow_inbound_sink = v;
        }
        if let Some(v) = update.allow_outbound_routing {
            policy.allow_outbound_routing = v;
        }
        if let Some(v) = update.max_volume_limit {
            policy.max_volume_limit = v;
        }
        if let Some(v) = update.allow_spatial_rendering {
            policy.allow_spatial_rendering = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_audio_routing_policy(policy)
    }

    /// Ingest and process an inbound CameraAnnounceFrame from a peer.
    pub fn receive_camera_announce(&self, peer: &DeviceId, frame: CameraAnnounceFrame) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            codecs = ?frame.capabilities.supported_codecs,
            lenses = ?frame.capabilities.lenses,
            "received camera announcement"
        );
    }

    /// Ingest and process an inbound CameraAttachFrame to act as a camera stream source (Lens role).
    pub async fn receive_camera_attach(&self, peer: &DeviceId, mut frame: CameraAttachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if CameraValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid camera attach frame rejected");
            return;
        }

        let policy = self.camera_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual camera streaming disabled".to_string())
        } else if !policy.allow_remote_capture {
            Some("remote camera capture disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, false) {
            Some("peer ineligible under camera policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = CameraDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "camera_attach_rejected",
                Some(peer_str.clone()),
                format!("Rejected camera attach from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        let session = ActiveCameraSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: CameraRole::Lens,
            lens_id: frame.lens_id.clone(),
            resolution: frame.resolution,
            target_fps: frame.target_fps,
            current_fps: frame.target_fps,
            codec: frame.codec,
            flashlight_on: frame.flashlight_on,
            zoom_ratio: frame.zoom_ratio,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self.shared.active_camera.lock().expect("active_camera") = Some(session);

        self.emit(UiEvent::camera_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "lens",
            &frame.lens_id,
            frame.resolution.width,
            frame.resolution.height,
            frame.target_fps,
            frame.codec.as_str(),
            frame.flashlight_on,
            frame.zoom_ratio,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "camera_lens_started",
            Some(peer_str.clone()),
            format!(
                "Camera session {} started as streaming lens ({}x{}@{}fps, codec={}) for host {}",
                frame.session_id,
                frame.resolution.width,
                frame.resolution.height,
                frame.target_fps,
                frame.codec.as_str(),
                peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound CameraDetachFrame.
    pub async fn receive_camera_detach(&self, peer: &DeviceId, mut frame: CameraDetachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self.shared.active_camera.lock().expect("active_camera");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::camera_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "camera_detached",
                Some(peer_str.clone()),
                format!("Camera detached by peer {}: {:?}", peer_str, frame.reason),
                true,
            );
        }
    }

    /// Ingest streaming camera frame chunk.
    pub fn receive_camera_frame_chunk(&self, peer: &DeviceId, frame: CameraFrameChunk) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if CameraValidator::validate_chunk_frame(&frame).is_err() {
            return;
        }

        let mut guard = self.shared.active_camera.lock().expect("active_camera");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.frames_received = session.frames_received.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.data.len() as u64);
            }
        }
    }

    /// Query status of active virtual camera session and governance policy.
    pub fn camera_status(&self) -> CameraStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.camera_policy();
        let mut guard = self.shared.active_camera.lock().expect("active_camera");

        let is_timed_out = if let Some(ref session) = *guard {
            let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::camera_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "camera_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Camera session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return CameraStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        CameraStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a remote peer's camera lens as local virtual camera feed.
    #[allow(clippy::too_many_arguments)]
    pub async fn camera_attach(
        &self,
        peer_query: &str,
        lens_id: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<u32>,
        codec: Option<CameraCodec>,
        flashlight: Option<bool>,
        zoom: Option<f32>,
    ) -> Result<CameraDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.camera_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual camera streaming disabled".to_string(),
            ));
        }
        if !policy.allow_inbound_stream {
            return Err(CoreError::InvalidPayload(
                "attaching remote camera disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under camera policy filters".to_string(),
            ));
        }

        let req_lens_id = lens_id.unwrap_or("default");
        let validated_lens_id = CameraValidator::validate_lens_id(req_lens_id)?;

        let req_width = width.unwrap_or(1920).min(policy.max_resolution_width);
        let req_height = height.unwrap_or(1080).min(policy.max_resolution_height);
        let clamped_res = CameraValidator::clamp_resolution(req_width, req_height);

        let req_fps = fps.unwrap_or(30).min(policy.max_fps);
        let clamped_fps = CameraValidator::clamp_fps(req_fps);

        let req_codec = codec.unwrap_or(CameraCodec::Mjpeg);
        let req_flashlight = flashlight.unwrap_or(false);
        let req_zoom = CameraValidator::clamp_zoom(zoom.unwrap_or(1.0));

        let session_id = format!("cam-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = CameraAttachFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            lens_id: validated_lens_id.clone(),
            resolution: clamped_res,
            target_fps: clamped_fps,
            codec: req_codec,
            flashlight_on: req_flashlight,
            zoom_ratio: req_zoom,
            timestamp_ms: now_ms,
        };

        CameraValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveCameraSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: CameraRole::Host,
            lens_id: validated_lens_id.clone(),
            resolution: clamped_res,
            target_fps: clamped_fps,
            current_fps: clamped_fps,
            codec: req_codec,
            flashlight_on: req_flashlight,
            zoom_ratio: req_zoom,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self.shared.active_camera.lock().expect("active_camera") = Some(session);

        self.emit(UiEvent::camera_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            &validated_lens_id,
            clamped_res.width,
            clamped_res.height,
            clamped_fps,
            req_codec.as_str(),
            req_flashlight,
            req_zoom,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "camera_attached",
            Some(peer_str.clone()),
            format!(
                "Attached camera from peer {} (lens '{}', {}x{}@{}fps, codec={:?}, zoom={:.1}x, torch={})",
                peer_str, validated_lens_id, clamped_res.width, clamped_res.height, clamped_fps, req_codec, req_zoom, req_flashlight
            ),
            true,
        );

        Ok(CameraDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(CameraRole::Host),
            message: "virtual camera session established to remote lens".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active camera session.
    pub async fn camera_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<CameraDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = CameraValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self.shared.active_camera.lock().expect("active_camera");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = CameraDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::camera_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "camera_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Camera session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(CameraDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("camera session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(CameraDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active camera session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current camera governance policy.
    pub fn camera_policy(&self) -> CameraPolicy {
        self.shared
            .camera_policy
            .lock()
            .expect("camera_policy")
            .clone()
    }

    /// Status and configuration report for camera policy.
    pub fn camera_policy_report(&self) -> CameraPolicyReport {
        CameraPolicyReport {
            ok: true,
            policy: self.camera_policy(),
            message: "camera policy report".to_string(),
        }
    }

    /// Update camera policy and persist to disk atomically with 0o600 permissions.
    pub fn set_camera_policy(
        &self,
        mut policy: CameraPolicy,
    ) -> Result<CameraPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_camera_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save camera_policy.json: {e}")))?;
        *self.shared.camera_policy.lock().expect("camera_policy") = policy.clone();

        self.emit(UiEvent::camera_policy_changed(
            policy.enabled,
            policy.allow_inbound_stream,
            policy.allow_remote_capture,
            policy.max_resolution_width,
            policy.max_resolution_height,
            policy.max_fps,
            policy.enforce_privacy_shutter_indicator,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "camera_policy_changed",
            None::<String>,
            format!(
                "Updated camera policy: enabled={}, allow_inbound={}, allow_capture={}, max_res={}x{}, max_fps={}, privacy_shutter={}",
                policy.enabled,
                policy.allow_inbound_stream,
                policy.allow_remote_capture,
                policy.max_resolution_width,
                policy.max_resolution_height,
                policy.max_fps,
                policy.enforce_privacy_shutter_indicator
            ),
            true,
        );

        Ok(CameraPolicyReport {
            ok: true,
            policy,
            message: "camera policy updated".to_string(),
        })
    }

    /// Partially update camera policy and persist to disk.
    pub fn update_camera_policy(
        &self,
        update: CameraPolicyUpdate,
    ) -> Result<CameraPolicyReport, CoreError> {
        let mut policy = self.camera_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_stream {
            policy.allow_inbound_stream = v;
        }
        if let Some(v) = update.allow_remote_capture {
            policy.allow_remote_capture = v;
        }
        if let Some(v) = update.max_resolution_width {
            policy.max_resolution_width = v;
        }
        if let Some(v) = update.max_resolution_height {
            policy.max_resolution_height = v;
        }
        if let Some(v) = update.max_fps {
            policy.max_fps = v;
        }
        if let Some(v) = update.enforce_privacy_shutter_indicator {
            policy.enforce_privacy_shutter_indicator = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_camera_policy(policy)
    }

    /// Ingest and process an inbound MicAnnounceFrame from a peer.
    pub fn receive_mic_announce(&self, peer: &DeviceId, frame: MicAnnounceFrame) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            codecs = ?frame.capabilities.supported_codecs,
            devices = ?frame.capabilities.devices,
            "received microphone announcement"
        );
    }

    /// Ingest and process an inbound MicAttachFrame to act as a microphone stream source.
    pub async fn receive_mic_attach(&self, peer: &DeviceId, mut frame: MicAttachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if MicValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid microphone attach frame rejected");
            return;
        }

        let policy = self.microphone_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual microphone streaming disabled".to_string())
        } else if !policy.allow_remote_capture {
            Some("remote microphone capture disabled by policy".to_string())
        } else if frame.mode == MicMode::Intercom && !policy.allow_intercom {
            Some("voice intercom disabled by policy".to_string())
        } else if frame.sample_rate > policy.max_sample_rate {
            Some("sample rate exceeds policy limit".to_string())
        } else if !policy.is_peer_eligible(&profile, false, frame.mode == MicMode::Intercom) {
            Some("peer ineligible under microphone policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = MicDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "mic_attach_rejected",
                Some(peer_str.clone()),
                format!("Rejected microphone attach from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        // Fail-closed privacy governance: the capturing side MUST light the privacy
        // indicator whenever the policy demands it, refusing silent capture.
        let privacy_active = frame.enforce_privacy_indicator || policy.enforce_privacy_indicator;

        let session = ActiveMicrophoneSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: MicRole::Source,
            mode: frame.mode,
            mic_id: frame.mic_id.clone(),
            codec: frame.codec,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            gain: frame.gain,
            muted: frame.muted || policy.default_muted,
            noise_suppression: frame.noise_suppression,
            echo_cancellation: frame.echo_cancellation,
            auto_gain: frame.auto_gain,
            privacy_indicator_active: privacy_active,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone") = Some(session.clone());

        self.emit(UiEvent::microphone_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "source",
            frame.mode.as_str(),
            &frame.mic_id,
            frame.codec.as_str(),
            frame.sample_rate,
            frame.channels,
            frame.gain,
            session.muted,
            privacy_active,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "mic_source_started",
            Some(peer_str.clone()),
            format!(
                "Microphone session {} started as capture source ({}Hz x{}ch, codec={}, mode={}, gain={:.2}, muted={}, privacy_indicator={}) for host {}",
                frame.session_id,
                frame.sample_rate,
                frame.channels,
                frame.codec.as_str(),
                frame.mode.as_str(),
                frame.gain,
                session.muted,
                privacy_active,
                peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound MicDetachFrame.
    pub async fn receive_mic_detach(&self, peer: &DeviceId, mut frame: MicDetachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_microphone
                .lock()
                .expect("active_microphone");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::microphone_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "mic_detached",
                Some(peer_str.clone()),
                format!(
                    "Microphone detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest streaming microphone audio chunk. Only session metadata (sequence,
    /// mute state, RMS dB telemetry) is tracked; raw sample buffers never leave
    /// this function and are never logged, audited, or broadcast (Zero-Plaintext).
    pub fn receive_mic_audio_chunk(&self, peer: &DeviceId, frame: MicAudioChunkFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if MicValidator::validate_chunk_frame(&frame).is_err() {
            return;
        }

        let mut guard = self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.frames_received = session.frames_received.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.data.len() as u64);
            }
        }
    }

    /// Query status of active microphone/intercom session and governance policy.
    pub fn microphone_status(&self) -> MicrophoneStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.microphone_policy();
        let mut guard = self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone");

        let is_timed_out = if let Some(ref session) = *guard {
            let idle_ms = (policy.idle_timeout_secs as u64) * 1000;
            let max_ms = (policy.max_session_duration_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > idle_ms
                || now_ms.saturating_sub(session.started_at_ms) > max_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::microphone_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle or duration timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "mic_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Microphone session {} closed due to idle or duration timeout",
                        session.session_id
                    ),
                    true,
                );
                return MicrophoneStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        MicrophoneStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a remote peer's microphone as local virtual mic / intercom feed.
    #[allow(clippy::too_many_arguments)]
    pub async fn mic_attach(
        &self,
        peer_query: &str,
        mic_id: Option<&str>,
        mode: MicMode,
        codec: Option<MicCodec>,
        sample_rate: Option<u32>,
        channels: Option<u32>,
        gain: Option<f32>,
        muted: Option<bool>,
        noise_suppression: Option<bool>,
        echo_cancellation: Option<bool>,
        auto_gain: Option<bool>,
    ) -> Result<MicrophoneDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.microphone_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual microphone streaming disabled".to_string(),
            ));
        }
        if !policy.allow_inbound_stream {
            return Err(CoreError::InvalidPayload(
                "attaching remote microphone disabled by policy".to_string(),
            ));
        }
        if mode == MicMode::Intercom && !policy.allow_intercom {
            return Err(CoreError::InvalidPayload(
                "voice intercom disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true, mode == MicMode::Intercom) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under microphone policy filters".to_string(),
            ));
        }

        let req_mic_id = MicValidator::validate_mic_id(mic_id.unwrap_or("default"))?;
        let req_codec = codec.unwrap_or(MicCodec::Opus);
        let req_rate = MicValidator::clamp_sample_rate(sample_rate.unwrap_or(48000));
        if req_rate > policy.max_sample_rate {
            return Err(CoreError::InvalidPayload(
                "sample rate exceeds policy limit".to_string(),
            ));
        }
        let req_channels = MicValidator::clamp_channels(channels.unwrap_or(1));
        let req_gain = MicValidator::clamp_gain(gain.unwrap_or(1.0));
        let req_muted = muted.unwrap_or(false) || policy.default_muted;
        let req_ns = noise_suppression.unwrap_or(true);
        let req_aec = echo_cancellation.unwrap_or(true);
        let req_agc = auto_gain.unwrap_or(true);

        let session_id = format!("mic-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = MicAttachFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            mode,
            mic_id: req_mic_id.clone(),
            codec: req_codec,
            sample_rate: req_rate,
            channels: req_channels,
            gain: req_gain,
            muted: req_muted,
            noise_suppression: req_ns,
            echo_cancellation: req_aec,
            auto_gain: req_agc,
            enforce_privacy_indicator: policy.enforce_privacy_indicator,
            timestamp_ms: now_ms,
        };

        MicValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveMicrophoneSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: MicRole::Host,
            mode,
            mic_id: req_mic_id.clone(),
            codec: req_codec,
            sample_rate: req_rate,
            channels: req_channels,
            gain: req_gain,
            muted: req_muted,
            noise_suppression: req_ns,
            echo_cancellation: req_aec,
            auto_gain: req_agc,
            privacy_indicator_active: policy.enforce_privacy_indicator,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone") = Some(session);

        self.emit(UiEvent::microphone_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            mode.as_str(),
            &req_mic_id,
            req_codec.as_str(),
            req_rate,
            req_channels,
            req_gain,
            req_muted,
            policy.enforce_privacy_indicator,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "mic_attached",
            Some(peer_str.clone()),
            format!(
                "Attached microphone from peer {} (mic '{}', {}Hz x{}ch, codec={:?}, mode={:?}, gain={:.2}, muted={})",
                peer_str, req_mic_id, req_rate, req_channels, req_codec, mode, req_gain, req_muted
            ),
            true,
        );

        Ok(MicrophoneDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(MicRole::Host),
            message: "virtual microphone session established to remote source".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active microphone/intercom session.
    pub async fn mic_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<MicrophoneDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = MicValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_microphone
                .lock()
                .expect("active_microphone");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = MicDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::microphone_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "mic_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Microphone session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(MicrophoneDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("microphone session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(MicrophoneDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active microphone session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current microphone governance policy.
    pub fn microphone_policy(&self) -> MicrophonePolicy {
        self.shared
            .microphone_policy
            .lock()
            .expect("microphone_policy")
            .clone()
    }

    /// Status and configuration report for microphone policy.
    pub fn microphone_policy_report(&self) -> MicrophonePolicyReport {
        MicrophonePolicyReport {
            ok: true,
            policy: self.microphone_policy(),
            message: "microphone policy report".to_string(),
        }
    }

    /// Update microphone policy and persist to disk atomically with 0o600 permissions.
    pub fn set_microphone_policy(
        &self,
        mut policy: MicrophonePolicy,
    ) -> Result<MicrophonePolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_microphone_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save microphone_policy.json: {e}")))?;
        *self
            .shared
            .microphone_policy
            .lock()
            .expect("microphone_policy") = policy.clone();

        self.emit(UiEvent::microphone_policy_changed(
            policy.enabled,
            policy.allow_inbound_stream,
            policy.allow_remote_capture,
            policy.allow_intercom,
            policy.max_sample_rate,
            policy.enforce_privacy_indicator,
            policy.default_muted,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "microphone_policy_changed",
            None::<String>,
            format!(
                "Updated microphone policy: enabled={}, allow_inbound={}, allow_capture={}, allow_intercom={}, max_rate={}Hz, privacy_indicator={}, default_muted={}",
                policy.enabled,
                policy.allow_inbound_stream,
                policy.allow_remote_capture,
                policy.allow_intercom,
                policy.max_sample_rate,
                policy.enforce_privacy_indicator,
                policy.default_muted
            ),
            true,
        );

        Ok(MicrophonePolicyReport {
            ok: true,
            policy,
            message: "microphone policy updated".to_string(),
        })
    }

    /// Partially update microphone policy and persist to disk.
    pub fn update_microphone_policy(
        &self,
        update: MicrophonePolicyUpdate,
    ) -> Result<MicrophonePolicyReport, CoreError> {
        let mut policy = self.microphone_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_stream {
            policy.allow_inbound_stream = v;
        }
        if let Some(v) = update.allow_remote_capture {
            policy.allow_remote_capture = v;
        }
        if let Some(v) = update.allow_intercom {
            policy.allow_intercom = v;
        }
        if let Some(v) = update.max_sample_rate {
            policy.max_sample_rate = v;
        }
        if let Some(v) = update.max_session_duration_secs {
            policy.max_session_duration_secs = v;
        }
        if let Some(v) = update.enforce_privacy_indicator {
            policy.enforce_privacy_indicator = v;
        }
        if let Some(v) = update.default_muted {
            policy.default_muted = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_microphone_policy(policy)
    }

    /// Ingest and process an inbound BenchRequestFrame: this node becomes the responder.
    pub async fn receive_bench_request(&self, peer: &DeviceId, frame: BenchRequestFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_request(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid bench request frame rejected");
            return;
        }

        let policy = self.bench_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("link benchmarking disabled".to_string())
        } else if !policy.allow_inbound_bench {
            Some("inbound benchmark serving disabled by policy".to_string())
        } else if frame.duration_secs > policy.max_duration_secs {
            Some("duration exceeds policy limit".to_string())
        } else if !policy.is_peer_eligible(&profile, false) {
            Some("peer ineligible under bench policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let accept = BenchAcceptFrame {
                session_id: frame.session_id.clone(),
                accepted: false,
                reject_reason: Some(reason.clone()),
                direction: frame.direction,
                duration_secs: frame.duration_secs,
                chunk_size: frame.chunk_size,
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = accept.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "bench_request_rejected",
                Some(peer_str.clone()),
                format!("Rejected bench request from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        // Responder clamps to its own policy envelope before agreeing.
        let duration_secs = frame.duration_secs.min(policy.max_duration_secs);
        let chunk_size = BenchValidator::clamp_chunk_size(frame.chunk_size);

        // Reject if a bench session is already active on this node.
        let already_active = {
            let guard = self.shared.active_bench.lock().expect("active_bench");
            guard.is_some()
        };
        if already_active {
            let accept = BenchAcceptFrame {
                session_id: frame.session_id.clone(),
                accepted: false,
                reject_reason: Some("a benchmark session is already active".to_string()),
                direction: frame.direction,
                duration_secs: frame.duration_secs,
                chunk_size: frame.chunk_size,
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = accept.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            return;
        }

        let session = ActiveBenchSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: BenchRole::Responder,
            direction: frame.direction,
            duration_secs,
            chunk_size,
            chunks_processed: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };
        *self.shared.active_bench.lock().expect("active_bench") = Some(session);

        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::bench_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "responder",
            frame.direction.as_str(),
            duration_secs,
            chunk_size,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bench_responder_started",
            Some(peer_str.clone()),
            format!(
                "Serving benchmark {} from peer {} (direction={}, duration={}s, chunk={}B)",
                frame.session_id,
                peer_str,
                frame.direction.as_str(),
                duration_secs,
                chunk_size
            ),
            true,
        );

        let accept = BenchAcceptFrame {
            session_id: frame.session_id.clone(),
            accepted: true,
            reject_reason: None,
            direction: frame.direction,
            duration_secs,
            chunk_size,
            timestamp_ms: now_ms,
        };
        if let Ok(inner) = accept.to_frame() {
            let _ = self.send_to(peer, inner).await;
        }

        // Pull direction: the responder is the payload sender; start its engine now.
        if frame.direction == BenchDirection::Pull {
            self.spawn_bench_sender(
                peer.clone(),
                frame.session_id.clone(),
                frame.direction,
                duration_secs,
                chunk_size,
                policy.max_bandwidth_mbps,
            );
        }
    }

    /// Ingest and process an inbound BenchAcceptFrame: this node is the initiator.
    pub async fn receive_bench_accept(&self, peer: &DeviceId, frame: BenchAcceptFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_session_id(&frame.session_id).is_err() {
            return;
        }

        if !frame.accepted {
            // Responder refused; tear down the local half of the session.
            let removed = {
                let mut guard = self.shared.active_bench.lock().expect("active_bench");
                guard
                    .as_ref()
                    .map(|s| s.session_id == frame.session_id && s.peer_id == peer_str)
                    .unwrap_or(false)
                    .then(|| guard.take())
                    .flatten()
            };
            if let Some(session) = removed {
                let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
                self.emit(UiEvent::bench_session_finished(
                    &session.session_id,
                    &peer_str,
                    &alias,
                    session.direction.as_str(),
                    now_ms.saturating_sub(session.started_at_ms),
                    session.bytes_transferred,
                    session.chunks_processed,
                    0.0,
                    0,
                    0,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Warning,
                    "bench_rejected_by_peer",
                    Some(peer_str.clone()),
                    format!(
                        "Benchmark {} refused by peer: {:?}",
                        frame.session_id, frame.reject_reason
                    ),
                    true,
                );
            }
            return;
        }

        // Only the initiator consumes accepts for a session it created; the
        // role check and parameter adoption happen in one critical section.
        let adopted = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            let is_initiator = guard
                .as_ref()
                .map(|s| {
                    s.session_id == frame.session_id
                        && s.peer_id == peer_str
                        && s.role == BenchRole::Initiator
                })
                .unwrap_or(false);
            if is_initiator {
                if let Some(ref mut session) = *guard {
                    session.duration_secs = frame.duration_secs;
                    session.chunk_size = frame.chunk_size;
                }
            }
            is_initiator
        };
        if !adopted {
            return;
        }

        if frame.direction == BenchDirection::Push {
            let policy = self.bench_policy();
            self.spawn_bench_sender(
                peer.clone(),
                frame.session_id.clone(),
                frame.direction,
                frame.duration_secs,
                frame.chunk_size,
                policy.max_bandwidth_mbps,
            );
        }
    }

    /// Ingest a benchmark payload chunk: the local node is the receiving side.
    /// Payload bytes are engine-generated random data and are never copied into
    /// events, logs, or audit records (Zero-Plaintext).
    pub fn receive_bench_data(&self, peer: &DeviceId, frame: BenchDataFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if BenchValidator::validate_data_frame(&frame).is_err() {
            return;
        }

        let mut guard = self.shared.active_bench.lock().expect("active_bench");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.chunks_processed = session.chunks_processed.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.data.len() as u64);
            }
        }
    }

    /// Ingest the final benchmark report from the sending side and finalize the
    /// local (receiving) half with authoritative link measurements.
    pub async fn receive_bench_report(&self, peer: &DeviceId, frame: BenchReportFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        if BenchValidator::validate_session_id(&frame.session_id).is_err() {
            return;
        }

        let removed = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            guard
                .as_ref()
                .map(|s| s.session_id == frame.session_id && s.peer_id == peer_str)
                .unwrap_or(false)
                .then(|| guard.take())
                .flatten()
        };

        if let Some(session) = removed {
            // Receiving side numbers are the authoritative link measurement.
            let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
            let throughput_mbps =
                (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;

            // Authoritative profile report: receiver-side bytes/chunks/throughput,
            // sender-side pacing latency/jitter retained for reference. Stored in
            // last_bench_report and distilled into the persistent peer profile.
            let authoritative = BenchReportFrame {
                session_id: session.session_id.clone(),
                direction: session.direction,
                duration_ms,
                bytes_transferred: session.bytes_transferred,
                chunks: session.chunks_processed,
                throughput_mbps,
                avg_chunk_latency_us: frame.avg_chunk_latency_us,
                jitter_us: frame.jitter_us,
                stopped_early: frame.stopped_early,
                timestamp_ms: now_ms,
            };
            *self
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(authoritative);

            self.record_bandwidth_profile(
                &session.peer_id,
                &alias,
                throughput_mbps,
                session.direction.as_str(),
                &session.session_id,
                now_ms,
            );

            self.emit(UiEvent::bench_session_finished(
                &session.session_id,
                &session.peer_id,
                &alias,
                session.direction.as_str(),
                duration_ms,
                session.bytes_transferred,
                session.chunks_processed,
                throughput_mbps,
                frame.avg_chunk_latency_us,
                frame.jitter_us,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Info,
                "bench_finished",
                Some(session.peer_id.clone()),
                format!(
                    "Benchmark {} finished: {:.2} Mbps received ({} bytes / {} chunks in {} ms)",
                    session.session_id,
                    throughput_mbps,
                    session.bytes_transferred,
                    session.chunks_processed,
                    duration_ms
                ),
                true,
            );
        }
    }

    /// Distill a completed receiver-side measurement into the persistent
    /// per-peer bandwidth profile, broadcast the update, and audit it.
    fn record_bandwidth_profile(
        &self,
        peer_id: &str,
        alias: &str,
        measured_mbps: f64,
        direction: &str,
        session_id: &str,
        now_ms: u64,
    ) {
        let tier = recommend_streaming_params(measured_mbps).tier;
        // v1.92: a fresh link measurement re-tiers a live H.264 stream hot
        // (the encoder rebuilds with an IDR at the new rate).
        if self.shared.screen_bitrate_auto {
            if let Some(sel) = &self.shared.screen_encode_codec {
                if self
                    .shared
                    .screen_engine
                    .lock()
                    .expect("screen")
                    .state()
                    .is_streaming()
                {
                    sel.set_bitrate_bps(crate::screen::screen_bitrate_from_profile(Some(
                        measured_mbps,
                    )));
                }
            }
        }
        {
            let mut table = self
                .shared
                .bandwidth_profiles
                .lock()
                .expect("bandwidth_profiles");
            table.upsert(PeerBandwidthProfile {
                peer_id: peer_id.to_string(),
                measured_mbps,
                direction: direction.to_string(),
                session_id: session_id.to_string(),
                measured_at_ms: now_ms,
            });
            if let Err(e) = save_bandwidth_profiles(&self.shared.cfg.data_dir, &table) {
                warn!(error = %e, "failed to persist bandwidth_profiles.json");
            }
        }
        self.emit(UiEvent::bandwidth_profile_updated(
            peer_id,
            alias,
            measured_mbps,
            direction,
            &tier,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bandwidth_profile_updated",
            Some(peer_id.to_string()),
            format!(
                "Bandwidth profile updated for peer {} ({}, session {}): {:.2} Mbps -> tier {}",
                peer_id, direction, session_id, measured_mbps, tier
            ),
            true,
        );
    }

    /// Spawn the paced payload-sending engine for the given benchmark session.
    /// Chunks carry engine-generated random bytes at a rate capped by the
    /// policy's `max_bandwidth_mbps` so coexisting streams stay usable.
    fn spawn_bench_sender(
        &self,
        peer: DeviceId,
        session_id: String,
        direction: BenchDirection,
        duration_secs: u32,
        chunk_size: u32,
        max_bandwidth_mbps: u32,
    ) {
        let node = self.clone();
        tokio::spawn(async move {
            let started_ms = node.shared.clock.unix_ms();
            let deadline = started_ms + (duration_secs as u64) * 1000;

            // One random payload template per session; chunk bodies are engine
            // data only and never contain user content.
            let mut template = vec![0u8; chunk_size as usize];
            let _ = getrandom::getrandom(&mut template);

            let bytes_per_sec = (max_bandwidth_mbps as u64).saturating_mul(125_000);
            let pacing_us = (chunk_size as u64 * 1_000_000)
                .checked_div(bytes_per_sec)
                .unwrap_or(0);
            let pacing = std::time::Duration::from_micros(pacing_us);

            let mut chunk_seq: u64 = 0;
            let mut last_send_ms: Option<u64> = None;
            // Send-side pacing metrics: mean inter-send gap and its mean absolute
            // deviation (jitter proxy). Receiver-side numbers stay authoritative.
            let mut gap_sum_us: u64 = 0;
            let mut mad_sum_us: u64 = 0;
            let mut gap_samples: u64 = 0;

            loop {
                let now_ms = node.shared.clock.unix_ms();
                if now_ms >= deadline {
                    break;
                }
                {
                    let guard = node.shared.active_bench.lock().expect("active_bench");
                    let still_active = guard
                        .as_ref()
                        .map(|s| s.session_id == session_id)
                        .unwrap_or(false);
                    if !still_active {
                        return; // stopped remotely or locally
                    }
                }

                if let Some(last) = last_send_ms {
                    let gap_us = now_ms.saturating_sub(last).saturating_mul(1000);
                    gap_sum_us = gap_sum_us.saturating_add(gap_us);
                    let running_mean = gap_sum_us / gap_samples.max(1);
                    mad_sum_us = mad_sum_us.saturating_add(gap_us.abs_diff(running_mean));
                    gap_samples = gap_samples.saturating_add(1);
                }
                last_send_ms = Some(now_ms);

                let inner =
                    match BenchDataFrame::to_frame_for(&session_id, chunk_seq, &template, now_ms) {
                        Ok(inner) => inner,
                        Err(_) => break, // unrecoverable codec failure; finalize below
                    };
                if node.send_to(&peer, inner).await.is_err() {
                    break;
                }
                chunk_seq = chunk_seq.saturating_add(1);

                {
                    let mut guard = node.shared.active_bench.lock().expect("active_bench");
                    if let Some(ref mut session) = *guard {
                        if session.session_id == session_id {
                            session.chunks_processed = session.chunks_processed.saturating_add(1);
                            session.bytes_transferred =
                                session.bytes_transferred.saturating_add(chunk_size as u64);
                            session.last_activity_ms = now_ms;
                        }
                    }
                }

                if !pacing.is_zero() {
                    tokio::time::sleep(pacing).await;
                }
            }

            // Sending window elapsed (or link dropped): report and finalize locally.
            let end_ms = node.shared.clock.unix_ms();
            let duration_ms = end_ms.saturating_sub(started_ms).max(1);
            let (bytes_sent, chunks_sent) = {
                let mut guard = node.shared.active_bench.lock().expect("active_bench");
                let stats = guard
                    .as_ref()
                    .filter(|s| s.session_id == session_id)
                    .map(|s| (s.bytes_transferred, s.chunks_processed))
                    .unwrap_or((0, 0));
                guard.take();
                stats
            };
            let throughput_mbps = (bytes_sent as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;
            let avg_gap_us = gap_sum_us.checked_div(gap_samples).unwrap_or(0);
            let jitter_us = mad_sum_us.checked_div(gap_samples).unwrap_or(0);

            let report = BenchReportFrame {
                session_id: session_id.clone(),
                direction,
                duration_ms,
                bytes_transferred: bytes_sent,
                chunks: chunks_sent,
                throughput_mbps,
                avg_chunk_latency_us: avg_gap_us,
                jitter_us,
                stopped_early: Some(end_ms < deadline),
                timestamp_ms: end_ms,
            };

            let alias = DeviceId::parse(&peer.to_string())
                .ok()
                .and_then(|id| node.peer_alias(&id))
                .unwrap_or_else(|| peer.to_string());
            node.emit(UiEvent::bench_session_finished(
                &session_id,
                peer.to_string(),
                &alias,
                direction.as_str(),
                duration_ms,
                bytes_sent,
                chunks_sent,
                throughput_mbps,
                avg_gap_us,
                jitter_us,
                end_ms,
            ));
            node.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Info,
                "bench_sender_finished",
                Some(peer.to_string()),
                format!(
                    "Benchmark {} send window closed: {:.2} Mbps sent ({} bytes / {} chunks in {} ms)",
                    session_id, throughput_mbps, bytes_sent, chunks_sent, duration_ms
                ),
                true,
            );

            *node
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(report.clone());
            if let Ok(inner) = report.to_frame() {
                let _ = node.send_to(&peer, inner).await;
            }
        });
    }

    /// Query status of the active benchmark session and governance policy.
    pub fn bench_status(&self) -> BenchStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.bench_policy();
        let mut guard = self.shared.active_bench.lock().expect("active_bench");

        // Timeout fallback: finalize a session whose send window plus grace has
        // elapsed without a report (e.g. peer disconnected mid-run).
        let expired = guard
            .as_ref()
            .map(|s| {
                now_ms.saturating_sub(s.started_at_ms) > (s.duration_secs as u64) * 1000 + 10_000
            })
            .unwrap_or(false);

        if expired {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());
                let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
                let throughput_mbps =
                    (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;
                self.emit(UiEvent::bench_session_finished(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    session.direction.as_str(),
                    duration_ms,
                    session.bytes_transferred,
                    session.chunks_processed,
                    throughput_mbps,
                    0,
                    0,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Notice,
                    "bench_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Benchmark {} finalized by timeout without peer report",
                        session.session_id
                    ),
                    true,
                );
                let last_report = self
                    .shared
                    .last_bench_report
                    .lock()
                    .expect("last_bench_report")
                    .clone();
                return BenchStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    last_report,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };
        let last_report = self
            .shared
            .last_bench_report
            .lock()
            .expect("last_bench_report")
            .clone();

        BenchStatusReport {
            ok: true,
            active,
            session,
            policy,
            last_report,
            timestamp_ms: now_ms,
        }
    }

    /// Start a link throughput benchmark against a remote peer.
    pub async fn bench_start(
        &self,
        peer_query: &str,
        direction: BenchDirection,
        duration_secs: Option<u32>,
        chunk_size: Option<u32>,
    ) -> Result<BenchDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.bench_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "link benchmarking disabled".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under bench policy filters".to_string(),
            ));
        }

        let req_duration = BenchValidator::clamp_duration(
            duration_secs.unwrap_or(5).min(policy.max_duration_secs),
        );
        let req_chunk = BenchValidator::clamp_chunk_size(chunk_size.unwrap_or(16384));

        let session_id = format!("bench-sess-{}", &now_ms.to_string()[6..]);
        let request = BenchRequestFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            direction,
            duration_secs: req_duration,
            chunk_size: req_chunk,
            timestamp_ms: now_ms,
        };

        BenchValidator::validate_request(&request)?;

        // Atomically reserve the single session slot before the await, so two
        // concurrent starts cannot both pass the emptiness check (TOCTOU).
        {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            if guard.is_some() {
                return Err(CoreError::InvalidPayload(
                    "a benchmark session is already active".to_string(),
                ));
            }
            *guard = Some(ActiveBenchSession {
                session_id: session_id.clone(),
                peer_id: peer_str.clone(),
                role: BenchRole::Initiator,
                direction,
                duration_secs: req_duration,
                chunk_size: req_chunk,
                chunks_processed: 0,
                bytes_transferred: 0,
                started_at_ms: now_ms,
                last_activity_ms: now_ms,
            });
        }

        let inner = request.to_frame()?;
        if let Err(e) = self.send_to(&peer_id, inner).await {
            // Roll back the reservation so a later start is not blocked forever.
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            if guard
                .as_ref()
                .map(|s| s.session_id == session_id)
                .unwrap_or(false)
            {
                *guard = None;
            }
            return Err(e);
        }

        self.emit(UiEvent::bench_session_started(
            &session_id,
            &peer_str,
            &alias,
            "initiator",
            direction.as_str(),
            req_duration,
            req_chunk,
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "bench_started",
            Some(peer_str.clone()),
            format!(
                "Started benchmark {} against peer {} (direction={}, duration={}s, chunk={}B)",
                session_id,
                peer_str,
                direction.as_str(),
                req_duration,
                req_chunk
            ),
            true,
        );

        Ok(BenchDecisionReport {
            ok: true,
            action: "start".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            message: "link benchmark session requested".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Stop the active benchmark session early, notifying the peer.
    pub async fn bench_stop(&self) -> Result<BenchDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();

        let active_opt = {
            let mut guard = self.shared.active_bench.lock().expect("active_bench");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let duration_ms = now_ms.saturating_sub(session.started_at_ms).max(1);
            let throughput_mbps =
                (session.bytes_transferred as f64 * 8.0) / (duration_ms as f64 / 1000.0) / 1e6;

            let report = BenchReportFrame {
                session_id: session.session_id.clone(),
                direction: session.direction,
                duration_ms,
                bytes_transferred: session.bytes_transferred,
                chunks: session.chunks_processed,
                throughput_mbps,
                avg_chunk_latency_us: 0,
                jitter_us: 0,
                stopped_early: Some(true),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = report.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            *self
                .shared
                .last_bench_report
                .lock()
                .expect("last_bench_report") = Some(report);

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::bench_session_finished(
                &session.session_id,
                &session.peer_id,
                &alias,
                session.direction.as_str(),
                duration_ms,
                session.bytes_transferred,
                session.chunks_processed,
                throughput_mbps,
                0,
                0,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "bench_stopped",
                Some(session.peer_id.clone()),
                format!("Benchmark {} stopped early by user", session.session_id),
                true,
            );

            Ok(BenchDecisionReport {
                ok: true,
                action: "stop".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                message: "benchmark session stopped".to_string(),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(BenchDecisionReport {
                ok: true,
                action: "stop".to_string(),
                session_id: None,
                peer_id: None,
                message: "no active benchmark session to stop".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current bench governance policy.
    pub fn bench_policy(&self) -> BenchPolicy {
        self.shared
            .bench_policy
            .lock()
            .expect("bench_policy")
            .clone()
    }

    /// Status and configuration report for bench policy.
    pub fn bench_policy_report(&self) -> BenchPolicyReport {
        BenchPolicyReport {
            ok: true,
            policy: self.bench_policy(),
            message: "bench policy report".to_string(),
        }
    }

    /// Update bench policy and persist to disk atomically with 0o600 permissions.
    pub fn set_bench_policy(
        &self,
        mut policy: BenchPolicy,
    ) -> Result<BenchPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_bench_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save bench_policy.json: {e}")))?;
        *self.shared.bench_policy.lock().expect("bench_policy") = policy.clone();

        self.emit(UiEvent::bench_policy_changed(
            policy.enabled,
            policy.allow_inbound_bench,
            policy.max_duration_secs,
            policy.max_bandwidth_mbps,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "bench_policy_changed",
            None::<String>,
            format!(
                "Updated bench policy: enabled={}, allow_inbound={}, max_duration={}s, max_bandwidth={}Mbps",
                policy.enabled, policy.allow_inbound_bench, policy.max_duration_secs, policy.max_bandwidth_mbps
            ),
            true,
        );

        Ok(BenchPolicyReport {
            ok: true,
            policy,
            message: "bench policy updated".to_string(),
        })
    }

    /// Partially update bench policy and persist to disk.
    pub fn update_bench_policy(
        &self,
        update: BenchPolicyUpdate,
    ) -> Result<BenchPolicyReport, CoreError> {
        let mut policy = self.bench_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_bench {
            policy.allow_inbound_bench = v;
        }
        if let Some(v) = update.max_duration_secs {
            policy.max_duration_secs = v;
        }
        if let Some(v) = update.max_bandwidth_mbps {
            policy.max_bandwidth_mbps = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_bench_policy(policy)
    }

    /// Bandwidth-profile-driven streaming parameter recommendation for one peer
    /// (ADR-078). Falls back to the conservative minimal tier when no
    /// receiver-side measurement exists for the peer yet.
    pub fn streaming_recommendation(
        &self,
        peer_query: &str,
    ) -> Result<StreamingRecommendationReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let profile = self
            .shared
            .bandwidth_profiles
            .lock()
            .expect("bandwidth_profiles")
            .get(&peer_str)
            .cloned();

        let (measured_mbps, direction, measured_at_ms, has_profile) = match &profile {
            Some(p) => (p.measured_mbps, p.direction.clone(), p.measured_at_ms, true),
            None => (0.0, "none".to_string(), 0, false),
        };
        let recommendation = recommend_streaming_params(measured_mbps);
        // A recommendation that cannot survive the owning validators is a
        // logic bug; surface it instead of returning unusable tiers.
        recommendation.validate()?;

        Ok(StreamingRecommendationReport {
            ok: true,
            peer_id: peer_str,
            peer_alias: alias,
            has_profile,
            measured_mbps,
            direction,
            measured_at_ms,
            recommendation,
            timestamp_ms: now_ms,
        })
    }

    /// Aggregate recommendation status across all known peer profiles.
    pub fn recommendation_status(&self) -> RecommendationStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let table = self
            .shared
            .bandwidth_profiles
            .lock()
            .expect("bandwidth_profiles")
            .clone();
        let peers = table
            .peers
            .iter()
            .map(|p| {
                let alias = DeviceId::parse(&p.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| p.peer_id.clone());
                PeerRecommendationSummary {
                    peer_id: p.peer_id.clone(),
                    peer_alias: alias,
                    measured_mbps: p.measured_mbps,
                    direction: p.direction.clone(),
                    measured_at_ms: p.measured_at_ms,
                    tier: recommend_streaming_params(p.measured_mbps).tier,
                }
            })
            .collect();
        RecommendationStatusReport {
            ok: true,
            peers,
            timestamp_ms: now_ms,
        }
    }

    /// Polls system clipboard for automatic outbound synchronization.
    /// Checks mode, changes, size constraints, sensitive content guard, and profile scoping.
    /// Returns the number of eligible peers to which the clipboard text was broadcast.
    pub async fn poll_clipboard_auto_sync(&self) -> Result<usize, CoreError> {
        let policy = self.clipboard_policy();
        if !policy.mode.can_send() {
            return Ok(0);
        }

        let text = match self.shared.clip.get_text()? {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(0),
        };

        let hash = tetherly_core::hex_sha256(text.as_bytes());

        // Check against last synced hash
        {
            let last = self
                .shared
                .last_auto_sync_hash
                .lock()
                .expect("last_auto_sync_hash");
            if last.as_deref() == Some(&hash) {
                return Ok(0);
            }
        }

        // Also check if hub already knows this hash as the latest
        {
            let hubs = self.shared.hubs.lock().expect("hubs");
            if hubs.clip.last_hash() == Some(&hash) {
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                return Ok(0);
            }
        }

        let now = self.shared.clock.unix_ms();

        // 1. Max payload ceiling guard
        if text.len() > policy.max_payload_bytes {
            *self
                .shared
                .last_auto_sync_hash
                .lock()
                .expect("last_auto_sync_hash") = Some(hash.clone());
            self.shared
                .clip_sensitive_blocked_count
                .fetch_add(1, Ordering::Relaxed);
            self.emit(UiEvent::clipboard_sync_blocked(
                "payload_too_large",
                text.len(),
                &hash,
                now,
            ));
            self.record_audit(
                AuditCategory::Clipboard,
                AuditSeverity::Warning,
                "clipboard_payload_too_large",
                None::<String>,
                format!(
                    "Blocked auto-sync of large clipboard payload ({} bytes exceeds limit {})",
                    text.len(),
                    policy.max_payload_bytes
                ),
                false,
            );
            return Ok(0);
        }

        // 2. Sensitive content leak guard
        if policy.sensitive_guard {
            if let Some(reason) = SensitiveContentGuard::inspect(&text) {
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                self.shared
                    .clip_sensitive_blocked_count
                    .fetch_add(1, Ordering::Relaxed);
                let reason_str = reason.as_str();
                self.emit(UiEvent::clipboard_sync_blocked(
                    reason_str,
                    text.len(),
                    &hash,
                    now,
                ));
                self.record_audit(
                    AuditCategory::Clipboard,
                    AuditSeverity::Warning,
                    "sensitive_clipboard_blocked",
                    None::<String>,
                    format!(
                        "Blocked auto-sync of sensitive {} ({} bytes, sha256={})",
                        reason_str,
                        text.len(),
                        hash
                    ),
                    false,
                );
                return Ok(0);
            }
        }

        // 3. Resolve eligible peers based on profile rules
        let live_peers = self.live_peers();
        let eligible_peers: Vec<DeviceId> = live_peers
            .into_iter()
            .map(|p| p.device_id)
            .filter(|peer_id| {
                let profile = self.peer_profile(peer_id);
                policy.is_peer_eligible(&profile)
            })
            .collect();

        // Mark current hash as observed so we don't re-process
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());

        if eligible_peers.is_empty() {
            return Ok(0);
        }

        // Prepare frame
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.prepare_local(&text)?
        };
        let Some(set) = set else {
            return Ok(0);
        };

        let seq = set.clip_seq;
        let bytes = text.len();
        let frame = set.to_frame()?;
        let count = eligible_peers.len();

        for peer in &eligible_peers {
            let _ = self.send_to(peer, frame.clone()).await;
        }

        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }

        self.shared
            .clip_auto_synced_out_count
            .fetch_add(1, Ordering::Relaxed);

        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });

        // Auto clear local clipboard if requested by policy
        if let Some(secs) = policy.auto_clear_timeout_secs {
            if secs > 0 {
                let clip = self.shared.clip.clone();
                let expected_text = text.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    if let Ok(Some(cur)) = clip.get_text() {
                        if cur == expected_text {
                            let _ = clip.set_text("");
                        }
                    }
                });
            }
        }

        Ok(count)
    }

    fn spawn_clipboard_auto_sync(&self) {
        let node = self.clone_handle();
        tokio::spawn(async move {
            let mut stop = node.shared.shutdown.subscribe();
            loop {
                if *stop.borrow() {
                    break;
                }
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(800)) => {
                        let _ = node.poll_clipboard_auto_sync().await;
                    }
                }
            }
        });
    }

    /// Syncs local clipboard to a specific peer or all connected peers.
    /// Returns the number of peers that were sent the frame (0 if unchanged/echo or no peers).
    pub async fn send_clipboard_to(
        &self,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        let text = self
            .shared
            .clip
            .get_text()?
            .ok_or_else(|| CoreError::Json("empty clipboard".into()))?;
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        let now = self.shared.clock.unix_ms();
        let bytes = text.len();
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.prepare_local(&text)?
        };
        let Some(set) = set else {
            return Ok(0);
        };
        let seq = set.clip_seq;
        let hash = set.hash.clone();
        let frame = set.to_frame()?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let count = targets.len();
        for p in &targets {
            let _ = self.send_to(p, frame.clone()).await;
        }
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());
        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });
        Ok(count)
    }

    /// Syncs local clipboard to all connected peers.
    pub async fn send_clipboard(&self) -> Result<(), CoreError> {
        self.send_clipboard_to(None).await.map(|_| ())
    }

    /// Explicitly broadcasts `text` to a specific peer or all connected peers.
    /// Writes `text` to the local clipboard and advances sequence even if previously sent.
    pub async fn send_clipboard_text(
        &self,
        text: &str,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        self.shared.clip.set_text(text)?;
        let now = self.shared.clock.unix_ms();
        let bytes = text.len();
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.force_prepare_local(text)?
        };
        let seq = set.clip_seq;
        let hash = set.hash.clone();
        let frame = set.to_frame()?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let count = targets.len();
        for p in &targets {
            let _ = self.send_to(p, frame.clone()).await;
        }
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());
        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });
        Ok(count)
    }

    fn resolve_clip_targets(
        &self,
        target_peer: Option<&DeviceId>,
    ) -> Result<Vec<DeviceId>, CoreError> {
        match target_peer {
            Some(target) => {
                if !self.live_peers().iter().any(|p| &p.device_id == target) {
                    return Err(CoreError::Json(format!("peer {} is not connected", target)));
                }
                Ok(vec![target.clone()])
            }
            None => Ok(self.live_peers().into_iter().map(|p| p.device_id).collect()),
        }
    }

    /// Reads the current text from the local clipboard.
    pub fn get_clipboard_text(&self) -> Result<Option<String>, CoreError> {
        self.shared.clip.get_text()
    }

    /// Inspect clipboard status, size, and synchronization statistics without leaking plaintext.
    ///
    /// An absent OS clipboard (headless Linux, no display server) is a state,
    /// not a failure: the report degrades with `clipboard_available: false`
    /// while sync statistics stay observable, instead of answering 500.
    pub fn clipboard_status(&self) -> Result<ClipStatusReport, CoreError> {
        let (has_text, bytes, clipboard_available, reason) = match self.shared.clip.get_text() {
            Ok(Some(ref t)) if !t.is_empty() => (true, t.len(), true, None),
            Ok(_) => (false, 0, true, None),
            Err(e) => (false, 0, false, Some(e.to_string())),
        };
        let hubs = self.shared.hubs.lock().expect("hubs");
        let (inbound_count, outbound_count) = hubs.clip.stats();
        let last_seq = hubs.clip.last_seq();
        let hash = hubs.clip.last_hash().map(|s| s.to_string());
        let history_count = hubs.clip.history_count();
        Ok(ClipStatusReport {
            ok: true,
            has_text,
            bytes,
            hash,
            last_seq,
            inbound_count,
            outbound_count,
            history_count,
            clipboard_available,
            reason,
        })
    }

    /// Bounded clipboard history query without leaking plaintext.
    pub fn clipboard_history(&self, limit: Option<usize>) -> ClipHistoryReport {
        let hubs = self.shared.hubs.lock().expect("hubs");
        let entries = hubs.clip.history(limit);
        let total = entries.len();
        let dtos = entries
            .into_iter()
            .map(|e| ClipMetaEntryDto {
                seq: e.seq,
                source: e
                    .source
                    .map_or_else(|| "local".to_string(), |s| s.to_string()),
                direction: match e.direction {
                    ClipDirection::Inbound => "inbound".to_string(),
                    ClipDirection::Outbound => "outbound".to_string(),
                },
                bytes: e.bytes,
                hash: e.hash,
                timestamp_ms: e.timestamp_ms,
            })
            .collect();
        ClipHistoryReport {
            ok: true,
            total,
            entries: dtos,
        }
    }

    /// Clears the system clipboard and resets hub state and history buffer.
    pub fn clear_clipboard(&self) -> Result<ClipClearReport, CoreError> {
        // A host with no OS clipboard has nothing to clear: reset the sync
        // state anyway and report the absence instead of failing.
        let clip_err = self.shared.clip.set_text("").err();
        let cleared_history_count = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.reset_state()
        };
        let now_ms = self.shared.clock.unix_ms();
        self.emit(UiEvent::ClipboardCleared {
            timestamp_ms: now_ms,
        });
        Ok(ClipClearReport {
            ok: true,
            cleared_history_count,
            timestamp_ms: now_ms,
            clipboard_available: clip_err.is_none(),
            reason: clip_err.map(|e| e.to_string()),
        })
    }

    pub async fn offer_and_send_bytes(
        &self,
        peer: &DeviceId,
        name: &str,
        data: &[u8],
        _wait_accept: bool,
    ) -> Result<String, CoreError> {
        // Symmetric with the receiver's cap: never offer bytes the peer is
        // obliged to refuse, and never clone a body we cannot hold.
        if data.len() as u64 > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        // Content-addressed: the same bytes re-offered after an interruption
        // carry the same id, so the receiver's persisted partial can be matched
        // and resumed. Different content -> different id -> fresh transfer.
        let digest = tetherly_core::hex_sha256(data);
        let transfer_id = format!("tr_{}", &digest[..16]);
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: name.into(),
                size: data.len() as u64,
                sha256: digest.clone(),
            }],
        };
        self.shared.pending_out.lock().expect("pending").insert(
            transfer_id.clone(),
            OutboundFile {
                peer: peer.clone(),
                name: name.into(),
                source: OutboundSource::Bytes(data.to_vec()),
                size: data.len() as u64,
            },
        );
        let now = self.shared.clock.unix_ms();
        self.shared
            .active_transfers
            .lock()
            .expect("transfers")
            .insert(
                transfer_id.clone(),
                ActiveTransfer {
                    transfer_id: transfer_id.clone(),
                    peer: peer.clone(),
                    direction: TransferDirection::Outbound,
                    filename: name.into(),
                    total: data.len() as u64,
                    transferred: 0,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: None,
                    is_resumed: false,
                },
            );
        let outbox_id = format!("out_{transfer_id}_{now}");
        let alias = self.peer_alias(peer);
        self.shared
            .outbox_engine
            .lock()
            .expect("outbox_engine")
            .push(OutboxEntry {
                id: outbox_id.clone(),
                transfer_id: transfer_id.clone(),
                peer: peer.to_string(),
                peer_alias: alias,
                filename: name.to_string(),
                size: data.len() as u64,
                sha256: digest,
                source_path: None,
                status: OutboxStatus::InFlight,
                error: None,
                started_ms: now,
                completed_ms: None,
                speed_bps: 0,
            });
        self.emit(UiEvent::outbox_updated(
            outbox_id,
            &transfer_id,
            peer.to_string(),
            name,
            data.len() as u64,
            OutboxStatus::InFlight.as_str(),
            now,
        ));
        self.send_to(peer, offer.to_frame()?).await?;
        let _ = _wait_accept;
        Ok(transfer_id)
    }

    /// Offer and stream a file directly from a filesystem path. Reads the file
    /// in 64 KiB chunks for incremental SHA-256 calculation and outbound streaming,
    /// avoiding full in-memory buffering (O(1) memory bound).
    pub async fn offer_and_send_file(
        &self,
        peer: &DeviceId,
        path: &std::path::Path,
        name: Option<&str>,
    ) -> Result<String, CoreError> {
        if !path.is_file() {
            return Err(CoreError::File("not a regular file".into()));
        }
        let meta = std::fs::metadata(path)?;
        let size = meta.len();
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let filename = name
            .map(|s| s.to_string())
            .or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "file.bin".to_string());
        let safe_name = tetherly_core::sanitize_file_name(&filename)?;

        // Stream file in 64 KiB chunks to compute SHA-256 with O(1) memory.
        let mut hasher = tetherly_core::IncrementalSha256::new();
        {
            use std::io::Read;
            let mut f = std::fs::File::open(path)?;
            let mut buf = [0u8; tetherly_core::FILE_CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        let digest = hasher.finalize_hex();
        let transfer_id = format!("tr_{}", &digest[..16]);
        let offer = FileOffer {
            transfer_id: transfer_id.clone(),
            files: vec![tetherly_core::FileMeta {
                name: safe_name.clone(),
                size,
                sha256: digest.clone(),
            }],
        };
        self.shared.pending_out.lock().expect("pending").insert(
            transfer_id.clone(),
            OutboundFile {
                peer: peer.clone(),
                name: safe_name.clone(),
                source: OutboundSource::Path(path.to_path_buf()),
                size,
            },
        );
        let now = self.shared.clock.unix_ms();
        self.shared
            .active_transfers
            .lock()
            .expect("transfers")
            .insert(
                transfer_id.clone(),
                ActiveTransfer {
                    transfer_id: transfer_id.clone(),
                    peer: peer.clone(),
                    direction: TransferDirection::Outbound,
                    filename: safe_name.clone(),
                    total: size,
                    transferred: 0,
                    speed_bps: 0,
                    phase: TransferPhase::Transferring,
                    error: None,
                    started_ms: now,
                    updated_ms: now,
                    resume_offset: None,
                    is_resumed: false,
                },
            );
        let outbox_id = format!("out_{transfer_id}_{now}");
        let alias = self.peer_alias(peer);
        self.shared
            .outbox_engine
            .lock()
            .expect("outbox_engine")
            .push(OutboxEntry {
                id: outbox_id.clone(),
                transfer_id: transfer_id.clone(),
                peer: peer.to_string(),
                peer_alias: alias,
                filename: safe_name.clone(),
                size,
                sha256: digest,
                source_path: Some(path.to_string_lossy().to_string()),
                status: OutboxStatus::InFlight,
                error: None,
                started_ms: now,
                completed_ms: None,
                speed_bps: 0,
            });
        self.emit(UiEvent::outbox_updated(
            outbox_id,
            &transfer_id,
            peer.to_string(),
            &safe_name,
            size,
            OutboxStatus::InFlight.as_str(),
            now,
        ));
        self.send_to(peer, offer.to_frame()?).await?;
        Ok(transfer_id)
    }

    /// Offer and stream a file to multiple targets matched by a `TargetSelector`.
    /// Performs single-pass file reading and SHA-256 calculation (O(1) memory bound),
    /// generates unique transfer IDs and Outbox entries for each online peer,
    /// sends FileOffer frames concurrently, and produces an aggregated MultiFileSendReport.
    pub async fn offer_and_send_file_multi(
        &self,
        selector: &TargetSelector,
        path: &std::path::Path,
        name: Option<&str>,
    ) -> Result<MultiFileSendReport, CoreError> {
        if !path.is_file() {
            return Err(CoreError::File("not a regular file".into()));
        }
        let meta = std::fs::metadata(path)?;
        let size = meta.len();
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let filename = name
            .map(|s| s.to_string())
            .or_else(|| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "file.bin".to_string());
        let safe_name = tetherly_core::sanitize_file_name(&filename)?;

        // Single-pass stream in 64 KiB chunks to compute SHA-256 with O(1) memory bound
        let mut hasher = tetherly_core::IncrementalSha256::new();
        {
            use std::io::Read;
            let mut f = std::fs::File::open(path)?;
            let mut buf = [0u8; tetherly_core::FILE_CHUNK];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
        }
        let digest = hasher.finalize_hex();

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now = self.shared.clock.unix_ms();
        static MULTI_TRANSFER_COUNTER: AtomicU64 = AtomicU64::new(1);

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerFileSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    transfer_id: None,
                    outbox_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = MULTI_TRANSFER_COUNTER.fetch_add(1, Ordering::Relaxed);
            let peer_short = &peer_id.as_str()[..8.min(peer_id.as_str().len())];
            let transfer_id = format!("tr_{}_{}_{seq}", &digest[..10], peer_short);
            let offer = FileOffer {
                transfer_id: transfer_id.clone(),
                files: vec![tetherly_core::FileMeta {
                    name: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                }],
            };

            self.shared.pending_out.lock().expect("pending").insert(
                transfer_id.clone(),
                OutboundFile {
                    peer: peer_id.clone(),
                    name: safe_name.clone(),
                    source: OutboundSource::Path(path.to_path_buf()),
                    size,
                },
            );

            self.shared
                .active_transfers
                .lock()
                .expect("transfers")
                .insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: peer_id.clone(),
                        direction: TransferDirection::Outbound,
                        filename: safe_name.clone(),
                        total: size,
                        transferred: 0,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: None,
                        is_resumed: false,
                    },
                );

            let outbox_id = format!("out_{transfer_id}_{now}");
            self.shared
                .outbox_engine
                .lock()
                .expect("outbox_engine")
                .push(OutboxEntry {
                    id: outbox_id.clone(),
                    transfer_id: transfer_id.clone(),
                    peer: peer_str.clone(),
                    peer_alias: alias.clone(),
                    filename: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                    source_path: Some(path.to_string_lossy().to_string()),
                    status: OutboxStatus::InFlight,
                    error: None,
                    started_ms: now,
                    completed_ms: None,
                    speed_bps: 0,
                });

            self.emit(UiEvent::outbox_updated(
                outbox_id.clone(),
                &transfer_id,
                peer_id.to_string(),
                &safe_name,
                size,
                OutboxStatus::InFlight.as_str(),
                now,
            ));

            let frame_res = offer.to_frame();
            let send_res = match frame_res {
                Ok(frame) => self.send_to(peer_id, frame).await,
                Err(e) => Err(e),
            };

            match send_res {
                Ok(_) => {
                    success_count += 1;
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    let err_str = e.to_string();
                    self.shared
                        .pending_out
                        .lock()
                        .expect("pending")
                        .remove(&transfer_id);
                    if let Ok(mut engine) = self.shared.outbox_engine.lock() {
                        engine.update_status(
                            &transfer_id,
                            OutboxStatus::Failed,
                            Some(err_str.clone()),
                            Some(now),
                            Some(0),
                        );
                    }
                    if let Ok(mut transfers) = self.shared.active_transfers.lock() {
                        if let Some(t) = transfers.get_mut(&transfer_id) {
                            t.phase = TransferPhase::Failed;
                            t.error = Some(err_str.clone());
                            t.updated_ms = now;
                        }
                    }
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: false,
                        error: Some(err_str),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "file_multi_sent",
            None::<String>,
            format!(
                "Multicast file '{}' ({size} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                safe_name,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "file",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now,
        ));

        Ok(MultiFileSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            filename: safe_name,
            size,
            sha256: digest,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    pub async fn offer_and_send_bytes_multi(
        &self,
        selector: &TargetSelector,
        name: &str,
        data: &[u8],
    ) -> Result<MultiFileSendReport, CoreError> {
        let size = data.len() as u64;
        if size > tetherly_core::FILE_MAX {
            return Err(CoreError::FileTooLarge(tetherly_core::FILE_MAX));
        }
        let safe_name = tetherly_core::sanitize_file_name(name)?;
        let digest = tetherly_core::hex_sha256(data);

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now = self.shared.clock.unix_ms();
        static MULTI_BYTES_COUNTER: AtomicU64 = AtomicU64::new(1);

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerFileSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    transfer_id: None,
                    outbox_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = MULTI_BYTES_COUNTER.fetch_add(1, Ordering::Relaxed);
            let peer_short = &peer_id.as_str()[..8.min(peer_id.as_str().len())];
            let transfer_id = format!("tr_{}_{}_{seq}", &digest[..10], peer_short);
            let offer = FileOffer {
                transfer_id: transfer_id.clone(),
                files: vec![tetherly_core::FileMeta {
                    name: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                }],
            };

            self.shared.pending_out.lock().expect("pending").insert(
                transfer_id.clone(),
                OutboundFile {
                    peer: peer_id.clone(),
                    name: safe_name.clone(),
                    source: OutboundSource::Bytes(data.to_vec()),
                    size,
                },
            );

            self.shared
                .active_transfers
                .lock()
                .expect("transfers")
                .insert(
                    transfer_id.clone(),
                    ActiveTransfer {
                        transfer_id: transfer_id.clone(),
                        peer: peer_id.clone(),
                        direction: TransferDirection::Outbound,
                        filename: safe_name.clone(),
                        total: size,
                        transferred: 0,
                        speed_bps: 0,
                        phase: TransferPhase::Transferring,
                        error: None,
                        started_ms: now,
                        updated_ms: now,
                        resume_offset: None,
                        is_resumed: false,
                    },
                );

            let outbox_id = format!("out_{transfer_id}_{now}");
            self.shared
                .outbox_engine
                .lock()
                .expect("outbox_engine")
                .push(OutboxEntry {
                    id: outbox_id.clone(),
                    transfer_id: transfer_id.clone(),
                    peer: peer_str.clone(),
                    peer_alias: alias.clone(),
                    filename: safe_name.clone(),
                    size,
                    sha256: digest.clone(),
                    source_path: None,
                    status: OutboxStatus::InFlight,
                    error: None,
                    started_ms: now,
                    completed_ms: None,
                    speed_bps: 0,
                });

            self.emit(UiEvent::outbox_updated(
                outbox_id.clone(),
                &transfer_id,
                peer_id.to_string(),
                &safe_name,
                size,
                OutboxStatus::InFlight.as_str(),
                now,
            ));

            let frame_res = offer.to_frame();
            let send_res = match frame_res {
                Ok(frame) => self.send_to(peer_id, frame).await,
                Err(e) => Err(e),
            };

            match send_res {
                Ok(_) => {
                    success_count += 1;
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    let err_str = e.to_string();
                    self.shared
                        .pending_out
                        .lock()
                        .expect("pending")
                        .remove(&transfer_id);
                    if let Ok(mut engine) = self.shared.outbox_engine.lock() {
                        engine.update_status(
                            &transfer_id,
                            OutboxStatus::Failed,
                            Some(err_str.clone()),
                            Some(now),
                            Some(0),
                        );
                    }
                    if let Ok(mut transfers) = self.shared.active_transfers.lock() {
                        if let Some(t) = transfers.get_mut(&transfer_id) {
                            t.phase = TransferPhase::Failed;
                            t.error = Some(err_str.clone());
                            t.updated_ms = now;
                        }
                    }
                    dispatches.push(PeerFileSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        transfer_id: Some(transfer_id),
                        outbox_id: Some(outbox_id),
                        success: false,
                        error: Some(err_str),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Info,
            "file_multi_sent",
            None::<String>,
            format!(
                "Multicast in-memory file '{}' ({size} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                safe_name,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "file",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now,
        ));

        Ok(MultiFileSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            filename: safe_name,
            size,
            sha256: digest,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Offer and stream multiple files as a batch operation to a connected peer.
    pub async fn send_files_batch(
        &self,
        peer: &DeviceId,
        paths: &[std::path::PathBuf],
    ) -> Result<BatchSendReport, CoreError> {
        let items: Vec<BatchSendItem> = paths
            .iter()
            .map(|p| BatchSendItem {
                path: Some(p.to_string_lossy().to_string()),
                name: None,
                data_base64: None,
            })
            .collect();
        self.send_batch_items(peer, items).await
    }

    /// Submit a heterogeneous batch of file paths or in-memory base64 buffers to a peer.
    pub async fn send_batch_items(
        &self,
        peer: &DeviceId,
        items: Vec<BatchSendItem>,
    ) -> Result<BatchSendReport, CoreError> {
        let total_files = items.len();
        if total_files == 0 {
            return Err(CoreError::File("batch items list is empty".into()));
        }
        let batch_hex = tetherly_core::hex_sha256(
            format!("{}:{}:{}", peer, self.shared.clock.unix_ms(), total_files).as_bytes(),
        );
        let batch_id = format!("tbt_{}", &batch_hex[..12]);
        let mut reports = Vec::with_capacity(total_files);
        let mut total_bytes: u64 = 0;
        let mut successful_files = 0;
        let mut failed_files = 0;

        for item in items {
            if let Some(ref path_str) = item.path {
                let p = std::path::Path::new(path_str);
                let fname = item.name.clone().unwrap_or_else(|| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("file.bin")
                        .to_string()
                });
                let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                    Ok(n) => n,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: fname,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid file name: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                if !p.is_file() {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size: 0,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("not a regular file or file not found".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                let meta = match std::fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("cannot read file metadata: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let size = meta.len();
                if size > tetherly_core::FILE_MAX {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("file exceeds FILE_MAX (64 GiB)".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                match self.offer_and_send_file(peer, p, Some(&safe_name)).await {
                    Ok(transfer_id) => {
                        let sha = transfer_id.strip_prefix("tr_").map(|s| s.to_string());
                        reports.push(BatchTransferItemReport {
                            transfer_id: Some(transfer_id),
                            name: safe_name,
                            size,
                            sha256: sha,
                            status: "sent".to_string(),
                            error: None,
                        });
                        successful_files += 1;
                        total_bytes += size;
                    }
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        });
                        failed_files += 1;
                    }
                }
            } else if let Some(ref b64) = item.data_base64 {
                use base64::Engine;
                let decoded = match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                {
                    Ok(b) => b,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: item.name.clone().unwrap_or_else(|| "payload.bin".into()),
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid base64: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let fname = item
                    .name
                    .clone()
                    .unwrap_or_else(|| "upload.bin".to_string());
                let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                    Ok(n) => n,
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: fname,
                            size: 0,
                            sha256: None,
                            status: "failed".to_string(),
                            error: Some(format!("invalid file name: {e}")),
                        });
                        failed_files += 1;
                        continue;
                    }
                };
                let size = decoded.len() as u64;
                if size > tetherly_core::FILE_MAX {
                    reports.push(BatchTransferItemReport {
                        transfer_id: None,
                        name: safe_name,
                        size,
                        sha256: None,
                        status: "failed".to_string(),
                        error: Some("file exceeds FILE_MAX (64 GiB)".into()),
                    });
                    failed_files += 1;
                    continue;
                }
                let digest = tetherly_core::hex_sha256(&decoded);
                match self
                    .offer_and_send_bytes(peer, &safe_name, &decoded, false)
                    .await
                {
                    Ok(transfer_id) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: Some(transfer_id),
                            name: safe_name,
                            size,
                            sha256: Some(digest),
                            status: "sent".to_string(),
                            error: None,
                        });
                        successful_files += 1;
                        total_bytes += size;
                    }
                    Err(e) => {
                        reports.push(BatchTransferItemReport {
                            transfer_id: None,
                            name: safe_name,
                            size,
                            sha256: Some(digest),
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        });
                        failed_files += 1;
                    }
                }
            } else {
                reports.push(BatchTransferItemReport {
                    transfer_id: None,
                    name: item.name.clone().unwrap_or_else(|| "unknown".into()),
                    size: 0,
                    sha256: None,
                    status: "failed".to_string(),
                    error: Some("either path or data_base64 must be provided".into()),
                });
                failed_files += 1;
            }
        }

        let peer_alias = self.peer_alias(peer);
        Ok(BatchSendReport {
            ok: successful_files > 0 || total_files == 0,
            batch_id,
            peer: peer.to_string(),
            peer_alias,
            total_files,
            successful_files,
            failed_files,
            total_bytes,
            items: reports,
        })
    }

    async fn on_peer_accepted_file(&self, decision: &tetherly_core::FileDecision) {
        let transfer_id = decision.transfer_id.as_str();
        let (outbound, cancel_rx) = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut pending = self.shared.pending_out.lock().expect("pending");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");

            let Some(t) = transfers.get_mut(transfer_id) else {
                return;
            };
            if t.phase == TransferPhase::Failed {
                // User cancelled before acceptance frame arrived
                pending.remove(transfer_id);
                return;
            }
            let Some(outbound) = pending.remove(transfer_id) else {
                return;
            };

            let start = decision.resume_from.unwrap_or(0);
            if start > outbound.size {
                warn!(transfer = %transfer_id, "resume offset beyond local data; aborting");
                t.phase = TransferPhase::Failed;
                t.error = Some("invalid resume offset".into());
                t.speed_bps = 0;
                t.updated_ms = self.shared.clock.unix_ms();
                return;
            }

            let now = self.shared.clock.unix_ms();
            t.transferred = start;
            if start > 0 {
                t.resume_offset = Some(start);
                t.is_resumed = true;
            }
            t.phase = TransferPhase::Transferring;
            t.updated_ms = now;

            let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
            cancels.insert(transfer_id.to_string(), cancel_tx);
            (outbound, cancel_rx)
        };
        let (addr, file_port, token) = {
            let live = self.shared.live.lock().expect("live");
            let Some(peer) = live.get(&outbound.peer) else {
                warn!(transfer = %transfer_id, "accepted file but peer gone");
                self.set_transfer_phase(
                    transfer_id,
                    TransferPhase::Failed,
                    Some("peer gone".into()),
                );
                return;
            };
            let token = tetherly_crypto::file_token(&peer.handshake_hash, transfer_id.as_bytes());
            (peer.addr, peer.file_port.unwrap_or(self.file_port()), token)
        };
        let start = decision.resume_from.unwrap_or(0);
        let mut cancel_rx = cancel_rx;
        let node = self.clone_handle();
        let tid = transfer_id.to_string();
        tokio::spawn(async move {
            let send_fut = async {
                match outbound.source {
                    OutboundSource::Path(p) => match tokio::fs::File::open(&p).await {
                        Ok(mut f) => {
                            if start > 0 {
                                use tokio::io::AsyncSeekExt;
                                if let Err(e) = f.seek(std::io::SeekFrom::Start(start)).await {
                                    warn!(%e, transfer = %tid, "failed to seek in source file");
                                    return Err(tetherly_net::NetError::Io(e));
                                }
                            }
                            let dest = SocketAddr::new(addr.ip(), file_port);
                            match TcpStream::connect(dest).await {
                                Ok(mut stream) => {
                                    let node_prog = node.clone_handle();
                                    let tid_prog = tid.clone();
                                    tetherly_net::filechan::send_stream_from_with_progress(
                                        &mut f,
                                        &mut stream,
                                        &token,
                                        &tid,
                                        outbound.size,
                                        start,
                                        move |offset, len| {
                                            node_prog.update_transfer_progress(
                                                &tid_prog,
                                                offset + len as u64,
                                            );
                                            node_prog.shared.telemetry.record_file_tx(len as u64);
                                            Ok(())
                                        },
                                    )
                                    .await
                                }
                                Err(e) => Err(tetherly_net::NetError::Io(e)),
                            }
                        }
                        Err(e) => Err(tetherly_net::NetError::Io(e)),
                    },
                    OutboundSource::Bytes(ref b) => {
                        node.send_file_data(addr, file_port, &token, &tid, outbound.size, start, b)
                            .await
                    }
                }
            };

            let res = tokio::select! {
                _ = &mut cancel_rx => {
                    info!(transfer = %tid, "outbound transfer cancelled by user");
                    node.shared.transfer_cancels.lock().expect("cancels").remove(&tid);
                    node.set_transfer_phase(
                        &tid,
                        TransferPhase::Failed,
                        Some("cancelled by user".into()),
                    );
                    return;
                }
                r = send_fut => {
                    node.shared.transfer_cancels.lock().expect("cancels").remove(&tid);
                    r
                }
            };

            match res {
                Ok(()) => {
                    node.set_transfer_phase(&tid, TransferPhase::Completed, None);
                }
                Err(ref e) => {
                    warn!(%e, transfer = %tid, "file data send failed");
                    node.set_transfer_phase(&tid, TransferPhase::Failed, Some(e.to_string()));
                }
            }
        });
    }

    /// Send file bytes from `start` (the resume offset) to the end of `data`,
    /// where `data` holds the whole file and `total` is its length on the wire.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_file_data(
        &self,
        peer_addr: SocketAddr,
        file_port: u16,
        token: &[u8; 32],
        transfer_id: &str,
        total: u64,
        start: u64,
        data: &[u8],
    ) -> Result<(), tetherly_net::NetError> {
        let dest = SocketAddr::new(peer_addr.ip(), file_port);
        let mut stream = TcpStream::connect(dest).await?;
        let res = tetherly_net::filechan::send_chunks_from(
            &mut stream,
            token,
            transfer_id,
            total,
            start,
            &data[start as usize..],
        )
        .await;
        if res.is_ok() {
            self.update_transfer_progress(transfer_id, total);
            self.shared
                .telemetry
                .record_file_tx(total.saturating_sub(start));
        }
        res
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

    pub fn input_port(&self) -> u16 {
        self.shared.bound_input.load(Ordering::SeqCst)
    }

    pub fn control_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.control_port()))
    }

    pub fn file_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.file_port()))
    }

    pub fn input_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.input_port()))
    }

    pub fn screen_port(&self) -> u16 {
        self.shared.bound_screen.load(Ordering::SeqCst)
    }

    pub fn screen_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.screen_port()))
    }

    pub fn input_seat(&self) -> Option<CursorSeat> {
        *self.shared.input_seat.lock().expect("seat")
    }

    /// "remote" while the logical cursor is on a peer, else "local". Encoding
    /// lives here so the UI never has to mirror the enum.
    pub fn input_seat_str(&self) -> &'static str {
        match self.input_seat() {
            Some(CursorSeat::Remote) => "remote",
            _ => "local",
        }
    }

    pub fn input_sink_snapshot(&self) -> MemorySink {
        self.shared.input_sink.lock().expect("sink").clone()
    }

    /// Open the resume-only input channel. Pairing on 45719 is refused.
    pub async fn open_input(&self, peer: &DeviceId) -> Result<(), tetherly_net::NetError> {
        let addr = {
            let live = self.shared.live.lock().expect("live");
            let p = live
                .get(peer)
                .ok_or(tetherly_net::NetError::InputRequiresTrust)?;
            let port = p.input_port.ok_or(tetherly_net::NetError::Handshake)?;
            SocketAddr::new(p.addr.ip(), port)
        };
        let stream = TcpStream::connect(addr).await?;
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        let sess = match dial_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        self.attach_input_server(sess).await;
        Ok(())
    }

    /// True while an input control channel to a peer is attached. The UI uses
    /// this to dial the channel on demand instead of tracking sessions.
    pub async fn input_channel_open(&self) -> bool {
        self.shared.input_out.lock().await.is_some()
    }

    /// Controller: stop forwarding local input to the remote seat and release
    /// the channel. The capture hooks stay armed (they are cheap while local).
    pub async fn input_stop(&self) -> Result<(), CoreError> {
        *self.shared.input_out.lock().await = None;
        let peer = self.shared.input_peer.lock().expect("input_peer").take();
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .on_peer_gone();
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        self.emit(UiEvent::input_seat_changed(
            "local",
            false,
            peer.as_ref().map(|p| p.to_string()),
        ));
        Ok(())
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

    /// Start the controller-side capture: low-level mouse hook (edge
    /// detection) and a keyboard hook armed only while the seat is remote.
    /// Events feed the same edge state machine `drive_input` uses, so CI and
    /// production share one path.
    #[cfg(windows)]
    pub fn spawn_input_capture(&self) -> Result<(), CoreError> {
        let (tx, rx) = std::sync::mpsc::channel::<crate::platform::RawInput>();
        let control = crate::platform::start_input_hooks(tx)?;
        let (w, h) = crate::platform::screen_size();
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .resize(w.max(1), h.max(1));
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| CoreError::Json("input capture needs a tokio runtime".into()))?;
        let node = self.clone_handle();
        std::thread::spawn(move || {
            let mut keyboard_armed = false;
            for ev in rx {
                // A failed send means the remote input channel is not open (yet)
                // or just closed. That is transient, not fatal: keep the hook
                // alive so a later `open_input` starts delivering again, rather
                // than wedging input sharing until process restart. Events are
                // paced by real user input, so retrying cannot busy-spin.
                match ev {
                    crate::platform::RawInput::Move { x, y } => {
                        let _ = handle.block_on(node.drive_input(x, y, w, h));
                    }
                    crate::platform::RawInput::Button { mask, down } => {
                        let _ = handle.block_on(node.drive_input_button(mask, down));
                    }
                    crate::platform::RawInput::Wheel { dx, dy } => {
                        let _ = handle.block_on(node.drive_input_wheel(dx, dy));
                    }
                    crate::platform::RawInput::Key { code, down } => {
                        let _ = handle.block_on(node.drive_input_key(code, down));
                    }
                }
                // The keyboard hook records real keystrokes: arm it only for
                // the remote seat, disarm the moment the cursor comes home.
                let remote = node.input_engine_seat() == CursorSeat::Remote;
                if remote != keyboard_armed && control.set_keyboard(remote).is_ok() {
                    keyboard_armed = remote;
                }
            }
        });
        Ok(())
    }

    /// Non-Windows builds have no low-level input hooks: report an honest
    /// error rather than silently doing nothing (macOS/Linux capture is
    /// Manual-required per the SSOT).
    #[cfg(not(windows))]
    pub fn spawn_input_capture(&self) -> Result<(), CoreError> {
        Err(CoreError::Json(
            "input capture is Windows-only in this build; macOS/Linux hooks are Manual-required"
                .into(),
        ))
    }

    /// Push local cursor position toward the edge state machine. Crossing the
    /// right edge emits Enter + Move. Seq is monotonic for the life of the
    /// node. No JSON, and never onto 45717. OS hooks stay Manual-required.
    pub async fn drive_input(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<tetherly_core::ScreenEdge>, CoreError> {
        let (edge, events) = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            server.resize(width, height);
            let edge = server.local_move(x, y);
            (edge, server.drain())
        };
        self.send_input_events(events).await?;
        if edge.is_some() {
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Remote);
            let peer = self
                .shared
                .input_peer
                .lock()
                .expect("input_peer")
                .as_ref()
                .map(|p| p.to_string());
            self.emit(UiEvent::input_seat_changed("remote", true, peer));
        }
        Ok(edge)
    }

    pub async fn drive_input_key(&self, code: u16, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::key(seq, code, down, 0)]
        };
        self.send_input_events(events).await
    }

    /// Push a physical mouse-button observation to the remote seat. No-op
    /// while the seat is local (the engine gates the same way).
    pub async fn drive_input_button(&self, mask: u8, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::button(seq, mask, down)]
        };
        self.send_input_events(events).await
    }

    /// Push a physical wheel observation to the remote seat.
    pub async fn drive_input_wheel(&self, dx: i16, dy: i16) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::wheel(seq, dx, dy)]
        };
        self.send_input_events(events).await
    }

    pub fn input_engine_seat(&self) -> CursorSeat {
        self.shared.input_engine.lock().expect("engine").seat()
    }

    async fn send_input_events(&self, events: Vec<InputEvent>) -> Result<(), CoreError> {
        let tx = self
            .shared
            .input_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("input channel closed".into()))?;
        for ev in events {
            match tx.try_send(ev.to_frame()?) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err(CoreError::Json("input queue full; peer too slow".into()));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(CoreError::Json("input peer gone".into()));
                }
            }
        }
        Ok(())
    }

    async fn accept_input(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        let sess = match accept_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        self.attach_input_client(sess).await;
        Ok(())
    }

    /// Accept a 45720 screen connection. The peer wants to view this machine's
    /// screen. We refuse to stream until the local user clicks `allow()`.
    async fn accept_screen(
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

    async fn attach_input_server(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let (tx, mut rx) = mpsc::channel::<InnerFrame>(SESSION_QUEUE_DEPTH);
        *self.shared.input_out.lock().await = Some(tx);
        *self.shared.input_peer.lock().expect("input_peer") = Some(peer_id.clone());
        let current_seat = self.input_seat_str();
        self.emit(UiEvent::input_seat_changed(
            current_seat,
            true,
            Some(peer_id.to_string()),
        ));
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
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
                                    tracing::warn!("input server: peer send failed; closing");
                                    break;
                                }
                                node.shared.telemetry.record_input_tx(f_len);
                            }
                            None => break,
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                node.shared.telemetry.record_input_rx(f_len);
                                if let Ok(ev) = InputEvent::from_frame(&frame) {
                                    if matches!(ev.kind, tetherly_core::InputKind::Leave) {
                                        node.shared.input_engine.lock().expect("engine").on_remote_leave();
                                        *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                                        node.emit(UiEvent::input_seat_changed(
                                            "local",
                                            true,
                                            Some(sess.peer_id.to_string()),
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "input server: session recv ended; closing");
                                break;
                            }
                        }
                    }
                }
            }
            *node.shared.input_out.lock().await = None;
            *node.shared.input_peer.lock().expect("input_peer") = None;
            node.shared
                .input_engine
                .lock()
                .expect("engine")
                .on_peer_gone();
            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            node.emit(UiEvent::input_seat_changed("local", false, None));
        });
    }

    async fn attach_input_client(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        *self.shared.input_peer.lock().expect("input_peer") = Some(peer_id.clone());
        let mut client = InputClient::new(MemorySink::default());
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        // Real injection rides behind the CI observer: the sink trace records
        // exactly the actions `apply` performed, so forwarding the tail keeps
        // OS injection in lockstep with protocol-level filtering.
        let real = node.shared.input_real_sink.clone();
        let mut forwarded: usize = 0;
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                node.shared.telemetry.record_input_rx(f_len);
                                if let Ok(ev) = InputEvent::from_frame(&frame) {
                                    if let Ok(Some(leave)) = client.apply(ev) {
                                        if let Ok(f) = leave.to_frame() {
                                            let l_len = (f.payload.len() + 12) as u64;
                                            if sess.send_bounded(f).await.is_ok() {
                                                node.shared.telemetry.record_input_tx(l_len);
                                            }
                                        }
                                    }
                                    *node.shared.input_sink.lock().expect("sink") = client.sink().clone();
                                    if let Some(sink) = &real {
                                        let mut sink = sink.lock().expect("real input sink");
                                        let trace = client.sink().applied.as_slices();
                                        let total = trace.0.len() + trace.1.len();
                                        // The bounded trace (256) can wrap; min()
                                        // then skips nothing new rather than
                                        // replaying stale actions.
                                        let start = forwarded.min(total);
                                        for kind in trace.0.iter().chain(trace.1.iter()).skip(start) {
                                            match *kind {
                                                tetherly_core::InputKind::Move { x, y } => {
                                                    sink.move_abs(x, y)
                                                }
                                                tetherly_core::InputKind::Button { mask, down } => {
                                                    sink.button(mask, down)
                                                }
                                                tetherly_core::InputKind::Wheel { dx, dy } => {
                                                    sink.wheel(dx, dy)
                                                }
                                                tetherly_core::InputKind::Key { code, down, mods } => {
                                                    sink.key(code, down, mods)
                                                }
                                                _ => {}
                                            }
                                        }
                                        forwarded = total;
                                    }
                                    let new_seat = if client.focused() {
                                        CursorSeat::Remote
                                    } else {
                                        CursorSeat::Local
                                    };
                                    let old_seat = *node.shared.input_seat.lock().expect("seat");
                                    *node.shared.input_seat.lock().expect("seat") = Some(new_seat);
                                    if Some(new_seat) != old_seat {
                                        let seat_str = if client.focused() { "remote" } else { "local" };
                                        node.emit(UiEvent::input_seat_changed(
                                            seat_str,
                                            true,
                                            Some(sess.peer_id.to_string()),
                                        ));
                                    }
                                } else if let Err(e) = InputEvent::from_frame(&frame) {
                                    tracing::warn!(%e, "input client: bad frame");
                                }
                            }
                            Err(e) => {
                                tracing::warn!(%e, "input client: session recv ended; closing");
                                break;
                            }
                        }
                    }
                }
            }
            client.on_peer_gone();
            *node.shared.input_peer.lock().expect("input_peer") = None;
            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            *node.shared.input_sink.lock().expect("sink") = client.sink().clone();
            node.emit(UiEvent::input_seat_changed("local", false, None));
        });
    }

    /// Host side of a 45720 session: this machine is being viewed. Consent is
    /// enforced here — the peer's `Start` is refused until `screen_allow()` has
    /// run locally. Frames are pushed by the app calling `screen_send_frame()`
    /// at its own cadence (CLI loop on real hardware).
    async fn attach_screen_server(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let (tx, mut rx) = mpsc::channel::<InnerFrame>(SCREEN_QUEUE_DEPTH);
        *self.shared.screen_out.lock().await = Some(tx);
        *self.shared.screen_peer.lock().expect("screen_peer") = Some(peer_id.clone());
        // A new screen session re-evaluates the ADR-078 bitrate tier.
        self.shared
            .screen_bitrate_applied
            .store(false, std::sync::atomic::Ordering::Relaxed);
        {
            self.shared
                .screen_engine
                .lock()
                .expect("screen")
                .on_peer_connected();
        }
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
                                if let Ok(msg) = ControlMsg::from_frame(&frame) {
                                    let now_us = node.shared.clock.unix_us();
                                    if matches!(msg, ControlMsg::KeyframeRequest) {
                                        node.shared.screen_source.force_idr();
                                        if let Some(sel) = &node.shared.screen_encode_codec {
                                            sel.force_idr();
                                        }
                                    }
                                    let (reply, dynamic_bitrate) = {
                                        let mut engine = node
                                            .shared
                                            .screen_engine
                                            .lock()
                                            .expect("screen");
                                        let reply = engine.on_control_with_time(msg, now_us);
                                        let bitrate = engine.bitrate_bps();
                                        (reply, bitrate)
                                    };
                                    if let Some(sel) = &node.shared.screen_encode_codec {
                                        if sel.bitrate_bps() != dynamic_bitrate {
                                            sel.set_bitrate_bps(dynamic_bitrate);
                                        }
                                    }
                                    node.shared.screen_source.set_bitrate_bps(dynamic_bitrate);
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
            node.shared
                .screen_engine
                .lock()
                .expect("screen")
                .on_peer_gone();
            *node.shared.screen_peer.lock().expect("screen_peer") = None;
            *node.shared.screen_out.lock().await = None;
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
    async fn attach_screen_client(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let (ctl_tx, mut ctl_rx) = mpsc::channel::<ControlMsg>(SCREEN_QUEUE_DEPTH);
        *self.shared.screen_ctl.lock().await = Some(ctl_tx);
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
        let mut receiver = ScreenReceiver::new();
        let mut assembler = ScreenAssembler::new();
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    ctl = ctl_rx.recv() => {
                        match ctl {
                            Some(msg) => {
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
                    inbound = sess.recv_inner() => {
                        match inbound {
                            Ok(frame) => {
                                let f_len = (frame.payload.len() + 12) as u64;
                                node.shared.telemetry.record_screen_rx(f_len);
                                match assembler.push(&frame) {
                                    Ok(Some(sf)) => {
                                        let codec_tag = sf.codec_tag;
                                        // Decode by the frame's wire tag:
                                        // zstd (1) or H.264 (3, ADR-079).
                                        let present_frame = if sf.compressed() {
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
                                                    let _ = node
                                                        .shared
                                                        .screen_sink
                                                        .present(&present_frame);
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
                                    Ok(None) => {}
                                    Err(e) => {
                                        tracing::warn!(%e, "screen assembler error; closing viewer");
                                        break;
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
            *node.shared.screen_peer.lock().expect("screen_peer") = None;
            *node.shared.screen_ctl.lock().await = None;
            node.shared
                .screen_bitrate_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_fps_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.shared
                .screen_scale_applied
                .store(false, std::sync::atomic::Ordering::Relaxed);
            node.emit(UiEvent::screen_state_changed("idle", false, None));
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
        let outbound = {
            let mut engine = self.shared.screen_engine.lock().expect("screen");
            let now_us = self.shared.clock.unix_us();
            // Congestion control / adaptive backpressure (ADR-080 / ADR-082):
            // When link is congested (in_flight >= 4 or RTT >= 250ms), drop non-initial frames
            // to eliminate bufferbloat and socket stall while maintaining presentation continuity.
            // On drop, AIMD multiplicative decrease cuts bitrate by 20% and arms an immediate IDR.
            if engine.is_congested() && engine.stats().sent > 0 {
                engine.record_drop_with_time(now_us);
                engine.consume_pacing(now_us);
                let new_rate = engine.bitrate_bps();
                self.shared.screen_source.force_idr();
                if let Some(sel) = &self.shared.screen_encode_codec {
                    sel.force_idr();
                    sel.set_bitrate_bps(new_rate);
                }
                self.shared.screen_source.set_bitrate_bps(new_rate);
                warn!(
                    in_flight = engine.in_flight_count(),
                    rtt_latest = ?engine.stats().rtt_latest_us,
                    bitrate_bps = new_rate,
                    "screen channel congested; dropping non-initial capture frame; flagged IDR and decreased bitrate"
                );
                return Ok(false);
            }
            if engine.push_with_time(frame, now_us).is_none() {
                return Ok(false);
            }
            engine.consume_pacing(now_us);
            engine.drain()
        };
        self.shared
            .screen_interval
            .lock()
            .expect("interval")
            .record();
        self.send_screen_frames(outbound).await?;
        Ok(true)
    }

    async fn send_screen_frames(&self, frames: Vec<ScreenFrame>) -> Result<(), CoreError> {
        let tx = self
            .shared
            .screen_out
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen channel closed".into()))?;
        for f in frames {
            let frags = f.to_frames()?;
            // Live video: under backpressure drop the whole frame rather than
            // stall the pump or grow memory without bound. A partial frame is
            // never queued, so the peer's assembler is never left mid-frame.
            if tx.capacity() < frags.len() {
                let now_us = self.shared.clock.unix_us();
                let new_rate = {
                    let mut engine = self.shared.screen_engine.lock().expect("screen");
                    engine.record_drop_with_time(now_us);
                    engine.bitrate_bps()
                };
                self.shared.screen_source.force_idr();
                if let Some(sel) = &self.shared.screen_encode_codec {
                    sel.force_idr();
                    sel.set_bitrate_bps(new_rate);
                }
                self.shared.screen_source.set_bitrate_bps(new_rate);
                warn!(
                    need = frags.len(),
                    have = tx.capacity(),
                    bitrate_bps = new_rate,
                    "screen queue full; dropping frame; flagged IDR and decreased bitrate"
                );
                continue;
            }
            for frag in frags {
                if tx.try_send(frag).is_err() {
                    return Err(CoreError::Json("screen peer gone".into()));
                }
            }
        }
        Ok(())
    }

    async fn send_screen_control(&self, msg: ControlMsg) -> Result<(), CoreError> {
        let tx = self
            .shared
            .screen_ctl
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::Json("screen control channel closed".into()))?;
        match tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                Err(CoreError::Json("screen control queue full".into()))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(CoreError::Json("screen peer gone".into()))
            }
        }
    }

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
    /// allowed locally.
    pub async fn screen_start(&self) -> Result<(), CoreError> {
        self.send_screen_control(ControlMsg::Start).await?;
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

    /// Comprehensive telemetry and lifecycle status report for remote screen streaming.
    pub async fn screen_status_report(&self) -> ScreenStatusReport {
        let stats = self.screen_stats();
        let rx_stats = self.screen_rx_stats();
        let p95 = self.screen_latency_p95_us();
        let channel_open =
            self.screen_channel_open().await || self.shared.screen_out.lock().await.is_some();
        let active_peer = self
            .shared
            .screen_peer
            .lock()
            .expect("screen_peer")
            .as_ref()
            .map(|p| p.to_string());
        let state = self.screen_state();
        let is_congested = self
            .shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_congested();
        ScreenStatusReport {
            ok: true,
            state: state.as_str().to_string(),
            channel_open,
            allowed: matches!(state, ScreenState::Allowed | ScreenState::Streaming),
            sent: stats.sent,
            received: rx_stats.received,
            dropped: stats.dropped,
            refused: stats.refused,
            bytes: stats.bytes,
            rx_bytes: rx_stats.bytes,
            p95_frame_interval_us: p95,
            active_peer,
            acked: stats.acked,
            in_flight: stats.in_flight,
            rtt_latest_us: stats.rtt_latest_us,
            rtt_p95_us: stats.rtt_p95_us,
            is_congested,
            keyframes_requested: stats.keyframes_requested,
            gaps: rx_stats.gaps,
            bitrate_bps: stats.bitrate_bps,
            fps_target: stats.fps_target,
            scale_pct: stats.scale_pct,
        }
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

    /// Check if target frame pacing interval has elapsed for the outbound screen pump (ADR-083).
    pub fn screen_is_pacing_ready(&self) -> bool {
        let now_us = self.shared.clock.unix_us();
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_pacing_ready(now_us)
    }

    /// Comprehensive status report for input sharing.
    pub async fn input_status_report(&self) -> InputStatusReport {
        let channel_open = self.input_channel_open().await;
        let active_peer = self
            .shared
            .input_peer
            .lock()
            .expect("input_peer")
            .as_ref()
            .map(|p| p.to_string());
        InputStatusReport {
            ok: true,
            channel_open,
            seat: self.input_seat_str().to_string(),
            active_peer,
        }
    }

    /// Start input sharing with channel auto-dial and strongly typed report return.
    pub async fn input_start_reported(
        &self,
        peer_target: Option<&str>,
    ) -> Result<InputControlReport, CoreError> {
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
                if let Some(existing) = self.shared.input_peer.lock().expect("input_peer").clone() {
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

        if !self.input_channel_open().await {
            self.open_input(&target_id)
                .await
                .map_err(|e| CoreError::Json(format!("input open: {e}")))?;
        }

        let now = self.shared.clock.unix_ms();
        Ok(InputControlReport {
            ok: true,
            action: "start".to_string(),
            seat: self.input_seat_str().to_string(),
            channel_open: true,
            peer: Some(target_id.to_string()),
            timestamp_ms: now,
        })
    }

    /// Stop input sharing with strongly typed report return.
    pub async fn input_stop_reported(&self) -> Result<InputControlReport, CoreError> {
        let peer = self
            .shared
            .input_peer
            .lock()
            .expect("input_peer")
            .as_ref()
            .map(|p| p.to_string());
        self.input_stop().await?;
        let now = self.shared.clock.unix_ms();
        Ok(InputControlReport {
            ok: true,
            action: "stop".to_string(),
            seat: self.input_seat_str().to_string(),
            channel_open: false,
            peer,
            timestamp_ms: now,
        })
    }

    pub fn screen_state(&self) -> ScreenState {
        self.shared.screen_engine.lock().expect("screen").state()
    }

    pub fn screen_stats(&self) -> ScreenStats {
        self.shared.screen_engine.lock().expect("screen").stats()
    }

    /// Whether the host screen channel is currently congested (ADR-080).
    pub fn is_screen_congested(&self) -> bool {
        self.shared
            .screen_engine
            .lock()
            .expect("screen")
            .is_congested()
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

    pub fn offered_files(&self) -> Vec<tetherly_core::IncomingTransfer> {
        self.shared.hubs.lock().expect("hubs").files.offered()
    }

    pub fn update_transfer_progress(&self, transfer_id: &str, transferred: u64) {
        let now = self.shared.clock.unix_ms();
        let mut transfers = self.shared.active_transfers.lock().expect("transfers");
        if let Some(t) = transfers.get_mut(transfer_id) {
            if t.phase == TransferPhase::Failed {
                return;
            }
            let delta_bytes = transferred.saturating_sub(t.transferred);
            let delta_ms = now.saturating_sub(t.updated_ms);
            if delta_ms > 0 && delta_bytes > 0 {
                let inst_speed = (delta_bytes * 1000) / delta_ms;
                t.speed_bps = if t.speed_bps == 0 {
                    inst_speed
                } else {
                    (t.speed_bps * 7 + inst_speed * 3) / 10
                };
            }
            t.transferred = transferred;
            t.updated_ms = now;
        }
    }

    fn prune_terminal_transfers_locked(transfers: &mut HashMap<String, ActiveTransfer>) {
        let mut terminal: Vec<(String, u64)> = transfers
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed
            })
            .map(|(id, t)| (id.clone(), t.updated_ms))
            .collect();
        if terminal.len() > MAX_TERMINAL_TRANSFERS {
            terminal.sort_by_key(|(_, ts)| *ts);
            let excess = terminal.len() - MAX_TERMINAL_TRANSFERS;
            for (id, _) in terminal.into_iter().take(excess) {
                transfers.remove(&id);
            }
        }
    }

    pub fn set_transfer_phase(
        &self,
        transfer_id: &str,
        phase: TransferPhase,
        error: Option<String>,
    ) {
        let now = self.shared.clock.unix_ms();
        let outbox_info = {
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");
            if let Some(t) = transfers.get_mut(transfer_id) {
                if t.phase == TransferPhase::Failed && phase != TransferPhase::Failed {
                    return;
                }
                let was_in_flight =
                    t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying;
                if was_in_flight {
                    let mut stats = self.shared.transfer_stats.lock().expect("transfer_stats");
                    match phase {
                        TransferPhase::Completed => {
                            match t.direction {
                                TransferDirection::Outbound => {
                                    stats.bytes_sent = stats.bytes_sent.saturating_add(t.total);
                                }
                                TransferDirection::Inbound => {
                                    stats.bytes_received =
                                        stats.bytes_received.saturating_add(t.total);
                                }
                            }
                            stats.completed_count = stats.completed_count.saturating_add(1);
                        }
                        TransferPhase::Failed => {
                            stats.failed_count = stats.failed_count.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                let speed = if phase == TransferPhase::Completed || phase == TransferPhase::Failed {
                    let dur = now.saturating_sub(t.started_ms);
                    if dur > 0 && t.transferred > 0 {
                        (t.transferred * 1000) / dur
                    } else {
                        0
                    }
                } else {
                    t.speed_bps
                };
                if phase == TransferPhase::Completed || phase == TransferPhase::Failed {
                    t.speed_bps = 0;
                }
                t.phase = phase;
                t.error = error.clone();
                t.updated_ms = now;

                let is_outbound = t.direction == TransferDirection::Outbound;
                let peer_str = t.peer.to_string();
                let filename = t.filename.clone();
                let size = t.total;

                if t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed {
                    Self::prune_terminal_transfers_locked(&mut transfers);
                }

                if is_outbound {
                    Some((peer_str, filename, size, speed))
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some((peer_str, filename, size, speed)) = outbox_info {
            let outbox_status = match phase {
                TransferPhase::Completed => Some(OutboxStatus::Completed),
                TransferPhase::Failed => {
                    if error.as_deref() == Some("rejected by peer") {
                        Some(OutboxStatus::Rejected)
                    } else if error.as_deref() == Some("cancelled by user") {
                        Some(OutboxStatus::Cancelled)
                    } else {
                        Some(OutboxStatus::Failed)
                    }
                }
                _ => None,
            };
            if let Some(status) = outbox_status {
                let outbox_id = format!("out_{}", transfer_id);
                {
                    let mut outbox = self.shared.outbox_engine.lock().expect("outbox_engine");
                    outbox.update_status(
                        transfer_id,
                        status,
                        error.clone(),
                        Some(now),
                        Some(speed),
                    );
                }
                self.emit(UiEvent::outbox_updated(
                    outbox_id,
                    transfer_id,
                    peer_str,
                    filename,
                    size,
                    status.as_str(),
                    now,
                ));
            }
        }
    }

    pub fn active_transfers(&self) -> Vec<ActiveTransfer> {
        let transfers = self.shared.active_transfers.lock().expect("transfers");
        let mut list: Vec<ActiveTransfer> = transfers.values().cloned().collect();
        list.sort_by_key(|a| std::cmp::Reverse(a.started_ms));
        list
    }

    pub fn transfer_by_id(&self, id: &str) -> Option<ActiveTransfer> {
        let transfers = self.shared.active_transfers.lock().expect("transfers");
        transfers.get(id).cloned()
    }

    pub fn transfers_report(&self) -> TransfersReport {
        let transfers = self.active_transfers();
        let stats = self.transfer_stats();
        let active_count = transfers
            .iter()
            .filter(|t| {
                t.phase == TransferPhase::Transferring || t.phase == TransferPhase::Verifying
            })
            .count();
        let terminal_count = transfers.len().saturating_sub(active_count);
        TransfersReport {
            ok: true,
            active_count,
            terminal_count,
            total_count: transfers.len(),
            transfers,
            stats,
        }
    }

    pub fn transfer_stats(&self) -> TransferStats {
        self.shared
            .transfer_stats
            .lock()
            .expect("transfer_stats")
            .clone()
    }

    pub fn clear_terminal_transfers(&self) -> usize {
        let mut transfers = self.shared.active_transfers.lock().expect("transfers");
        let to_remove: Vec<String> = transfers
            .iter()
            .filter(|(_, t)| {
                t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &to_remove {
            transfers.remove(id);
        }
        to_remove.len()
    }

    /// Cancel an active in-flight transfer (inbound or outbound).
    pub async fn cancel_transfer(&self, transfer_id: &str) -> Result<(), CoreError> {
        let (cancel_tx, peer_to_notify, is_outbound, filename, size) = {
            let mut cancels = self.shared.transfer_cancels.lock().expect("cancels");
            let mut pending = self.shared.pending_out.lock().expect("pending");
            let mut transfers = self.shared.active_transfers.lock().expect("transfers");

            let t = transfers
                .get_mut(transfer_id)
                .ok_or(CoreError::UnknownTransfer)?;
            if t.phase != TransferPhase::Transferring && t.phase != TransferPhase::Verifying {
                return Err(CoreError::File("transfer is not active".into()));
            }
            {
                let mut stats = self.shared.transfer_stats.lock().expect("transfer_stats");
                stats.failed_count = stats.failed_count.saturating_add(1);
            }
            t.phase = TransferPhase::Failed;
            t.error = Some("cancelled by user".into());
            t.speed_bps = 0;
            t.updated_ms = self.shared.clock.unix_ms();

            let is_outbound = t.direction == TransferDirection::Outbound;
            let filename = t.filename.clone();
            let size = t.total;
            let peer = t.peer.clone();
            pending.remove(transfer_id);
            Self::prune_terminal_transfers_locked(&mut transfers);
            (
                cancels.remove(transfer_id),
                peer,
                is_outbound,
                filename,
                size,
            )
        };

        if let Some(tx) = cancel_tx {
            let _ = tx.send(());
        }

        let now = self.shared.clock.unix_ms();
        if is_outbound {
            let outbox_id = format!("out_{}", transfer_id);
            {
                let mut outbox = self.shared.outbox_engine.lock().expect("outbox_engine");
                outbox.update_status(
                    transfer_id,
                    OutboxStatus::Cancelled,
                    Some("cancelled by user".into()),
                    Some(now),
                    Some(0),
                );
            }
            self.emit(UiEvent::outbox_updated(
                outbox_id,
                transfer_id,
                peer_to_notify.to_string(),
                filename,
                size,
                OutboxStatus::Cancelled.as_str(),
                now,
            ));
        }

        let reject_decision = tetherly_core::FileDecision {
            transfer_id: transfer_id.to_string(),
            resume_from: None,
        };
        if let Ok(frame) = reject_decision.reject_frame() {
            let _ = self.send_to(&peer_to_notify, frame).await;
        }

        info!(transfer = %transfer_id, "transfer cancelled by user");
        Ok(())
    }
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

#[cfg(test)]
mod tests {
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
mod clipboard_policy_tests {
    use super::*;
    use tetherly_core::{ClipSyncMode, ManualClock};

    fn temp_test_node(tag: &str) -> (Node, std::path::PathBuf) {
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
        assert_eq!(report.policy.mode, ClipSyncMode::Manual);
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

        // Allow async task to complete subprocess execution
        tokio::time::sleep(Duration::from_millis(150)).await;

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
}
