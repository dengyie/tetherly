// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN + optional EasyTier sidecar overlay. Overlay is unicast only; no mDNS on TUN.

use crate::ancs::AncsInbound;
use crate::events::{file_offered, peer_up, CandidateViewDto, UiEvent};
use crate::lan::{bind_many, bind_many_optional, bind_overlay, control_bind_addrs, lan_ips};
use crate::platform::{insert_foreground, MemoryClip, SystemClip};
use crate::sidecar::{self, OverlayConfig, OverlaySource, OverlayStatus};
use crate::store::{load_trust, remember_route, save_trust, RouteTable};
use crate::telemetry::{TrafficReport, TrafficTelemetry};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::frame::{
    ping, pong, CapsUpdate, ClipSet, FileDone, FileOffer, InnerFrame, NotifyPush, NotifyReply,
};
use tetherly_core::liveness::{Liveness, SESSION_HEARTBEAT};
use tetherly_core::pin::{format_pin, generate_pin};
use tetherly_core::ports::{Clip, Clock, Rng, SystemClock};
use tetherly_core::session::LockoutTable;
use tetherly_core::session::{backoff_delay, PIN_TTL};
use tetherly_core::{
    ancs_actions, ancs_source_id, path_kind, AncsEvent, AncsIngress, AncsState, AncsTransport,
    ClipApply, ClipDirection, ClipHub, ClipMetaEntry, ConnectOutcome, ControlMsg, CoreError,
    CursorSeat, DeviceId, FileHub, IngestOutcome, InputClient, InputEvent, InputServer, InputSink,
    MemoryScreenSink, MemorySink, MemoryTrustStore, NotifyHub, OpenAllowlist, PathKind, ReplyView,
    ScreenAssembler, ScreenCodec, ScreenFrame, ScreenReceiver, ScreenSender, ScreenSink,
    ScreenSource, ScreenState, ScreenStats, TcpLimiter, TrustStore, CLIPBOARD_OTP_CLEAR_MS,
    OVERLAY_PROBE_MS,
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
    /// Screen presentation sink. `None` defaults to the in-memory observer;
    /// production passes `SystemSink` for real output.
    pub screen_sink: Option<Arc<dyn ScreenSink>>,
    /// Real key/mouse injection for the controlled side of 45719. `None`
    /// keeps CI's MemorySink-only behavior (the observer always records).
    pub input_sink: Option<Arc<Mutex<dyn InputSink>>>,
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
            loopback_only: false,
            reconnect: true,
            ui_port: 45716,
            overlay: OverlayConfig::default(),
            ancs: None,
            ancs_peripheral: "local-iphone".into(),
            screen_source: None,
            screen_codec: None,
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
        let bound_control = AtomicU16::new(cfg.control_port);
        let bound_file = AtomicU16::new(cfg.file_port);
        let bound_input = AtomicU16::new(cfg.input_port);
        let bound_screen = AtomicU16::new(cfg.screen_port);
        let bound_ui = AtomicU16::new(0);
        let ancs_source = ancs_source_id(&cfg.ancs_peripheral);
        let ancs = cfg.ancs.clone().map(|t| AncsIngress::new(t, clock.clone()));
        let screen_source: Arc<dyn ScreenSource> = cfg.screen_source.clone().unwrap_or_else(|| {
            // Production: real DXGI capture → zstd lossless → wire.
            // Tests inject MemoryScreenSource raw; this default is only
            // active in the bin on Windows.
            Arc::new(crate::screen::EncodedSource::new(
                crate::screen::SystemSource::new(),
                Arc::new(crate::screen::ZstdCodec::default()),
            ))
        });
        let screen_codec: Arc<dyn ScreenCodec> = cfg
            .screen_codec
            .clone()
            .unwrap_or_else(|| Arc::new(crate::screen::ZstdCodec::default()));
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
            screen_engine: Mutex::new(ScreenSender::new()),
            screen_source,
            screen_observer: Mutex::new(screen_observer),
            screen_ctl: AsyncMutex::new(None),
            screen_codec,
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
                s
            }),
            blocked: Mutex::new(initial_blocked),
            auto_accept_trusted: AtomicBool::new(false),
            auto_accepted_count: AtomicU64::new(0),
            discovered: Mutex::new(HashMap::new()),
            rescan_trigger,
        });
        Ok(Self { shared })
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
        Ok(format_pin(&pin))
    }

    pub fn set_pin(&self, digits: [u8; 8]) {
        *self.shared.pin.lock().expect("pin") = Some((digits, self.shared.clock.unix_ms()));
        let _ = self
            .shared
            .events
            .send(UiEvent::PinStateChanged { active: true });
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
        let outcome = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.ingest_push(source, push, &*self.shared.clock)
        };
        if let IngestOutcome::Candidate(v) = &outcome {
            self.emit(UiEvent::Candidate {
                candidate: CandidateViewDto::from(v.clone()),
            });
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

        Ok(InboxClearReport {
            ok: true,
            deleted_count,
            freed_bytes,
        })
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
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = keep_stop.changed() => {
                        if *keep_stop.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(SESSION_HEARTBEAT) => {
                        match keep_tx.try_send(ping(keep_clock.unix_ms())) {
                            Ok(()) => {}
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
            let mut live = Liveness::new(clock.unix_ms());
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
                        if self.shared.auto_accept_trusted.load(Ordering::Relaxed)
                            && self.is_trusted(peer)
                        {
                            if let Err(e) = self.user_accept_file(&tid).await {
                                warn!(%e, transfer = %tid, "auto-accept failed");
                            } else {
                                self.shared
                                    .auto_accepted_count
                                    .fetch_add(1, Ordering::Relaxed);
                                info!(transfer = %tid, "auto-accepted file from trusted peer");
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
                self.emit(UiEvent::PeerCapsChanged {
                    device_id: peer.to_string(),
                    caps: update.caps,
                    features,
                });
            }
            SessionEvent::Unknown(_) => {}
        }
        Ok(())
    }

    fn apply_clip(&self, peer: &DeviceId, clip: ClipSet) -> Result<(), tetherly_net::NetError> {
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

    /// Configures the auto-accept policy for trusted peers and emits `UiEvent::FileAutoAcceptChanged`.
    pub fn set_auto_accept(&self, enabled: bool) -> AutoAcceptReport {
        self.shared
            .auto_accept_trusted
            .store(enabled, Ordering::Relaxed);
        self.emit(UiEvent::FileAutoAcceptChanged { enabled });
        self.auto_accept_status()
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
    pub fn clipboard_status(&self) -> Result<ClipStatusReport, CoreError> {
        let cur = self.shared.clip.get_text()?;
        let (has_text, bytes) = match cur {
            Some(ref t) if !t.is_empty() => (true, t.len()),
            _ => (false, 0),
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
        self.shared.clip.set_text("")?;
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
                sha256: digest,
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
                sha256: digest,
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
                    filename: safe_name,
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
        self.send_to(peer, offer.to_frame()?).await?;
        Ok(transfer_id)
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
                                    let reply = node
                                        .shared
                                        .screen_engine
                                        .lock()
                                        .expect("screen")
                                        .on_control(msg);
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
                                        // Decompress if the sender zstd-compressed
                                        // this frame.
                                        let present_frame = if sf.compressed {
                                            match node.shared.screen_codec.decompress(
                                                &sf.bytes,
                                                sf.raw_len(),
                                            ) {
                                                Ok(raw) => ScreenFrame {
                                                    bytes: raw,
                                                    compressed: false,
                                                    ..sf
                                                },
                                                Err(e) => {
                                                    tracing::warn!(%e, seq = sf.seq, "screen decompress failed; closing viewer");
                                                    break;
                                                }
                                            }
                                        } else {
                                            sf
                                        };
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
            if engine.push(frame).is_none() {
                return Ok(false);
            }
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
                warn!(
                    need = frags.len(),
                    have = tx.capacity(),
                    "screen queue full; dropping frame"
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
        }
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
                                stats.bytes_received = stats.bytes_received.saturating_add(t.total);
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
            if phase == TransferPhase::Completed || phase == TransferPhase::Failed {
                t.speed_bps = 0;
            }
            t.phase = phase;
            t.error = error;
            t.updated_ms = now;

            if t.phase == TransferPhase::Completed || t.phase == TransferPhase::Failed {
                Self::prune_terminal_transfers_locked(&mut transfers);
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
        let (cancel_tx, peer_to_notify) = {
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

            let peer = t.peer.clone();
            pending.remove(transfer_id);
            Self::prune_terminal_transfers_locked(&mut transfers);
            (cancels.remove(transfer_id), peer)
        };

        if let Some(tx) = cancel_tx {
            let _ = tx.send(());
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
