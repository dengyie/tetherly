// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN discovery, candidate inbox, peer probing, heartbeat and link quality.

use super::*;
use crate::events::{CandidateViewDto, UiEvent};
use crate::lan::lan_ips;
use crate::platform::insert_foreground;
use crate::store::remember_route;
use crate::telemetry::TrafficReport;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tetherly_core::frame::{ping, CapsUpdate};
use tetherly_core::session::backoff_delay;
use tetherly_core::{CoreError, DeviceId, PathKind};
use tetherly_net::session::dial_session;
use tokio::net::TcpStream;
use tokio::sync::{oneshot, watch};
use tracing::{debug, info, warn};

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

impl Node {
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

    pub(super) fn spawn_discovery_browser(&self) {
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

    pub(super) fn advertise_mdns(&self) {
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

    pub(super) fn spawn_reconnect(&self) {
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
}
