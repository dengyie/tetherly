// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node state -> v1 DTOs. The device merge is pure over its four sources so
//! the precedence rules are unit-tested without a running node.

use crate::runtime::{extract_features, DiscoveredPeerDto, LivePeer};
use crate::store::BlockedPeer;
use crate::Node;
use std::collections::BTreeMap;
use tetherly_api::{
    Activity, ActivityCandidate, ActivityHandoff, ActivityMessage, ActivityReply, AuditClearReport,
    AuditEntryDto, AuditTrail, ChannelTraffic, ClipClearReport, ClipContent, ClipHistory,
    ClipHistoryEntry, ClipPolicyDto, ClipPollReport, ClipStatus, Device, FileBatchItem,
    FileBatchReport, FileDecision, FileDecisionReport, FileDispatchItem, FileDispatchReport,
    FileOffer, FileOffers, HandoffStatus, HeartbeatConfig, Inbox, InboxFile, Input, Layout,
    LayoutEdge, LayoutPolicy, LinkPath, LinkQuality, LinkQualityPeer, LocalCaps, MessageDirection,
    Outbox, OutboxClearReport, OutboxEntryDto, OutboxResendReport, Pairing, PeerCaps, PeerCounts,
    PeerProfileDto, PeerProfiles, Ports, ProtocolMessages, ResumeMarker, ResumeMarkers, Settings,
    SettingsGeneral, SettingsTransfer, Snapshot, Storage, StorageCleanReport, System,
    TrafficTelemetry, Transfer, TransferDirection, TransferPhase, TransferStats, Transfers,
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
