// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Topology layout, links and peers.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_topology_layout, save_topology_policy, TopologyPolicy};
use std::collections::HashMap;
use tetherly_core::{
    CoreError, DeviceId, DisplayEdge, DisplayRect, EdgeCrossingConfig, EdgeNeighbor,
    TopologyStateFrame, TopologySyncReqFrame, TopologyValidator,
};
use tracing::debug;

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

impl Node {
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
}
