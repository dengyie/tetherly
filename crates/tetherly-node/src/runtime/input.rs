// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Input-tunnel policy, sessions and cursor crossing.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_input_policy, DeviceType, InputTunnelPolicy};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tetherly_core::frame::InnerFrame;
use tetherly_core::{
    map_edge_exit_to_entry, CoreError, CursorSeat, DeviceId, DisplayEdge, DisplayRect, InputClient,
    InputEvent, InputTunnelHeartbeatFrame, InputTunnelReleaseFrame, InputTunnelRequestFrame,
    InputTunnelRespFrame, InputTunnelRole, InputTunnelValidator, MemorySink, TopologyValidator,
};
use tetherly_net::session::{accept_session, dial_session, ActiveSession};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::warn;

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

impl Node {
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

    pub fn input_port(&self) -> u16 {
        self.shared.bound_input.load(Ordering::SeqCst)
    }

    pub fn input_addr(&self) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], self.input_port()))
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

    pub(super) async fn send_input_events(&self, events: Vec<InputEvent>) -> Result<(), CoreError> {
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

    pub(super) async fn accept_input(
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

    pub(super) async fn attach_input_server(&self, mut sess: ActiveSession) {
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

    pub(super) async fn attach_input_client(&self, mut sess: ActiveSession) {
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
                                        node.screen_force_wake_squelch();
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
}
