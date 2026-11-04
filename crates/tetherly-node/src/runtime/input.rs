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
    InputEvent, InputSink, InputTunnelHeartbeatFrame, InputTunnelReleaseFrame,
    InputTunnelRequestFrame, InputTunnelRespFrame, InputTunnelRole, InputTunnelValidator,
    MemorySink, TopologyValidator,
};
use tetherly_net::session::{accept_session, dial_session, ActiveSession};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};
/// Monotonic generation counter for outbound input channels.
static INPUT_CHANNEL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct InputTunnelStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveInputTunnelSession>,
    pub policy: InputTunnelPolicy,
    #[serde(default)]
    pub accessibility_trusted: bool,
    #[serde(default)]
    pub capture_active: bool,
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel_speed: Option<f64>,
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

/// Composite sink combining in-memory CI telemetry snapshotting with real OS injection.
/// All input operations (including release_all_inputs flushes on Leave, Enter, boundary return,
/// and peer disconnect) are executed synchronously against both the memory observer and the
/// physical platform driver.
#[derive(Clone)]
struct DualInputSink {
    memory: MemorySink,
    real: Option<Arc<Mutex<dyn InputSink>>>,
}

impl DualInputSink {
    fn new(real: Option<Arc<Mutex<dyn InputSink>>>) -> Self {
        Self {
            memory: MemorySink::default(),
            real,
        }
    }

    fn memory(&self) -> &MemorySink {
        &self.memory
    }
}

impl InputSink for DualInputSink {
    fn move_abs(&mut self, x: i32, y: i32) {
        self.memory.move_abs(x, y);
        if let Some(r) = &self.real {
            if let Ok(mut sink) = r.lock() {
                sink.move_abs(x, y);
            }
        }
    }

    fn button(&mut self, mask: u8, down: bool) {
        self.memory.button(mask, down);
        if let Some(r) = &self.real {
            if let Ok(mut sink) = r.lock() {
                sink.button(mask, down);
            }
        }
    }

    fn wheel(&mut self, dx: i16, dy: i16) {
        self.memory.wheel(dx, dy);
        if let Some(r) = &self.real {
            if let Ok(mut sink) = r.lock() {
                sink.wheel(dx, dy);
            }
        }
    }

    fn key(&mut self, code: u16, down: bool, mods: u8) {
        self.memory.key(code, down, mods);
        if let Some(r) = &self.real {
            if let Ok(mut sink) = r.lock() {
                sink.key(code, down, mods);
            }
        }
    }
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

        // Polite takeover: if this node is currently the CONTROLLER of another
        // input tunnel, the incoming request hands control over to the peer.
        // Demote ourselves first — otherwise both machines keep an open
        // outbound channel and both cursors are driven simultaneously (two
        // controllers fighting), which is unreachable-to-debug drift and is
        // exactly what broke reverse (Windows -> Mac) control.
        if self.active_tunnel_role() == Some(InputTunnelRole::Controller) {
            info!(peer = %peer_str, "input tunnel takeover: releasing local controller session");
            let _ = self.input_stop().await;
            self.record_audit(
                AuditCategory::Config,
                AuditSeverity::Info,
                "input_tunnel_takeover_released",
                Some(peer_str.clone()),
                "Released local controller session: peer took over as controller".to_string(),
                true,
            );
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

        // Reset any leftover arming/dwell state from an earlier session.
        self.reset_reverse_arming_state();

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
            let _ = self.ensure_input_capture();
            if !self.input_channel_open().await {
                if let Err(e) = self.open_input(peer).await {
                    warn!(%peer, %e, "input channel open during tunnel accept response failed");
                }
            }

            // A tunnel handshake establishes network connectivity, NOT cursor boundary crossing.
            // The cursor seat MUST remain Local until the user physically moves past the screen edge.
            *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
            self.emit(UiEvent::input_seat_changed(
                "local",
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
            if policy.idle_timeout_secs == 0 {
                false
            } else {
                let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
                now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
            }
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
                let accessibility_trusted = crate::platform::is_accessibility_trusted();
                let capture_active = self.shared.input_capture_active.load(Ordering::SeqCst);
                return InputTunnelStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    accessibility_trusted,
                    capture_active,
                    timestamp_ms: now_ms,
                };
            }
        }

        let accessibility_trusted = crate::platform::is_accessibility_trusted();
        let capture_active = self.shared.input_capture_active.load(Ordering::SeqCst);
        if accessibility_trusted && !capture_active && (guard.is_some() || policy.edge.is_some()) {
            let _ = self.ensure_input_capture();
        }
        let capture_active = self.shared.input_capture_active.load(Ordering::SeqCst);

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        InputTunnelStatusReport {
            ok: true,
            active,
            session,
            policy,
            accessibility_trusted,
            capture_active,
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

        let effective_edge: tetherly_core::ScreenEdge = if let Some(e) = edge {
            e.into()
        } else if let Some(e) = self.input_policy().edge.as_deref() {
            e.parse::<tetherly_core::ScreenEdge>()
                .unwrap_or(tetherly_core::ScreenEdge::Right)
        } else {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            layout
                .neighbors
                .iter()
                .find_map(|(de, n)| {
                    if n.peer_id == peer_str {
                        Some((*de).into())
                    } else {
                        None
                    }
                })
                .unwrap_or(tetherly_core::ScreenEdge::Right)
        };
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .set_edge(effective_edge);

        let _ = self.ensure_input_capture();
        // Never reuse an existing outbound channel across an attach: on a
        // takeover the demoted peer tears its side of the old connection
        // down (input_stop), so the previous channel is dying by design —
        // reusing it would leave this controller with a dead channel and no
        // rebuild. Drop it and dial fresh.
        *self.shared.input_out.lock().expect("input_out") = None;
        if let Err(e) = self.open_input(&peer_id).await {
            warn!(%peer_id, %e, "input channel open during tunnel attach failed");
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
            self.disarm_input_capture();
            self.reset_reverse_arming_state();
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
        // A keymap switch reinterprets every subsequent key event. If a
        // modifier is currently held under the old mapping, its release
        // would be remapped differently and the remote key stays stuck
        // until the next seat change; a Leave+Enter cycle flushes all held
        // inputs on the client and re-enters at the same position.
        let keymap_changed = self.input_policy().keymap != policy.keymap;
        save_input_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save input_policy.json: {e}")))?;
        *self.shared.input_policy.lock().expect("input_policy") = policy.clone();
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .set_edge(policy.resolved_edge());

        if keymap_changed && self.input_channel_open_sync() {
            let events = {
                let mut engine = self.shared.input_engine.lock().expect("engine");
                engine.refresh_remote_session();
                engine.drain()
            };
            if let Err(e) = self.send_input_events(events) {
                tracing::warn!(%e, "keymap change: failed to flush remote session");
            }
        }

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
            policy.keymap.clone(),
            policy.edge.clone(),
            policy.wheel_speed,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "input_tunnel_policy_changed",
            None::<String>,
            format!(
                "Updated input policy: enabled={}, allow_inbound={}, keyboard={}, mouse={}, timeout={}s, keymap={:?}, edge={:?}, wheel_speed={:.2}",
                policy.enabled,
                policy.allow_inbound_control,
                policy.allow_keyboard,
                policy.allow_mouse,
                policy.idle_timeout_secs,
                policy.keymap,
                policy.edge,
                policy.wheel_speed
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
        if let Some(v) = update.keymap {
            policy.keymap = Some(v);
        }
        if let Some(v) = update.edge {
            policy.edge = Some(v);
        }
        if let Some(v) = update.wheel_speed {
            if v > 0.0 && v <= 20.0 {
                policy.wheel_speed = v;
            }
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
        // Single dialer: concurrent callers (link watcher tick, takeover
        // attach, UI) must not open competing connections to the same peer —
        // the loser's session becomes a zombie and its eventual teardown
        // flips `input_out`/seat events while the winner is serving. The
        // caller that lost the race returns Ok: the in-flight dial will
        // establish the channel, and a later tick re-checks.
        // A healthy channel to a peer makes another dial pure churn: the
        // new session would orphan the old one. Callers that need a REBUILD
        // clear `input_out` first (see input_tunnel_attach).
        if self.input_channel_open_sync() {
            return Ok(());
        }
        if self
            .shared
            .input_dialing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(());
        }
        struct ResetDialing<'a>(&'a std::sync::atomic::AtomicBool);
        impl Drop for ResetDialing<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _reset_dialing = ResetDialing(&self.shared.input_dialing);

        let addr = {
            let live = self.shared.live.lock().expect("live");
            let p = live
                .get(peer)
                .ok_or(tetherly_net::NetError::InputRequiresTrust)?;
            let port = p.input_port.ok_or(tetherly_net::NetError::Handshake)?;
            SocketAddr::new(p.addr.ip(), port)
        };
        let stream = TcpStream::connect(addr).await?;
        let _ = stream.set_nodelay(true);
        let mut cfg = self.session_config_inner(true);
        cfg.pin = None;
        let sess = match dial_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        self.attach_input_server(sess);
        Ok(())
    }

    /// True while an input control channel to a peer is attached.
    #[inline]
    pub fn input_channel_open_sync(&self) -> bool {
        self.shared.input_out.lock().expect("input_out").is_some()
    }

    /// True while an input control channel to a peer is attached. The UI uses
    /// this to dial the channel on demand instead of tracking sessions.
    pub async fn input_channel_open(&self) -> bool {
        self.input_channel_open_sync()
    }

    /// Controller: stop forwarding local input to the remote seat and release
    /// the channel. The capture hooks stay armed (they are cheap while local).
    pub async fn input_stop(&self) -> Result<(), CoreError> {
        *self
            .shared
            .input_reconnect_deadline
            .lock()
            .expect("deadline") = None;
        self.shared
            .input_buffered_frames
            .lock()
            .expect("buffered")
            .clear();
        *self.shared.input_out.lock().expect("input_out") = None;
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

    /// Arm low-level input capture: enable keyboard swallowing, disassociate hardware mouse, and hide local cursor.
    pub fn arm_input_capture(&self) {
        if let Ok(guard) = self.shared.input_hook_control.lock() {
            if let Some(ref control) = *guard {
                let _ = control.set_keyboard(true);
            }
        }
    }

    /// Check if low-level keyboard hook is currently armed and actively swallowing keystrokes.
    pub fn is_keyboard_armed(&self) -> bool {
        if let Ok(guard) = self.shared.input_hook_control.lock() {
            if let Some(ref control) = *guard {
                return control.is_armed();
            }
        }
        false
    }

    /// Disarm low-level input capture: disable keyboard swallowing and re-associate mouse cursor.
    /// This is the master safety valve ensuring the local machine's input is never trapped or locked.
    pub fn disarm_input_capture(&self) {
        if let Ok(guard) = self.shared.input_hook_control.lock() {
            if let Some(ref control) = *guard {
                let _ = control.set_keyboard(false);
            }
        }
    }

    /// Start the controller-side capture: low-level mouse hook (edge
    /// detection) and a keyboard hook armed only while the seat is remote.
    /// Events feed the same edge state machine `drive_input` uses, so CI and
    /// production share one path.
    #[cfg(any(windows, target_os = "macos"))]
    pub fn spawn_input_capture(&self) -> Result<(), CoreError> {
        let (tx, rx) = std::sync::mpsc::channel::<crate::platform::RawInput>();
        let control = crate::platform::start_input_hooks(tx)?;
        // Move, never clone: HookControl is Copy on Windows (clippy
        // clone_on_copy) and Clone-only on macOS; `control` is not used again.
        *self.shared.input_hook_control.lock().expect("hook_control") = Some(control);
        let (w, h) = crate::platform::screen_size();
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .resize(w.max(1), h.max(1));
        let node = self.clone_handle();
        std::thread::spawn(move || {
            while let Ok(ev) = rx.recv() {
                // A failed send means the remote input channel is not open (yet)
                // or just closed. That is transient, not fatal: keep the hook
                // alive so a later `open_input` starts delivering again, rather
                // than wedging input sharing until process restart. Events are
                // paced by real user input, so retrying cannot busy-spin.
                let mut cur_ev = ev;
                loop {
                    match cur_ev {
                        crate::platform::RawInput::Move {
                            mut x,
                            mut y,
                            mut dx,
                            mut dy,
                        } => {
                            // Coalesce backlogged consecutive Move frames to eliminate queue delay
                            let mut next_non_move = None;
                            while let Ok(next) = rx.try_recv() {
                                match next {
                                    crate::platform::RawInput::Move {
                                        x: nx,
                                        y: ny,
                                        dx: ndx,
                                        dy: ndy,
                                    } => {
                                        x = nx;
                                        y = ny;
                                        dx += ndx;
                                        dy += ndy;
                                    }
                                    other => {
                                        next_non_move = Some(other);
                                        break;
                                    }
                                }
                            }
                            let res = node.drive_input_delta(x, y, dx, dy, w, h);
                            if let Err(e) = res {
                                if node.input_engine_seat() == CursorSeat::Remote {
                                    tracing::warn!(%e, "network error while in remote seat; emergency rescue to local");
                                    let (events, (cx, cy)) = {
                                        let mut server =
                                            node.shared.input_engine.lock().expect("engine");
                                        server.force_local();
                                        (server.drain(), server.local_pos())
                                    };
                                    let _ = node.send_input_events(events);
                                    *node.shared.input_seat.lock().expect("seat") =
                                        Some(CursorSeat::Local);
                                    node.disarm_input_capture();
                                    crate::platform::warp_mouse_position(cx, cy);
                                    node.emit(UiEvent::input_seat_changed("local", false, None));
                                }
                            }
                            if let Some(other) = next_non_move {
                                cur_ev = other;
                                continue;
                            }
                            break;
                        }
                        crate::platform::RawInput::Button { mask, down } => {
                            let _ = node.drive_input_button(mask, down);
                            break;
                        }
                        crate::platform::RawInput::Wheel { dx, dy } => {
                            let _ = node.drive_input_wheel(dx, dy);
                            break;
                        }
                        crate::platform::RawInput::Key { code, down } => {
                            let _ = node.drive_input_key(code, down);
                            break;
                        }
                        crate::platform::RawInput::EmergencyEscape => {
                            *node
                                .shared
                                .input_reconnect_deadline
                                .lock()
                                .expect("deadline") = None;
                            node.shared
                                .input_buffered_frames
                                .lock()
                                .expect("buffered")
                                .clear();
                            let (events, (cx, cy)) = {
                                let mut server = node.shared.input_engine.lock().expect("engine");
                                server.force_local();
                                (server.drain(), server.local_pos())
                            };
                            let _ = node.send_input_events(events);
                            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                            node.disarm_input_capture();
                            crate::platform::warp_mouse_position(cx, cy);
                            node.emit(UiEvent::input_seat_changed("local", true, None));
                            crate::platform::show_system_toast(
                                "Tetherly",
                                "已安全返回 Mac 本地控制 (双击 Esc)",
                            );
                            break;
                        }
                    }
                }
                // The keyboard hook records real keystrokes: arm it only for
                // the remote seat, disarm the moment the cursor comes home.
                // Check canonical atomic HookControl state to prevent shadow-variable desync.
                let remote = node.input_engine_seat() == CursorSeat::Remote;
                let is_armed = node.is_keyboard_armed();
                if remote != is_armed {
                    if remote {
                        node.arm_input_capture();
                    } else {
                        node.disarm_input_capture();
                    }
                }
            }
        });
        self.shared
            .input_capture_active
            .store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Check if low-level input capture hooks are currently active.
    pub fn is_input_capture_active(&self) -> bool {
        self.shared.input_capture_active.load(Ordering::SeqCst)
    }

    /// Dynamically check and arm low-level input capture hooks if not yet active.
    /// Idempotent: returns Ok(true) if already running, or attempts to spawn it.
    pub fn ensure_input_capture(&self) -> Result<bool, CoreError> {
        #[cfg(any(windows, target_os = "macos"))]
        {
            if self.shared.input_capture_active.load(Ordering::SeqCst) {
                return Ok(true);
            }
            match self.spawn_input_capture() {
                Ok(()) => {
                    tracing::info!("ensure_input_capture: input capture hooks successfully armed");
                    Ok(true)
                }
                Err(e) => {
                    tracing::debug!(%e, "ensure_input_capture: capture not ready");
                    Err(e)
                }
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            Ok(false)
        }
    }

    /// Non-Windows and non-macOS builds have no low-level input hooks: report an honest
    /// error rather than silently doing nothing (Linux hooks are Manual-required per the SSOT).
    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn spawn_input_capture(&self) -> Result<(), CoreError> {
        Err(CoreError::Json(
            "input capture is supported on Windows and macOS; Linux hooks are Manual-required"
                .into(),
        ))
    }

    /// Clear all distance arming, dwell, and movement tracking state for auto-reverse.
    fn reset_reverse_arming_state(&self) {
        self.shared.reverse_move_accum_px.store(0, Ordering::SeqCst);
        *self.shared.reverse_last_move.lock().expect("reverse_move") = None;
        *self.shared.reverse_last_pos.lock().expect("reverse_pos") = None;
        *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
        *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
    }

    /// Refresh physical activity timestamp without accumulating movement distance.
    fn note_reverse_physical_activity(&self) {
        *self.shared.reverse_last_move.lock().expect("reverse_move") =
            Some(std::time::Instant::now());
    }

    /// Push local cursor position toward the edge state machine. Crossing the
    /// right edge emits Enter + Move. Seq is monotonic for the life of the
    /// node. No JSON, and never onto 45717. OS hooks stay Manual-required.
    pub fn drive_input(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<tetherly_core::ScreenEdge>, CoreError> {
        self.drive_input_delta(x, y, 0, 0, width, height)
    }

    /// Push local cursor position and displacement delta toward the edge state machine.
    /// When the seat is remote, deltas drive smooth free roaming on the target screen.
    pub fn drive_input_delta(
        &self,
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
        width: i32,
        height: i32,
    ) -> Result<Option<tetherly_core::ScreenEdge>, CoreError> {
        // Deskflow alignment: while this node is the *target* of an active
        // input tunnel, its engine must not roam. The cursor on this screen
        // is driven by the remote controller, and a locally-observed position
        // touching an edge must never silently promote this node to
        // controller — that race produced two controllers driving each
        // other's cursors (cursor fights / drift).
        //
        // The single exception is the Deskflow "switch at the shared edge":
        // a sustained, deliberate physical push through THIS node's own
        // configured edge requests a polite takeover, making the reverse
        // direction click-free. Everything else stays suppressed.
        if self.active_tunnel_role() == Some(tetherly_core::InputTunnelRole::Target) {
            // Distance arming: a mouse that just got plugged into this
            // machine (or is being bumped on the desk) must traverse a real
            // range before its edge-push may take over. Accumulate
            // coordinate deltas since the last idle gap; a stale gap resets
            // the accumulator AND skips this event's jump (the first event
            // after plug-in can carry an arbitrary position).
            // Use monotonic Instant to prevent NTP / clock adjustments from altering the gap.
            let now = std::time::Instant::now();
            let idle_reset = {
                let last = self.shared.reverse_last_move.lock().expect("reverse_move");
                last.map(|t| {
                    now.duration_since(t).as_millis() as u64 > Self::REVERSE_RESET_AFTER_MS
                })
                .unwrap_or(false)
            };
            let mut accum = if idle_reset {
                0
            } else {
                self.shared.reverse_move_accum_px.load(Ordering::SeqCst)
            };
            {
                let mut last_pos = self.shared.reverse_last_pos.lock().expect("reverse_pos");
                if !idle_reset {
                    if let Some((lx, ly)) = *last_pos {
                        accum += (x - lx).abs() as i64 + (y - ly).abs() as i64;
                    }
                }
                *last_pos = Some((x, y));
            }
            self.shared
                .reverse_move_accum_px
                .store(accum, Ordering::SeqCst);
            *self.shared.reverse_last_move.lock().expect("reverse_move") = Some(now);
            self.maybe_auto_reverse(x, y, dx, dy, width, height);
            return Ok(None);
        }
        // Guard: If the network input channel is not connected and we are not in grace period, never allow crossing into Remote seat.
        let in_grace = {
            let dl = self
                .shared
                .input_reconnect_deadline
                .lock()
                .expect("deadline");
            dl.map(|d| std::time::Instant::now() < d).unwrap_or(false)
        };
        if !self.input_channel_open_sync() && !in_grace {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() == CursorSeat::Remote {
                server.force_local();
                let (cx, cy) = server.local_pos();
                *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                self.disarm_input_capture();
                crate::platform::warp_mouse_position(cx, cy);
                self.emit(UiEvent::input_seat_changed("local", false, None));
            }
            return Ok(None);
        }

        let (edge, events, current_seat, local_pos, active_edge) = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            server.resize(width, height);
            let active_edge = if let Some(e) = self.input_policy().edge.as_deref() {
                e.parse::<tetherly_core::ScreenEdge>()
                    .unwrap_or(tetherly_core::ScreenEdge::Right)
            } else {
                let peer_opt = self.shared.input_peer.lock().expect("input_peer").clone();
                let edge_from_topology = peer_opt.and_then(|p| {
                    let layout = self.shared.topology_layout.lock().expect("topology_layout");
                    let p_str = p.to_string();
                    layout.neighbors.iter().find_map(|(e, n)| {
                        if n.peer_id == p_str {
                            Some((*e).into())
                        } else {
                            None
                        }
                    })
                });
                edge_from_topology.unwrap_or(tetherly_core::ScreenEdge::Right)
            };
            server.set_edge(active_edge);
            // On physical displays, OS cursor coordinates are clamped to [0, width - 1] and [0, height - 1].
            // To reliably trigger boundary crossings when touching the physical screen boundary,
            // normalize boundary-touch coordinates to the engine's edge thresholds.
            let (target_x, target_y) = match active_edge {
                tetherly_core::ScreenEdge::Right
                    if x >= width.saturating_sub(2) || (x >= width.saturating_sub(8) && dx > 0) =>
                {
                    (width, y)
                }
                tetherly_core::ScreenEdge::Bottom
                    if y >= height.saturating_sub(2)
                        || (y >= height.saturating_sub(8) && dy > 0) =>
                {
                    (x, height)
                }
                tetherly_core::ScreenEdge::Top if y <= 2 || (y <= 8 && dy < 0) => (x, 0),
                tetherly_core::ScreenEdge::Left if x <= 2 || (x <= 8 && dx < 0) => (0, y),
                _ => (x, y),
            };
            let edge = server.local_move_delta(target_x, target_y, dx, dy);
            (
                edge,
                server.drain(),
                server.seat(),
                server.local_pos(),
                active_edge,
            )
        };

        let prev_seat = *self.shared.input_seat.lock().expect("seat");
        if current_seat == CursorSeat::Remote {
            if prev_seat != Some(CursorSeat::Remote) {
                // Deskflow OSXScreen::leave & PR-10231 alignment:
                // 1) On macOS, WindowServer enforces cursor visibility over the Menu Bar (y = 0) and Dock (y = height - 1),
                //    refusing hide requests when the cursor sits at the display boundary.
                //    Parking at screen center moves the pointer into neutral content space away from Menu Bar and Dock.
                // 2) Warp to safe neutral position BEFORE arming input capture / hiding the cursor.
                #[cfg(target_os = "macos")]
                let (park_x, park_y) = {
                    let _ = active_edge;
                    (width / 2, height / 2)
                };
                #[cfg(not(target_os = "macos"))]
                let (park_x, park_y) = match active_edge {
                    tetherly_core::ScreenEdge::Top => (local_pos.0, 0),
                    tetherly_core::ScreenEdge::Bottom => (local_pos.0, height.saturating_sub(1)),
                    tetherly_core::ScreenEdge::Left => (0, local_pos.1),
                    tetherly_core::ScreenEdge::Right => (width.saturating_sub(1), local_pos.1),
                };
                crate::platform::warp_mouse_position(park_x, park_y);
                self.arm_input_capture();
            } else if !self.is_keyboard_armed() {
                self.arm_input_capture();
            }
        }

        if let Err(e) = self.send_input_events(events) {
            let in_grace = {
                let dl = self
                    .shared
                    .input_reconnect_deadline
                    .lock()
                    .expect("deadline");
                dl.map(|d| std::time::Instant::now() < d).unwrap_or(false)
            };
            if !in_grace {
                let mut server = self.shared.input_engine.lock().expect("engine");
                if server.seat() == CursorSeat::Remote {
                    server.force_local();
                    let (cx, cy) = server.local_pos();
                    *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                    self.disarm_input_capture();
                    crate::platform::warp_mouse_position(cx, cy);
                    self.emit(UiEvent::input_seat_changed("local", false, None));
                }
                return Err(e);
            }
        }

        if Some(current_seat) != prev_seat {
            *self.shared.input_seat.lock().expect("seat") = Some(current_seat);
            let peer = self
                .shared
                .input_peer
                .lock()
                .expect("input_peer")
                .as_ref()
                .map(|p| p.to_string());
            let seat_str = match current_seat {
                CursorSeat::Local => "local",
                CursorSeat::Remote => "remote",
            };
            self.emit(UiEvent::input_seat_changed(seat_str, true, peer));
            if current_seat == CursorSeat::Local {
                self.disarm_input_capture();
                crate::platform::warp_mouse_position(local_pos.0, local_pos.1);
            }
        }
        if let Ok(mut guard) = self.shared.active_input_tunnel.lock() {
            if let Some(ref mut session) = *guard {
                session.last_activity_ms = self.shared.clock.unix_ms();
            }
        }
        Ok(edge)
    }

    pub fn drive_input_key(&self, code: u16, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let peer_platform = self.input_peer_platform();
            let keymap = self
                .input_policy()
                .resolved_keymap_profile(peer_platform.as_deref());
            let remapped_code = keymap.remap_vk(code);
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::key(seq, remapped_code, down, 0)]
        };
        self.send_input_events(events)
    }

    /// Best-effort platform string for the current input peer (from the live
    /// peer table), used by the `auto` keymap profile to detect same-OS
    /// pairs where remapping must be `identity`.
    fn input_peer_platform(&self) -> Option<String> {
        let peer = self.shared.input_peer.lock().ok()?.clone()?;
        let id = tetherly_core::DeviceId::parse(peer.as_str()).ok()?;
        self.shared
            .live
            .lock()
            .ok()?
            .get(&id)
            .map(|p| p.platform.clone())
    }

    /// Role of this node in the currently active input tunnel, if any.
    pub fn active_tunnel_role(&self) -> Option<tetherly_core::InputTunnelRole> {
        self.shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel")
            .as_ref()
            .map(|s| s.role)
    }

    /// Physical movement a mouse must accumulate (px, Manhattan over
    /// coordinate deltas) after its last idle gap before an edge-push may
    /// take over. ~1/4 of a 1080p screen height: real use from anywhere on
    /// the screen crosses it, plug-in jitter (a few px) never does.
    const REVERSE_ARM_MIN_PX: i64 = 300;
    /// Idle gap that resets the movement accumulator (and absorbs the
    /// plug-in/unplug position jump).
    const REVERSE_RESET_AFTER_MS: u64 = 2000;

    /// Deskflow "switch at the shared edge" for the controlled side.
    ///
    /// While this node is the Target, a sustained physical push through its
    /// own configured edge takes over as controller automatically — the
    /// reverse direction (e.g. Windows -> Mac) needs no UI click. Accidental
    /// flips are fenced by three gates:
    ///
    /// 1. the node's own `edge` policy must be configured (a direction must
    ///    be declared before it can be triggered); the engine already mirrors
    ///    this edge (`set_edge` on attach and policy change), so it is read
    ///    lock-only here — no policy clone on the per-move hot path;
    /// 2. gap-continuity dwell: boundary-touching events must keep arriving
    ///    (gap <= 150ms) for the topology `delay_ms` (default 100ms). A
    ///    sustained push emits a dense event stream and fires; two isolated
    ///    bumps any timespan apart never do, because the second bump's gap
    ///    since the previous touch event resets the dwell. (A plain
    ///    elapsed-since-first-touch check would flip on "bump + bump 500ms
    ///    later", and a "recent input within 2s" gate is tautological — the
    ///    triggering event itself is recent input.)
    /// 3. a 3s cooldown between automatic flips prevents storm flip-flops.
    ///
    /// The flip itself reuses the polite-takeover attach path, so the
    /// global single-outbound-channel invariant still holds.
    fn maybe_auto_reverse(&self, x: i32, y: i32, dx: i32, dy: i32, width: i32, height: i32) {
        if self.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
            return;
        }
        // Distance gate input: the accumulator was just updated by the caller
        // (idle-reset + coordinate deltas; see drive_input_delta).
        let distance_armed =
            self.shared.reverse_move_accum_px.load(Ordering::SeqCst) >= Self::REVERSE_ARM_MIN_PX;
        let edge = {
            let server = self.shared.input_engine.lock().expect("engine");
            server.edge()
        };
        let now = std::time::Instant::now();
        {
            let last_flip = self.shared.last_auto_reverse.lock().expect("last_flip");
            if let Some(t) = *last_flip {
                if now.duration_since(t).as_millis() < 3000 {
                    return;
                }
            }
        }

        // Same boundary normalization the controller path uses: the OS
        // clamps the cursor to [0, size-1], so a boundary touch is promoted
        // to the true edge when the push direction agrees.
        let (target_x, target_y) = match edge {
            tetherly_core::ScreenEdge::Right
                if x >= width.saturating_sub(2) || (x >= width.saturating_sub(8) && dx > 0) =>
            {
                (width, y)
            }
            tetherly_core::ScreenEdge::Bottom
                if y >= height.saturating_sub(2) || (y >= height.saturating_sub(8) && dy > 0) =>
            {
                (x, height)
            }
            tetherly_core::ScreenEdge::Top if y <= 2 || (y <= 8 && dy < 0) => (x, 0),
            tetherly_core::ScreenEdge::Left if x <= 2 || (x <= 8 && dx < 0) => (0, y),
            _ => {
                *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
                *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
                return;
            }
        };
        let crossed = match edge {
            tetherly_core::ScreenEdge::Right => target_x >= width,
            tetherly_core::ScreenEdge::Bottom => target_y >= height,
            tetherly_core::ScreenEdge::Top => target_y <= 0,
            tetherly_core::ScreenEdge::Left => target_x <= 0,
        };

        // Distance gate: an un-armed mouse (freshly plugged in, or bumped
        // without real traversal) can never start the takeover dwell.
        if !distance_armed {
            *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
            *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
            return;
        }

        // Gap-continuity dwell: touch events must keep arriving within
        // TOUCH_GAP_MS of each other for `delay_ms` before the push counts
        // as sustained. Real mice report at 8-16ms intervals, so 150ms
        // means the pushing hand actually paused; two isolated accidental
        // bumps (typically >=300ms apart) reset the dwell instead. Leaving
        // the edge for longer than TOUCH_GAP_MS disarms the dwell.
        const TOUCH_GAP_MS: u64 = 150;
        let now = std::time::Instant::now();
        let fired = if !crossed {
            // Left the edge: the push is over, reset the dwell.
            *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
            *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
            false
        } else {
            let delay_ms = {
                let layout = self.shared.topology_layout.lock().expect("topology_layout");
                layout.config.delay_ms.max(50) as u64
            };
            let mut arm = self.shared.reverse_arm_since.lock().expect("reverse_arm");
            let mut last_tick = self.shared.reverse_last_tick.lock().expect("reverse_tick");
            match (*arm, *last_tick) {
                (Some(started), Some(prev_tick)) => {
                    let sustained = now.duration_since(started).as_millis() as u64 >= delay_ms;
                    let contiguous =
                        now.duration_since(prev_tick).as_millis() as u64 <= TOUCH_GAP_MS;
                    if sustained && contiguous {
                        *arm = None;
                        *last_tick = None;
                        true
                    } else if contiguous {
                        // Still pushing: keep accumulating towards delay_ms.
                        false
                    } else {
                        // Gap too long: a fresh push, not a continuation.
                        *arm = Some(now);
                        *last_tick = Some(now);
                        false
                    }
                }
                _ => {
                    // First touch of a new push.
                    *arm = Some(now);
                    *last_tick = Some(now);
                    false
                }
            }
        };
        if !fired {
            return;
        }
        *self.shared.last_auto_reverse.lock().expect("last_flip") = Some(now);

        let Some(peer) = self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel")
            .as_ref()
            .map(|s| s.peer_id.clone())
        else {
            return;
        };
        info!(peer = %peer, %edge, "auto-reverse: sustained edge push; requesting polite takeover");
        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "input_tunnel_auto_reverse",
            Some(peer.clone()),
            format!("Edge-push takeover requested toward peer {peer} (edge {edge})"),
            true,
        );

        let node = self.clone_handle();
        let display_edge: tetherly_core::DisplayEdge = edge.into();
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        if let Some(handle) = rt {
            handle.spawn(async move {
                if let Err(e) = node
                    .input_tunnel_attach(&peer, Some(display_edge), None, None)
                    .await
                {
                    warn!(peer = %peer, %e, "auto-reverse takeover attach failed");
                }
            });
        } else {
            warn!("auto-reverse: no runtime handle available; cannot take over");
        }
    }

    /// Push a physical mouse-button observation to the remote seat. No-op
    /// while the seat is local (the engine gates the same way).
    pub fn drive_input_button(&self, mask: u8, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            self.note_reverse_physical_activity();
            server.note_button(mask, down);
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::button(seq, mask, down)]
        };
        self.send_input_events(events)
    }

    /// Push a physical wheel observation to the remote seat.
    pub fn drive_input_wheel(&self, dx: i16, dy: i16) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            self.note_reverse_physical_activity();
            let speed = self.input_policy().wheel_speed;
            let (scaled_dx, scaled_dy) = if (speed - 1.0).abs() > 0.001 && speed > 0.0 {
                let sy = ((dy as f64) * speed).round() as i16;
                let sx = ((dx as f64) * speed).round() as i16;
                (
                    if dx != 0 && sx == 0 { dx.signum() } else { sx },
                    if dy != 0 && sy == 0 { dy.signum() } else { sy },
                )
            } else {
                (dx, dy)
            };
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::wheel(seq, scaled_dx, scaled_dy)]
        };
        self.send_input_events(events)
    }

    pub fn input_engine_seat(&self) -> CursorSeat {
        self.shared.input_engine.lock().expect("engine").seat()
    }

    /// The engine's parked local cursor position (pixels). Used by tests to
    /// assert the Deskflow-style proportional return landing.
    pub fn input_engine_local_pos(&self) -> (i32, i32) {
        self.shared.input_engine.lock().expect("engine").local_pos()
    }

    pub(super) fn send_input_events(&self, events: Vec<InputEvent>) -> Result<(), CoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let tx_opt = self
            .shared
            .input_out
            .lock()
            .expect("input_out")
            .as_ref()
            .map(|(_, tx)| tx.clone());
        if let Some(tx) = tx_opt {
            let mut disconnected = false;
            for ev in &events {
                match tx.try_send(ev.to_frame()?) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        // Drop Move frames on queue pressure so clicks/keys are never blocked
                        if !matches!(ev.kind, tetherly_core::InputKind::Move { .. }) {
                            return Err(CoreError::Json("input queue full; peer too slow".into()));
                        }
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            if !disconnected {
                return Ok(());
            }
        }

        // Channel is closed or dropped; check if seat is remote for grace period buffering
        let is_remote =
            self.shared.input_engine.lock().expect("engine").seat() == CursorSeat::Remote;
        if is_remote {
            let in_grace = {
                let mut dl = self
                    .shared
                    .input_reconnect_deadline
                    .lock()
                    .expect("deadline");
                match *dl {
                    Some(d) => std::time::Instant::now() < d,
                    None => {
                        *dl = Some(
                            std::time::Instant::now() + std::time::Duration::from_millis(2500),
                        );
                        true
                    }
                }
            };
            if in_grace {
                let mut buf = self.shared.input_buffered_frames.lock().expect("buffered");
                for ev in events {
                    if buf.len() >= 64 {
                        buf.pop_front();
                    }
                    buf.push_back(ev.to_frame()?);
                }
                return Ok(());
            }
        }
        Err(CoreError::Json("input channel closed".into()))
    }

    pub(super) async fn accept_input(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let _ = stream.set_nodelay(true);
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

    pub(super) fn attach_input_server(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        let (tx, mut rx) = mpsc::channel::<InnerFrame>(SESSION_QUEUE_DEPTH);

        let was_in_grace = self
            .shared
            .input_reconnect_deadline
            .lock()
            .expect("deadline")
            .take()
            .is_some();
        let is_remote =
            self.shared.input_engine.lock().expect("engine").seat() == CursorSeat::Remote;

        if was_in_grace || is_remote {
            // Re-assert focus and cursor position on target InputClient
            let resume_frames: Vec<InnerFrame> = {
                let mut engine = self.shared.input_engine.lock().expect("engine");
                engine.resume_remote();
                engine
                    .drain()
                    .into_iter()
                    .filter_map(|ev| ev.to_frame().ok())
                    .collect()
            };
            for f in resume_frames {
                let _ = tx.try_send(f);
            }

            // Flush frames buffered during the grace period
            let buffered: Vec<InnerFrame> = {
                let mut buf = self.shared.input_buffered_frames.lock().expect("buffered");
                buf.drain(..).collect()
            };
            let buf_count = buffered.len();
            for f in buffered {
                let _ = tx.try_send(f);
            }
            tracing::info!(peer = %peer_id, buf_count, "input session resumed; flushed buffered frames");
        }

        let channel_gen = INPUT_CHANNEL_SEQ.fetch_add(1, Ordering::SeqCst);
        *self.shared.input_out.lock().expect("input_out") = Some((channel_gen, tx));
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
                                        node.disarm_input_capture();
                                        let (cx, cy) = node.shared.input_engine.lock().expect("engine").local_pos();
                                        crate::platform::warp_mouse_position(cx, cy);
                                        node.emit(UiEvent::input_seat_changed(
                                            "local",
                                            true,
                                            Some(sess.peer_id.to_string()),
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                let err_s = e.to_string();
                                if err_s.contains("stream closed during header")
                                    || err_s.contains("Connection reset")
                                    || err_s.contains("Broken pipe")
                                {
                                    tracing::info!("input server: peer session closed cleanly ({err_s})");
                                } else {
                                    tracing::warn!(%e, "input server: session recv ended; closing");
                                }
                                break;
                            }
                        }
                    }
                }
            }
            {
                let mut out = node.shared.input_out.lock().expect("input_out");
                if let Some((gen, _)) = out.as_ref() {
                    if *gen == channel_gen {
                        *out = None;
                    }
                }
            }

            // Supersession guard: this teardown must only run while this
            // task's session is still the current one (same peer, same
            // Controller role). On a takeover the peer demotes us (role ->
            // Target) and rebuilds the channel — the dying task must NOT
            // touch seat/engine/grace state. This exact race shipped a bug
            // where the demoted side's dying task flipped the freshly
            // rebuilt session back to Local.
            let still_current = {
                let role_now = node
                    .shared
                    .active_input_tunnel
                    .lock()
                    .expect("active_input_tunnel")
                    .as_ref()
                    .map(|s| s.role)
                    // Bare input channels (no tunnel session) keep the legacy
                    // controller-side cleanup; only an actual demotion
                    // (role = Target) suppresses it.
                    .unwrap_or(InputTunnelRole::Controller);
                let peer_now = node.shared.input_peer.lock().expect("input_peer").clone();
                role_now == InputTunnelRole::Controller
                    && peer_now.as_ref().map(|d| d.to_string()).as_deref() == Some(peer_id.as_str())
            };
            if !still_current {
                return;
            }

            let is_remote =
                node.shared.input_engine.lock().expect("engine").seat() == CursorSeat::Remote;
            let is_shutting_down = *node.shared.shutdown.subscribe().borrow();

            if is_remote && !is_shutting_down {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2500);
                *node
                    .shared
                    .input_reconnect_deadline
                    .lock()
                    .expect("deadline") = Some(deadline);
                tracing::info!(peer = %peer_id, "input session disconnected while in Remote seat; entering 2.5s grace period");

                let retry_node = node.clone_handle();
                let target_peer = peer_id.clone();
                tokio::spawn(async move {
                    let mut backoff_ms = 30u64;
                    while std::time::Instant::now() < deadline {
                        tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                        if std::time::Instant::now() >= deadline {
                            break;
                        }
                        if retry_node.input_channel_open().await {
                            tracing::info!(
                                "fast input reconnect: channel is already re-established"
                            );
                            return;
                        }
                        if retry_node
                            .shared
                            .input_engine
                            .lock()
                            .expect("engine")
                            .seat()
                            != CursorSeat::Remote
                        {
                            tracing::info!(
                                "fast input reconnect: seat is no longer remote; stopping retry"
                            );
                            return;
                        }
                        tracing::debug!(peer = %target_peer, backoff_ms, "attempting fast input reconnect");
                        if let Ok(()) = retry_node.open_input(&target_peer).await {
                            tracing::info!(peer = %target_peer, "fast input reconnect succeeded");
                            return;
                        }
                        backoff_ms = (backoff_ms * 2).min(200);
                    }

                    if retry_node
                        .shared
                        .input_reconnect_deadline
                        .lock()
                        .expect("deadline")
                        .take()
                        .is_some()
                    {
                        tracing::warn!(peer = %target_peer, "input grace period expired (2.5s); returning seat to local");
                        retry_node
                            .shared
                            .input_buffered_frames
                            .lock()
                            .expect("buffered")
                            .clear();
                        *retry_node.shared.input_peer.lock().expect("input_peer") = None;
                        retry_node
                            .shared
                            .input_engine
                            .lock()
                            .expect("engine")
                            .force_local();
                        *retry_node.shared.input_seat.lock().expect("seat") =
                            Some(CursorSeat::Local);
                        retry_node.disarm_input_capture();
                        let (cx, cy) = retry_node
                            .shared
                            .input_engine
                            .lock()
                            .expect("engine")
                            .local_pos();
                        crate::platform::warp_mouse_position(cx, cy);
                        retry_node.emit(UiEvent::input_seat_changed("local", false, None));
                    }
                });
            } else {
                *node
                    .shared
                    .input_reconnect_deadline
                    .lock()
                    .expect("deadline") = None;
                node.shared
                    .input_buffered_frames
                    .lock()
                    .expect("buffered")
                    .clear();
                *node.shared.input_peer.lock().expect("input_peer") = None;
                node.shared
                    .input_engine
                    .lock()
                    .expect("engine")
                    .on_peer_gone();
                *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                node.disarm_input_capture();
                let (cx, cy) = node.shared.input_engine.lock().expect("engine").local_pos();
                crate::platform::warp_mouse_position(cx, cy);
                node.emit(UiEvent::input_seat_changed("local", false, None));
            }
        });
    }

    pub(super) async fn attach_input_client(&self, mut sess: ActiveSession) {
        let peer_id = sess.peer_id.clone();
        *self.shared.input_peer.lock().expect("input_peer") = Some(peer_id.clone());
        let real = self.shared.input_real_sink.clone();
        let mut client = InputClient::new(DualInputSink::new(real));
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        let task = tokio::spawn(async move {
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
                                    if let Ok(leave_opt) = client.apply(ev) {
                                        if let Some(leave) = leave_opt {
                                            if let Ok(f) = leave.to_frame() {
                                                let l_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_input_tx(l_len);
                                                }
                                            }
                                        }
                                        *node.shared.input_sink.lock().expect("sink") = client.sink().memory().clone();
                                        node.screen_force_wake_squelch();
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
                                    } else {
                                        // A rejected seq means a replayed or
                                        // reordered frame arrived: log it so
                                        // desyncs stay diagnosable instead of
                                        // silently disappearing.
                                        tracing::warn!("input client rejected event (seq jump or invalid state)");
                                    }
                                } else if let Err(e) = InputEvent::from_frame(&frame) {
                                    tracing::warn!(%e, "input client: bad frame");
                                }
                            }
                            Err(e) => {
                                let err_s = e.to_string();
                                if err_s.contains("stream closed during header")
                                    || err_s.contains("Connection reset")
                                    || err_s.contains("Broken pipe")
                                {
                                    tracing::info!("input client: peer session closed cleanly ({err_s})");
                                } else {
                                    tracing::warn!(%e, "input client: session recv ended; closing");
                                }
                                break;
                            }
                        }
                    }
                }
            }
            // Supersession guard (mirror of the server-side one): after a
            // takeover this node may already be the Controller with a live
            // roaming engine — the dying target-era session must not force
            // it back to Local.
            let still_current = match node
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel")
                .as_ref()
            {
                // A tunnel session exists: only the still-current target-era
                // session may run the teardown. After a takeover this node is
                // the Controller with a live roaming engine — the dying
                // target-era session must not force it back to Local.
                Some(s) => s.role == InputTunnelRole::Target && s.peer_id == peer_id.as_str(),
                // Bare input channel (no tunnel session): legacy behavior.
                None => true,
            };
            if still_current {
                client.on_peer_gone();
                *node.shared.input_peer.lock().expect("input_peer") = None;
                *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                *node.shared.input_sink.lock().expect("sink") = client.sink().memory().clone();
                node.reset_reverse_arming_state();
                node.emit(UiEvent::input_seat_changed("local", false, None));
            }
        });

        // Abort any previous inbound client task: stacked accepts (reconnect
        // races, controller takeover) must never leave two clients applying
        // frames against the same injection sink.
        let mut guard = self
            .shared
            .input_client_task
            .lock()
            .expect("input_client_task");
        if let Some(old) = guard.replace(task) {
            old.abort();
        }
        drop(guard);
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

        let _ = self.ensure_input_capture();

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

#[cfg(test)]
mod tests {
    use super::*;
    use tetherly_core::InputKind;

    #[tokio::test]
    async fn test_input_grace_period_buffering() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("input-grace");

        // 1. Initially local seat: send_input_events errors if channel closed
        let ev = InputEvent::move_to(1, 100, 200);
        assert!(node.send_input_events(vec![ev.clone()]).is_err());
        assert_eq!(node.shared.input_buffered_frames.lock().unwrap().len(), 0);

        // 2. Set engine seat to Remote:
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(1920, 540, 10, 0); // cross right edge
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }

        // 3. Now send_input_events should enter grace period and buffer the frames
        let res = node.send_input_events(vec![ev]);
        assert!(res.is_ok());
        assert!(node
            .shared
            .input_reconnect_deadline
            .lock()
            .unwrap()
            .is_some());
        assert_eq!(node.shared.input_buffered_frames.lock().unwrap().len(), 1);

        // 4. Input stop clears grace period and buffer
        let stop_res = node.input_stop().await;
        assert!(stop_res.is_ok());
        assert!(node
            .shared
            .input_reconnect_deadline
            .lock()
            .unwrap()
            .is_none());
        assert_eq!(node.shared.input_buffered_frames.lock().unwrap().len(), 0);
    }

    #[test]
    fn test_dual_input_sink_forwards_all_events_and_flushes_to_real_sink() {
        let real_sink = Arc::new(Mutex::new(MemorySink::default()));
        let dual = DualInputSink::new(Some(real_sink.clone()));
        let mut client = InputClient::new(dual);

        // 1. Enter and move
        client.apply(InputEvent::enter(1)).unwrap();
        client.apply(InputEvent::move_to(2, 500, 500)).unwrap();

        // 2. Press a modifier key and a mouse button
        client.apply(InputEvent::key(3, 0xA0, true, 0)).unwrap(); // VK_LSHIFT down
        client
            .apply(InputEvent::button(4, tetherly_core::BTN_LEFT, true))
            .unwrap(); // Left click down

        // Verify real sink received the down events
        {
            let guard = real_sink.lock().unwrap();
            assert_eq!(guard.cursor, (500, 500));
            assert_eq!(guard.buttons, tetherly_core::BTN_LEFT);
            assert_eq!(guard.keys_down, vec![0xA0]);
        }

        // 3. Leave event triggers release_all_inputs on both memory observer AND real sink
        client.apply(InputEvent::leave(5)).unwrap();

        {
            let guard = real_sink.lock().unwrap();
            // Verify real sink received key release and button release
            assert_eq!(guard.buttons, 0);
            assert!(guard.keys_down.is_empty());
            assert!(guard.applied.contains(&InputKind::Key {
                code: 0xA0,
                down: false,
                mods: 0
            }));
            assert!(guard.applied.contains(&InputKind::Button {
                mask: tetherly_core::BTN_LEFT,
                down: false
            }));
        }

        // 4. Focus again, hold a key, and trigger on_peer_gone (network disconnect)
        client.apply(InputEvent::enter(6)).unwrap();
        client.apply(InputEvent::key(7, 0xA2, true, 0)).unwrap(); // VK_LCONTROL down

        client.on_peer_gone();

        {
            let guard = real_sink.lock().unwrap();
            assert!(guard.keys_down.is_empty());
            assert!(guard.applied.contains(&InputKind::Key {
                code: 0xA2,
                down: false,
                mods: 0
            }));
        }
    }
}
