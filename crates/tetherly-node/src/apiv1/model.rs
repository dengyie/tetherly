// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node state -> v1 DTOs. The device merge is pure over its four sources so
//! the precedence rules are unit-tested without a running node.

use crate::runtime::{extract_features, DiscoveredPeerDto, LivePeer};
use crate::store::BlockedPeer;
use crate::Node;
use std::collections::BTreeMap;
use tetherly_api::{
    ActionHistory,
    ActionHistoryEntry,
    ActionPolicyDto,
    ActionTriggerReport,
    ActiveBenchSessionDto,
    ActiveCameraSessionDto,
    ActiveInputSessionDto,
    Activity,
    ActivityCandidate,
    ActivityHandoff,
    ActivityMessage,
    ActivityReply,
    AncsAllowRequest,
    AncsClearRulesReport,
    AncsConnectReport,
    AncsDisconnectReport,
    AncsRule,
    AncsRuleReport,
    AncsStatus,
    // §167: screen domain contract types.
    AnnotationItemDto,
    AnnotationKindDto,
    AudioDecision,
    AudioPolicyDto,
    AudioStatus,
    AudioTierDto,
    AuditClearReport,
    AuditEntryDto,
    AuditTrail,
    BatteryPolicyDto,
    BatteryState,
    BatteryStateDto,
    BenchDecision,
    BenchPolicyDto,
    BenchReportDto,
    BenchStatus,
    CameraDecision,
    CameraPolicyDto,
    CameraStatus,
    CameraTierDto,
    ChannelTraffic,
    ClipClearReport,
    ClipContent,
    ClipHistory,
    ClipHistoryEntry,
    ClipPolicyDto,
    ClipPollReport,
    ClipStatus,
    Device,
    DisplayDecision,
    DisplayModeDto,
    DisplayPolicyDto,
    DisplayTierDto,
    DisplayVirtualStatus,
    FileBatchItem,
    FileBatchReport,
    FileDecision,
    FileDecisionReport,
    FileDispatchItem,
    FileDispatchReport,
    FileOffer,
    FileOffers,
    FocusPeers,
    FocusPolicyDto,
    FocusSetReport,
    FocusState,
    FocusStateDto,
    FocusSyncReport,
    HandoffDispatchItem,
    HandoffInbox,
    HandoffInboxActionReport,
    HandoffInboxClearReport,
    HandoffInboxEntryDto,
    HandoffPolicyDto,
    HandoffSendMultiReport,
    HandoffSendReport,
    HandoffStatus,
    HeartbeatConfig,
    Inbox,
    InboxFile,
    Input,
    InputControl,
    InputPolicyDto,
    InputStatus,
    InputTrace,
    InputTraceEntryDto,
    InputTunnelDecision,
    InputTunnelStatus,
    LaserPointerDto,
    Layout,
    LayoutEdge,
    LayoutPolicy,
    LinkPath,
    LinkQuality,
    LinkQualityPeer,
    LocalActionDto,
    LocalActions,
    LocalCaps,
    LocatePolicyDto,
    LocateRingReport,
    LocateStatus,
    LocateStopReport,
    MediaCommandMultiReport,
    MediaCommandReport,
    MediaDispatchItem,
    MediaPolicyDto,
    MediaState,
    MediaStateDto,
    MediaStateUpdateReport,
    MessageDirection,
    MicDecision,
    MicPolicyDto,
    MicStatus,
    MicTierDto,
    Outbox,
    OutboxClearReport,
    OutboxEntryDto,
    OutboxResendReport,
    Pairing,
    PeerCaps,
    PeerCounts,
    PeerFocusDto,
    PeerProfileDto,
    PeerProfiles,
    Ports,
    ProtocolMessages,
    RecommendationDto,
    RecommendationStatus,
    RecommendationSummaryDto,
    RemoteActionDto,
    RemoteActions,
    ResumeMarker,
    ResumeMarkers,
    ScreenAnnotations,
    ScreenBitrate,
    ScreenColor,
    ScreenControl,
    ScreenCursor,
    ScreenDelta,
    ScreenDisplayInfoDto,
    ScreenDisplays,
    ScreenFec,
    ScreenFocus,
    ScreenFps,
    ScreenHarq,
    ScreenLatency,
    ScreenLatencyBreakdownDto,
    ScreenLtr,
    ScreenMatrix,
    ScreenMotion,
    ScreenPacer,
    ScreenPir,
    ScreenPlayback,
    ScreenPlayout,
    ScreenPrivacy,
    ScreenProbe,
    ScreenProfile,
    ScreenQoe,
    ScreenQualityMetric,
    ScreenRecording,
    ScreenRecordingReplay,
    ScreenReplayRecordDto,
    ScreenRoi,
    ScreenScale,
    ScreenSnapshot,
    ScreenSquelch,
    ScreenStatus,
    ScreenSvc,
    ScreenSync,
    ScreenTile,
    ScreenTileBatch,
    ScreenTileCache,
    ScreenTileLod,
    ScreenTileMotion,
    ScreenTileNack,
    ScreenTilePalette,
    ScreenTilePredict,
    ScreenTilePresent,
    ScreenTileSaliency,
    ScreenTrendline,
    ScreenTuning,
    ScreenTuningDecisionDto,
    ScreenTwcc,
    ScreenWatermark,
    Settings,
    SettingsGeneral,
    SettingsTransfer,
    Snapshot,
    SpatialPositionDto,
    Storage,
    StorageCleanReport,
    StreamingRecommendationDto,
    System,
    TrafficTelemetry,
    Transfer,
    TransferDirection,
    TransferPhase,
    TransferStats,
    Transfers,
    WhiteboardPolicyDto,
    WhiteboardSnapshotDto,
    API_VERSION,
};
use tetherly_core::{CoreError, DisplayEdge, PathKind, TrustedPeer};

#[derive(Default)]
struct Parts {
    live: Option<LivePeer>,
    trusted: Option<TrustedPeer>,
    discovered: Option<DiscoveredPeerDto>,
    blocked: Option<BlockedPeer>,
}

/// Merge every source into one device per id, sorted by id. `profile`
/// returns `(device_type, tags, group)` for an id.
pub fn merge(
    live: Vec<LivePeer>,
    trusted: Vec<TrustedPeer>,
    discovered: Vec<DiscoveredPeerDto>,
    blocked: Vec<BlockedPeer>,
    profile: impl Fn(&str) -> (String, Vec<String>, Option<String>),
) -> Vec<Device> {
    let mut parts: BTreeMap<String, Parts> = BTreeMap::new();
    for p in live {
        let key = p.device_id.to_string();
        parts.entry(key).or_default().live = Some(p);
    }
    for t in trusted {
        let key = t.device_id.to_string();
        parts.entry(key).or_default().trusted = Some(t);
    }
    for d in discovered {
        let key = d.device_id.clone();
        parts.entry(key).or_default().discovered = Some(d);
    }
    for b in blocked {
        let key = b.device_id.clone();
        parts.entry(key).or_default().blocked = Some(b);
    }
    parts
        .into_iter()
        .map(|(id, p)| build(id, p, &profile))
        .collect()
}

fn build(
    id: String,
    p: Parts,
    profile: &impl Fn(&str) -> (String, Vec<String>, Option<String>),
) -> Device {
    let alias = p
        .trusted
        .as_ref()
        .map(|t| t.alias.clone())
        .filter(|a| !a.trim().is_empty());
    let name = p
        .live
        .as_ref()
        .map(|l| l.name.clone())
        .or_else(|| p.discovered.as_ref().map(|d| d.name.clone()))
        .filter(|n| !n.trim().is_empty())
        .or_else(|| alias.clone())
        .unwrap_or_else(|| id.clone());
    let caps = p
        .live
        .as_ref()
        .map(|l| l.caps.clone())
        .or_else(|| p.discovered.as_ref().map(|d| d.caps.clone()))
        .unwrap_or_default();
    let (device_type, tags, group) = profile(&id);
    Device {
        name,
        alias,
        platform: p.live.as_ref().map(|l| l.platform.clone()),
        device_type,
        online: p.live.is_some(),
        trusted: p.trusted.as_ref().is_some_and(|t| !t.revoked),
        blocked: p.blocked.is_some(),
        block_reason: p.blocked.and_then(|b| b.reason),
        path: p.live.as_ref().map(|l| match l.path {
            PathKind::Lan => LinkPath::Lan,
            PathKind::Overlay => LinkPath::Overlay,
        }),
        addr: p.live.as_ref().map(|l| l.addr.to_string()),
        rtt_ms: p.live.as_ref().and_then(|l| l.rtt_ms),
        last_seen_ms: p
            .live
            .as_ref()
            .map(|l| l.last_seen_ms)
            .or_else(|| p.discovered.as_ref().map(|d| d.last_seen_ms)),
        features: extract_features(&caps),
        caps,
        tags,
        group,
        id,
    }
}

pub fn devices(node: &Node) -> Vec<Device> {
    merge(
        node.live_peers(),
        node.trusted(),
        node.discovered_peers().peers,
        node.blocked_peers_report().peers,
        |id| profile_of(node, id),
    )
}

pub fn device(node: &Node, id: &str) -> Option<Device> {
    devices(node).into_iter().find(|d| d.id == id)
}

fn profile_of(node: &Node, id: &str) -> (String, Vec<String>, Option<String>) {
    match tetherly_core::DeviceId::parse(id) {
        Ok(did) => {
            let p = node.peer_profile(&did);
            (p.device_type.as_str().to_string(), p.tags, p.group)
        }
        Err(_) => (
            crate::store::DeviceType::default().as_str().to_string(),
            Vec::new(),
            None,
        ),
    }
}

pub fn pairing(node: &Node) -> Pairing {
    let r = node.pin_report();
    Pairing {
        pin_active: r.active,
        pin_expires_in_secs: r.expires_in_secs,
    }
}

pub async fn input(node: &Node) -> Input {
    let r = node.input_status_report().await;
    Input {
        seat: r.seat,
        channel_open: r.channel_open,
        peer: r.active_peer,
    }
}

pub fn system(node: &Node) -> System {
    let h = node.health_report();
    let caps = node.local_caps_report();
    System {
        device_id: h.device_id,
        name: h.name,
        platform: h.platform,
        version: h.version,
        api: API_VERSION,
        pid: h.pid,
        uptime_secs: h.uptime_secs,
        ports: Ports {
            control: h.ports.control,
            file: h.ports.file,
            input: h.ports.input,
            screen: h.ports.screen,
            ui: h.ports.ui,
        },
        peers: PeerCounts {
            live: h.live_peers as u64,
            trusted: h.trusted_peers as u64,
        },
        caps: caps.caps,
        features: caps.features,
        lan_only: !node.overlay_status().present,
    }
}

pub async fn snapshot(node: &Node) -> Snapshot {
    Snapshot {
        system: system(node),
        devices: devices(node),
        pairing: pairing(node),
        input: input(node).await,
    }
}

/// The transfers resource: daemon report mapped 1:1 onto the contract.
pub fn transfers(node: &Node) -> Transfers {
    let r = node.transfers_report();
    let s = r.stats;
    Transfers {
        active_count: r.active_count as u64,
        terminal_count: r.terminal_count as u64,
        stats: TransferStats {
            bytes_sent: s.bytes_sent,
            bytes_received: s.bytes_received,
            completed_count: s.completed_count,
            failed_count: s.failed_count,
        },
        transfers: r
            .transfers
            .iter()
            .map(|t| Transfer {
                transfer_id: t.transfer_id.clone(),
                peer: t.peer.to_string(),
                direction: match t.direction {
                    crate::runtime::TransferDirection::Inbound => TransferDirection::Inbound,
                    crate::runtime::TransferDirection::Outbound => TransferDirection::Outbound,
                },
                filename: t.filename.clone(),
                total: t.total,
                transferred: t.transferred,
                speed_bps: t.speed_bps,
                phase: match t.phase {
                    crate::runtime::TransferPhase::Transferring => TransferPhase::Transferring,
                    crate::runtime::TransferPhase::Verifying => TransferPhase::Verifying,
                    crate::runtime::TransferPhase::Completed => TransferPhase::Completed,
                    crate::runtime::TransferPhase::Failed => TransferPhase::Failed,
                },
                error: t.error.clone(),
                started_ms: t.started_ms,
                updated_ms: t.updated_ms,
            })
            .collect(),
    }
}

/// The inbox resource: files waiting in `data_dir/inbox`. §162a: `search` /
/// `limit` mirror the legacy `/api/file/inbox` query (substring match on the
/// file name, capped listing) — `None` returns the unfiltered resource.
pub fn inbox(node: &Node, search: Option<&str>, limit: Option<usize>) -> Inbox {
    let r = node.inbox_report(search, limit);
    Inbox {
        count: r.count as u64,
        total_bytes: r.total_bytes,
        files: r
            .files
            .iter()
            .map(|f| InboxFile {
                name: f.name.clone(),
                size: f.size,
                modified_at_secs: f.modified_at_secs,
                is_safe: f.is_safe,
            })
            .collect(),
    }
}

// ---- §162b: file-domain mappers (offers / send / outbox / storage) ----

/// The pending-offers resource: legacy `/api/file/offers` report mapped onto
/// the contract; a `peer` filter that resolves to nothing yields the empty
/// resource (legacy parity).
pub fn file_offers(
    node: &Node,
    id: Option<&str>,
    peer: Option<&str>,
) -> Result<FileOffers, CoreError> {
    let r = node.file_offers(id, peer)?;
    Ok(FileOffers {
        count: r.count as u64,
        auto_accept_trusted: r.auto_accept_trusted,
        offers: r
            .offers
            .iter()
            .map(|o| FileOffer {
                transfer_id: o.transfer_id.clone(),
                filename: o.filename.clone(),
                total_bytes: o.total_bytes,
                resume_from: o.resume_from,
                source: o.source.clone(),
                peer: o.peer.clone(),
                peer_alias: o.peer_alias.clone(),
                size: o.size,
                sha256: o.sha256.clone(),
                timestamp_ms: o.timestamp_ms,
                offered_ms: o.offered_ms,
            })
            .collect(),
    })
}

/// The decision report folds the single (`id`) and batch (`all`/`peer`)
/// legacy shapes: one decision or counts + per-offer outcomes.
pub fn file_decision_single(
    accepted: bool,
    rep: crate::runtime::FileDecisionReport,
) -> FileDecisionReport {
    let decision = FileDecision {
        accepted,
        action: rep.action.clone(),
        transfer_id: rep.transfer_id.clone(),
        peer: rep.peer.clone(),
        filename: rep.filename.clone(),
        total_bytes: rep.total_bytes,
        resume_from: rep.resume_from,
        timestamp_ms: rep.timestamp_ms,
    };
    FileDecisionReport {
        accepted: Some(accepted),
        action: Some(rep.action),
        transfer_id: Some(rep.transfer_id),
        peer: Some(rep.peer),
        filename: Some(rep.filename),
        total_bytes: Some(rep.total_bytes),
        accepted_count: u64::from(accepted),
        rejected_count: u64::from(!accepted),
        total_evaluated: 1,
        decisions: vec![decision],
    }
}

/// Batch accept/reject report mapping (legacy `FileBatchDecisionReport`).
pub fn file_decision_batch(rep: crate::runtime::FileBatchDecisionReport) -> FileDecisionReport {
    FileDecisionReport {
        accepted: None,
        action: Some(rep.action),
        transfer_id: None,
        peer: None,
        filename: None,
        total_bytes: None,
        accepted_count: rep.accepted_count as u64,
        rejected_count: rep.rejected_count as u64,
        total_evaluated: rep.total_evaluated as u64,
        decisions: rep
            .decisions
            .into_iter()
            .map(|d| FileDecision {
                accepted: d.accepted,
                action: d.action,
                transfer_id: d.transfer_id,
                peer: d.peer,
                filename: d.filename,
                total_bytes: d.total_bytes,
                resume_from: d.resume_from,
                timestamp_ms: d.timestamp_ms,
            })
            .collect(),
    }
}

/// Multi-peer file dispatch report mapping (legacy `MultiFileSendReport`);
/// the selector serializes in its `as_label` form like messages dispatch.
pub fn file_dispatch(rep: crate::runtime::MultiFileSendReport) -> FileDispatchReport {
    FileDispatchReport {
        selector: rep.selector.as_label(),
        filename: rep.filename,
        size: rep.size,
        sha256: rep.sha256,
        target_count: rep.target_count as u64,
        success_count: rep.success_count as u64,
        skipped_offline_count: rep.skipped_offline_count as u64,
        failed_count: rep.failed_count as u64,
        dispatches: rep
            .dispatches
            .into_iter()
            .map(|d| FileDispatchItem {
                peer: d.peer,
                peer_alias: d.peer_alias,
                transfer_id: d.transfer_id,
                outbox_id: d.outbox_id,
                success: d.success,
                error: d.error,
            })
            .collect(),
    }
}

/// Batch send report mapping (legacy `BatchSendReport`).
pub fn file_batch(rep: crate::runtime::BatchSendReport) -> FileBatchReport {
    FileBatchReport {
        batch_id: rep.batch_id,
        peer: rep.peer,
        peer_alias: rep.peer_alias,
        total_files: rep.total_files as u64,
        successful_files: rep.successful_files as u64,
        failed_files: rep.failed_files as u64,
        total_bytes: rep.total_bytes,
        items: rep
            .items
            .into_iter()
            .map(|i| FileBatchItem {
                transfer_id: i.transfer_id,
                name: i.name,
                size: i.size,
                sha256: i.sha256,
                status: i.status,
                error: i.error,
            })
            .collect(),
    }
}

/// The outbox resource: ring total plus the queried page (peer/status/limit).
pub fn outbox(
    node: &Node,
    peer: Option<&str>,
    status: Option<crate::runtime::OutboxStatus>,
    limit: Option<usize>,
) -> Result<Outbox, CoreError> {
    let r = node.outbox_history(peer, status, limit)?;
    Ok(Outbox {
        total: r.total as u64,
        returned: r.returned as u64,
        entries: r.entries.into_iter().map(outbox_entry).collect(),
    })
}

/// One outbox record mapping (`OutboxStatus` serializes snake_case).
fn outbox_entry(e: crate::runtime::OutboxEntry) -> OutboxEntryDto {
    OutboxEntryDto {
        id: e.id,
        transfer_id: e.transfer_id,
        peer: e.peer,
        peer_alias: e.peer_alias,
        filename: e.filename,
        size: e.size,
        sha256: e.sha256,
        source_path: e.source_path,
        status: e.status.as_str().to_string(),
        error: e.error,
        started_ms: e.started_ms,
        completed_ms: e.completed_ms,
        speed_bps: e.speed_bps,
    }
}

/// `DELETE /outbox` report mapping (peer/status targets preserved).
pub fn outbox_clear(rep: crate::runtime::OutboxClearReport) -> OutboxClearReport {
    OutboxClearReport {
        cleared_count: rep.cleared_count as u64,
        target_peer: rep.target_peer,
        target_status: rep.target_status.map(|s| s.as_str().to_string()),
    }
}

/// `POST /outbox/{id}/resend` report mapping.
pub fn outbox_resend(rep: crate::runtime::OutboxResendReport) -> OutboxResendReport {
    OutboxResendReport {
        original_id: rep.original_id,
        new_transfer_id: rep.new_transfer_id,
    }
}

/// The resume-markers resource mapping (`ResumeMarkerDto` field-for-field).
pub fn resume_markers(node: &Node) -> ResumeMarkers {
    let r = node.resume_markers_report();
    ResumeMarkers {
        count: r.count as u64,
        total_partial_bytes: r.total_partial_bytes,
        markers: r
            .markers
            .into_iter()
            .map(|m| ResumeMarker {
                transfer_id: m.transfer_id,
                source: m.source,
                source_alias: m.source_alias,
                total_bytes: m.total_bytes,
                offset_bytes: m.offset_bytes,
                progress_percent: m.progress_percent,
                chunk_aligned: m.chunk_aligned,
                part_file_exists: m.part_file_exists,
                part_file_bytes: m.part_file_bytes,
            })
            .collect(),
    }
}

/// The storage resource mapping (resume marker detail folds into the count).
pub fn storage(node: &Node) -> Storage {
    let r = node.storage_report();
    Storage {
        data_dir: r.data_dir,
        inbox_dir: r.inbox_dir,
        inbox_total_bytes: r.inbox_total_bytes,
        inbox_file_count: r.inbox_file_count as u64,
        partial_bytes: r.partial_bytes,
        partial_count: r.partial_count as u64,
        orphaned_partial_count: r.orphaned_partial_count as u64,
        orphaned_partial_bytes: r.orphaned_partial_bytes,
        meta_bytes: r.meta_bytes,
        resume_markers_count: r.resume_markers_count as u64,
    }
}

/// `POST /storage/clean` report mapping.
pub fn storage_clean(rep: crate::runtime::StorageCleanReport) -> StorageCleanReport {
    StorageCleanReport {
        freed_bytes: rep.freed_bytes,
        deleted_files_count: rep.deleted_files_count as u64,
        cleared_resume_entries: rep.cleared_resume_entries as u64,
    }
}

/// The auto-accept policy resource: policy + session auto-accept counter.
pub fn auto_accept_policy(node: &Node) -> tetherly_api::AutoAcceptPolicyDto {
    let rep = node.auto_accept_policy_report();
    tetherly_api::AutoAcceptPolicyDto {
        enabled: rep.policy.enabled,
        allowed_groups: rep.policy.allowed_groups,
        allowed_tags: rep.policy.allowed_tags,
        allowed_types: rep
            .policy
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        max_file_size: rep.policy.max_file_size,
        auto_accepted_count: rep.auto_accepted_count,
    }
}

/// The layout resource: local display, configured edges and crossing knobs.
pub fn layout(node: &Node) -> Layout {
    let l = node.topology_layout();
    let policy = node.topology_policy();
    let mut edges: Vec<LayoutEdge> = l
        .neighbors
        .iter()
        .map(|(edge, n)| LayoutEdge {
            edge: edge.as_str().to_string(),
            peer: Some(n.peer_id.clone()),
        })
        .collect();
    if edges.is_empty() {
        // Keep all four edges visible so a fresh install still renders a grid.
        for edge in [
            DisplayEdge::Left,
            DisplayEdge::Right,
            DisplayEdge::Top,
            DisplayEdge::Bottom,
        ] {
            edges.push(LayoutEdge {
                edge: edge.as_str().to_string(),
                peer: None,
            });
        }
    }
    edges.sort_by(|a, b| a.edge.cmp(&b.edge));
    Layout {
        display: tetherly_api::DisplayRect {
            width: l.display.width,
            height: l.display.height,
            scale_factor: l.display.scale_factor,
        },
        edges,
        config: tetherly_api::CrossingConfig {
            resistance_px: l.config.resistance_px,
            delay_ms: l.config.delay_ms,
            corner_lock_size_px: l.config.corner_lock_size_px,
            wrap_around: l.config.wrap_around,
            redundancy_band_px: l.config.redundancy_band_px,
            slow_dwell_ms: l.config.slow_dwell_ms,
            fast_push_delta_px: l.config.fast_push_delta_px,
            entry_return_grace_ms: l.config.entry_return_grace_ms,
            remote_speed_percent: l.config.remote_speed_percent,
        },
        policy: LayoutPolicy {
            enabled: policy.enabled,
            allow_inbound_control: policy.allow_inbound_control,
            auto_link_paired_peers: policy.auto_link_paired_peers,
        },
    }
}

// ---- §156b: topology / routes / network domains ----

/// Shared converter: runtime `EdgeCrossingConfig` -> contract `CrossingConfig`.
pub fn crossing(r: tetherly_core::topology::EdgeCrossingConfig) -> tetherly_api::CrossingConfig {
    tetherly_api::CrossingConfig {
        resistance_px: r.resistance_px,
        delay_ms: r.delay_ms,
        corner_lock_size_px: r.corner_lock_size_px,
        wrap_around: r.wrap_around,
        redundancy_band_px: r.redundancy_band_px,
        slow_dwell_ms: r.slow_dwell_ms,
        fast_push_delta_px: r.fast_push_delta_px,
        entry_return_grace_ms: r.entry_return_grace_ms,
        remote_speed_percent: r.remote_speed_percent,
    }
}

pub fn display_rect(r: tetherly_core::topology::DisplayRect) -> tetherly_api::DisplayRect {
    tetherly_api::DisplayRect {
        width: r.width,
        height: r.height,
        scale_factor: r.scale_factor,
    }
}

fn display_rect_of(r: &tetherly_core::topology::DisplayRect) -> tetherly_api::DisplayRect {
    display_rect(*r)
}

/// `GET /topology` — local spatial display topology (legacy
/// `/api/topology/layout` slice; `ok`/`message` folded, the HashMap keyed by
/// edge becomes parallel `neighbors` + `neighbor_edges` vectors).
pub fn topology(node: &Node) -> tetherly_api::Topology {
    let l = node.topology_layout();
    // Sort by edge name so the wire shape is deterministic.
    let mut entries: Vec<(DisplayEdge, &tetherly_core::topology::EdgeNeighbor)> =
        l.neighbors.iter().map(|(e, n)| (*e, n)).collect();
    entries.sort_by_key(|(e, _)| e.as_str());
    let mut neighbors = Vec::new();
    let mut neighbor_edges = Vec::new();
    for (edge, n) in entries {
        neighbor_edges.push(edge.as_str().to_string());
        neighbors.push(tetherly_api::TopologyEdgeNeighbor {
            peer_id: n.peer_id.clone(),
            peer_edge: n.peer_edge.as_str().to_string(),
            offset: n.offset,
        });
    }
    tetherly_api::Topology {
        display: display_rect_of(&l.display),
        neighbors,
        neighbor_edges,
        config: crossing(l.config),
        peer_count: l.peer_count as u64,
    }
}

/// `GET /topology/peers` — all known peer display topologies (legacy
/// `/api/topology/peers` slice; `ok`/`message` folded, per-peer neighbors
/// count replaces the full map).
pub fn peer_topologies(node: &Node) -> tetherly_api::PeerTopologies {
    let r = node.topology_peers();
    let peers: Vec<tetherly_api::PeerTopologyDto> = r
        .peers
        .into_iter()
        .map(|p| tetherly_api::PeerTopologyDto {
            peer_id: p.peer_id,
            peer_alias: p.peer_alias,
            display: display_rect_of(&p.display),
            config: crossing(p.config),
            timestamp_ms: p.timestamp_ms,
            neighbors_count: p.neighbors.len() as u64,
        })
        .collect();
    tetherly_api::PeerTopologies {
        count: peers.len() as u64,
        peers,
    }
}

/// `GET /topology/policy` — governance policy readback.
pub fn topology_policy(node: &Node) -> tetherly_api::TopologyPolicy {
    let p = node.topology_policy();
    tetherly_api::TopologyPolicy {
        enabled: p.enabled,
        allow_inbound_control: p.allow_inbound_control,
        auto_link_paired_peers: p.auto_link_paired_peers,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

/// `GET /routes` — cached physical addressing routes (legacy
/// `/api/routes` slice; `ok` folded, count kept).
pub fn routes(node: &Node) -> tetherly_api::Routes {
    let r = node.routes_report();
    tetherly_api::Routes {
        count: r.count as u64,
        routes: r
            .routes
            .into_iter()
            .map(|e| tetherly_api::RouteEntryDto {
                device_id: e.device_id,
                addr: e.addr,
                alias: e.alias,
                name: e.name,
                online: e.online,
                trusted: e.trusted,
            })
            .collect(),
    }
}

/// `GET /net` — network interfaces and overlay status (legacy `/api/net`
/// slice; `ok` folded away).
pub fn network(node: &Node) -> tetherly_api::Network {
    let r = node.network_report_dto();
    tetherly_api::Network {
        interfaces: r
            .interfaces
            .into_iter()
            .map(|i| tetherly_api::IfaceReport {
                iface: i.iface,
                ip: i.ip,
                listen: i.listen,
                reason: i.reason,
            })
            .collect(),
        overlay: tetherly_api::OverlayStatus {
            enabled: r.overlay.enabled,
            present: r.overlay.present,
            source: r.overlay.source,
            local_ips: r.overlay.local_ips,
            peer_count: r.overlay.peer_count as u64,
            peers: r.overlay.peers,
        },
    }
}

/// `POST /net/refresh` — rescan + overlay probe (legacy `ok`/`action`/
/// `timestamp_ms` folded).
pub fn network_refresh(
    rep: crate::runtime::NetworkRefreshReport,
) -> tetherly_api::NetworkRefreshReport {
    tetherly_api::NetworkRefreshReport {
        ifaces_scanned: rep.ifaces_scanned as u64,
        ifaces_listening: rep.ifaces_listening as u64,
        overlay_present: rep.overlay_present,
        overlay_source: rep.overlay_source,
        overlay_local_ips: rep.overlay_local_ips,
        overlay_peers_count: rep.overlay_peers_count as u64,
    }
}

// ---- §160: discovery / daemon lifecycle ----

/// `GET /discovery` — the mDNS discovery view (legacy `ok` envelope folded).
pub fn discovery(node: &Node) -> tetherly_api::Discovery {
    let r = node.discovered_peers();
    tetherly_api::Discovery {
        count: r.peers.len() as u64,
        peers: r
            .peers
            .into_iter()
            .map(|p| tetherly_api::DiscoveredPeer {
                device_id: p.device_id,
                name: p.name,
                addrs: p.addrs,
                port: p.port,
                fp: p.fp,
                caps: p.caps,
                is_trusted: p.is_trusted,
                is_online: p.is_online,
                last_seen_ms: p.last_seen_ms,
            })
            .collect(),
    }
}

/// `POST /discovery` — kick a fresh LAN rescan (legacy `ok`/`message`
/// folded; the message carried no information beyond "rescan started").
pub fn discovery_rescan(node: &Node) -> tetherly_api::DiscoveryRescanReport {
    let _ = node.rescan_discovery();
    tetherly_api::DiscoveryRescanReport {
        message: "discovery rescan started".to_string(),
    }
}

/// §152: the link resource — quality snapshot + heartbeat config, field
/// names mirrored from the runtime report (CLI printers unchanged).
pub fn link_quality(node: &Node) -> LinkQuality {
    let r = node.link_quality_report();
    LinkQuality {
        heartbeat: HeartbeatConfig {
            interval_secs: r.heartbeat.interval_secs,
            deadline_secs: r.heartbeat.deadline_secs,
        },
        total_peers: r.total_peers as u64,
        online_peers: r.online_peers as u64,
        links: r
            .links
            .into_iter()
            .map(|l| LinkQualityPeer {
                device_id: l.device_id,
                name: l.name,
                addr: l.addr,
                is_online: l.is_online,
                last_seen_ms: l.last_seen_ms,
                rtt_ms: l.rtt_ms,
                min_rtt_ms: l.min_rtt_ms,
                max_rtt_ms: l.max_rtt_ms,
                avg_rtt_ms: l.avg_rtt_ms,
                jitter_ms: l.jitter_ms,
                pings_sent: l.pings_sent,
                pongs_recv: l.pongs_recv,
                packet_loss_rate: l.packet_loss_rate,
                grade: l.grade.as_str().to_string(),
                samples_count: l.samples_count as u64,
                connected_uptime_secs: l.connected_uptime_secs,
            })
            .collect(),
    }
}

/// §152: the heartbeat config resource view.
pub fn heartbeat(node: &Node) -> HeartbeatConfig {
    let c = node.heartbeat_config();
    HeartbeatConfig {
        interval_secs: c.interval_secs,
        deadline_secs: c.deadline_secs,
    }
}

/// §153: the traffic/protocol telemetry resource view — legacy `/api/metrics`
/// field-for-field (the legacy `ok: true` flag folds away; a 200 body IS ok).
pub fn traffic(node: &Node) -> TrafficTelemetry {
    let r = node.traffic_report();
    let ch = |c: &crate::telemetry::ChannelTrafficDto| ChannelTraffic {
        tx_bytes: c.tx_bytes,
        rx_bytes: c.rx_bytes,
        tx_frames: c.tx_frames,
        rx_frames: c.rx_frames,
    };
    TrafficTelemetry {
        uptime_secs: r.uptime_secs,
        total_tx_bytes: r.total_tx_bytes,
        total_rx_bytes: r.total_rx_bytes,
        total_tx_frames: r.total_tx_frames,
        total_rx_frames: r.total_rx_frames,
        control: ch(&r.control),
        file: ch(&r.file),
        input: ch(&r.input),
        screen: ch(&r.screen),
        messages: ProtocolMessages {
            pings_sent: r.messages.pings_sent,
            pings_recv: r.messages.pings_recv,
            pongs_sent: r.messages.pongs_sent,
            pongs_recv: r.messages.pongs_recv,
            notifies_sent: r.messages.notifies_sent,
            notifies_recv: r.messages.notifies_recv,
            clips_sent: r.messages.clips_sent,
            clips_recv: r.messages.clips_recv,
            file_offers_sent: r.messages.file_offers_sent,
            file_offers_recv: r.messages.file_offers_recv,
        },
        sessions_established: r.sessions_established,
        sessions_closed: r.sessions_closed,
        handshakes_failed: r.handshakes_failed,
    }
}

/// The settings resource: read-only view of general config and the
/// auto-accept master toggle.
pub fn settings(node: &Node) -> Settings {
    let r = node.config_report();
    let a = node.auto_accept_policy();
    Settings {
        general: SettingsGeneral {
            name: r.name,
            advertise: r.advertise,
            reconnect: r.reconnect,
            loopback_only: r.loopback_only,
            overlay_enabled: r.overlay_enabled,
            tray_owner: node.tray_owner(),
        },
        transfer: SettingsTransfer {
            auto_accept: a.enabled,
        },
    }
}

/// `GET /profiles` — the peer profile table with the already-parsed
/// `tag`/`group`/`type` query filters (parsed at the HTTP layer).
pub fn peer_profiles(
    node: &Node,
    tag: Option<String>,
    group: Option<String>,
    dev_type: Option<String>,
) -> PeerProfiles {
    let dev_type = dev_type.and_then(|t| t.parse::<crate::store::DeviceType>().ok());
    let r = node.peer_profiles(tag.as_deref(), group.as_deref(), dev_type);
    PeerProfiles {
        total: r.total,
        profiles: r.profiles.into_iter().map(profile_dto).collect(),
    }
}

fn profile_dto(p: crate::store::PeerProfile) -> PeerProfileDto {
    PeerProfileDto {
        device_id: p.device_id,
        device_type: p.device_type.as_str().to_string(),
        tags: p.tags,
        group: p.group,
        notes: p.notes,
        updated_ms: p.updated_ms,
    }
}

/// `GET /caps` — local node capability report (`ok` folded away).
pub fn local_caps(node: &Node) -> LocalCaps {
    let r = node.local_caps_report();
    LocalCaps {
        device_id: r.device_id,
        caps: r.caps,
        features: r.features,
        file_port: r.file_port,
        input_port: r.input_port,
        screen_port: r.screen_port,
    }
}

/// `GET /devices/{id}/caps` — one peer's capability profile (the legacy
/// `/api/peers/caps?peer=` single-target slice; `ok`/`count` folded away).
pub fn peer_caps(node: &Node, target: &str) -> Result<PeerCaps, CoreError> {
    let r = node.peers_caps_report(Some(target))?;
    r.peers
        .into_iter()
        .next()
        .map(|p| PeerCaps {
            device_id: p.device_id,
            name: p.name,
            alias: p.alias,
            caps: p.caps,
            features: p.features,
            file_port: p.file_port,
            input_port: p.input_port,
            screen_port: p.screen_port,
            online: p.online,
        })
        .ok_or_else(|| CoreError::PeerNotFound(target.to_string()))
}

/// `GET /caps/peers` — all live peers' capability profiles (the legacy
/// untargeted `/api/peers/caps` slice, envelope folded to a bare array).
/// §158: the all-peers caps fetch propagates resolve failures instead of
/// folding them into an empty list (an empty list means "no live peers", not
/// "the query failed"); v1 maps the error onto its envelope.
pub fn peer_caps_all(node: &Node) -> Result<Vec<PeerCaps>, CoreError> {
    let r = node.peers_caps_report(None)?;
    Ok(r.peers
        .into_iter()
        .map(|p| PeerCaps {
            device_id: p.device_id,
            name: p.name,
            alias: p.alias,
            caps: p.caps,
            features: p.features,
            file_port: p.file_port,
            input_port: p.input_port,
            screen_port: p.screen_port,
            online: p.online,
        })
        .collect())
}

/// The activity resource: notification candidates, pending replies, the
/// recent direct-message ring and the handoff inbox, in one report.
pub fn activity(node: &Node) -> Activity {
    let messages = match node.message_history(None, Some(50), false, false) {
        Ok(r) => r
            .messages
            .into_iter()
            .map(|m| ActivityMessage {
                id: m.id,
                peer: m.peer,
                direction: match m.direction {
                    crate::runtime::MessageDirection::Inbound => MessageDirection::Inbound,
                    crate::runtime::MessageDirection::Outbound => MessageDirection::Outbound,
                },
                text: m.text,
                timestamp_ms: m.timestamp_ms,
                read: m.read,
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    let handoffs = node
        .handoff_inbox_list(None)
        .into_iter()
        .map(|h| ActivityHandoff {
            id: h.id,
            sender_id: h.sender_id,
            sender_alias: h.sender_alias,
            url: h.url,
            title: h.title,
            status: match h.status {
                tetherly_core::HandoffStatus::Pending => HandoffStatus::Pending,
                tetherly_core::HandoffStatus::Opened => HandoffStatus::Opened,
                tetherly_core::HandoffStatus::Dismissed => HandoffStatus::Dismissed,
                tetherly_core::HandoffStatus::Rejected => HandoffStatus::Rejected,
            },
            received_ms: h.received_ms,
            opened_ms: h.opened_ms,
        })
        .collect();
    Activity {
        candidates: node
            .candidates()
            .into_iter()
            .map(|c| ActivityCandidate {
                id: c.id,
                source: c.source,
                app_id: c.app_id,
                app_name: c.app_name,
                uid: c.uid,
                title: c.title,
                has_otp: c.has_otp,
                created_ms: c.created_ms,
                expires_ms: c.expires_ms,
                actions: c.actions,
            })
            .collect(),
        replies: node
            .reply_views()
            .into_iter()
            .map(crate::events::ReplyViewDto::from)
            .map(|r| ActivityReply {
                id: r.id,
                source: r.source,
                app_id: r.app_id,
                uid: r.uid,
            })
            .collect(),
        messages,
        handoffs,
    }
}

/// The flat clipboard-policy resource: policy + session sync counters
/// (legacy `/api/clip/policy` report with the `ok` envelope folded off and
/// the nested `policy` flattened into the resource).
pub fn clip_policy(node: &Node) -> ClipPolicyDto {
    let rep = node.clipboard_policy_report();
    ClipPolicyDto {
        mode: rep.policy.mode.as_str().to_string(),
        sensitive_guard: rep.policy.sensitive_guard,
        allowed_groups: rep.policy.allowed_groups,
        allowed_tags: rep.policy.allowed_tags,
        allowed_types: rep
            .policy
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        max_payload_bytes: rep.policy.max_payload_bytes as u64,
        auto_clear_timeout_secs: rep.policy.auto_clear_timeout_secs,
        auto_synced_out_count: rep.auto_synced_out_count,
        auto_synced_in_count: rep.auto_synced_in_count,
        sensitive_blocked_count: rep.sensitive_blocked_count,
    }
}

/// `POST /clip/poll` — one manual auto-sync sweep tick, readback included.
pub fn clip_poll(node: &Node, dispatched_peers: u64) -> ClipPollReport {
    ClipPollReport {
        dispatched_peers,
        policy: clip_policy(node),
    }
}

/// The clipboard status snapshot (no plaintext).
pub fn clip_status(node: &Node) -> Result<ClipStatus, CoreError> {
    let r = node.clipboard_status()?;
    Ok(ClipStatus {
        has_text: r.has_text,
        bytes: r.bytes as u64,
        hash: r.hash,
        last_seq: r.last_seq,
        inbound_count: r.inbound_count,
        outbound_count: r.outbound_count,
        history_count: r.history_count as u64,
        clipboard_available: r.clipboard_available,
        reason: r.reason,
    })
}

/// The clipboard sync history (bounded, no plaintext).
pub fn clip_history(node: &Node, limit: Option<usize>) -> ClipHistory {
    let r = node.clipboard_history(limit);
    ClipHistory {
        total: r.total,
        entries: r
            .entries
            .into_iter()
            .map(|e| ClipHistoryEntry {
                seq: e.seq,
                source: e.source,
                direction: e.direction,
                bytes: e.bytes as u64,
                hash: e.hash,
                timestamp_ms: e.timestamp_ms,
            })
            .collect(),
    }
}

/// `DELETE /clip` — history reset + (where available) OS clipboard clear.
pub fn clip_clear(node: &Node) -> Result<ClipClearReport, CoreError> {
    let r = node.clear_clipboard()?;
    Ok(ClipClearReport {
        cleared_history_count: r.cleared_history_count as u64,
        timestamp_ms: r.timestamp_ms,
        clipboard_available: r.clipboard_available,
        reason: r.reason,
    })
}

/// `GET /clip` — the one plaintext readback (legacy shape, `ok` folded off).
pub fn clip_content(node: &Node) -> ClipContent {
    match node.get_clipboard_text() {
        Ok(Some(text)) => ClipContent {
            has_text: true,
            bytes: text.len() as u64,
            hash: Some(tetherly_core::hex_sha256(text.as_bytes())),
            text: Some(text),
            clipboard_available: true,
            reason: None,
        },
        Ok(None) => ClipContent {
            has_text: false,
            bytes: 0,
            hash: None,
            text: None,
            clipboard_available: true,
            reason: None,
        },
        // A host with no display server has no clipboard to read: report
        // the absence instead of answering 500 (legacy v1.87 semantics).
        Err(e) => ClipContent {
            has_text: false,
            bytes: 0,
            hash: None,
            text: None,
            clipboard_available: false,
            reason: Some(e.to_string()),
        },
    }
}

/// The filtered audit trail resource.
pub fn audit_trail(
    node: &Node,
    category: Option<crate::runtime::AuditCategory>,
    severity: Option<crate::runtime::AuditSeverity>,
    limit: Option<u64>,
    since_ms: Option<u64>,
) -> AuditTrail {
    let r = node.audit_report(category, severity, limit.map(|l| l as usize), since_ms);
    AuditTrail {
        total: r.total,
        returned: r.returned,
        capacity: r.capacity,
        entries: r
            .entries
            .into_iter()
            .map(|e| AuditEntryDto {
                id: e.id,
                timestamp_ms: e.timestamp_ms,
                category: e.category.to_string(),
                severity: e.severity.to_string(),
                action: e.action,
                target: e.target,
                summary: e.summary,
                success: e.success,
            })
            .collect(),
    }
}

/// `DELETE /audit` — drain the audit ring.
pub fn audit_clear(node: &Node) -> AuditClearReport {
    let r = node.clear_audit_log();
    AuditClearReport {
        cleared_count: r.cleared_count as u64,
        message: r.message,
    }
}

// ---- §164: remote action domain ----

fn local_action_dto(a: crate::store::LocalActionDefinition) -> LocalActionDto {
    LocalActionDto {
        id: a.id,
        name: a.name,
        description: a.description,
        icon: a.icon,
        command: a.command,
        args: a.args,
        timeout_secs: a.timeout_secs,
        enabled: a.enabled,
    }
}

/// `GET /action/local` — registered local quick-actions.
pub fn local_actions(node: &Node) -> LocalActions {
    let r = node.local_action_report();
    LocalActions {
        count: r.count,
        actions: r.actions.into_iter().map(local_action_dto).collect(),
    }
}

/// `POST /action/local` — register or replace one local action.
pub fn local_action_add(
    node: &Node,
    action: crate::store::LocalActionDefinition,
) -> Result<LocalActions, CoreError> {
    let r = node.add_local_action(action)?;
    Ok(LocalActions {
        count: r.count,
        actions: r.actions.into_iter().map(local_action_dto).collect(),
    })
}

/// `DELETE /action/local` — remove one local action by id.
pub fn local_action_remove(node: &Node, id: &str) -> Result<LocalActions, CoreError> {
    let r = node.remove_local_action(id)?;
    Ok(LocalActions {
        count: r.count,
        actions: r.actions.into_iter().map(local_action_dto).collect(),
    })
}

/// `GET /action/remote` — cached peer-advertised actions.
pub fn remote_actions(node: &Node, peer: &str) -> Result<RemoteActions, CoreError> {
    let r = node.remote_actions(peer)?;
    Ok(remote_actions_report(r))
}

/// Map a runtime `RemoteActionReport` (also covers the refresh path).
pub fn remote_actions_report(r: crate::runtime::RemoteActionReport) -> RemoteActions {
    RemoteActions {
        peer: r.peer,
        count: r.count,
        actions: r
            .actions
            .into_iter()
            .map(|a| RemoteActionDto {
                id: a.id,
                name: a.name,
                description: a.description,
                icon: a.icon,
            })
            .collect(),
    }
}

/// `POST /action/trigger` — dispatch one remote action.
pub async fn action_trigger(
    node: &Node,
    peer: &str,
    action_id: &str,
) -> Result<ActionTriggerReport, CoreError> {
    let r = node.trigger_remote_action(peer, action_id).await?;
    Ok(ActionTriggerReport {
        trigger_id: r.trigger_id,
        action_id: r.action_id,
        target_peer: r.target_peer,
        status: r.status,
    })
}

/// `GET /action/history` — recent remote-action executions.
pub fn action_history(node: &Node) -> ActionHistory {
    let r = node.action_history();
    ActionHistory {
        count: r.count,
        results: r
            .results
            .into_iter()
            .map(|e| ActionHistoryEntry {
                trigger_id: e.trigger_id,
                action_id: e.action_id,
                success: e.success,
                exit_code: e.exit_code,
                output_summary: e.output_summary,
                error_message: e.error_message,
                duration_ms: e.duration_ms,
                timestamp_ms: e.timestamp_ms,
            })
            .collect(),
    }
}

fn action_policy_dto(p: crate::store::ActionPolicy) -> ActionPolicyDto {
    ActionPolicyDto {
        mode: p.mode.as_str().to_string(),
        max_timeout_secs: p.max_timeout_secs,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        expose_action_list: p.expose_action_list,
    }
}

/// `GET /action/policy` — inbound remote-action governance policy.
pub fn action_policy(node: &Node) -> ActionPolicyDto {
    action_policy_dto(node.action_policy())
}

/// `PATCH /action/policy` — sparse policy update.
pub fn set_action_policy(node: &Node, policy: crate::store::ActionPolicy) -> ActionPolicyDto {
    let r = node
        .set_action_policy(policy)
        .expect("validated action policy");
    action_policy_dto(r.policy)
}

// ---- §164: link benchmark domain ----

fn bench_policy_dto(p: crate::store::BenchPolicy) -> BenchPolicyDto {
    BenchPolicyDto {
        enabled: p.enabled,
        allow_inbound_bench: p.allow_inbound_bench,
        max_duration_secs: p.max_duration_secs,
        max_bandwidth_mbps: p.max_bandwidth_mbps,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

/// `GET /bench` — benchmark engine status + policy + last measurement.
pub fn bench_status(node: &Node) -> BenchStatus {
    let r = node.bench_status();
    BenchStatus {
        active: r.active,
        session: r.session.map(|s| ActiveBenchSessionDto {
            session_id: s.session_id,
            peer_id: s.peer_id,
            role: s.role.as_str().to_string(),
            direction: s.direction.as_str().to_string(),
            duration_secs: s.duration_secs,
            chunk_size: s.chunk_size,
            chunks_processed: s.chunks_processed,
            bytes_transferred: s.bytes_transferred,
            started_at_ms: s.started_at_ms,
            last_activity_ms: s.last_activity_ms,
        }),
        policy: bench_policy_dto(r.policy),
        last_report: r.last_report.map(|rep| BenchReportDto {
            session_id: rep.session_id,
            direction: rep.direction.as_str().to_string(),
            duration_ms: rep.duration_ms,
            bytes_transferred: rep.bytes_transferred,
            chunks: rep.chunks,
            throughput_mbps: rep.throughput_mbps,
            avg_chunk_latency_us: rep.avg_chunk_latency_us,
            jitter_us: rep.jitter_us,
            stopped_early: rep.stopped_early,
            timestamp_ms: rep.timestamp_ms,
        }),
        timestamp_ms: r.timestamp_ms,
    }
}

/// `GET /bench/policy` — link benchmark governance policy.
pub fn bench_policy(node: &Node) -> BenchPolicyDto {
    bench_policy_dto(node.bench_policy())
}

/// `PATCH /bench/policy` — sparse policy update.
pub fn set_bench_policy(node: &Node, policy: crate::store::BenchPolicy) -> BenchPolicyDto {
    let r = node
        .set_bench_policy(policy)
        .expect("validated bench policy");
    bench_policy_dto(r.policy)
}

/// `POST /bench/start` + `POST /bench/stop` decision report.
pub fn bench_decision(r: crate::runtime::BenchDecisionReport) -> BenchDecision {
    BenchDecision {
        action: r.action,
        session_id: r.session_id,
        peer_id: r.peer_id,
        message: r.message,
        timestamp_ms: r.timestamp_ms,
    }
}

/// `GET /recommendation?peer=` — one peer's measured link + streaming tiers.
pub fn recommendation(node: &Node, peer: &str) -> Result<RecommendationDto, CoreError> {
    let r = node.streaming_recommendation(peer)?;
    Ok(RecommendationDto {
        peer_id: r.peer_id,
        peer_alias: r.peer_alias,
        has_profile: r.has_profile,
        measured_mbps: r.measured_mbps,
        direction: r.direction,
        measured_at_ms: r.measured_at_ms,
        recommendation: recommendation_dto(r.recommendation),
        timestamp_ms: r.timestamp_ms,
    })
}

/// `GET /recommendation` — aggregate recommendation status across peers.
pub fn recommendation_status(node: &Node) -> RecommendationStatus {
    let r = node.recommendation_status();
    RecommendationStatus {
        peers: r
            .peers
            .into_iter()
            .map(|p| RecommendationSummaryDto {
                peer_id: p.peer_id,
                peer_alias: p.peer_alias,
                measured_mbps: p.measured_mbps,
                direction: p.direction,
                measured_at_ms: p.measured_at_ms,
                tier: p.tier,
            })
            .collect(),
        timestamp_ms: r.timestamp_ms,
    }
}

fn recommendation_dto(
    r: tetherly_core::recommend::StreamingRecommendation,
) -> StreamingRecommendationDto {
    StreamingRecommendationDto {
        measured_mbps: r.measured_mbps,
        tier: r.tier,
        display: DisplayTierDto {
            width: r.display.width,
            height: r.display.height,
            fps: r.display.fps,
        },
        camera: CameraTierDto {
            width: r.camera.width,
            height: r.camera.height,
            fps: r.camera.fps,
            codec: r.camera.codec.as_str().to_string(),
        },
        audio: AudioTierDto {
            sample_rate: r.audio.sample_rate,
            channels: r.audio.channels,
            codec: r.audio.codec.as_str().to_string(),
        },
        mic: MicTierDto {
            sample_rate: r.mic.sample_rate,
            channels: r.mic.channels,
            codec: r.mic.codec.as_str().to_string(),
        },
    }
}

// ---- §165: media control domain ----

fn media_state_dto(f: tetherly_core::MediaStateFrame) -> MediaStateDto {
    MediaStateDto {
        player_id: f.player_id,
        state: f.state.as_str().to_string(),
        title: f.title,
        artist: f.artist,
        album: f.album,
        position_ms: f.position_ms,
        duration_ms: f.duration_ms,
        volume_percent: f.volume_percent,
        muted: f.muted,
        can_play: f.can_play,
        can_pause: f.can_pause,
        can_go_next: f.can_go_next,
        can_go_previous: f.can_go_previous,
        can_seek: f.can_seek,
        timestamp_ms: f.timestamp_ms,
    }
}

/// `GET /media/state?peer=` — local or a peer's last playback snapshot.
pub fn media_state(node: &Node, peer: Option<&str>) -> Result<MediaState, CoreError> {
    let r = node.media_get_state(peer)?;
    Ok(MediaState {
        peer: r.peer,
        state: r.state.map(media_state_dto),
    })
}

/// `PATCH /media/state` — sparse local playback update; returns the
/// post-broadcast frame.
pub fn set_media_state(
    node: &Node,
    state: tetherly_core::MediaStateFrame,
) -> Result<MediaStateUpdateReport, CoreError> {
    let st = node.media_update_local_state(state)?;
    Ok(MediaStateUpdateReport {
        state: media_state_dto(st),
    })
}

/// `POST /media/command` single-peer dispatch.
pub async fn media_command(
    node: &Node,
    peer: &str,
    command: tetherly_core::MediaCommand,
    target_player: Option<String>,
) -> Result<MediaCommandReport, CoreError> {
    let r = node
        .media_send_command(peer, command, target_player)
        .await?;
    Ok(MediaCommandReport {
        id: r.id,
        peer: r.peer,
        peer_alias: r.peer_alias,
        command: r.command,
        target_player: r.target_player,
        timestamp_ms: r.timestamp_ms,
    })
}

/// `POST /media/command` selector fan-out.
pub async fn media_command_multi(
    node: &Node,
    selector: &crate::store::TargetSelector,
    command: tetherly_core::MediaCommand,
    target_player: Option<String>,
) -> Result<MediaCommandMultiReport, CoreError> {
    let r = node
        .media_send_command_multi(selector, command, target_player)
        .await?;
    Ok(MediaCommandMultiReport {
        selector: r.selector.as_label(),
        command: r.command,
        target_count: r.target_count,
        success_count: r.success_count,
        skipped_offline_count: r.skipped_offline_count,
        failed_count: r.failed_count,
        dispatches: r
            .dispatches
            .into_iter()
            .map(|d| MediaDispatchItem {
                peer: d.peer,
                peer_alias: d.peer_alias,
                command_id: d.command_id,
                success: d.success,
                error: d.error,
            })
            .collect(),
    })
}

fn media_policy_dto(p: crate::store::MediaControlPolicy) -> MediaPolicyDto {
    MediaPolicyDto {
        mode: p.mode.as_str().to_string(),
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        max_volume_limit: p.max_volume_limit,
        allow_seek: p.allow_seek,
    }
}

/// `GET /media/policy` — media control governance policy.
pub fn media_policy(node: &Node) -> MediaPolicyDto {
    media_policy_dto(node.media_policy())
}

/// `PATCH /media/policy` — sparse policy update.
pub fn set_media_policy(node: &Node, policy: crate::store::MediaControlPolicy) -> MediaPolicyDto {
    let r = node
        .set_media_policy(policy)
        .expect("validated media policy");
    media_policy_dto(r.policy)
}

// ---- §165: battery/power domain ----

fn battery_state_dto(f: tetherly_core::BatteryStateFrame) -> BatteryStateDto {
    BatteryStateDto {
        battery_level: f.battery_level,
        is_charging: f.is_charging,
        power_source: f.power_source.as_str().to_string(),
        battery_saver: f.battery_saver,
        health_percent: f.health_percent,
        timestamp_ms: f.timestamp_ms,
    }
}

/// `GET /battery/state?peer=` — local or a peer's last broadcast measurement.
pub fn battery_state(node: &Node, peer: Option<&str>) -> Result<BatteryState, CoreError> {
    let r = node.battery_get_state(peer)?;
    Ok(BatteryState {
        peer: r.peer,
        state: r.state.map(battery_state_dto),
    })
}

/// `PATCH /battery/state` — sparse local state update; returns the
/// post-broadcast frame.
pub async fn set_battery_state(
    node: &Node,
    state: tetherly_core::BatteryStateFrame,
) -> Result<BatteryStateDto, CoreError> {
    let r = node.battery_update_local_state(state).await?;
    Ok(battery_state_dto(r.state))
}

fn battery_policy_dto(p: crate::store::BatteryPolicy) -> BatteryPolicyDto {
    BatteryPolicyDto {
        low_battery_threshold: p.low_battery_threshold,
        critical_battery_threshold: p.critical_battery_threshold,
        alert_on_low_battery: p.alert_on_low_battery,
        auto_throttle_on_battery_saver: p.auto_throttle_on_battery_saver,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

/// `GET /battery/policy` — battery governance policy.
pub fn battery_policy(node: &Node) -> BatteryPolicyDto {
    battery_policy_dto(node.battery_policy())
}

/// `PATCH /battery/policy` — sparse policy update.
pub fn set_battery_policy(node: &Node, policy: crate::store::BatteryPolicy) -> BatteryPolicyDto {
    let r = node
        .set_battery_policy(policy)
        .expect("validated battery policy");
    battery_policy_dto(r.policy)
}

// ---- §165: ANCS (iPhone notification bridge) domain ----

/// Fold a runtime open-rule into the contract `AncsRule`.
fn ancs_rule(r: crate::runtime::OpenRuleDto) -> AncsRule {
    AncsRule {
        app_id: r.app_id,
        url: r.url,
    }
}

pub fn ancs_status(node: &Node) -> AncsStatus {
    let rep = node.ancs_report();
    AncsStatus {
        attached: rep.attached,
        state: rep.state,
        peripheral: rep.peripheral,
        retry_at_ms: rep.retry_at_ms,
        rules: rep.rules.into_iter().map(ancs_rule).collect(),
        allowlist: rep.allowlist,
    }
}

pub fn ancs_connect(node: &Node) -> Result<AncsConnectReport, CoreError> {
    let rep = node.ancs_connect_reported()?;
    Ok(AncsConnectReport {
        state: rep.state,
        retry_at_ms: rep.retry_at_ms,
        message: rep.message,
    })
}

pub fn ancs_disconnect(node: &Node) -> AncsDisconnectReport {
    let rep = node.ancs_disconnect_reported();
    AncsDisconnectReport {
        state: rep.state,
        retry_at_ms: rep.retry_at_ms,
        message: rep.message,
    }
}

pub fn ancs_allow(node: &Node, req: AncsAllowRequest) -> Result<AncsRuleReport, CoreError> {
    let rep = node.add_open_rule_reported(tetherly_core::OpenRule {
        app_id: req.app_id,
        url: req.url,
    })?;
    Ok(AncsRuleReport {
        action: rep.action,
        rule: rep.rule.map(ancs_rule),
        total_rules: rep.total_rules,
        timestamp_ms: rep.timestamp_ms,
    })
}

pub fn ancs_remove_rule(node: &Node, app_id: &str) -> AncsRuleReport {
    let rep = node.remove_open_rule_reported(app_id);
    AncsRuleReport {
        action: rep.action,
        rule: rep.rule.map(ancs_rule),
        total_rules: rep.total_rules,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn ancs_clear_rules(node: &Node) -> AncsClearRulesReport {
    let rep = node.clear_open_rules_reported();
    AncsClearRulesReport {
        cleared_count: rep.cleared_count,
        total_rules: rep.total_rules,
        timestamp_ms: rep.timestamp_ms,
    }
}

// ---- §165: input tunnel (remote pointer/keyboard) domain ----

/// Fold a runtime active-tunnel session into the contract DTO.
fn active_input_session_dto(s: crate::runtime::ActiveInputTunnelSession) -> ActiveInputSessionDto {
    ActiveInputSessionDto {
        session_id: s.session_id,
        peer_id: s.peer_id,
        role: s.role.as_str().to_string(),
        requested_edge: s.requested_edge.map(|e| e.as_str().to_string()),
        entry_x: s.entry_x,
        entry_y: s.entry_y,
        capture_width: s.capture_width,
        capture_height: s.capture_height,
        pre_arm_epoch: s.pre_arm_epoch,
    }
}

/// Fold the store policy into the contract DTO (DeviceType -> wire strings).
fn input_policy_dto(p: crate::store::InputTunnelPolicy) -> InputPolicyDto {
    InputPolicyDto {
        enabled: p.enabled,
        allow_inbound_control: p.allow_inbound_control,
        allow_keyboard: p.allow_keyboard,
        allow_mouse: p.allow_mouse,
        hotkey_release_enabled: p.hotkey_release_enabled,
        idle_timeout_secs: p.idle_timeout_secs,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        keymap: p.keymap,
        edge: p.edge,
        wheel_speed: p.wheel_speed,
    }
}

pub async fn input_status(node: &Node) -> InputStatus {
    let rep = node.input_status_report().await;
    InputStatus {
        channel_open: rep.channel_open,
        seat: rep.seat,
        active_peer: rep.active_peer,
        applied: rep.applied,
        dropped: rep.dropped,
        rx_buttons: rep.rx_buttons,
        rx_wheel: rep.rx_wheel,
        rx_keys: rep.rx_keys,
        tx_buttons: rep.tx_buttons,
    }
}

pub async fn input_trace(node: &Node) -> InputTrace {
    let rep = node.input_trace_report().await;
    InputTrace {
        seat: rep.seat,
        channel_open: rep.channel_open,
        capacity: rep.capacity,
        entries: rep
            .entries
            .into_iter()
            .map(|e| InputTraceEntryDto {
                ts_ms: e.ts_ms,
                kind: e.kind,
                detail: e.detail,
            })
            .collect(),
    }
}

pub async fn input_control_start(
    node: &Node,
    peer: Option<String>,
) -> Result<InputControl, CoreError> {
    let rep = node.input_start_reported(peer.as_deref()).await?;
    Ok(InputControl {
        action: rep.action,
        seat: rep.seat,
        channel_open: rep.channel_open,
        peer: rep.peer,
        timestamp_ms: rep.timestamp_ms,
    })
}

pub async fn input_control_stop(node: &Node) -> Result<InputControl, CoreError> {
    let rep = node.input_stop_reported().await?;
    Ok(InputControl {
        action: rep.action,
        seat: rep.seat,
        channel_open: rep.channel_open,
        peer: rep.peer,
        timestamp_ms: rep.timestamp_ms,
    })
}

pub async fn input_tunnel_status(node: &Node) -> InputTunnelStatus {
    let rep = node.input_tunnel_status();
    InputTunnelStatus {
        active: rep.active,
        session: rep.session.map(active_input_session_dto),
        policy: input_policy_dto(rep.policy),
        accessibility_trusted: rep.accessibility_trusted,
        capture_active: rep.capture_active,
        timestamp_ms: rep.timestamp_ms,
    }
}

fn input_tunnel_decision(rep: crate::runtime::InputTunnelDecisionReport) -> InputTunnelDecision {
    InputTunnelDecision {
        action: rep.action,
        session_id: rep.session_id,
        peer_id: rep.peer_id,
        role: rep.role.map(|r| r.as_str().to_string()),
        message: rep.message,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub async fn input_tunnel_attach(
    node: &Node,
    peer: &str,
    edge: Option<tetherly_core::DisplayEdge>,
    x: Option<i32>,
    y: Option<i32>,
) -> Result<InputTunnelDecision, CoreError> {
    let rep = node.input_tunnel_attach(peer, edge, x, y).await?;
    Ok(input_tunnel_decision(rep))
}

pub async fn input_tunnel_release(
    node: &Node,
    reason: Option<&str>,
) -> Result<InputTunnelDecision, CoreError> {
    let rep = node.input_tunnel_release(reason).await?;
    Ok(input_tunnel_decision(rep))
}

pub fn input_policy(node: &Node) -> InputPolicyDto {
    input_policy_dto(node.input_policy())
}

pub fn set_input_policy(
    node: &Node,
    policy: crate::store::InputTunnelPolicy,
) -> Result<InputPolicyDto, CoreError> {
    let rep = node.set_input_policy(policy)?;
    Ok(input_policy_dto(rep.policy))
}

// ---- §165: handoff (URL/resource inbox) domain ----

/// Fold a core inbox entry into the contract DTO. The core `HandoffEntry`
/// does not carry the send-time `auto_open_requested` flag (it is consumed
/// on receipt: auto-open leaves the entry `Opened`); surface that as the
/// observable auto-open signal.
fn handoff_entry_dto(e: tetherly_core::HandoffEntry) -> HandoffInboxEntryDto {
    HandoffInboxEntryDto {
        id: e.id,
        from_peer: e.sender_id,
        from_alias: (!e.sender_alias.is_empty()).then_some(e.sender_alias),
        url: e.url,
        title: e.title,
        auto_open_requested: e.status == tetherly_core::HandoffStatus::Opened,
        received_at_ms: e.received_ms,
    }
}

/// Fold the store policy into the contract DTO.
fn handoff_policy_dto(p: crate::store::HandoffPolicy) -> HandoffPolicyDto {
    HandoffPolicyDto {
        action: p.action.as_str().to_string(),
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
        max_inbox_entries: p.max_inbox_entries,
    }
}

pub fn handoff_inbox(node: &Node, status: Option<tetherly_core::HandoffStatus>) -> HandoffInbox {
    let list: Vec<_> = node.handoff_inbox_list(status);
    let entries: Vec<HandoffInboxEntryDto> = list.into_iter().map(handoff_entry_dto).collect();
    HandoffInbox {
        count: entries.len(),
        entries,
    }
}

pub async fn handoff_send(
    node: &Node,
    peer: &str,
    url: &str,
    title: Option<&str>,
    auto_open: bool,
) -> Result<HandoffSendReport, CoreError> {
    let rep = node.handoff_send(peer, url, title, auto_open).await?;
    Ok(HandoffSendReport {
        id: rep.id,
        peer: rep.peer,
        peer_alias: rep.peer_alias,
        url: rep.url,
        title: rep.title,
        auto_open_requested: rep.auto_open_requested,
        timestamp_ms: rep.timestamp_ms,
    })
}

pub async fn handoff_send_multi(
    node: &Node,
    selector: &crate::store::TargetSelector,
    url: &str,
    title: Option<&str>,
    auto_open: bool,
) -> Result<HandoffSendMultiReport, CoreError> {
    let rep = node
        .handoff_send_multi(selector, url, title, auto_open)
        .await?;
    Ok(HandoffSendMultiReport {
        selector: rep.selector.as_label(),
        url: rep.url,
        title: rep.title,
        target_count: rep.target_count,
        success_count: rep.success_count,
        skipped_offline_count: rep.skipped_offline_count,
        failed_count: rep.failed_count,
        dispatches: rep
            .dispatches
            .into_iter()
            .map(|d| HandoffDispatchItem {
                peer: d.peer,
                peer_alias: d.peer_alias,
                id: d.handoff_id,
                success: d.success,
                error: d.error,
            })
            .collect(),
    })
}

pub fn handoff_inbox_open(node: &Node, id: &str) -> Result<HandoffInboxActionReport, CoreError> {
    let entry = node.handoff_inbox_open(id)?;
    Ok(HandoffInboxActionReport {
        action: "open".to_string(),
        id: entry.id,
        remaining: node.handoff_inbox_list(None).len(),
        timestamp_ms: entry.opened_ms.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        }),
    })
}

pub fn handoff_inbox_dismiss(node: &Node, id: &str) -> Result<HandoffInboxActionReport, CoreError> {
    let entry = node.handoff_inbox_dismiss(id)?;
    Ok(HandoffInboxActionReport {
        action: "dismiss".to_string(),
        id: entry.id,
        remaining: node.handoff_inbox_list(None).len(),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    })
}

pub fn handoff_inbox_clear(node: &Node) -> Result<HandoffInboxClearReport, CoreError> {
    let cleared = node.handoff_inbox_clear()?;
    Ok(HandoffInboxClearReport {
        cleared_count: cleared,
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    })
}

pub fn handoff_policy(node: &Node) -> HandoffPolicyDto {
    handoff_policy_dto(node.handoff_policy())
}

pub fn set_handoff_policy(
    node: &Node,
    policy: crate::store::HandoffPolicy,
) -> Result<HandoffPolicyDto, CoreError> {
    let rep = node.set_handoff_policy(policy)?;
    Ok(handoff_policy_dto(rep.policy))
}

// ---- §165: focus (DND / quiet hours) domain ----

/// Fold a core frame into the contract DTO (FocusMode -> wire string).
fn focus_state_dto(f: tetherly_core::FocusStateFrame) -> FocusStateDto {
    FocusStateDto {
        enabled: f.enabled,
        mode: f.mode.as_str().to_string(),
        expires_ms: f.expires_ms,
        reason: f.reason,
        timestamp_ms: f.timestamp_ms,
    }
}

/// Fold the store policy into the contract DTO.
fn focus_policy_dto(p: crate::store::FocusPolicy) -> FocusPolicyDto {
    FocusPolicyDto {
        enabled: p.enabled,
        default_mode: p.default_mode.as_str().to_string(),
        mirror_inbound_focus: p.mirror_inbound_focus,
        allow_emergency_breakthrough: p.allow_emergency_breakthrough,
        quiet_hours_enabled: p.quiet_hours_enabled,
        quiet_hours_start_minute: p.quiet_hours_start_minute,
        quiet_hours_end_minute: p.quiet_hours_end_minute,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

/// One peer's focus report (runtime `PeerFocusReport` folded).
fn peer_focus_dto(p: crate::runtime::PeerFocusReport) -> PeerFocusDto {
    PeerFocusDto {
        peer: p.peer,
        peer_alias: p.peer_alias,
        state: focus_state_dto(p.state),
        active: p.active,
        remaining_secs: p.remaining_secs,
    }
}

pub fn focus_state(node: &Node) -> FocusState {
    let rep = node.focus_status();
    FocusState {
        state: FocusStateDto {
            enabled: rep.enabled,
            mode: rep.mode.as_str().to_string(),
            expires_ms: rep.expires_ms,
            reason: rep.reason,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        },
        active: rep.active,
        remaining_secs: rep.remaining_secs,
    }
}

pub async fn set_focus_state(
    node: &Node,
    enabled: bool,
    mode: Option<tetherly_core::FocusMode>,
    duration_mins: Option<u32>,
    reason: Option<String>,
    broadcast: bool,
) -> Result<FocusSetReport, CoreError> {
    let rep = node
        .set_focus_state(enabled, mode, duration_mins, reason, broadcast)
        .await?;
    Ok(FocusSetReport {
        enabled: rep.enabled,
        mode: rep.mode.as_str().to_string(),
        active: rep.active,
        expires_ms: rep.expires_ms,
        reason: rep.reason,
        broadcast_count: rep.broadcast_count,
        message: rep.message,
    })
}

pub fn focus_peers(node: &Node) -> FocusPeers {
    let rep = node.peer_focus_states();
    FocusPeers {
        count: rep.count,
        peers: rep.peers.into_iter().map(peer_focus_dto).collect(),
    }
}

pub async fn focus_sync(node: &Node) -> Result<FocusSyncReport, CoreError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let sent = node.request_peers_focus_sync().await?;
    Ok(FocusSyncReport {
        requested: sent,
        responded: 0,
        peers: Vec::new(),
        timestamp_ms: now_ms,
    })
}

pub fn focus_policy(node: &Node) -> FocusPolicyDto {
    focus_policy_dto(node.focus_policy())
}

pub fn set_focus_policy(
    node: &Node,
    policy: crate::store::FocusPolicy,
) -> Result<FocusPolicyDto, CoreError> {
    let rep = node.set_focus_policy(policy)?;
    Ok(focus_policy_dto(rep.policy))
}

// ---- §165: camera (virtual camera streaming) domain ----

/// Fold a runtime session into the contract DTO (resolution flattened).
fn active_camera_session_dto(s: crate::runtime::ActiveCameraSession) -> ActiveCameraSessionDto {
    ActiveCameraSessionDto {
        session_id: s.session_id,
        peer_id: s.peer_id,
        role: s.role.as_str().to_string(),
        lens_id: s.lens_id,
        width: s.resolution.width,
        height: s.resolution.height,
        target_fps: s.target_fps,
        current_fps: s.current_fps,
        codec: s.codec.as_str().to_string(),
        flashlight_on: s.flashlight_on,
        zoom_ratio: s.zoom_ratio,
        frames_sent: s.frames_sent,
        frames_received: s.frames_received,
        bytes_transferred: s.bytes_transferred,
        started_at_ms: s.started_at_ms,
        last_activity_ms: s.last_activity_ms,
    }
}

/// Fold the store policy into the contract DTO.
fn camera_policy_dto(p: crate::store::CameraPolicy) -> CameraPolicyDto {
    CameraPolicyDto {
        enabled: p.enabled,
        allow_inbound_stream: p.allow_inbound_stream,
        allow_remote_capture: p.allow_remote_capture,
        max_resolution_width: p.max_resolution_width,
        max_resolution_height: p.max_resolution_height,
        max_fps: p.max_fps,
        enforce_privacy_shutter_indicator: p.enforce_privacy_shutter_indicator,
        idle_timeout_secs: p.idle_timeout_secs,
        allowed_groups: p.allowed_groups,
        allowed_tags: p.allowed_tags,
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

pub fn camera_status(node: &Node) -> CameraStatus {
    let rep = node.camera_status();
    CameraStatus {
        active: rep.active,
        session: rep.session.map(active_camera_session_dto),
        policy: camera_policy_dto(rep.policy),
        timestamp_ms: rep.timestamp_ms,
    }
}

fn camera_decision(rep: crate::runtime::CameraDecisionReport) -> CameraDecision {
    CameraDecision {
        action: rep.action,
        session_id: rep.session_id,
        peer_id: rep.peer_id,
        role: rep.role.map(|r| r.as_str().to_string()),
        message: rep.message,
        timestamp_ms: rep.timestamp_ms,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn camera_attach(
    node: &Node,
    peer: &str,
    lens_id: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    codec: Option<tetherly_core::CameraCodec>,
    flashlight: Option<bool>,
    zoom: Option<f32>,
) -> Result<CameraDecision, CoreError> {
    let rep = node
        .camera_attach(peer, lens_id, width, height, fps, codec, flashlight, zoom)
        .await?;
    Ok(camera_decision(rep))
}

pub async fn camera_detach(node: &Node, reason: Option<&str>) -> Result<CameraDecision, CoreError> {
    let rep = node.camera_detach(reason).await?;
    Ok(camera_decision(rep))
}

pub fn camera_policy(node: &Node) -> CameraPolicyDto {
    camera_policy_dto(node.camera_policy())
}

pub fn set_camera_policy(
    node: &Node,
    policy: crate::store::CameraPolicy,
) -> Result<CameraPolicyDto, CoreError> {
    let rep = node.set_camera_policy(policy)?;
    Ok(camera_policy_dto(rep.policy))
}

// ---- §165b: mic domain ----

fn active_mic_session_dto(
    s: &crate::runtime::ActiveMicrophoneSession,
) -> tetherly_api::ActiveMicSessionDto {
    tetherly_api::ActiveMicSessionDto {
        session_id: s.session_id.clone(),
        peer_id: s.peer_id.clone(),
        role: s.role.as_str().to_string(),
        mode: s.mode.as_str().to_string(),
        mic_id: s.mic_id.clone(),
        codec: s.codec.as_str().to_string(),
        sample_rate: s.sample_rate,
        channels: s.channels,
        gain: s.gain,
        muted: s.muted,
        noise_suppression: s.noise_suppression,
        echo_cancellation: s.echo_cancellation,
        auto_gain: s.auto_gain,
        privacy_indicator_active: s.privacy_indicator_active,
        frames_sent: s.frames_sent,
        frames_received: s.frames_received,
        bytes_transferred: s.bytes_transferred,
        started_at_ms: s.started_at_ms,
        last_activity_ms: s.last_activity_ms,
    }
}

fn mic_policy_dto(p: &crate::store::MicrophonePolicy) -> MicPolicyDto {
    MicPolicyDto {
        enabled: p.enabled,
        allow_inbound_stream: p.allow_inbound_stream,
        allow_remote_capture: p.allow_remote_capture,
        allow_intercom: p.allow_intercom,
        max_sample_rate: p.max_sample_rate,
        max_session_duration_secs: p.max_session_duration_secs,
        enforce_privacy_indicator: p.enforce_privacy_indicator,
        default_muted: p.default_muted,
        idle_timeout_secs: p.idle_timeout_secs,
        allowed_groups: p.allowed_groups.clone(),
        allowed_tags: p.allowed_tags.clone(),
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

pub fn mic_status(node: &Node) -> MicStatus {
    let rep = node.microphone_status();
    MicStatus {
        active: rep.active,
        session: rep.session.as_ref().map(active_mic_session_dto),
        policy: mic_policy_dto(&rep.policy),
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn mic_decision(rep: crate::runtime::MicrophoneDecisionReport) -> MicDecision {
    MicDecision {
        action: rep.action,
        session_id: rep.session_id,
        peer_id: rep.peer_id,
        role: rep.role.map(|r| r.as_str().to_string()),
        message: rep.message,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn mic_policy(node: &Node) -> MicPolicyDto {
    mic_policy_dto(&node.microphone_policy())
}

#[allow(clippy::too_many_arguments)]
pub async fn mic_attach(
    node: &Node,
    peer_query: &str,
    mic_id: Option<&str>,
    mode: tetherly_core::MicMode,
    codec: Option<tetherly_core::MicCodec>,
    sample_rate: Option<u32>,
    channels: Option<u32>,
    gain: Option<f32>,
    muted: Option<bool>,
    noise_suppression: Option<bool>,
    echo_cancellation: Option<bool>,
    auto_gain: Option<bool>,
) -> Result<MicDecision, CoreError> {
    node.mic_attach(
        peer_query,
        mic_id,
        mode,
        codec,
        sample_rate,
        channels,
        gain,
        muted,
        noise_suppression,
        echo_cancellation,
        auto_gain,
    )
    .await
    .map(mic_decision)
}

pub async fn mic_detach(node: &Node, reason: Option<&str>) -> Result<MicDecision, CoreError> {
    node.mic_detach(reason).await.map(mic_decision)
}

pub fn set_mic_policy(
    node: &Node,
    policy: crate::store::MicrophonePolicy,
) -> Result<MicPolicyDto, CoreError> {
    let rep = node.set_microphone_policy(policy)?;
    Ok(mic_policy_dto(&rep.policy))
}

// ---- §165b: audio routing domain ----

fn spatial_position_dto(p: &tetherly_core::AudioSpatialPosition) -> SpatialPositionDto {
    SpatialPositionDto {
        x: p.x,
        y: p.y,
        z: p.z,
    }
}

fn active_audio_session_dto(
    s: &crate::runtime::ActiveAudioRoutingSession,
) -> tetherly_api::ActiveAudioSessionDto {
    tetherly_api::ActiveAudioSessionDto {
        session_id: s.session_id.clone(),
        peer_id: s.peer_id.clone(),
        role: s.role.as_str().to_string(),
        layout: s.layout.as_str().to_string(),
        sample_rate: s.sample_rate,
        channels: s.channels,
        format: s.format.as_str().to_string(),
        codec: s.codec.as_str().to_string(),
        spatial_pos: s.spatial_pos.as_ref().map(spatial_position_dto),
        volume_gain: s.volume_gain,
        chunks_sent: s.chunks_sent,
        chunks_received: s.chunks_received,
        bytes_transferred: s.bytes_transferred,
        started_at_ms: s.started_at_ms,
        last_activity_ms: s.last_activity_ms,
    }
}

fn audio_policy_dto(p: &crate::store::AudioRoutingPolicy) -> AudioPolicyDto {
    AudioPolicyDto {
        enabled: p.enabled,
        allow_inbound_sink: p.allow_inbound_sink,
        allow_outbound_routing: p.allow_outbound_routing,
        max_volume_limit: p.max_volume_limit,
        allow_spatial_rendering: p.allow_spatial_rendering,
        idle_timeout_secs: p.idle_timeout_secs,
        allowed_groups: p.allowed_groups.clone(),
        allowed_tags: p.allowed_tags.clone(),
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

pub fn audio_status(node: &Node) -> AudioStatus {
    let rep = node.audio_routing_status();
    AudioStatus {
        active: rep.active,
        session: rep.session.as_ref().map(active_audio_session_dto),
        policy: audio_policy_dto(&rep.policy),
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn audio_decision(rep: crate::runtime::AudioRoutingDecisionReport) -> AudioDecision {
    AudioDecision {
        action: rep.action,
        session_id: rep.session_id,
        peer_id: rep.peer_id,
        role: rep.role.map(|r| r.as_str().to_string()),
        message: rep.message,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn audio_policy(node: &Node) -> AudioPolicyDto {
    audio_policy_dto(&node.audio_routing_policy())
}

pub fn set_audio_policy(
    node: &Node,
    policy: crate::store::AudioRoutingPolicy,
) -> Result<AudioPolicyDto, CoreError> {
    let rep = node.set_audio_routing_policy(policy)?;
    Ok(audio_policy_dto(&rep.policy))
}

#[allow(clippy::too_many_arguments)]
pub async fn audio_attach(
    node: &Node,
    peer_query: &str,
    layout: Option<tetherly_core::AudioChannelLayout>,
    sample_rate: Option<u32>,
    channels: Option<u8>,
    spatial_x: Option<f32>,
    spatial_y: Option<f32>,
    spatial_z: Option<f32>,
    volume_gain: Option<f32>,
) -> Result<AudioDecision, CoreError> {
    node.audio_routing_attach(
        peer_query,
        layout,
        sample_rate,
        channels,
        spatial_x,
        spatial_y,
        spatial_z,
        volume_gain,
    )
    .await
    .map(audio_decision)
}

pub async fn audio_detach(node: &Node, reason: Option<&str>) -> Result<AudioDecision, CoreError> {
    node.audio_routing_detach(reason).await.map(audio_decision)
}

// ---- §165b: locate domain ----

fn active_locate_ring_dto(
    r: &crate::runtime::ActiveLocateRing,
) -> tetherly_api::ActiveLocateRingDto {
    tetherly_api::ActiveLocateRingDto {
        request_id: r.request_id.clone(),
        initiator_peer: r.initiator_peer.clone(),
        initiator_alias: r.initiator_alias.clone(),
        duration_secs: r.duration_secs,
        pattern: r.pattern.as_str().to_string(),
        override_silent: r.override_silent,
        message: r.message.clone(),
        started_at_ms: r.started_at_ms,
        expires_at_ms: r.expires_at_ms,
    }
}

fn locate_policy_dto(p: &crate::store::LocatorPolicy) -> LocatePolicyDto {
    LocatePolicyDto {
        enabled: p.enabled,
        allow_override_silent: p.allow_override_silent,
        max_duration_secs: p.max_duration_secs,
        allowed_groups: p.allowed_groups.clone(),
        allowed_tags: p.allowed_tags.clone(),
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

pub fn locate_status(node: &Node) -> LocateStatus {
    let rep = node.locate_status();
    LocateStatus {
        is_ringing: rep.is_ringing,
        active_ring: rep.active_ring.as_ref().map(active_locate_ring_dto),
        policy: locate_policy_dto(&rep.policy),
        message: rep.message,
    }
}

pub fn locate_ring(rep: crate::runtime::LocateRingReport) -> LocateRingReport {
    LocateRingReport {
        request_id: rep.request_id,
        target_peer: rep.target_peer,
        duration_secs: rep.duration_secs,
        pattern: rep.pattern.as_str().to_string(),
        override_silent: rep.override_silent,
        message: rep.message,
    }
}

pub fn locate_stop(rep: crate::runtime::LocateStopReport) -> LocateStopReport {
    LocateStopReport {
        request_id: rep.request_id,
        target_peer: rep.target_peer,
        reason: rep.reason,
        message: rep.message,
    }
}

pub fn locate_policy(node: &Node) -> LocatePolicyDto {
    locate_policy_dto(&node.locator_policy())
}

pub fn set_locate_policy(
    node: &Node,
    policy: crate::store::LocatorPolicy,
) -> Result<LocatePolicyDto, CoreError> {
    let rep = node.set_locator_policy(policy)?;
    Ok(locate_policy_dto(&rep.policy))
}

pub async fn locate_ring_request(
    node: &Node,
    peer_target: &str,
    duration_secs: Option<u32>,
    pattern: Option<tetherly_core::LocatePattern>,
    override_silent: Option<bool>,
    message: Option<String>,
) -> Result<LocateRingReport, CoreError> {
    node.locate_ring(
        peer_target,
        duration_secs,
        pattern,
        override_silent,
        message,
    )
    .await
    .map(locate_ring)
}

pub async fn locate_stop_request(
    node: &Node,
    peer_target: Option<&str>,
    request_id: Option<String>,
    reason: Option<String>,
) -> Result<LocateStopReport, CoreError> {
    node.locate_stop(peer_target, request_id, reason)
        .await
        .map(locate_stop)
}

// ---- §165b: virtual display domain ----

fn display_mode_dto(m: &tetherly_core::VirtualDisplayMode) -> DisplayModeDto {
    DisplayModeDto {
        width: m.width,
        height: m.height,
        refresh_rate: m.refresh_rate,
        scale_factor: m.scale_factor,
        pixel_format: match m.pixel_format {
            tetherly_core::VirtualDisplayPixelFormat::Bgra8 => "bgra8".to_string(),
            tetherly_core::VirtualDisplayPixelFormat::Rgba8 => "rgba8".to_string(),
        },
    }
}

fn active_display_session_dto(
    s: &crate::runtime::ActiveVirtualDisplaySession,
) -> tetherly_api::ActiveDisplaySessionDto {
    tetherly_api::ActiveDisplaySessionDto {
        session_id: s.session_id.clone(),
        peer_id: s.peer_id.clone(),
        role: s.role.as_str().to_string(),
        assigned_edge: s.assigned_edge.to_string(),
        mode: display_mode_dto(&s.mode),
        codec: s.codec.as_str().to_string(),
        target_fps: s.target_fps,
        current_fps: s.current_fps,
        rtt_us: s.rtt_us,
        dropped_frames: s.dropped_frames,
        started_at_ms: s.started_at_ms,
        last_activity_ms: s.last_activity_ms,
    }
}

fn display_policy_dto(p: &crate::store::VirtualDisplayPolicy) -> DisplayPolicyDto {
    DisplayPolicyDto {
        enabled: p.enabled,
        allow_hosting: p.allow_hosting,
        allow_remote_extend: p.allow_remote_extend,
        max_resolution_width: p.max_resolution_width,
        max_resolution_height: p.max_resolution_height,
        max_fps: p.max_fps,
        idle_timeout_secs: p.idle_timeout_secs,
        allowed_groups: p.allowed_groups.clone(),
        allowed_tags: p.allowed_tags.clone(),
        allowed_types: p
            .allowed_types
            .iter()
            .map(|t| t.as_str().to_string())
            .collect(),
    }
}

pub fn display_status(node: &Node) -> DisplayVirtualStatus {
    let rep = node.virtual_display_status();
    DisplayVirtualStatus {
        active: rep.active,
        session: rep.session.as_ref().map(active_display_session_dto),
        policy: display_policy_dto(&rep.policy),
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn display_decision(rep: crate::runtime::VirtualDisplayDecisionReport) -> DisplayDecision {
    DisplayDecision {
        action: rep.action,
        session_id: rep.session_id,
        peer_id: rep.peer_id,
        role: rep.role.map(|r| r.as_str().to_string()),
        message: rep.message,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn display_policy(node: &Node) -> DisplayPolicyDto {
    display_policy_dto(&node.virtual_display_policy())
}

pub fn set_display_policy(
    node: &Node,
    policy: crate::store::VirtualDisplayPolicy,
) -> Result<DisplayPolicyDto, CoreError> {
    let rep = node.set_virtual_display_policy(policy)?;
    Ok(display_policy_dto(&rep.policy))
}

pub async fn display_attach(
    node: &Node,
    peer_query: &str,
    edge: Option<tetherly_core::DisplayEdge>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<u32>,
) -> Result<DisplayDecision, CoreError> {
    node.virtual_display_attach(peer_query, edge, width, height, fps)
        .await
        .map(display_decision)
}

pub async fn display_detach(
    node: &Node,
    reason: Option<&str>,
) -> Result<DisplayDecision, CoreError> {
    node.virtual_display_detach(reason)
        .await
        .map(display_decision)
}

// ---- §167: screen domain ----

/// Serializes a runtime enum to its exact legacy wire string (serde is the
/// canonical wire form the legacy HTTP layer emitted directly).
fn wire_enum<T: serde::Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

pub fn screen_status_of(rep: crate::runtime::ScreenStatusReport) -> ScreenStatus {
    ScreenStatus {
        state: rep.state,
        channel_open: rep.channel_open,
        allowed: rep.allowed,
        sent: rep.sent,
        received: rep.received,
        dropped: rep.dropped,
        refused: rep.refused,
        bytes: rep.bytes,
        rx_bytes: rep.rx_bytes,
        p95_frame_interval_us: rep.p95_frame_interval_us,
        active_peer: rep.active_peer,
        acked: rep.acked,
        in_flight: rep.in_flight,
        rtt_latest_us: rep.rtt_latest_us,
        rtt_p95_us: rep.rtt_p95_us,
        is_congested: rep.is_congested,
        keyframes_requested: rep.keyframes_requested,
        gaps: rep.gaps,
        bitrate_bps: rep.bitrate_bps,
        fps_target: rep.fps_target,
        scale_pct: rep.scale_pct,
        profile: rep.profile,
        qp_min: rep.qp_min,
        qp_max: rep.qp_max,
        matrix_mode: rep.matrix_mode,
        network_health: rep.network_health,
        playout_mode: rep.playout_mode,
        playout_delay_ms: rep.playout_delay_ms,
        playout_jitter_ms: rep.playout_jitter_ms,
        playout_buffered: rep.playout_buffered,
        playout_dropped_late: rep.playout_dropped_late,
        cursor_mode: rep.cursor_mode,
        cursor_x: rep.cursor_x,
        cursor_y: rep.cursor_y,
        cursor_visible: rep.cursor_visible,
        cursor_shape: rep.cursor_shape,
        cursor_updates_sent: rep.cursor_updates_sent,
        cursor_updates_received: rep.cursor_updates_received,
        squelch_mode: rep.squelch_mode,
        is_squelched: rep.is_squelched,
        squelched_frames: rep.squelched_frames,
        fec_mode: rep.fec_mode,
        fec_redundancy_pct: rep.fec_redundancy_pct,
        fec_recovered_shards: rep.fec_recovered_shards,
        fec_unrecoverable_blocks: rep.fec_unrecoverable_blocks,
        pacer_mode: rep.pacer_mode,
        pacer_packets_paced: rep.pacer_packets_paced,
        pacer_bursts_smoothed: rep.pacer_bursts_smoothed,
        pacer_avg_delay_us: rep.pacer_avg_delay_us,
        trendline_mode: rep.trendline_mode,
        trendline_state: rep.trendline_state,
        trendline_slope_us: rep.trendline_slope_us,
        trendline_overuse_count: rep.trendline_overuse_count,
        harq_mode: rep.harq_mode,
        harq_nacks_sent: rep.harq_nacks_sent,
        harq_nacks_received: rep.harq_nacks_received,
        harq_shards_retransmitted: rep.harq_shards_retransmitted,
        harq_recovered_frames: rep.harq_recovered_frames,
        harq_retransmit_timeouts: rep.harq_retransmit_timeouts,
        harq_buffer_occupancy: rep.harq_buffer_occupancy,
        twcc_mode: rep.twcc_mode,
        twcc_feedbacks_sent: rep.twcc_feedbacks_sent,
        twcc_feedbacks_received: rep.twcc_feedbacks_received,
        twcc_delivery_rate_bps: rep.twcc_delivery_rate_bps,
        twcc_delay_gradient_us: rep.twcc_delay_gradient_us,
        twcc_loss_ratio: rep.twcc_loss_ratio,
        probe_mode: rep.probe_mode,
        probe_state: rep.probe_state,
        probes_triggered: rep.probes_triggered,
        probes_completed: rep.probes_completed,
        probes_failed: rep.probes_failed,
        probe_target_bps: rep.probe_target_bps,
        probe_measured_bps: rep.probe_measured_bps,
        sync_mode: rep.sync_mode,
        sync_beacons_sent: rep.sync_beacons_sent,
        sync_beacons_received: rep.sync_beacons_received,
        sync_skew_us: rep.sync_skew_us,
        sync_clock_drift_ppm: rep.sync_clock_drift_ppm,
        sync_adjustments_applied: rep.sync_adjustments_applied,
        in_sync: rep.in_sync,
        active_display_id: rep.active_display_id,
        display_count: rep.display_count,
        display_switches_count: rep.display_switches_count,
        privacy_enabled: rep.privacy_enabled,
        privacy_input_locked: rep.privacy_input_locked,
        privacy_blanking_method: rep.privacy_blanking_method,
        latency_mode: rep.latency_mode,
        latency_bottleneck: rep.latency_bottleneck,
        latency_total_e2e_us: rep.latency_total_e2e_us,
        latency_p95_e2e_us: rep.latency_p95_e2e_us,
        latency_frames_profiled: rep.latency_frames_profiled,
        tuning_mode: rep.tuning_mode,
        tuning_actions_total: rep.tuning_actions_total,
        tuning_recoveries: rep.tuning_recoveries,
        tuning_last_stage: rep.tuning_last_stage,
        tuning_last_action: rep.tuning_last_action,
        recording_active: rep.recording_active,
        peer_recording: rep.peer_recording,
        recording_frames: rep.recording_frames,
        playback_active: rep.playback_active,
        playback_state: rep.playback_state,
        playback_frame: rep.playback_frame,
        snapshot_count: rep.snapshot_count,
        watermark_enabled: rep.watermark_enabled,
        watermark_enforced: rep.watermark_enforced,
        watermark_frames: rep.watermark_frames,
        annotations_count: rep.annotations_count,
        laser_active: rep.laser_active,
        annotations_total: rep.annotations_total,
        laser_updates_total: rep.laser_updates_total,
        whiteboard_undo_count: rep.whiteboard_undo_count,
        whiteboard_redo_count: rep.whiteboard_redo_count,
        whiteboard_read_only: rep.whiteboard_read_only,
        whiteboard_max_items: rep.whiteboard_max_items,
        roi_active: rep.roi_active,
        roi_x: rep.roi_x,
        roi_y: rep.roi_y,
        roi_w: rep.roi_w,
        roi_h: rep.roi_h,
        roi_zoom_percent: rep.roi_zoom_percent,
        roi_updates: rep.roi_updates,
        color_mode: rep.color_mode,
        color_threshold: rep.color_threshold,
        color_updates: rep.color_updates,
        delta_enabled: rep.delta_enabled,
        delta_keyframe_interval: rep.delta_keyframe_interval,
        delta_frames_sent: rep.delta_frames_sent,
        delta_keyframes_sent: rep.delta_keyframes_sent,
        delta_frames_received: rep.delta_frames_received,
        delta_keyframes_received: rep.delta_keyframes_received,
        delta_saved_bytes: rep.delta_saved_bytes,
        metric_enabled: rep.metric_enabled,
        metric_sample_interval: rep.metric_sample_interval,
        metric_evaluated_frames: rep.metric_evaluated_frames,
        metric_psnr_db: rep.metric_psnr_db,
        metric_ssim: rep.metric_ssim,
        metric_quality_warning: rep.metric_quality_warning,
        qoe_enabled: rep.qoe_enabled,
        qoe_mode: rep.qoe_mode,
        qoe_target_psnr_db: rep.qoe_target_psnr_db,
        qoe_target_ssim_pct: rep.qoe_target_ssim_pct,
        qoe_actions_total: rep.qoe_actions_total,
        qoe_last_action: rep.qoe_last_action,
        pir_enabled: rep.pir_enabled,
        pir_mode: rep.pir_mode,
        pir_bands: rep.pir_bands,
        pir_sweeps_completed: rep.pir_sweeps_completed,
        pir_slices_sent: rep.pir_slices_sent,
        ltr_enabled: rep.ltr_enabled,
        ltr_auto_interval: rep.ltr_auto_interval,
        ltr_current_seq: rep.ltr_current_seq,
        ltr_anchors_established: rep.ltr_anchors_established,
        ltr_recoveries_completed: rep.ltr_recoveries_completed,
        svc_enabled: rep.svc_enabled,
        svc_mode: rep.svc_mode,
        svc_drop_t1: rep.svc_drop_t1,
        svc_t0_sent: rep.svc_t0_sent,
        svc_t1_sent: rep.svc_t1_sent,
        svc_t1_dropped: rep.svc_t1_dropped,
        motion_enabled: rep.motion_enabled,
        motion_max_displacement: rep.motion_max_displacement,
        motion_frames_sent: rep.motion_frames_sent,
        motion_fallback_frames_sent: rep.motion_fallback_frames_sent,
        motion_saved_bytes: rep.motion_saved_bytes,
        tile_enabled: rep.tile_enabled,
        tile_size: rep.tile_size,
        tile_frames_sent: rep.tile_frames_sent,
        dirty_tiles_sent: rep.dirty_tiles_sent,
        skipped_tiles_count: rep.skipped_tiles_count,
        tile_saved_bytes: rep.tile_saved_bytes,
        tile_cache_enabled: rep.tile_cache_enabled,
        tile_cache_max_entries: rep.tile_cache_max_entries,
        tile_cache_entries: rep.tile_cache_entries,
        tile_cache_hits: rep.tile_cache_hits,
        tile_cache_misses: rep.tile_cache_misses,
        tile_cache_hit_ratio_pct: rep.tile_cache_hit_ratio_pct,
        tile_cache_saved_bytes: rep.tile_cache_saved_bytes,
        focus_enabled: rep.focus_enabled,
        focus_mode: rep.focus_mode,
        focus_radius: rep.focus_radius,
        focus_peripheral_skip_ratio: rep.focus_peripheral_skip_ratio,
        focus_box_x: rep.focus_box_x,
        focus_box_y: rep.focus_box_y,
        focus_box_w: rep.focus_box_w,
        focus_box_h: rep.focus_box_h,
        focus_tiles_sent: rep.focus_tiles_sent,
        peripheral_tiles_sent: rep.peripheral_tiles_sent,
        peripheral_tiles_skipped: rep.peripheral_tiles_skipped,
        focus_saved_bandwidth_bytes: rep.focus_saved_bandwidth_bytes,
        tile_palette_enabled: rep.tile_palette_enabled,
        tile_palette_max_colors: rep.tile_palette_max_colors,
        tile_palette_enable_rle: rep.tile_palette_enable_rle,
        tile_palette_solid_tiles: rep.tile_palette_solid_tiles,
        tile_palette_bilevel_tiles: rep.tile_palette_bilevel_tiles,
        tile_palette_palette_tiles: rep.tile_palette_palette_tiles,
        tile_palette_truecolor_tiles: rep.tile_palette_truecolor_tiles,
        tile_palette_saved_bytes: rep.tile_palette_saved_bytes,
        tile_predict_enabled: rep.tile_predict_enabled,
        tile_predict_filter_mode: rep.tile_predict_filter_mode,
        tile_predict_enable_rct: rep.tile_predict_enable_rct,
        tile_predict_tiles_sent: rep.tile_predict_tiles_sent,
        tile_predict_saved_bytes: rep.tile_predict_saved_bytes,
        tile_present_enabled: rep.tile_present_enabled,
        tile_present_vsync_mode: rep.tile_present_vsync_mode,
        tile_present_double_buffer: rep.tile_present_double_buffer,
        tile_present_tear_prevention: rep.tile_present_tear_prevention,
        tile_present_committed_transactions: rep.tile_present_committed_transactions,
        tile_present_swapped_frames: rep.tile_present_swapped_frames,
        tile_present_torn_frames_prevented: rep.tile_present_torn_frames_prevented,
        tile_present_avg_swap_latency_us: rep.tile_present_avg_swap_latency_us,
        tile_nack_enabled: rep.tile_nack_enabled,
        tile_nack_timeout_ms: rep.tile_nack_timeout_ms,
        tile_nack_max_retries: rep.tile_nack_max_retries,
        tile_nack_requests_sent: rep.tile_nack_requests_sent,
        tile_nack_tiles_retransmitted: rep.tile_nack_tiles_retransmitted,
        tile_nack_degraded_commits: rep.tile_nack_degraded_commits,
        tile_saliency_enabled: rep.tile_saliency_enabled,
        tile_saliency_high_threshold: rep.tile_saliency_high_threshold,
        tile_saliency_low_threshold: rep.tile_saliency_low_threshold,
        tile_saliency_drop_background_on_congestion: rep
            .tile_saliency_drop_background_on_congestion,
        tile_saliency_critical_tiles: rep.tile_saliency_critical_tiles,
        tile_saliency_normal_tiles: rep.tile_saliency_normal_tiles,
        tile_saliency_background_tiles: rep.tile_saliency_background_tiles,
        tile_saliency_background_skipped: rep.tile_saliency_background_skipped,
        tile_lod_enabled: rep.tile_lod_enabled,
        tile_lod_max_lod: rep.tile_lod_max_lod,
        tile_lod_auto_lod_on_congestion: rep.tile_lod_auto_lod_on_congestion,
        tile_lod0_tiles: rep.tile_lod0_tiles,
        tile_lod1_tiles: rep.tile_lod1_tiles,
        tile_lod2_tiles: rep.tile_lod2_tiles,
        tile_lod_bytes_saved: rep.tile_lod_bytes_saved,
        tile_batch_enabled: rep.tile_batch_enabled,
        tile_batch_target_chunk_bytes: rep.tile_batch_target_chunk_bytes,
        tile_batch_pacing_interval_us: rep.tile_batch_pacing_interval_us,
        tile_batches_sent: rep.tile_batches_sent,
        tile_chunks_sent: rep.tile_chunks_sent,
        tile_batch_avg_chunk_bytes: rep.tile_batch_avg_chunk_bytes,
        tile_batch_pacing_pauses: rep.tile_batch_pacing_pauses,
        tile_motion_enabled: rep.tile_motion_enabled,
        tile_motion_max_displacement: rep.tile_motion_max_displacement,
        tile_motion_min_match_ratio: rep.tile_motion_min_match_ratio,
        tile_motion_tiles_sent: rep.tile_motion_tiles_sent,
        tile_motion_fallback_tiles_sent: rep.tile_motion_fallback_tiles_sent,
        tile_motion_bytes_saved: rep.tile_motion_bytes_saved,
    }
}

pub fn screen_control_of(rep: crate::runtime::ScreenControlReport) -> ScreenControl {
    ScreenControl {
        action: rep.action,
        state: rep.state,
        allowed: rep.allowed,
        peer: rep.peer,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_displays_of(rep: tetherly_core::screen::ScreenDisplayReport) -> ScreenDisplays {
    ScreenDisplays {
        active_display_id: rep.active_display_id,
        displays: rep
            .displays
            .into_iter()
            .map(screen_display_info_dto_of)
            .collect(),
        switches_count: rep.switches_count,
        last_switch_us: rep.last_switch_us,
    }
}

pub fn screen_display_info_dto_of(
    rep: tetherly_core::screen::ScreenDisplayInfo,
) -> ScreenDisplayInfoDto {
    ScreenDisplayInfoDto {
        display_id: rep.display_id,
        title: rep.title,
        width: rep.width,
        height: rep.height,
        scale_factor_pct: rep.scale_factor_pct,
        is_primary: rep.is_primary,
        is_virtual: rep.is_virtual,
    }
}

pub fn screen_privacy_of(rep: tetherly_core::screen::ScreenPrivacyReport) -> ScreenPrivacy {
    ScreenPrivacy {
        enabled: rep.enabled,
        lock_input: rep.lock_input,
        blanking_method: rep.blanking_method,
        last_change_us: rep.last_change_us,
    }
}

pub fn screen_recording_of(rep: crate::runtime::ScreenRecordingReport) -> ScreenRecording {
    ScreenRecording {
        active: rep.active,
        path: rep.path,
        frames_recorded: rep.frames_recorded,
        bytes_recorded: rep.bytes_recorded,
        started_at_us: rep.started_at_us,
        duration_us: rep.duration_us,
        last_frame_seq: rep.last_frame_seq,
        peer_recording: rep.peer_recording,
        width: rep.width,
        height: rep.height,
    }
}

pub fn screen_recording_replay_of(
    rep: crate::runtime::ScreenRecordingReplayReport,
) -> ScreenRecordingReplay {
    ScreenRecordingReplay {
        path: rep.path,
        width: rep.width,
        height: rep.height,
        format_tag: rep.format_tag,
        codec_tag: rep.codec_tag,
        started_at_us: rep.started_at_us,
        records: rep
            .records
            .into_iter()
            .map(screen_replay_record_dto_of)
            .collect(),
        records_returned: rep.records_returned,
        truncated: rep.truncated,
    }
}

pub fn screen_replay_record_dto_of(
    rep: crate::runtime::ScreenRecordingReplayRecord,
) -> ScreenReplayRecordDto {
    ScreenReplayRecordDto {
        seq: rep.seq,
        pts_us: rep.pts_us,
        len: rep.len,
    }
}

pub fn screen_playback_of(rep: tetherly_core::screen::ScreenPlaybackReport) -> ScreenPlayback {
    ScreenPlayback {
        state: rep.state,
        path: rep.path,
        speed: rep.speed,
        loop_enabled: rep.loop_enabled,
        current_frame: rep.current_frame,
        total_frames: rep.total_frames,
        elapsed_ms: rep.elapsed_ms,
        duration_ms: rep.duration_ms,
        width: rep.width,
        height: rep.height,
        format: rep.format,
        codec: rep.codec,
    }
}

pub fn screen_snapshot_of(rep: tetherly_core::screen::ScreenSnapshotReport) -> ScreenSnapshot {
    ScreenSnapshot {
        source: rep.source,
        frame_seq: rep.frame_seq,
        pts_us: rep.pts_us,
        width: rep.width,
        height: rep.height,
        format: rep.format,
        bytes_len: rep.bytes_len,
        privacy_blanked: rep.privacy_blanked,
        watermarked: rep.watermarked,
        path: rep.path,
        data_base64: rep.data_base64,
    }
}

pub fn screen_pir_of(rep: tetherly_core::screen::ScreenPirReport) -> ScreenPir {
    ScreenPir {
        mode: rep.mode,
        bands: rep.bands,
        sweeps_completed: rep.sweeps_completed,
        slices_sent: rep.slices_sent,
        current_band: rep.current_band,
        active_sweep: rep.active_sweep,
    }
}

pub fn screen_ltr_of(rep: tetherly_core::screen::ScreenLtrReport) -> ScreenLtr {
    ScreenLtr {
        enabled: rep.enabled,
        auto_interval: rep.auto_interval,
        current_ltr_seq: rep.current_ltr_seq,
        anchors_established: rep.anchors_established,
        recoveries_requested: rep.recoveries_requested,
        recoveries_completed: rep.recoveries_completed,
        saved_burst_bytes: rep.saved_burst_bytes,
    }
}

pub fn screen_svc_of(rep: tetherly_core::screen::ScreenSvcReport) -> ScreenSvc {
    ScreenSvc {
        enabled: rep.enabled,
        mode: rep.mode,
        drop_t1: rep.drop_t1,
        t0_frames_sent: rep.t0_frames_sent,
        t1_frames_sent: rep.t1_frames_sent,
        t1_frames_dropped: rep.t1_frames_dropped,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_cursor_of(rep: crate::runtime::ScreenCursorReport) -> ScreenCursor {
    ScreenCursor {
        mode: rep.mode,
        x: rep.x,
        y: rep.y,
        visible: rep.visible,
        shape: rep.shape,
        updates_sent: rep.updates_sent,
        updates_received: rep.updates_received,
        updates_dropped_dedup: rep.updates_dropped_dedup,
        updates_dropped_rate: rep.updates_dropped_rate,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_trendline_of(rep: tetherly_core::screen::ScreenTrendlineReport) -> ScreenTrendline {
    ScreenTrendline {
        mode: wire_enum(&rep.mode),
        state: wire_enum(&rep.state),
        samples_count: rep.samples_count,
        current_slope_us_per_frame: rep.current_slope_us_per_frame,
        threshold_us_per_frame: rep.threshold_us_per_frame,
        overuse_count: rep.overuse_count,
        underuse_count: rep.underuse_count,
        accumulated_delay_us: rep.accumulated_delay_us,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_probe_of(rep: tetherly_core::screen::ScreenProbeReport) -> ScreenProbe {
    ScreenProbe {
        mode: wire_enum(&rep.mode),
        state: wire_enum(&rep.state),
        probes_triggered: rep.probes_triggered,
        probes_completed: rep.probes_completed,
        probes_failed: rep.probes_failed,
        last_target_bps: rep.last_target_bps,
        last_measured_bps: rep.last_measured_bps,
        last_rtt_us: rep.last_rtt_us,
        last_loss_ratio: rep.last_loss_ratio,
        last_delay_gradient_us: rep.last_delay_gradient_us,
        active_cluster_id: rep.active_cluster_id,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_squelch_of(rep: crate::runtime::ScreenSquelchReport) -> ScreenSquelch {
    ScreenSquelch {
        mode: rep.mode,
        is_squelched: rep.is_squelched,
        frames_evaluated: rep.frames_evaluated,
        frames_passed: rep.frames_passed,
        frames_squelched: rep.frames_squelched,
        keepalive_frames: rep.keepalive_frames,
        last_dirty_ratio: rep.last_dirty_ratio,
        consecutive_static_frames: rep.consecutive_static_frames,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_fec_of(rep: tetherly_core::screen::ScreenFecReport) -> ScreenFec {
    ScreenFec {
        mode: wire_enum(&rep.mode),
        redundancy_pct: rep.redundancy_pct,
        blocks_encoded: rep.blocks_encoded,
        data_shards_sent: rep.data_shards_sent,
        parity_shards_sent: rep.parity_shards_sent,
        blocks_decoded: rep.blocks_decoded,
        recovered_shards: rep.recovered_shards,
        unrecoverable_blocks: rep.unrecoverable_blocks,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_pacer_of(rep: tetherly_core::screen::ScreenPacerReport) -> ScreenPacer {
    ScreenPacer {
        mode: wire_enum(&rep.mode),
        packets_paced: rep.packets_paced,
        bursts_smoothed: rep.bursts_smoothed,
        avg_delay_us: rep.avg_delay_us,
        last_delay_us: rep.last_delay_us,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_harq_of(rep: tetherly_core::screen::ScreenHarqReport) -> ScreenHarq {
    ScreenHarq {
        mode: wire_enum(&rep.mode),
        nacks_sent: rep.nacks_sent,
        nacks_received: rep.nacks_received,
        shards_retransmitted: rep.shards_retransmitted,
        recovered_frames: rep.recovered_frames,
        retransmit_timeouts: rep.retransmit_timeouts,
        buffer_occupancy: rep.buffer_occupancy,
        buffer_capacity: rep.buffer_capacity,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_twcc_of(rep: tetherly_core::screen::ScreenTwccReport) -> ScreenTwcc {
    ScreenTwcc {
        mode: wire_enum(&rep.mode),
        feedbacks_sent: rep.feedbacks_sent,
        feedbacks_received: rep.feedbacks_received,
        packets_tracked: rep.packets_tracked,
        packets_acknowledged: rep.packets_acknowledged,
        packets_lost: rep.packets_lost,
        loss_ratio: rep.loss_ratio,
        delivery_rate_bps: rep.delivery_rate_bps,
        delay_gradient_us: rep.delay_gradient_us,
        smoothed_rtt_us: rep.smoothed_rtt_us,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_sync_of(rep: tetherly_core::screen::ScreenAudioSyncReport) -> ScreenSync {
    ScreenSync {
        mode: wire_enum(&rep.mode),
        beacons_sent: rep.beacons_sent,
        beacons_received: rep.beacons_received,
        current_skew_us: rep.current_skew_us,
        clock_drift_ppm: rep.clock_drift_ppm,
        adjustments_applied: rep.adjustments_applied,
        in_sync: rep.in_sync,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_latency_of(rep: tetherly_core::screen::ScreenLatencyReport) -> ScreenLatency {
    ScreenLatency {
        mode: rep.mode,
        frames_profiled: rep.frames_profiled,
        bottleneck: rep.bottleneck,
        current: screen_latency_breakdown_dto_of(rep.current),
        avg_e2e_us: rep.avg_e2e_us,
        p50_e2e_us: rep.p50_e2e_us,
        p90_e2e_us: rep.p90_e2e_us,
        p95_e2e_us: rep.p95_e2e_us,
        p99_e2e_us: rep.p99_e2e_us,
        min_e2e_us: rep.min_e2e_us,
        max_e2e_us: rep.max_e2e_us,
        avg_capture_us: rep.avg_capture_us,
        avg_encode_us: rep.avg_encode_us,
        avg_pacer_us: rep.avg_pacer_us,
        avg_network_us: rep.avg_network_us,
        avg_jitter_us: rep.avg_jitter_us,
        avg_decode_us: rep.avg_decode_us,
        avg_render_us: rep.avg_render_us,
    }
}

pub fn screen_latency_breakdown_dto_of(
    rep: tetherly_core::screen::ScreenLatencyBreakdown,
) -> ScreenLatencyBreakdownDto {
    ScreenLatencyBreakdownDto {
        frame_seq: rep.frame_seq,
        capture_us: rep.capture_us,
        encode_us: rep.encode_us,
        pacer_us: rep.pacer_us,
        network_us: rep.network_us,
        jitter_us: rep.jitter_us,
        decode_us: rep.decode_us,
        render_us: rep.render_us,
        total_e2e_us: rep.total_e2e_us,
    }
}

pub fn screen_tuning_of(rep: tetherly_core::screen::ScreenTuningReport) -> ScreenTuning {
    ScreenTuning {
        mode: rep.mode,
        actions_applied: rep.actions_applied,
        recoveries_applied: rep.recoveries_applied,
        last_stage: rep.last_stage,
        last_action: rep.last_action,
        bottleneck_streak: rep.bottleneck_streak,
        balanced_streak: rep.balanced_streak,
        last_decision: rep.last_decision.map(screen_tuning_decision_dto_of),
    }
}

pub fn screen_tuning_decision_dto_of(
    rep: tetherly_core::screen::ScreenTuningDecision,
) -> ScreenTuningDecisionDto {
    ScreenTuningDecisionDto {
        frame_seq: rep.frame_seq,
        stage: rep.stage,
        action: rep.action,
        new_bitrate_bps: rep.new_bitrate_bps,
        new_fps: rep.new_fps,
        new_scale_pct: rep.new_scale_pct,
        new_qp_max: rep.new_qp_max,
        new_playout_delay_us: rep.new_playout_delay_us,
        recovered: rep.recovered,
        at_us: rep.at_us,
    }
}

pub fn screen_playout_of(rep: crate::runtime::ScreenPlayoutReport) -> ScreenPlayout {
    ScreenPlayout {
        mode: rep.mode,
        target_delay_ms: rep.target_delay_ms,
        current_jitter_ms: rep.current_jitter_ms,
        target_delay_us: rep.target_delay_us,
        current_jitter_us: rep.current_jitter_us,
        buffered_frames: rep.buffered_frames,
        played_frames: rep.played_frames,
        dropped_late_frames: rep.dropped_late_frames,
        dropped_overflow_frames: rep.dropped_overflow_frames,
        custom_delay_us: rep.custom_delay_us,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_matrix_of(rep: crate::runtime::ScreenMatrixReport) -> ScreenMatrix {
    ScreenMatrix {
        mode: rep.mode,
        health: rep.health,
        bitrate_bps: rep.bitrate_bps,
        fps_target: rep.fps_target,
        scale_pct: rep.scale_pct,
        profile: rep.profile,
        qp_min: rep.qp_min,
        qp_max: rep.qp_max,
        auto_fps: rep.auto_fps,
        auto_scale: rep.auto_scale,
        consecutive_healthy_acks: rep.consecutive_healthy_acks,
        consecutive_drops: rep.consecutive_drops,
        in_flight: rep.in_flight,
        rtt_us: rep.rtt_us,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_profile_of(rep: crate::runtime::ScreenProfileReport) -> ScreenProfile {
    ScreenProfile {
        profile: wire_enum(&rep.profile),
        min_qp: rep.min_qp,
        max_qp: rep.max_qp,
        is_custom_qp: rep.is_custom_qp,
        auto_profile: rep.auto_profile,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_scale_of(rep: crate::runtime::ScreenScaleReport) -> ScreenScale {
    ScreenScale {
        scale_pct: rep.scale_pct,
        min_scale_pct: rep.min_scale_pct,
        max_scale_pct: rep.max_scale_pct,
        auto_profile: rep.auto_profile,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_fps_of(rep: crate::runtime::ScreenFpsReport) -> ScreenFps {
    ScreenFps {
        fps: rep.fps,
        min_fps: rep.min_fps,
        max_fps: rep.max_fps,
        interval_us: rep.interval_us,
        auto_profile: rep.auto_profile,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_bitrate_of(rep: crate::runtime::ScreenBitrateReport) -> ScreenBitrate {
    ScreenBitrate {
        bitrate_bps: rep.bitrate_bps,
        bitrate_kbps: rep.bitrate_kbps,
        min_bps: rep.min_bps,
        max_bps: rep.max_bps,
        timestamp_ms: rep.timestamp_ms,
    }
}

pub fn screen_color_of(rep: tetherly_core::screen::ScreenColorReport) -> ScreenColor {
    ScreenColor {
        mode: wire_enum(&rep.mode),
        threshold: rep.threshold,
        is_active: rep.is_active,
        update_count: rep.update_count,
    }
}

pub fn screen_delta_of(rep: tetherly_core::screen::ScreenDeltaReport) -> ScreenDelta {
    ScreenDelta {
        enabled: rep.enabled,
        keyframe_interval: rep.keyframe_interval,
        delta_frames_sent: rep.delta_frames_sent,
        keyframes_sent: rep.keyframes_sent,
        delta_frames_received: rep.delta_frames_received,
        keyframes_received: rep.keyframes_received,
        saved_bytes: rep.saved_bytes,
    }
}

pub fn screen_quality_metric_of(
    rep: tetherly_core::screen::ScreenQualityMetricReport,
) -> ScreenQualityMetric {
    ScreenQualityMetric {
        enabled: rep.enabled,
        sample_interval: rep.sample_interval,
        evaluated_frames: rep.evaluated_frames,
        avg_mse: rep.avg_mse,
        avg_psnr_db: rep.avg_psnr_db,
        avg_ssim: rep.avg_ssim,
        last_psnr_db: rep.last_psnr_db,
        last_ssim: rep.last_ssim,
        quality_warning: rep.quality_warning,
    }
}

pub fn screen_qoe_of(rep: tetherly_core::screen::ScreenQoeReport) -> ScreenQoe {
    ScreenQoe {
        mode: rep.mode,
        target_psnr_db: rep.target_psnr_db,
        target_ssim_pct: rep.target_ssim_pct,
        actions_total: rep.actions_total,
        last_action: rep.last_action,
        last_action_reason: rep.last_action_reason,
        in_cooldown: rep.in_cooldown,
        active_boosts: rep.active_boosts,
    }
}

pub fn screen_motion_of(rep: tetherly_core::screen::ScreenMotionReport) -> ScreenMotion {
    ScreenMotion {
        enabled: rep.enabled,
        max_displacement: rep.max_displacement,
        motion_frames_sent: rep.motion_frames_sent,
        fallback_frames_sent: rep.fallback_frames_sent,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_roi_of(rep: tetherly_core::screen::ScreenRoiReport) -> ScreenRoi {
    ScreenRoi {
        active: rep.active,
        x: rep.x,
        y: rep.y,
        w: rep.w,
        h: rep.h,
        zoom_percent: rep.zoom_percent,
        source_w: rep.source_w,
        source_h: rep.source_h,
        effective_w: rep.effective_w,
        effective_h: rep.effective_h,
        update_count: rep.update_count,
    }
}

pub fn screen_watermark_of(rep: tetherly_core::screen::ScreenWatermarkReport) -> ScreenWatermark {
    ScreenWatermark {
        enabled: rep.enabled,
        host_enforced: rep.host_enforced,
        text_template: rep.text_template,
        rendered_text: rep.rendered_text,
        opacity: rep.opacity,
        opacity_pct: rep.opacity_pct,
        color_rgb: rep.color_rgb,
        color_hex: rep.color_hex,
        scale: rep.scale,
        pattern: rep.pattern,
        spacing_x: rep.spacing_x,
        spacing_y: rep.spacing_y,
        frames_watermarked: rep.frames_watermarked,
    }
}

pub fn screen_annotations_of(
    rep: tetherly_core::screen::ScreenAnnotationReport,
) -> ScreenAnnotations {
    ScreenAnnotations {
        annotations_count: rep.annotations_count,
        annotations: rep
            .annotations
            .into_iter()
            .map(annotation_item_dto)
            .collect(),
        laser_active: rep.laser_active,
        laser: rep.laser.map(laser_pointer_dto_of),
        total_annotations_added: rep.total_annotations_added,
        laser_updates_total: rep.laser_updates_total,
        timestamp_ms: rep.timestamp_ms,
        whiteboard_policy: whiteboard_policy_dto_of(rep.whiteboard_policy),
        redo_count: rep.redo_count,
    }
}

pub fn laser_pointer_dto_of(rep: tetherly_core::screen::LaserPointerState) -> LaserPointerDto {
    LaserPointerDto {
        x: rep.x,
        y: rep.y,
        active: rep.active,
        color_rgb: rep.color_rgb,
        radius: rep.radius,
    }
}

pub fn whiteboard_policy_dto_of(
    rep: tetherly_core::screen::WhiteboardPolicy,
) -> WhiteboardPolicyDto {
    WhiteboardPolicyDto {
        read_only: rep.read_only,
        allow_peer_clear: rep.allow_peer_clear,
        max_items: rep.max_items,
    }
}

pub fn whiteboard_snapshot_dto_of(
    rep: tetherly_core::screen::WhiteboardSnapshot,
) -> WhiteboardSnapshotDto {
    WhiteboardSnapshotDto {
        version: rep.version,
        created_at_ms: rep.created_at_ms,
        session_id: rep.session_id,
        active_items: rep
            .active_items
            .into_iter()
            .map(annotation_item_dto)
            .collect(),
        total_added_counter: rep.total_added_counter,
    }
}

pub fn screen_tile_of(rep: tetherly_core::screen::ScreenTileReport) -> ScreenTile {
    ScreenTile {
        enabled: rep.enabled,
        tile_size: rep.tile_size,
        grid_frames_sent: rep.grid_frames_sent,
        dirty_tiles_sent: rep.dirty_tiles_sent,
        skipped_tiles_count: rep.skipped_tiles_count,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_tile_cache_of(rep: tetherly_core::screen::ScreenTileCacheReport) -> ScreenTileCache {
    ScreenTileCache {
        enabled: rep.enabled,
        max_entries: rep.max_entries,
        current_entries: rep.current_entries,
        cache_hits: rep.cache_hits,
        cache_misses: rep.cache_misses,
        hit_ratio_pct: rep.hit_ratio_pct,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_focus_of(rep: tetherly_core::screen::ScreenFocusReport) -> ScreenFocus {
    ScreenFocus {
        enabled: rep.enabled,
        mode: wire_enum(&rep.mode),
        radius: rep.radius,
        peripheral_skip_ratio: rep.peripheral_skip_ratio,
        box_x: rep.box_x,
        box_y: rep.box_y,
        box_w: rep.box_w,
        box_h: rep.box_h,
        focus_tiles_sent: rep.focus_tiles_sent,
        peripheral_tiles_sent: rep.peripheral_tiles_sent,
        peripheral_tiles_skipped: rep.peripheral_tiles_skipped,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_tile_palette_of(
    rep: tetherly_core::screen::ScreenTilePaletteReport,
) -> ScreenTilePalette {
    ScreenTilePalette {
        enabled: rep.enabled,
        max_colors: rep.max_colors,
        enable_rle: rep.enable_rle,
        solid_tiles_count: rep.solid_tiles_count,
        bilevel_tiles_count: rep.bilevel_tiles_count,
        palette_tiles_count: rep.palette_tiles_count,
        truecolor_tiles_count: rep.truecolor_tiles_count,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_tile_predict_of(
    rep: tetherly_core::screen::ScreenTilePredictReport,
) -> ScreenTilePredict {
    ScreenTilePredict {
        enabled: rep.enabled,
        filter_mode: wire_enum(&rep.filter_mode),
        enable_rct: rep.enable_rct,
        predict_tiles_count: rep.predict_tiles_count,
        saved_bandwidth_bytes: rep.saved_bandwidth_bytes,
    }
}

pub fn screen_tile_present_of(
    rep: tetherly_core::screen::ScreenTilePresentReport,
) -> ScreenTilePresent {
    ScreenTilePresent {
        enabled: rep.enabled,
        vsync_mode: rep.vsync_mode,
        double_buffer: rep.double_buffer,
        tear_prevention: rep.tear_prevention,
        committed_transactions: rep.committed_transactions,
        swapped_frames: rep.swapped_frames,
        torn_frames_prevented: rep.torn_frames_prevented,
        avg_swap_latency_us: rep.avg_swap_latency_us,
    }
}

pub fn screen_tile_nack_of(rep: tetherly_core::screen::ScreenTileNackReport) -> ScreenTileNack {
    ScreenTileNack {
        enabled: rep.enabled,
        timeout_ms: rep.timeout_ms,
        max_retries: rep.max_retries,
        nack_requests_sent: rep.nack_requests_sent,
        tiles_retransmitted: rep.tiles_retransmitted,
        degraded_commits: rep.degraded_commits,
    }
}

pub fn screen_tile_saliency_of(
    rep: tetherly_core::screen::ScreenTileSaliencyReport,
) -> ScreenTileSaliency {
    ScreenTileSaliency {
        enabled: rep.enabled,
        high_threshold: rep.high_threshold,
        low_threshold: rep.low_threshold,
        drop_background_on_congestion: rep.drop_background_on_congestion,
        critical_tiles_sent: rep.critical_tiles_sent,
        normal_tiles_sent: rep.normal_tiles_sent,
        background_tiles_sent: rep.background_tiles_sent,
        background_tiles_skipped: rep.background_tiles_skipped,
    }
}

pub fn screen_tile_lod_of(rep: tetherly_core::screen::ScreenTileLodReport) -> ScreenTileLod {
    ScreenTileLod {
        enabled: rep.enabled,
        max_lod: rep.max_lod,
        auto_lod_on_congestion: rep.auto_lod_on_congestion,
        lod0_tiles_sent: rep.lod0_tiles_sent,
        lod1_tiles_sent: rep.lod1_tiles_sent,
        lod2_tiles_sent: rep.lod2_tiles_sent,
        bytes_saved: rep.bytes_saved,
    }
}

pub fn screen_tile_batch_of(rep: tetherly_core::screen::ScreenTileBatchReport) -> ScreenTileBatch {
    ScreenTileBatch {
        enabled: rep.enabled,
        target_chunk_bytes: rep.target_chunk_bytes,
        pacing_interval_us: rep.pacing_interval_us,
        batches_sent: rep.batches_sent,
        chunks_sent: rep.chunks_sent,
        avg_chunk_bytes: rep.avg_chunk_bytes,
        pacing_pauses: rep.pacing_pauses,
    }
}

pub fn screen_tile_motion_of(
    rep: tetherly_core::screen::ScreenTileMotionReport,
) -> ScreenTileMotion {
    ScreenTileMotion {
        enabled: rep.enabled,
        max_displacement: rep.max_displacement,
        min_match_ratio: rep.min_match_ratio,
        motion_tiles_sent: rep.motion_tiles_sent,
        fallback_tiles_sent: rep.fallback_tiles_sent,
        bytes_saved: rep.bytes_saved,
    }
}

pub fn annotation_item_dto(a: tetherly_core::screen::AnnotationItem) -> AnnotationItemDto {
    AnnotationItemDto {
        id: a.id,
        author: a.author,
        kind: annotation_kind_dto(&a.kind),
        color_rgb: a.color_rgb,
        thickness: a.thickness,
        alpha: a.alpha,
        ttl_ms: a.ttl_ms,
        created_at_ms: a.created_at_ms,
    }
}

/// Inverse mapper: contract `AnnotationKindDto` -> runtime core `AnnotationKind`.
fn annotation_kind_runtime(k: AnnotationKindDto) -> tetherly_core::screen::AnnotationKind {
    match k {
        AnnotationKindDto::Stroke { points } => {
            tetherly_core::screen::AnnotationKind::Stroke { points }
        }
        AnnotationKindDto::Rect { x, y, w, h } => {
            tetherly_core::screen::AnnotationKind::Rect { x, y, w, h }
        }
        AnnotationKindDto::Arrow { from, to } => {
            tetherly_core::screen::AnnotationKind::Arrow { from, to }
        }
        AnnotationKindDto::Text { x, y, text, scale } => {
            tetherly_core::screen::AnnotationKind::Text { x, y, text, scale }
        }
    }
}

/// Inverse mapper: contract `AnnotationItemDto` -> runtime core `AnnotationItem`.
pub fn annotation_item_runtime(dto: AnnotationItemDto) -> tetherly_core::AnnotationItem {
    tetherly_core::AnnotationItem {
        id: dto.id,
        author: dto.author,
        kind: annotation_kind_runtime(dto.kind),
        color_rgb: dto.color_rgb,
        thickness: dto.thickness,
        alpha: dto.alpha,
        ttl_ms: dto.ttl_ms,
        created_at_ms: dto.created_at_ms,
    }
}

/// Inverse mapper: contract `WhiteboardSnapshotDto` -> runtime core `WhiteboardSnapshot`.
pub fn whiteboard_snapshot_runtime(
    dto: WhiteboardSnapshotDto,
) -> tetherly_core::WhiteboardSnapshot {
    tetherly_core::WhiteboardSnapshot {
        version: dto.version,
        created_at_ms: dto.created_at_ms,
        session_id: dto.session_id,
        active_items: dto
            .active_items
            .into_iter()
            .map(annotation_item_runtime)
            .collect(),
        total_added_counter: dto.total_added_counter,
    }
}

pub fn annotation_kind_dto(k: &tetherly_core::screen::AnnotationKind) -> AnnotationKindDto {
    match k {
        tetherly_core::screen::AnnotationKind::Stroke { points } => AnnotationKindDto::Stroke {
            points: points.clone(),
        },
        tetherly_core::screen::AnnotationKind::Rect { x, y, w, h } => AnnotationKindDto::Rect {
            x: *x,
            y: *y,
            w: *w,
            h: *h,
        },
        tetherly_core::screen::AnnotationKind::Arrow { from, to } => AnnotationKindDto::Arrow {
            from: *from,
            to: *to,
        },
        tetherly_core::screen::AnnotationKind::Text { x, y, text, scale } => {
            AnnotationKindDto::Text {
                x: *x,
                y: *y,
                text: text.clone(),
                scale: *scale,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tetherly_core::DeviceId;

    fn id(n: u8) -> DeviceId {
        DeviceId::from_id_pk(&[n; 32])
    }

    fn live(n: u8) -> LivePeer {
        LivePeer {
            device_id: id(n),
            name: format!("live{n}"),
            platform: "macos".into(),
            addr: "10.0.0.2:4100".parse::<SocketAddr>().unwrap(),
            handshake_hash: [0; 32],
            file_port: None,
            input_port: None,
            screen_port: None,
            caps: vec!["input".into()],
            path: PathKind::Overlay,
            rtt_ms: Some(7),
            last_seen_ms: 99,
        }
    }

    fn trusted(n: u8, alias: &str, revoked: bool) -> TrustedPeer {
        TrustedPeer {
            device_id: id(n),
            id_pk: [n; 32],
            n_pk: [0; 32],
            alias: alias.into(),
            paired_at: 1,
            revoked,
        }
    }

    fn discovered(n: u8) -> DiscoveredPeerDto {
        DiscoveredPeerDto {
            device_id: id(n).to_string(),
            name: format!("mdns{n}"),
            addrs: vec!["10.0.0.9".into()],
            port: 4100,
            fp: None,
            caps: vec!["clip".into()],
            is_trusted: false,
            is_online: false,
            last_seen_ms: 55,
        }
    }

    fn blocked(n: u8) -> BlockedPeer {
        BlockedPeer {
            device_id: id(n).to_string(),
            reason: None,
            blocked_at: 3,
        }
    }

    fn profile(_: &str) -> (String, Vec<String>, Option<String>) {
        ("laptop".into(), vec!["home".into()], Some("studio".into()))
    }

    fn find(devs: &[Device], n: u8) -> &Device {
        let want = id(n).to_string();
        devs.iter().find(|d| d.id == want).expect("device present")
    }

    #[test]
    fn live_trusted_device_carries_link_and_alias() {
        let devs = merge(
            vec![live(1)],
            vec![trusted(1, "Desk", false)],
            vec![discovered(1)],
            vec![],
            profile,
        );
        assert_eq!(devs.len(), 1);
        let d = find(&devs, 1);
        assert_eq!(d.name, "live1");
        assert_eq!(d.alias.as_deref(), Some("Desk"));
        assert!(d.online && d.trusted && !d.blocked);
        assert_eq!(d.path, Some(LinkPath::Overlay));
        assert_eq!(d.addr.as_deref(), Some("10.0.0.2:4100"));
        assert_eq!(d.rtt_ms, Some(7));
        assert_eq!(d.last_seen_ms, Some(99));
        assert_eq!(d.caps, vec!["input".to_string()]);
        assert_eq!(d.device_type, "laptop");
        assert_eq!(d.tags, vec!["home".to_string()]);
        assert_eq!(d.group.as_deref(), Some("studio"));
        assert_eq!(d.platform.as_deref(), Some("macos"));
    }

    #[test]
    fn offline_devices_fall_back_to_discovery_then_alias_then_id() {
        let devs = merge(
            vec![],
            vec![trusted(2, "Laptop", false), trusted(3, "Tablet", false)],
            vec![discovered(2)],
            vec![blocked(4)],
            profile,
        );
        let d2 = find(&devs, 2);
        assert_eq!(d2.name, "mdns2");
        assert!(!d2.online && d2.trusted);
        assert_eq!(d2.path, None);
        assert_eq!(d2.last_seen_ms, Some(55));
        assert_eq!(d2.caps, vec!["clip".to_string()]);
        assert_eq!(find(&devs, 3).name, "Tablet");
        assert_eq!(find(&devs, 3).last_seen_ms, None);
        let d4 = find(&devs, 4);
        assert_eq!(d4.name, d4.id);
        assert!(d4.blocked && !d4.trusted);
    }

    #[test]
    fn revoked_trust_is_not_trusted_and_empty_alias_is_none() {
        let devs = merge(vec![], vec![trusted(5, "", true)], vec![], vec![], profile);
        let d = find(&devs, 5);
        assert!(!d.trusted);
        assert_eq!(d.alias, None);
    }

    #[test]
    fn devices_are_unique_and_sorted_by_id() {
        let devs = merge(
            vec![live(9), live(1)],
            vec![trusted(1, "a", false), trusted(9, "b", false)],
            vec![discovered(9)],
            vec![blocked(1)],
            profile,
        );
        let ids: Vec<&str> = devs.iter().map(|d| d.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert_eq!(ids.len(), 2);
        assert!(find(&devs, 1).blocked);
    }
}
