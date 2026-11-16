// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node state -> v1 DTOs. The device merge is pure over its four sources so
//! the precedence rules are unit-tested without a running node.

use crate::runtime::{extract_features, DiscoveredPeerDto, LivePeer};
use crate::store::BlockedPeer;
use crate::Node;
use std::collections::BTreeMap;
use tetherly_api::{
    Activity, ActivityCandidate, ActivityHandoff, ActivityMessage, ActivityReply, Device,
    HandoffStatus, Inbox, InboxFile, Input, Layout, LayoutEdge, LayoutPolicy, LinkPath,
    MessageDirection, Pairing, PeerCounts, Ports, Settings, SettingsGeneral, SettingsTransfer,
    Snapshot, System, Transfer, TransferDirection, TransferPhase, Transfers, API_VERSION,
};
use tetherly_core::{DisplayEdge, PathKind, TrustedPeer};

#[derive(Default)]
struct Parts {
    live: Option<LivePeer>,
    trusted: Option<TrustedPeer>,
    discovered: Option<DiscoveredPeerDto>,
    blocked: bool,
}

/// Merge every source into one device per id, sorted by id. `profile`
/// returns `(device_type, tags)` for an id.
pub fn merge(
    live: Vec<LivePeer>,
    trusted: Vec<TrustedPeer>,
    discovered: Vec<DiscoveredPeerDto>,
    blocked: Vec<BlockedPeer>,
    profile: impl Fn(&str) -> (String, Vec<String>),
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
        parts.entry(b.device_id).or_default().blocked = true;
    }
    parts
        .into_iter()
        .map(|(id, p)| build(id, p, &profile))
        .collect()
}

fn build(id: String, p: Parts, profile: &impl Fn(&str) -> (String, Vec<String>)) -> Device {
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
    let (device_type, tags) = profile(&id);
    Device {
        name,
        alias,
        platform: p.live.as_ref().map(|l| l.platform.clone()),
        device_type,
        online: p.live.is_some(),
        trusted: p.trusted.as_ref().is_some_and(|t| !t.revoked),
        blocked: p.blocked,
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

fn profile_of(node: &Node, id: &str) -> (String, Vec<String>) {
    match tetherly_core::DeviceId::parse(id) {
        Ok(did) => {
            let p = node.peer_profile(&did);
            (p.device_type.as_str().to_string(), p.tags)
        }
        Err(_) => (
            crate::store::DeviceType::default().as_str().to_string(),
            Vec::new(),
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
    Transfers {
        active_count: r.active_count as u64,
        terminal_count: r.terminal_count as u64,
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

/// The inbox resource: files waiting in `data_dir/inbox`.
pub fn inbox(node: &Node) -> Inbox {
    let r = node.inbox_report(None, None);
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

    fn profile(_: &str) -> (String, Vec<String>) {
        ("laptop".into(), vec!["home".into()])
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
