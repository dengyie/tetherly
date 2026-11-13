// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Input-tunnel policy, sessions and cursor crossing.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_input_policy, DeviceType, InputTunnelPolicy};
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use tetherly_core::frame::InnerFrame;
#[cfg(test)]
use tetherly_core::DisplayRect;
use tetherly_core::{
    map_edge_exit_to_entry, CoreError, CursorSeat, DeviceId, DisplayEdge, InputClient, InputEvent,
    InputKind, InputSink, InputTunnelHeartbeatFrame, InputTunnelReleaseFrame,
    InputTunnelRequestFrame, InputTunnelRespFrame, InputTunnelRole, InputTunnelValidator,
    MemorySink, RemoteSessionCloseFrame, RemoteSessionRejectReason, RemoteSessionRequestFrame,
    RemoteSessionRespFrame, RemoteSessionStateMachine, ScreenState,
};
use tetherly_net::session::{accept_session, dial_session, ActiveSession};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};
/// Monotonic generation counter for outbound input channels.
static INPUT_CHANNEL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Monotonic epoch for pre-armed input tunnel grants. Bumped on every
/// grant construction (controller pre-arm AND target accept) so a stale
/// channel pump can tell its own grant from a newer one (see
/// `ActiveInputTunnelSession::pre_arm_epoch`).
static INPUT_PRE_ARM_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_pre_arm_epoch() -> u64 {
    INPUT_PRE_ARM_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
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
    /// Monotonic pre-arm generation. `input_tunnel_attach` bumps this when
    /// it overwrites the grant slot; a dying channel pump captured the
    /// epoch it started under, and its teardown may only release a grant
    /// whose epoch matches. Without this, a takeover's fresh pre-arm is
    /// released by the OLD pump's "peer session gone" teardown — the peer's
    /// ACCEPTED resp then finds no armed session and the takeover dead-ends
    /// (production 2026-11-11 23:47 race + phase3 m3_* 2026-11-11).
    #[serde(default)]
    pub pre_arm_epoch: u64,
}

/// Active ADR-138 remote desktop session metadata (controller or target side).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ActiveRemoteSessionState {
    pub session_id: String,
    pub peer_id: String,
    pub role: InputTunnelRole,
    /// Host physical resolution — the viewer's viewport mapping baseline.
    pub screen_width: u32,
    pub screen_height: u32,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// One remote session slot: metadata plus its pure state machine. Stored in
/// `Shared.remote_session` (single slot — at most one session per node).
#[derive(Debug, Clone)]
pub struct RemoteSessionState {
    pub state: ActiveRemoteSessionState,
    pub machine: RemoteSessionStateMachine,
}

impl std::ops::Deref for RemoteSessionState {
    type Target = ActiveRemoteSessionState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
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
    /// v0.2.31 apply counters (Target side): frames applied to the sink and
    /// frames dropped by the seq monotonic check since process start.
    #[serde(default)]
    pub applied: u64,
    #[serde(default)]
    pub dropped: u64,
    /// Inbound Button / Wheel / Key frames received (Target side).
    #[serde(default)]
    pub rx_buttons: u64,
    #[serde(default)]
    pub rx_wheel: u64,
    #[serde(default)]
    pub rx_keys: u64,
    /// Button frames this node pushed as Controller.
    #[serde(default)]
    pub tx_buttons: u64,
}

/// Capacity of the input-engine decision ring behind `GET /api/input/trace`.
/// 64 entries cover several roaming stints (a stint is ~6 decisions:
/// crossing, park warp, seat change, return seat change, release, kick).
pub const INPUT_TRACE_CAP: usize = 64;

/// One input-engine decision (v0.2.30). `kind` is a short stable token
/// (`crossing`, `park_warp`, `seat`, `release`, `rescue`, `reconcile`,
/// `attach_kick`, `apply_enter`, `push_enter`, `push_clamp`; v0.2.31 adds
/// `push_button`, `apply_button`, `apply_reject`, `pump_exit`,
/// `channel_open`, `zone_gate`); `detail` is free text for the operator.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputTraceEntry {
    pub ts_ms: u64,
    pub kind: String,
    pub detail: String,
}

/// `GET /api/input/trace` report: current seat/channel plus the decision
/// ring, oldest first.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct InputTraceReport {
    pub ok: bool,
    pub seat: String,
    pub channel_open: bool,
    pub capacity: usize,
    pub entries: Vec<InputTraceEntry>,
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
        } else if self.remote_session_active() {
            // ADR-138 §2.7 single-session invariant, inbound half: an active
            // remote desktop session owns the seat, so a roaming tunnel
            // request (from a third node or the session peer) must not stack
            // a second controller on the same desktop.
            Some("remote desktop session active".to_string())
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
        //
        // NOTE (production 2026-11-12): a same-peer request here is
        // ambiguous — the peer watcher's stale redial (which must NOT demote
        // a live roaming session) and the peer user's explicit reverse
        // takeover (which must) are wire-identical. The demotion stays, and
        // the fix lives on the requesting side: the watcher debounces a
        // pending (<10s) Controller session instead of re-arming every tick
        // (cmd_core watcher loop), so a stale redial never reaches this
        // branch while the user's roaming is alive.
        if self.active_tunnel_role() == Some(InputTunnelRole::Controller) {
            info!(peer = %peer_str, "input tunnel takeover: releasing local controller session");
            eprintln!(
                "[takeover] demote: releasing local controller session toward peer={peer_str}"
            );
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
            pre_arm_epoch: next_pre_arm_epoch(),
        };

        *self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel") = Some(session);

        // Production 2026-11-12 (v0.2.18): the TARGET side never told the
        // engine which shared edge this session sits on. The controller side
        // resolves its edge in `input_tunnel_attach` (policy → topology →
        // Right); here nothing did, so a target that has no policy edge
        // (Windows production box) kept the engine's default Right while the
        // real pair is bottom. The user's push at the bottom edge then
        // matched no zone in `maybe_auto_reverse` / the drive normalization
        // (both read `engine.edge()`) and was silently swallowed — "Windows
        // bottom cannot reach the Mac", with zero crossing logs. Resolve the
        // same ladder on accept: the controller's requested edge INVERTED
        // (the boundary it crossed is this side's OPPOSITE edge — the mac's
        // Top crossing and windows' Bottom sit on the same physical
        // boundary), then local policy, then the topology neighbor for this
        // peer, then Right. Also seed `input_peer` so the drive path's
        // topology fallback resolves while the session lives (the channel
        // may not be dialed yet).
        //
        // Production 2026-11-12 (v0.2.22): arming the engine with the
        // controller's edge AS SENT pinned windows' shared edge to Top —
        // the controller's own side of the boundary. Every bottom-edge
        // push matched no zone (`_` arm, silent) and the reverse takeover
        // could never fire: "windows mouse never leaves windows". The
        // inversion is symmetric: the reverse takeover sends THIS side's
        // engine edge (Bottom) as requested_edge, and the mac's accept
        // inverts it back to its own Top.
        let target_edge: tetherly_core::ScreenEdge = if let Some(e) = frame.requested_edge {
            let screen_edge: tetherly_core::ScreenEdge = e.into();
            screen_edge.opposite()
        } else if let Some(e) = policy.edge.as_deref() {
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
        *self.shared.input_peer.lock().expect("input_peer") = Some(peer.clone());
        {
            let mut engine = self.shared.input_engine.lock().expect("engine");
            engine.set_edge(target_edge);
            let (ew, eh) = crate::platform::screen_size();
            engine.resize(ew.max(1), eh.max(1));
        }

        // Reset any leftover arming/dwell state from an earlier session.
        self.reset_reverse_arming_state();
        // v0.2.31: a pre-arm accept carries no entry placement — the
        // controller has not crossed, the cursor on this screen is still
        // the local user's, so there is no parked entry position for the
        // zone gate to protect. Leave the gate OPEN: production 2026-11-13
        // 22:28 the peer returned home, re-pre-armed within ms, and this
        // node's user — cursor already resting on the shared edge where
        // the peer's stint had left it — pushed into the edge for 8 s
        // against a closed gate ("dwell gated") before moving interior
        // and back. The gate closes again when the controller's Enter
        // actually arrives (`note_controller_entered`).
        if clamped_entry_x.is_none() || clamped_entry_y.is_none() {
            self.shared.reverse_zone_dirty.store(true, Ordering::SeqCst);
            self.trace_input(
                "zone_gate",
                format!("open: pre-arm accept without entry placement peer={peer_str}"),
            );
            // v0.2.33: same fact, stronger consequence — the local user
            // holds the cursor, so an edge push takes the controller-parity
            // fast path (production 13:05:07: 3.6 s first entry).
            self.shared
                .reverse_local_owner
                .store(true, Ordering::SeqCst);
            self.trace_input("reverse_owner", "local: pre-arm accept");
        }

        // Apply the controller's proportional entry placement: without this
        // the cursor on this screen stays parked wherever the previous
        // session left it (production showed a constant bottom-center start)
        // until the first Move frame happens to arrive. The entry is in this
        // display's pixel coordinates (clamped above); warp the real cursor
        // there so control begins exactly where the cross-screen mapping
        // says. SetCursorPos warps are injected events and are filtered by
        // the local capture hook, so no feedback loop.
        if let (Some(ex), Some(ey)) = (clamped_entry_x, clamped_entry_y) {
            let (sw, sh) = crate::platform::screen_size();
            let px = ex.clamp(0, sw.saturating_sub(1));
            let py = ey.clamp(0, sh.saturating_sub(1));
            crate::platform::warp_mouse_position(px, py);
            {
                let mut sink = self.shared.input_sink.lock().expect("sink");
                sink.cursor = (px, py);
            }
            self.record_audit(
                AuditCategory::Config,
                AuditSeverity::Info,
                "input_tunnel_entry_applied",
                Some(peer_str.clone()),
                format!("Placed cursor at controller-mapped entry ({px}, {py})"),
                true,
            );
        }

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
                tracing::info!(session_id = %frame.session_id, peer = %peer_str, "input tunnel resp: no armed session; dropping stale resp");
                eprintln!(
                    "[takeover] resp: NO ARMED SESSION, dropped session={}",
                    frame.session_id
                );
                return;
            };
            if session.session_id != frame.session_id {
                tracing::info!(session_id = %frame.session_id, armed = %session.session_id, "input tunnel resp: session id mismatch; dropping stale resp");
                eprintln!(
                    "[takeover] resp: SESSION ID MISMATCH dropped={} armed={}",
                    frame.session_id, session.session_id
                );
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
            eprintln!(
                "[takeover] resp: ACCEPTED session={} peer={peer_str}",
                frame.session_id
            );
            // v0.2.32: the accept carries the target's screen size; hand it
            // to the engine so remote deltas normalize against the PEER
            // screen (1 local px = 1 remote px at 100 %) instead of the
            // local one (0.77/0.89 pt per px and anisotropic on a 1920x1080
            // controller driving a 1470x956 Mac, production 2026-11-13).
            let remote_screen = match (frame.capture_width, frame.capture_height) {
                (Some(w), Some(h)) if w > 0 && h > 0 => {
                    Some((w.min(i32::MAX as u32) as i32, h.min(i32::MAX as u32) as i32))
                }
                _ => None,
            };
            self.shared
                .input_engine
                .lock()
                .expect("engine")
                .set_remote_screen(remote_screen);
            self.trace_input(
                "remote_screen",
                match remote_screen {
                    Some((w, h)) => format!("peer={peer_str} {w}x{h}"),
                    None => format!("peer={peer_str} unknown (local-size fallback)"),
                },
            );
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
        // ADR-138: the controller's session heartbeat also keeps the *session*
        // slot alive on the host. Without this the host-side
        // `remote_session.last_activity_ms` is written only once at accept and
        // the idle sweep force-closes every session after `idle_timeout_secs`
        // even while the operator is actively typing.
        if let Some(s) = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .as_mut()
        {
            if s.state.session_id == frame.session_id && s.state.peer_id == peer_str {
                s.state.last_activity_ms = now_ms;
            }
        }
    }

    /// The 45719 receive pump calls this for every inbound input frame:
    /// frames flowing on the tunnel ARE operator activity. Heartbeats are
    /// ADR-138-session-only, so a plain roaming target has no other
    /// keepalive — without this touch its slot goes stale during active
    /// control and the target's own watcher sweep reaps it mid-use (the
    /// controller-side counterpart lives in `send_input_events`).
    pub fn note_inbound_tunnel_activity(&self, peer: &str) {
        let now_ms = self.shared.clock.unix_ms();
        if let Ok(mut guard) = self.shared.active_input_tunnel.lock() {
            if let Some(session) = guard.as_mut() {
                if session.peer_id == peer {
                    session.last_activity_ms = now_ms;
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // ADR-138 remote desktop session mode.
    //
    // A session is a single permanent Enter: seat → Remote + focused=true,
    // with no boundary semantics on either side. The wire format is unchanged
    // (`InputEvent::to_frame`/`from_frame`), only the Enter source differs —
    // session establishment instead of an edge crossing. `InputClient`'s
    // focused gating, seq dedup, and `release_all_inputs` flush are reused
    // verbatim.
    // ------------------------------------------------------------------

    /// Target side: enter Session mode. A single permanent Enter puts the
    /// client in focus and parks the cursor at the host screen center; the
    /// controller's first Move frame re-places it inside the session window.
    /// Nothing here touches edge state — Session mode has no boundaries.
    pub fn enter_session(&self) {
        let events = {
            let mut engine = self.shared.input_engine.lock().expect("engine");
            engine.clear_stale_remote();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            // Session mode = permanent Enter at screen center. push() assigns
            // the next seq; the client dedups by seq as with any other Enter.
            engine.force_remote_center();
            engine.drain()
        };
        // The resume/grace machinery exists for edge roaming recovery; a
        // session Enter goes straight through the live channel or fails.
        if let Err(e) = self.send_input_events(events, "remote_session_enter") {
            warn!(%e, "remote session: failed to send Enter frames");
        }
        {
            let mut sink = self.shared.input_sink.lock().expect("sink");
            let (w, h) = crate::platform::screen_size();
            sink.cursor = (w / 2, h / 2);
        }
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Remote);
        self.emit(UiEvent::input_seat_changed("remote", true, None));
    }

    /// Target side: leave Session mode. Flushes every held key/button via the
    /// engine's Leave frame (client `release_all_inputs`), restores Local seat.
    pub fn exit_session(&self) {
        let events = {
            let mut engine = self.shared.input_engine.lock().expect("engine");
            engine.force_local();
            engine.drain()
        };
        if let Err(e) = self.send_input_events(events, "remote_session_exit") {
            warn!(%e, "remote session: failed to send Leave frame");
        }
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        self.emit(UiEvent::input_seat_changed("local", false, None));
    }

    /// Host side of an ADR-138 session request. Validate-first: a malformed
    /// session_id is rejected outright, never aliased into a sentinel that
    /// would keep flowing through policy checks.
    pub async fn receive_remote_session_request(
        &self,
        peer: &DeviceId,
        mut frame: RemoteSessionRequestFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if InputTunnelValidator::validate_session_id(&frame.session_id).is_err() {
            warn!(peer = %peer, "invalid remote session_id rejected");
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "remote_session_rejected",
                Some(peer_str.clone()),
                "Rejected remote session request: invalid session_id".to_string(),
                false,
            );
            return;
        }

        // Single-session invariant: Busy while this node is already hosting
        // (or negotiating) a remote session, regardless of requester.
        let policy = self.input_policy();
        let profile = self.peer_profile(peer);

        let (accept, reject_reason) = {
            let guard = self.shared.remote_session.lock().expect("remote_session");
            if guard.is_some() {
                (false, Some(RemoteSessionRejectReason::Busy))
            } else if !policy.enabled
                || !policy.allow_inbound_control
                || !policy.is_peer_eligible(&profile)
            {
                (false, Some(RemoteSessionRejectReason::PolicyDisabled))
            } else if !matches!(
                self.screen_state(),
                ScreenState::Allowed | ScreenState::Streaming
            ) {
                // ADR-138 §2.4: a host that never allowed screen streaming
                // auto-allows for the session — the user explicitly chose to
                // be controlled via the session handshake. Policy gating
                // happened above. `Requested` (a 45720 dial that raced this
                // handler) counts as never-allowed; `Idle/Requested` →
                // allow. An explicit prior `Allowed`/`Streaming` is kept.
                // `allow()` only transitions from `Requested|Allowed`, so
                // mark the imminent 45720 dial first (idempotent with the
                // real dial's on_peer_connected) — from Idle the allow
                // would be a silent no-op and the later Start would be
                // refused at Requested (trace: refused=1, state wedged at
                // Requested, screen never streams).
                self.shared
                    .screen_engine
                    .lock()
                    .expect("screen")
                    .on_peer_connected();
                self.screen_allow();
                // Mark the consent as session-scoped so teardown revokes it;
                // an operator's pre-existing Allowed|Streaming must survive.
                self.shared
                    .remote_session_auto_allowed
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                (true, None)
            } else {
                (true, None)
            }
        };

        if !accept {
            if let Some(reason) = reject_reason {
                let resp = RemoteSessionRespFrame {
                    session_id: frame.session_id.clone(),
                    accepted: false,
                    reject_reason: Some(reason.as_str().to_string()),
                    screen_width: 0,
                    screen_height: 0,
                    keymap_profile: None,
                    timestamp_ms: now_ms,
                };
                if let Ok(inner) = resp.to_frame() {
                    let _ = self.send_to(peer, inner).await;
                }
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Warning,
                    "remote_session_rejected",
                    Some(peer_str.clone()),
                    format!(
                        "Rejected remote session request from {}: {}",
                        peer_str,
                        reason.as_str()
                    ),
                    false,
                );
            }
            return;
        }

        // Accept: register as SessionTarget, then answer with this machine's
        // physical resolution and suggested keymap preset.
        let (width, height) = {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            (layout.display.width, layout.display.height)
        };
        let state = ActiveRemoteSessionState {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: InputTunnelRole::SessionTarget,
            screen_width: width,
            screen_height: height,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };
        let mut sm = RemoteSessionStateMachine::new();
        let _ = sm.request_received();
        let _ = sm.accepted();
        *self.shared.remote_session.lock().expect("remote_session") = Some(RemoteSessionState {
            state: state.clone(),
            machine: sm,
        });

        // Screen flow gate: consent is already open (or auto-allowed above);
        // the controller drives Start via the existing screen control path.
        let resp = RemoteSessionRespFrame {
            session_id: frame.session_id.clone(),
            accepted: true,
            reject_reason: None,
            screen_width: width,
            screen_height: height,
            keymap_profile: policy.keymap.clone(),
            timestamp_ms: now_ms,
        };
        if let Ok(inner) = resp.to_frame() {
            let _ = self.send_to(peer, inner).await;
        }

        self.emit(UiEvent::input_tunnel_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "session_target",
            Some(width),
            Some(height),
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "remote_session_started",
            Some(peer_str.clone()),
            format!(
                "Remote desktop session {} accepted from peer {} (target role)",
                frame.session_id, peer_str
            ),
            true,
        );
    }

    /// Controller side: process the host's session answer. On accept, store
    /// the viewport baseline and enter Session mode (Enter frame out).
    pub async fn receive_remote_session_resp(
        &self,
        peer: &DeviceId,
        mut frame: RemoteSessionRespFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        frame.sanitize();
        if InputTunnelValidator::validate_session_id(&frame.session_id).is_err() {
            warn!(peer = %peer, "invalid remote session_id in resp");
            return;
        }

        let current = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .clone();
        let Some(state) = current else {
            return; // No session in flight; a late resp must not resurrect it.
        };
        if state.session_id != frame.session_id || state.peer_id != peer_str {
            return; // Stale or foreign resp.
        }
        if !frame.accepted {
            let reason = frame
                .reject_reason
                .as_deref()
                .and_then(RemoteSessionRejectReason::from_reason)
                .map(|r| r.as_str().to_string())
                .unwrap_or_else(|| "unspecified".to_string());
            *self.shared.remote_session.lock().expect("remote_session") = None;
            *self.shared.remote_session_keymap.lock().expect("rs_keymap") = None;
            // Controller-side reject: if a *previous* session on this node had
            // left the auto-allow latch set (crash race), clear it — the
            // reject path itself never granted consent.
            self.shared
                .remote_session_auto_allowed
                .store(false, std::sync::atomic::Ordering::Relaxed);
            self.emit(UiEvent::input_tunnel_session_closed(
                &frame.session_id,
                &peer_str,
                self.peer_alias(peer).unwrap_or_else(|| peer_str.clone()),
                &reason,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "remote_session_rejected",
                Some(peer_str.clone()),
                format!("Remote session rejected by host: {reason}"),
                false,
            );
            return;
        }

        let mut sm = state.machine;
        // Viewer-side transitions: Requesting --accepted()--> Streaming
        // (session established, Enter outgoing) --input_enabled()-->
        // Controlling (Enter sent through 45719, Session mode active). The
        // wire's `accepted` flag drives BOTH hops: without the second hop the
        // viewer machine never reaches Controlling and every start times out.
        if sm.accepted().is_err() {
            return; // Duplicate resp or concurrent teardown.
        }
        {
            let mut guard = self.shared.remote_session.lock().expect("remote_session");
            if let Some(s) = guard.as_mut() {
                s.machine = sm;
                s.state.screen_width = frame.screen_width.max(1);
                s.state.screen_height = frame.screen_height.max(1);
                s.state.last_activity_ms = now_ms;
            }
        }
        // Session-scoped keymap: the host's suggested preset drives how this
        // controller's keystrokes are remapped for the lifetime of the
        // session (cleared on close). The operator's own policy stays intact.
        if let Some(profile) = frame.keymap_profile.as_deref() {
            if tetherly_core::KeyMapProfile::from_preset(profile).is_some() {
                *self.shared.remote_session_keymap.lock().expect("rs_keymap") =
                    Some(profile.to_string());
            }
        }
        self.enter_session();
        {
            let mut guard = self.shared.remote_session.lock().expect("remote_session");
            if let Some(s) = guard.as_mut() {
                // enter_session sent the Enter frames — the session is now
                // controlling. A concurrent close may have cleared the slot;
                // that close wins and the transition is dropped.
                let _ = s.machine.input_enabled();
            }
        }
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "remote_session_controlling",
            Some(peer_str.clone()),
            format!("Remote session {} entered Controlling", frame.session_id),
            true,
        );
    }

    /// Either side: gracefully end the session. Flushes held inputs on the
    /// target via `exit_session` and clears the slot; idempotent.
    pub async fn receive_remote_session_close(
        &self,
        peer: &DeviceId,
        frame: RemoteSessionCloseFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let current = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .clone();
        let Some(state) = current else {
            return;
        };
        // Match on session_id OR peer so a half-open peer may still tear down.
        if state.session_id != frame.session_id && state.peer_id != peer_str {
            return;
        }
        *self.shared.remote_session.lock().expect("remote_session") = None;
        *self.shared.remote_session_keymap.lock().expect("rs_keymap") = None;
        // Session-scoped screen consent: the accept path may have auto-allowed
        // streaming — pull it back so the peer cannot re-dial 45720 after the
        // session without a fresh grant. An operator allow granted before the
        // session is left untouched. The 45720 channel survives the Close
        // (remote_session_start reuses it across sessions), so the revocation
        // must NOT use `on_peer_gone`: that resets `next_seq` to 1 while the
        // still-attached receiver keeps `last_seq`, and leaves the engine in
        // Idle where `allow()` is a no-op and every later Start is refused.
        // `revoke()` lands in `Requested` and keeps the sequence monotonic;
        // genuine channel death is still reset by the gen-guarded loop-exit
        // `on_peer_gone`.
        if self
            .shared
            .remote_session_auto_allowed
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.shared.screen_engine.lock().expect("screen").revoke();
            let state = self.screen_state();
            let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
            // The channel typically survives the session — report the real
            // peer so the UI does not show a viewer-less state while the
            // 45720 connection is still open.
            let peer = self
                .shared
                .screen_peer
                .lock()
                .expect("screen_peer")
                .as_ref()
                .map(|p| p.to_string());
            self.emit(UiEvent::screen_state_changed(state.as_str(), allowed, peer));
        }
        if state.role.is_receiving() {
            self.exit_session();
        } else {
            // Controller side: drop the screen channel with the session.
            self.screen_stop().await.ok();
        }
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::input_tunnel_session_closed(
            &state.session_id,
            &state.peer_id,
            &alias,
            InputTunnelValidator::sanitize_reason(&frame.reason),
            now_ms,
        ));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "remote_session_closed",
            Some(state.peer_id.clone()),
            format!(
                "Remote session {} closed: {}",
                state.session_id,
                InputTunnelValidator::sanitize_reason(&frame.reason)
            ),
            true,
        );
    }

    /// Refresh session liveness. Called by the controller's heartbeat loop.
    pub fn touch_remote_session(&self, session_id: &str) {
        let now_ms = self.shared.clock.unix_ms();
        let mut guard = self.shared.remote_session.lock().expect("remote_session");
        if let Some(s) = guard.as_mut() {
            if s.session_id == session_id {
                s.state.last_activity_ms = now_ms;
            }
        }
    }

    /// Idle-timeout sweep, mirrors `input_tunnel_status` semantics. Returns
    /// `true` when a session was force-closed.
    pub fn remote_session_idle_check(&self) -> bool {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.input_policy();
        let expired = {
            let guard = self.shared.remote_session.lock().expect("remote_session");
            match guard.as_ref() {
                Some(s) => {
                    policy.idle_timeout_secs != 0
                        && now_ms.saturating_sub(s.last_activity_ms)
                            > (policy.idle_timeout_secs as u64) * 1000
                }
                None => false,
            }
        };
        if !expired {
            return false;
        }
        let state = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .clone();
        if let Some(s) = state {
            let peer = DeviceId::parse(&s.peer_id).ok();
            *self.shared.remote_session.lock().expect("remote_session") = None;
            *self.shared.remote_session_keymap.lock().expect("rs_keymap") = None;
            if self
                .shared
                .remote_session_auto_allowed
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                // Same rationale as the Close handler: the 45720 channel
                // survives the session, so revoke to `Requested` (keep seq
                // monotonic) instead of the channel-death `on_peer_gone`.
                self.shared.screen_engine.lock().expect("screen").revoke();
                let state = self.screen_state();
                let allowed = matches!(state, ScreenState::Allowed | ScreenState::Streaming);
                let live_peer = self
                    .shared
                    .screen_peer
                    .lock()
                    .expect("screen_peer")
                    .as_ref()
                    .map(|p| p.to_string());
                self.emit(UiEvent::screen_state_changed(
                    state.as_str(),
                    allowed,
                    live_peer,
                ));
            }
            if s.role.is_receiving() {
                self.exit_session();
            }
            if let Some(peer) = peer {
                let close = RemoteSessionCloseFrame {
                    session_id: s.session_id.clone(),
                    reason: "idle timeout exceeded".to_string(),
                    timestamp_ms: now_ms,
                };
                if let Ok(inner) = close.to_frame() {
                    let node = self.clone_handle();
                    tokio::spawn(async move {
                        let _ = node.send_to(&peer, inner).await;
                    });
                }
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "remote_session_idle_timeout",
                Some(s.peer_id.clone()),
                format!("Remote session {} closed due to idle timeout", s.session_id),
                true,
            );
            return true;
        }
        false
    }

    /// Current remote session snapshot for status/REST consumers.
    pub fn remote_session_status(&self) -> Option<ActiveRemoteSessionState> {
        self.shared
            .remote_session
            .lock()
            .expect("remote_session")
            .as_ref()
            .map(|s| s.state.clone())
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
            // An engine seat parked in Remote IS active use, even when not a
            // single event flows: the operator is sitting on the remote
            // machine (Deskflow model — you stay controlling until you push
            // back). Reaping a roaming stint here didn't just drop a slot —
            // the 1s watcher immediately re-armed a fresh session, each re-arm
            // ships a same-peer takeover request, and the target's
            // polite-takeover branch DEMOTED the live stint (production
            // 2026-11-13, v0.2.27: perpetual started→idle_timeout→started
            // loop every ~11s in the audit trail, keyboard dead mid-stint;
            // the strandings then rescue the seat and the operator is bounced
            // to the local machine). The idle clock therefore governs
            // LEFTOVER slots (seat Local); a roaming stint ends through the
            // roaming machinery (push-back / Leave / rescue), not the sweep.
            // Stranded Remote seats are still cleaned by the watcher's
            // reconcile (it keys on channel state, not this slot).
            let roaming = self.input_engine_seat() == CursorSeat::Remote;
            if policy.idle_timeout_secs == 0 || roaming {
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

        // Pre-arm BEFORE the request hits the wire. The comment above the
        // callee always promised this ("pre-arms the controller session
        // before the request is answered"), but the arm actually happened
        // after `send_to` — on a loaded runtime the peer's ACCEPTED resp can
        // be processed before this task resumes, and `receive_input_tunnel_resp`
        // drops a resp that finds no armed session, dead-ending the takeover
        // (m3_11, ubuntu CI 2026-11-11: server=Some(Target), client=None).
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
                pre_arm_epoch: next_pre_arm_epoch(),
            });
        }
        // Never reuse an existing outbound channel across an attach: on a
        // takeover the demoted peer tears its side of the old connection
        // down (input_stop), so the previous channel is dying by design —
        // reusing it would leave this controller with a dead channel and no
        // rebuild. Drop it and dial fresh.
        //
        // v0.2.32: dropped BEFORE the request hits the wire. The peer's
        // ACCEPT resp can be processed while this task is still inside
        // `send_to`; the resp path then dials channel gen N, and a drop
        // placed after the send threw that fresh channel away and forced a
        // second dial (gen N+1) on the first drive — production trace
        // showed two `channel_open` per takeover and a stalled first entry.
        *self.shared.input_out.lock().expect("input_out") = None;
        let inner = req_frame.to_frame()?;
        tracing::info!(session_id = %session_id, peer = %peer_str, "input tunnel attach: controller session pre-armed");
        eprintln!("[takeover] attach: pre-armed session={session_id} peer={peer_str}");
        if let Err(e) = self.send_to(&peer_id, inner).await {
            // Roll the pre-arm back — no request on the wire, no grant.
            warn!(session_id = %session_id, peer = %peer_str, %e, "input tunnel attach: send failed; pre-arm rolled back");
            eprintln!("[takeover] attach: SEND FAILED, rolled back session={session_id} err={e}");
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            if guard.as_ref().is_some_and(|s| s.session_id == session_id) {
                *guard = None;
            }
            return Err(CoreError::InvalidPayload(format!(
                "failed to send input tunnel request to peer: {e}"
            )));
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
        // A NEW controller session must not inherit engine state from the
        // previous stint. A leftover Remote seat (a takeover demotes this
        // node mid-roam without resetting the seat) makes attach_input_server
        // replay the stale rem via resume_remote and permanently blocks the
        // fresh edge crossing — production showed the cursor emerging
        // mid-screen instead of at the mapped entry. Frames buffered across
        // the session boundary belong to the dead session too.
        {
            let mut engine = self.shared.input_engine.lock().expect("engine");
            if engine.seat() == CursorSeat::Remote {
                engine.clear_stale_remote();
                self.shared
                    .input_buffered_frames
                    .lock()
                    .expect("buffered")
                    .clear();
                *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                self.emit(UiEvent::input_seat_changed(
                    "local",
                    true,
                    Some(peer_str.clone()),
                ));
                info!(%peer_id, "tunnel attach: cleared stale remote seat; next edge push crosses fresh");
            }
        }
        if let Err(e) = self.open_input(&peer_id).await {
            // A takeover demotes the peer right as this dial races its
            // teardown; a single failure would leave a LIVE session with a
            // DEAD channel until the next watcher tick — every edge push in
            // that window is silently eaten ("occasionally can't cross").
            // Retry briefly before giving up; the watcher remains the
            // backstop.
            let mut last_err = Some(e);
            for _ in 0..3u8 {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                match self.open_input(&peer_id).await {
                    Ok(()) => {
                        last_err = None;
                        break;
                    }
                    Err(e) => last_err = Some(e),
                }
            }
            if let Some(e) = last_err {
                warn!(%peer_id, %e, "input channel open during tunnel attach failed after retries");
            }
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

        let session = self.release_input_tunnel_local(&sanitized_reason);

        if let Some(session) = session {
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

    /// Synchronously end the active input tunnel grant (if any) and restore
    /// the local seat. No frame is sent to the peer — callers that must
    /// notify the peer use the async [`Self::input_tunnel_release`]; the
    /// session-end paths (engine return, inbound Leave, peer gone, input
    /// stop) call this one because they run on sync hot paths and the peer
    /// learns of the end through its own Leave/release handling.
    ///
    /// Production 2026-11-11: the target's tunnel survived the control
    /// session's end (edge return processed the Leave, seat went Local, but
    /// `active_input_tunnel` kept role Target). Every local mouse event was
    /// then routed into the reverse-takeover arming branch — the normal
    /// edge-crossing path never ran and Windows could not cross into the
    /// Mac until the tunnel was manually released.
    pub fn release_input_tunnel_local(&self, reason: &str) -> Option<ActiveInputTunnelSession> {
        let session = {
            let mut guard = self
                .shared
                .active_input_tunnel
                .lock()
                .expect("active_input_tunnel");
            guard.take()
        };
        let session = session?;
        eprintln!(
            "[takeover] release: tunnel grant released reason={reason} role={:?} peer={}",
            session.role, session.peer_id
        );
        self.trace_input(
            "release",
            format!(
                "{reason} role={:?} peer={} session={}",
                session.role, session.peer_id, session.session_id
            ),
        );

        let now_ms = self.shared.clock.unix_ms();
        let sanitized_reason = InputTunnelValidator::sanitize_reason(reason);
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        // The peer screen belongs to this grant; the next peer may differ.
        self.shared
            .input_engine
            .lock()
            .expect("engine")
            .set_remote_screen(None);
        if session.role == InputTunnelRole::Controller {
            // v0.2.37: a grant release (takeover demotion, operator stop)
            // ends a stint the edge return never saw; restore the cursor
            // the same way the return does.
            self.trace_input("cursor_restore", format!("grant released: {reason}"));
        }
        self.disarm_input_capture();
        crate::platform::restore_arrow_cursor();
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

        Some(session)
    }

    /// Returns the current input tunneling and virtual peripheral sharing policy.
    pub fn input_policy(&self) -> InputTunnelPolicy {
        self.shared
            .input_policy
            .lock()
            .expect("input_policy")
            .clone()
    }

    /// Hot-path accessors: read a single policy field without deep-cloning
    /// the whole struct (Vec<String> allowlists + Options) per input event.
    /// Callers on the per-move / per-key / per-wheel paths MUST use these
    /// instead of `input_policy()` — see the no-policy-clone invariant noted
    /// in `maybe_auto_reverse`.
    fn policy_edge_str(&self) -> Option<String> {
        self.shared
            .input_policy
            .lock()
            .expect("input_policy")
            .edge
            .clone()
    }

    fn policy_keymap_name(&self) -> Option<String> {
        // ADR-138: while a remote session is active on the controller side,
        // the host's suggested preset (from the session resp) wins over the
        // local operator policy — the target decided how its keys translate.
        if self.remote_session_active() {
            if let Some(km) = self
                .shared
                .remote_session_keymap
                .lock()
                .expect("rs_keymap")
                .clone()
            {
                return Some(km);
            }
        }
        self.shared
            .input_policy
            .lock()
            .expect("input_policy")
            .keymap
            .clone()
    }

    fn policy_wheel_speed(&self) -> f64 {
        self.shared
            .input_policy
            .lock()
            .expect("input_policy")
            .wheel_speed
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
            if let Err(e) = self.send_input_events(events, "keymap_refresh") {
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
        // A channel that still has a sender but no live reader (a takeover
        // demolished the peer side, the dial task died late, TCP went
        // half-open) makes `input_out.is_some()` a lie: every crossing's
        // Enter+Move lands in the mpsc buffer and nothing ever reaches the
        // peer — the "black hole" seen in production. Probe with a harmless
        // ClipHint (the client counts it without touching the cursor) and
        // rebuild on a dead queue instead of trusting the slot.
        if self.input_channel_open_sync() {
            let probe_alive = {
                let probe = {
                    let mut engine = self.shared.input_engine.lock().expect("engine");
                    let seq = engine.next_seq();
                    engine.note_seq(seq);
                    InputEvent::clip_hint(seq).to_frame()
                };
                match probe {
                    Ok(frame) => {
                        let tx_opt = self
                            .shared
                            .input_out
                            .lock()
                            .expect("input_out")
                            .as_ref()
                            .map(|(_, tx)| tx.clone());
                        match tx_opt {
                            Some(tx) => !matches!(
                                tx.try_send(frame),
                                Err(mpsc::error::TrySendError::Closed(_))
                            ),
                            None => false,
                        }
                    }
                    Err(_) => false,
                }
            };
            if probe_alive {
                return Ok(());
            }
            warn!("input channel probe found a dead queue; rebuilding");
            *self.shared.input_out.lock().expect("input_out") = None;
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
    /// the channel. The mouse capture hook stays installed (it is cheap and
    /// needed for edge-crossing detection while local), but the REMOTE-seat
    /// arming must be released immediately: on macOS `arm_input_capture` is
    /// what disassociated the hardware mouse and hid the cursor behind
    /// shields, so leaving it armed after a takeover demotion keeps this
    /// machine in "controlling someone" state (hidden cursor, unassociated
    /// mouse) while it is now the controlled target. The capture worker's
    /// tail only realigns armed-state on the next LOCAL physical event,
    /// which may never come while injected remote moves pass the tap.
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
        // An operator-ordered stop ends the tunnel grant too — a stale
        // Target role would keep hijacking local mouse events into the
        // reverse-arming branch after the stop (production 2026-11-11).
        self.release_input_tunnel_local("input stopped");
        self.disarm_input_capture();
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

    /// v0.2.33: Windows hides the system cursors BEFORE the center park
    /// warp and restores them only AFTER the return warp, so the arrow is
    /// never painted at screen center. macOS keeps its proven order (warp,
    /// then dissociate+hide; re-associate, then warp home).
    const CURSOR_HIDE_AROUND_WARPS: bool = cfg!(windows);

    /// Seat moved to Remote: park the physical cursor and arm capture.
    /// `hide_first` hides the local cursor before the park warp.
    fn enter_remote_seat_capture(&self, park_x: i32, park_y: i32, hide_first: bool) {
        if hide_first {
            self.trace_input("cursor_hide", "before park warp");
            if let Ok(guard) = self.shared.input_hook_control.lock() {
                if let Some(ref control) = *guard {
                    control.hide_cursor();
                }
            }
        }
        tracing::info!(park_x, park_y, "park warp before remote roaming");
        self.trace_input("park_warp", format!("({park_x},{park_y})"));
        crate::platform::warp_mouse_position(park_x, park_y);
        if !hide_first {
            self.trace_input("cursor_hide", "with capture arm");
        }
        self.arm_input_capture();
    }

    /// Seat returned to Local: disarm capture and put the cursor back at
    /// the engine's local position. `restore_last` warps home before the
    /// cursor is shown again.
    fn leave_remote_seat_capture(&self, x: i32, y: i32, restore_last: bool) {
        if restore_last {
            self.trace_input("return_warp", format!("({x},{y})"));
            crate::platform::warp_mouse_position(x, y);
            self.trace_input("cursor_restore", "after return warp");
            self.disarm_input_capture();
        } else {
            self.trace_input("cursor_restore", "before return warp");
            self.disarm_input_capture();
            self.trace_input("return_warp", format!("({x},{y})"));
            crate::platform::warp_mouse_position(x, y);
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
                                    let _ = node.send_input_events(events, "hook_rescue");
                                    *node.shared.input_seat.lock().expect("seat") =
                                        Some(CursorSeat::Local);
                                    node.release_input_tunnel_local("emergency rescue to local");
                                    node.disarm_input_capture();
                                    crate::platform::warp_mouse_position(cx, cy);
                                    node.emit(UiEvent::input_seat_changed("local", false, None));
                                }
                            }
                            // Deskflow MSWindowsComputer::onMouseMove
                            // alignment: while roaming Remote on the Windows
                            // hook-diff pipeline, pull the physical cursor
                            // back to screen center after every consumed
                            // motion batch, so the delta chain — anchored at
                            // the warp target by the echo gate — always
                            // measures genuine hand motion from center and
                            // can never saturate at a screen boundary (the
                            // controller hand and the remote cursor own
                            // independent positions). The entry batch is
                            // covered by the park warp in
                            // `drive_input_delta`; the decision method is
                            // seat-scoped (Remote-only) and the skip below
                            // only short-circuits a redundant same-position
                            // warp on the entry batch itself.
                            #[cfg(windows)]
                            if node.input_engine_seat() == CursorSeat::Remote {
                                if let Some((rx, ry)) = node.roaming_recenter_target(w, h) {
                                    crate::platform::warp_mouse_position(rx, ry);
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
                        crate::platform::RawInput::GestureSwipe { direction } => {
                            let _ = node.drive_gesture_swipe(direction);
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
                            let _ = node.send_input_events(events, "guard_force_local");
                            *node.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
                            node.release_input_tunnel_local("emergency rescue to local");
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

    /// Watcher reconciliation for a stranded remote seat (1s tick, no user
    /// input required).
    ///
    /// Production 2026-11-11: after a control stint ended, the peer pushed
    /// its OWN edge crossing (reverse roaming) — but the persistent channel
    /// belonged to the old controller's loop, so this node had no
    /// `input_out`: the push frames buffered into the 2.5s grace window and
    /// then the user's hand stopped. No further local events means the
    /// drive-path guard never runs again: the engine sat Remote with
    /// MOUSE_RELAY armed, swallowing the physical mouse indefinitely
    /// ("Windows 鼠标都动不了了"). Force the seat back to local and let the
    /// watcher's reconnect dial rebuild the channel; the arming alignment
    /// loop revives the mouse on the next tick.
    ///
    /// A genuinely controlled target is never rescued: its engine
    /// legitimately rides Remote while the peer drives it (role receiving /
    /// active session). A pending takeover is protected by the grace
    /// deadline check — its ACCEPT resp re-places the seat.
    /// Pull a stranded Remote seat back to Local: engine force_local + seat
    /// write + capture disarm + cursor warp + UI event. Shared by every
    /// rescue path (channel-closed guard, ghost-roam guard, send-failure,
    /// watcher reconcile) so a change to the rescue semantics can never be
    /// applied to one path and missed by another (m6_10 lesson).
    ///
    /// `release_grant` is ONLY for the watcher reconcile: it already proved
    /// the grant is unrescuable (no channel, no grace, no receiving role).
    /// The drive-path rescues must keep a pending takeover's pre-armed grant
    /// alive — the peer's late ACCEPT still matches it (CI m3_11); releasing
    /// there dead-ends the takeover.
    fn rescue_seat_to_local(&self, release_grant: bool) -> (i32, i32) {
        let mut engine = self.shared.input_engine.lock().expect("engine");
        engine.force_local();
        let (cx, cy) = engine.local_pos();
        *self.shared.input_seat.lock().expect("seat") = Some(CursorSeat::Local);
        if release_grant {
            self.release_input_tunnel_local("stranded remote seat rescued");
        }
        self.leave_remote_seat_capture(cx, cy, Self::CURSOR_HIDE_AROUND_WARPS);
        self.emit(UiEvent::input_seat_changed("local", false, None));
        self.trace_input(
            "rescue",
            format!("seat forced local release_grant={release_grant} park=({cx},{cy})"),
        );
        (cx, cy)
    }

    /// CAS the ghost-roam attach mutex. `true` means the caller owns the
    /// attach and MUST clear `input_attaching` when its spawned task
    /// settles; `false` means another attach (drive kick or watcher
    /// auto-attach) is already in flight — skip, its pre-arm will serve.
    fn kick_attach(&self, _peer: &DeviceId) -> bool {
        self.claim_attach()
    }

    /// Watcher-side attach claim (pub wrapper; same slot as the drive kick).
    pub fn watcher_attach_claim(&self, _peer: &DeviceId) -> bool {
        self.claim_attach()
    }

    /// Release the watcher-side attach claim after its awaited attach.
    pub fn watcher_attach_clear(&self) {
        self.shared
            .input_attaching
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    fn claim_attach(&self) -> bool {
        self.shared
            .input_attaching
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
    }

    pub fn reconcile_stranded_remote_seat(&self) -> bool {
        if self.input_engine_seat() != CursorSeat::Remote {
            return false;
        }
        // A live Controller grant means roaming is authorized and the
        // remote seat is legitimate. The persistent channel survives
        // session releases, so "channel open" alone is NOT that: the
        // ghost-roam freeze (production 2026-11-11 23:47, both engines
        // Remote, frames flowing, no session anywhere, both physical mice
        // dead until an operator stop) passed exactly this check. Skip
        // only when the grant exists; rescue open-channel-without-grant.
        if self.input_channel_open_sync()
            && self.active_tunnel_role() == Some(tetherly_core::InputTunnelRole::Controller)
        {
            return false;
        }
        let in_grace = {
            let dl = self
                .shared
                .input_reconnect_deadline
                .lock()
                .expect("deadline");
            dl.map(|d| std::time::Instant::now() < d).unwrap_or(false)
        };
        if in_grace {
            return false;
        }
        if self.remote_session_active()
            || self.active_tunnel_role().is_some_and(|r| r.is_receiving())
        {
            return false;
        }
        let (cx, cy) = self.rescue_seat_to_local(true);
        crate::platform::warp_mouse_position(cx, cy);
        self.emit(UiEvent::input_seat_changed("local", false, None));
        tracing::info!(
            "watcher: stranded remote seat rescued (remote seat with no outbound channel and no grace)"
        );
        self.trace_input("reconcile", "watcher rescued stranded remote seat");
        true
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

    /// Resolve the peer a self-heal dial should target. Same priority chain
    /// as the watcher reconnect: the tunnel session's peer, then the
    /// topology neighbor on our configured edge, then the single live peer.
    fn reconnect_target_peer(&self) -> Option<DeviceId> {
        if let Some(s) = self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel")
            .as_ref()
        {
            if let Ok(id) = DeviceId::parse(&s.peer_id) {
                return Some(id);
            }
        }
        let live = self.shared.live.lock().expect("live");
        if let Some(edge) = self
            .input_policy()
            .edge
            .as_deref()
            .and_then(|e| e.parse::<tetherly_core::DisplayEdge>().ok())
        {
            let layout = self.shared.topology_layout.lock().expect("topology_layout");
            if let Some(n) = layout.neighbors.get(&edge) {
                if let Ok(id) = DeviceId::parse(&n.peer_id) {
                    if live.contains_key(&id) {
                        return Some(id);
                    }
                }
            }
        }
        let mut peers: Vec<String> = live.keys().map(|d| d.to_string()).collect();
        peers.sort();
        if peers.len() == 1 {
            DeviceId::parse(&peers.pop().expect("single peer")).ok()
        } else {
            None
        }
    }

    /// Clear all distance arming, dwell, and movement tracking state for auto-reverse.
    fn reset_reverse_arming_state(&self) {
        self.shared.reverse_move_accum_px.store(0, Ordering::SeqCst);
        // v0.2.27 zone gate: a fresh Target session starts with the cursor
        // parked at the entry edge (inside the reverse-trigger zone), so the
        // gate closes until an event proves the cursor entered the screen.
        self.shared
            .reverse_zone_dirty
            .store(false, Ordering::SeqCst);
        self.shared
            .reverse_local_owner
            .store(false, Ordering::SeqCst);
        *self.shared.reverse_last_move.lock().expect("reverse_move") = None;
        *self.shared.reverse_last_pos.lock().expect("reverse_pos") = None;
        *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
        *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
        *self
            .shared
            .reverse_pinned_touch
            .lock()
            .expect("reverse_pinned") = None;
    }

    /// The controller's Enter was applied: its first Move parks the cursor
    /// on this screen's shared edge (inside the reverse-trigger zone), so
    /// every arming state restarts and the zone gate closes until a
    /// physical event proves the cursor moved into the screen (v0.2.27
    /// semantics, anchored at the Enter instead of the accept since
    /// v0.2.31 — a pre-arm accept has no parked cursor yet).
    pub(crate) fn note_controller_entered(&self) {
        // v0.2.34: the controller's injected events bypass the local tap,
        // so the idle hide from the last Leave is revealed here.
        crate::platform::reveal_idle_hidden_cursor();
        // v0.2.37: a controller driving this screen means this node is not
        // roaming anywhere — drop any capture a takeover demotion left armed
        // and force the arrow image (a cursor shield's blank NSCursor
        // survived the demotion, so the Windows-driven cursor entered the
        // Mac menu bar invisible; production 2026-11-13 18:34:53).
        self.trace_input("cursor_reveal", "controller entered");
        self.disarm_input_capture();
        crate::platform::restore_arrow_cursor();
        self.reset_reverse_arming_state();
        self.trace_input("zone_gate", "close: controller entered");
    }

    /// The controller's Leave was applied: the cursor on this screen is the
    /// local user's again. The grant release above has already reset the
    /// arming state (closing the gate); re-open it so a push at the edge
    /// where the stint ended can arm a takeover without a detour into the
    /// interior first.
    pub(crate) fn note_controller_left(&self) {
        self.shared.reverse_zone_dirty.store(true, Ordering::SeqCst);
        self.trace_input("zone_gate", "open: controller left");
        self.shared
            .reverse_local_owner
            .store(true, Ordering::SeqCst);
        self.trace_input("reverse_owner", "local: controller left");
    }

    /// v0.2.34: a Target parks its cursor at screen center when the
    /// controller leaves, instead of leaving it on the shared edge. Mac only:
    /// the Mac had no hidden-cursor handoff, so it left a visible leftover
    /// pointer, and that pinned clamp-band spot let one stray zero-delta
    /// touch complete the local-owner short dwell and steal the controller
    /// role (production 2026-11-13 16:01:51.14). Windows stays off: its
    /// cursor is the user's primary pointer.
    const TARGET_PARK_ON_LEAVE: bool = cfg!(target_os = "macos");

    /// Warp the cursor to the center of `screen`, drop every pinned-edge
    /// reverse anchor, and (when the local hook is live to reveal it on the
    /// next physical motion) hide the cursor until the local user moves.
    /// Returns the park point, or `None` when disabled.
    pub(crate) fn park_target_cursor_after_leave(
        &self,
        enabled: bool,
        screen: (i32, i32),
    ) -> Option<(i32, i32)> {
        if !enabled || screen.0 <= 0 || screen.1 <= 0 {
            return None;
        }
        let (cx, cy) = (screen.0 / 2, screen.1 / 2);
        *self
            .shared
            .reverse_pinned_touch
            .lock()
            .expect("reverse_pinned") = None;
        *self.shared.reverse_last_pos.lock().expect("reverse_pos") = None;
        *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
        *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
        self.trace_input("target_park", format!("({cx},{cy}) controller left"));
        crate::platform::warp_mouse_position(cx, cy);
        let hook_live = self
            .shared
            .input_hook_control
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if hook_live {
            self.trace_input("cursor_idle_hide", "until local motion");
            crate::platform::hide_cursor_until_local_motion();
        }
        Some((cx, cy))
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
        //
        // ADR-138: while a remote desktop SESSION is driving this node
        // (SessionTarget), even the deliberate edge-push takeover is
        // suppressed — the session window is the only control surface until
        // it closes (m6_3 asserts the watcher skip too). The CONTROLLER side
        // must NOT be suppressed: its physical deltas are the session's
        // input stream (seat==Remote forwards them over 45719). An
        // unconditional `remote_session_active()` check here dead-ended the
        // controller's MOUSE while its keyboard (drive_input_key, which
        // never had this guard) kept working — the "Windows mouse cannot
        // control the Mac" session bug.
        let session_drives_this_node = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .as_ref()
            .is_some_and(|s| s.role.is_receiving());
        if session_drives_this_node
            || self.active_tunnel_role() == Some(tetherly_core::InputTunnelRole::Target)
        {
            if session_drives_this_node {
                return Ok(None);
            }
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
            {
                let remote = self.input_engine_seat() == CursorSeat::Remote;
                if remote {
                    // Transient, NOT a session end: a pending takeover's
                    // pre-armed grant must survive (CI m3_11).
                    self.rescue_seat_to_local(false);
                }
            }
            // Self-heal: the watcher reconnects on its own tick, but a push
            // landing inside that gap is silently eaten — the user experiences
            // "occasionally can't cross". Kick a dial right here (deduped by
            // the input_dialing CAS inside open_input) so the NEXT push
            // crosses instead of vanishing.
            if !self.shared.input_dialing.load(Ordering::SeqCst) {
                let peer = self.reconnect_target_peer();
                if let Some(peer) = peer {
                    let rt = tokio::runtime::Handle::try_current()
                        .ok()
                        .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
                    if let Some(handle) = rt {
                        let node = self.clone_handle();
                        handle.spawn(async move {
                            if let Err(e) = node.open_input(&peer).await {
                                tracing::debug!(%peer, %e, "crossing-guard self-heal dial failed");
                            }
                        });
                    }
                }
            }
            return Ok(None);
        }

        // Ghost-roam guard: a channel WITHOUT a Controller grant must not
        // enter remote roaming. The persistent channel survives session
        // releases, so after the peer's Leave (or any grant end) the next
        // physical edge push would otherwise cross into Remote with no
        // authorization: the physical mouse dissociates, deltas ship to the
        // peer over the leftover channel, and the watcher reconcile skipped
        // rescuing because "the channel is open" (production 2026-11-11
        // 23:47:46, both engines stranded Remote, both mice frozen).
        //
        // The missing grant is repaired HERE, on the push itself, the same
        // way the (production-dead) evaluate_cursor_crossing auto-link used
        // to: spawn input_tunnel_attach. It pre-arms the Controller grant
        // synchronously before its first await, so the very next event
        // passes this guard and roams; this push stays local — one dead
        // push instead of an unauthenticated roam or a freeze.
        //
        // ADR-138 session exemption: a session this node CONTROLS (role not
        // receiving) is its own authorization — the session controller's
        // deltas are the session's input stream (m6_10: guarding them
        // dead-ended the session mouse, the "Windows mouse cannot control
        // the Mac" bug class).
        let session_drives_this_node_ctrl = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .as_ref()
            .is_some_and(|s| !s.role.is_receiving());
        if !in_grace
            && !session_drives_this_node_ctrl
            && self.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Controller)
        {
            {
                if self.input_engine_seat() == CursorSeat::Remote {
                    // Transient, NOT a session end — and this branch only
                    // runs when no grant exists at all.
                    self.rescue_seat_to_local(false);
                }
            }
            // Attach-kick debounce: the hook feeds this path up to 1000 Hz.
            // One kick per second is plenty (the pre-arm lands in ms) and a
            // storm would overwrite the just-pre-armed session — the resp of
            // a dropped pre-arm dead-ends the takeover (CI m3_11 class).
            // Monotonic Instant, not wall clock: an NTP step must not stall
            // the rescue.
            let now = std::time::Instant::now();
            let due = {
                let mut last = self.shared.input_attach_kick_at.lock().expect("kick_at");
                let due = last
                    .map(|t| now.duration_since(t) > std::time::Duration::from_millis(1000))
                    .unwrap_or(true);
                if due {
                    *last = Some(now);
                }
                due
            };
            if due {
                if let Some(peer) = self.reconnect_target_peer() {
                    if self.kick_attach(&peer) {
                        self.trace_input(
                            "attach_kick",
                            format!("ghost-roam guard pre-arm -> {peer}"),
                        );
                        let rt = tokio::runtime::Handle::try_current()
                            .ok()
                            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
                        if let Some(handle) = rt {
                            let node = self.clone_handle();
                            handle.spawn(async move {
                                if let Err(e) = node
                                    .input_tunnel_attach(&peer.to_string(), None, None, None)
                                    .await
                                {
                                    tracing::debug!(%peer, %e, "ghost-roam guard attach kick failed");
                                }
                                node.shared
                                    .input_attaching
                                    .store(false, std::sync::atomic::Ordering::SeqCst);
                            });
                        }
                    }
                }
            }
            return Ok(None);
        }

        let (edge, events, current_seat, local_pos, _active_edge) = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            server.resize(width, height);
            let active_edge = if let Some(e) = self.policy_edge_str().as_deref() {
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
            // Speed-aware Top/Bottom redundancy band (macOS menu bar /
            // Windows task bar): mirror the persisted topology config into
            // the engine on every event (cheap copy; keeps the engine
            // current with live config edits).
            {
                let cfg = self
                    .shared
                    .topology_layout
                    .lock()
                    .expect("topology_layout")
                    .config;
                server.set_tb_redundancy_band(
                    cfg.redundancy_band_px,
                    cfg.slow_dwell_ms,
                    cfg.fast_push_delta_px,
                );
                server.set_entry_return_grace(std::time::Duration::from_millis(
                    cfg.entry_return_grace_ms as u64,
                ));
                server.set_remote_speed_percent(cfg.remote_speed_percent);
            }
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
            // Diagnostic (v0.1.81): one line per actual edge crossing — the
            // hook x and the derived rem prove the proportional mapping at
            // the source.
            if let Some(e) = edge {
                tracing::info!(
                    hook_x = x, hook_y = y, edge = ?e,
                    "engine crossing fired"
                );
                let (rx, ry) = server.remote_pos();
                self.trace_input(
                    "crossing",
                    format!("edge={e:?} hook=({x},{y}) delta=({dx},{dy}) rem=({rx},{ry})"),
                );
            }
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
                // Deskflow OSXScreen::leave & MSWindowsComputer::leave
                // alignment — park the physical cursor at screen CENTER on
                // entering Remote, on every platform:
                // 1) macOS: WindowServer enforces cursor visibility over the
                //    Menu Bar (y = 0) and Dock (y = height - 1), refusing
                //    hide requests at display boundaries; center sits in
                //    neutral content space.
                // 2) Windows: per-batch roaming recenter anchors the delta
                //    chain at center anyway, and a center park means the
                //    entry state equals the steady state (no first-batch
                //    pull-off); parking at the shared edge instead is what
                //    stranded the delta chain at saturation (v0.2.8 limit
                //    cycle).
                // 3) Warp BEFORE arming input capture / dissociating — but
                //    on Windows hide the system cursors FIRST (v0.2.33), or
                //    the arrow paints at center for the ~6ms the hide takes.
                self.enter_remote_seat_capture(
                    width / 2,
                    height / 2,
                    Self::CURSOR_HIDE_AROUND_WARPS,
                );
            } else if !self.is_keyboard_armed() {
                self.arm_input_capture();
            }
        }

        if let Err(e) = self.send_input_events(events, "drive_delta") {
            let in_grace = {
                let dl = self
                    .shared
                    .input_reconnect_deadline
                    .lock()
                    .expect("deadline");
                dl.map(|d| std::time::Instant::now() < d).unwrap_or(false)
            };
            if !in_grace {
                if self.input_engine_seat() == CursorSeat::Remote {
                    // Transient, NOT a session end: a pending takeover's
                    // pre-armed grant must survive (CI m3_11).
                    self.rescue_seat_to_local(false);
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
            self.trace_input(
                "seat",
                format!(
                    "{} -> {seat_str} peer={}",
                    match prev_seat {
                        Some(CursorSeat::Remote) => "remote",
                        Some(CursorSeat::Local) => "local",
                        None => "none",
                    },
                    peer.as_deref().unwrap_or("none")
                ),
            );
            self.emit(UiEvent::input_seat_changed(seat_str, true, peer));
            if current_seat == CursorSeat::Local {
                // The roaming control stint is over (sustained-push return);
                // the tunnel grant must die with it or the stale Target role
                // hijacks every local mouse event into the reverse-arming
                // branch and the normal edge crossing never runs again
                // (production 2026-11-11).
                //
                // ONLY a stint this node STARTED with its own drive-path
                // crossing counts. While being controlled, the target's
                // engine also sits in Remote (`resume_remote`), and the
                // takeover attach opens the outbound channel before the resp
                // lands — an engine oscillation there passed the channel
                // check and killed the just-pre-armed grant (CI m3_11
                // 2026-11-11: "roaming returned to local seat" right after
                // "attach: pre-armed").
                let own_stint = self
                    .shared
                    .roaming_own_crossing
                    .swap(false, std::sync::atomic::Ordering::SeqCst);
                if own_stint && self.input_channel_open_sync() {
                    self.release_input_tunnel_local("roaming returned to local seat");
                }
                self.leave_remote_seat_capture(
                    local_pos.0,
                    local_pos.1,
                    Self::CURSOR_HIDE_AROUND_WARPS,
                );
            } else {
                // The drive path crossed INTO remote roaming: this node
                // started a controller stint with its own edge crossing.
                // Marks the engine return as a genuine stint end.
                self.shared
                    .roaming_own_crossing
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                // Becoming the controlled side: reset the push-back arming so
                // the next reverse takeover needs a fresh deliberate push.
                self.reset_reverse_arming_state();
            }
        }
        // Activity accounting moved into `send_input_events` so every input
        // kind (not just mouse moves) counts as operator activity.
        Ok(edge)
    }

    pub fn drive_input_key(&self, code: u16, down: bool) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            let peer_platform = self.input_peer_platform();
            // Hot path: derive the profile from just the keymap name instead of
            // deep-cloning the whole policy struct per keystroke.
            let keymap_name = self.policy_keymap_name();
            let probe = InputTunnelPolicy {
                keymap: keymap_name,
                ..InputTunnelPolicy::default()
            };
            let keymap = probe.resolved_keymap_profile(peer_platform.as_deref());
            let remapped_code = keymap.remap_vk(code);
            let seq = server.next_seq();
            server.note_seq(seq);
            vec![InputEvent::key(seq, remapped_code, down, 0)]
        };
        self.send_input_events(events, "drive_key")
    }

    pub fn drive_gesture_swipe(&self, direction: i8) -> Result<(), CoreError> {
        const GESTURE_LOG_THROTTLE_MS: u64 = 30_000;

        // The gesture the tap observed, in the same content-follows-fingers
        // convention the platform layer documents. `direction` 0 is the
        // vertical (Mission Control) swipe.
        let gesture = match direction {
            0 => tetherly_core::gesture_map::GestureId::Swipe4Up,
            d if d < 0 => tetherly_core::gesture_map::GestureId::Swipe4Left,
            _ => tetherly_core::gesture_map::GestureId::Swipe4Right,
        };

        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            // Seat Remote alone is not enough: the CONTROLLED side is also in
            // the Remote seat. Suppress only the role where THIS node is the
            // one being driven — an ADR-138 receiving session or an input
            // tunnel with role=Target. Everything else in the Remote seat
            // (roaming tunnel, bare input channel, ADR-138 controlling) is a
            // controller by construction, because the dock-swipe tap only
            // fires while this node's capture hooks are armed for remote
            // roaming. Slot presence (tunnel/remote_session) is NOT the
            // source of truth here: a bare-channel controller (edge-crossing
            // roam without a tunnel session) owns no slot but still controls.
            let session_receiving = self
                .shared
                .remote_session
                .lock()
                .expect("remote_session")
                .as_ref()
                .is_some_and(|s| s.role.is_receiving());
            let tunnel_target =
                self.active_tunnel_role() == Some(tetherly_core::InputTunnelRole::Target);
            if server.seat() != CursorSeat::Remote || session_receiving || tunnel_target {
                let now = self.shared.clock.unix_ms();
                let last = self
                    .shared
                    .gesture_swipe_last_ignored_ms
                    .load(std::sync::atomic::Ordering::Relaxed);
                if now.saturating_sub(last) > GESTURE_LOG_THROTTLE_MS {
                    self.shared
                        .gesture_swipe_last_ignored_ms
                        .store(now, std::sync::atomic::Ordering::Relaxed);
                    tracing::info!(
                        direction,
                        gesture = gesture.as_str(),
                        seat = ?server.seat(),
                        tunnel_role = ?self.active_tunnel_role(),
                        session_receiving = self
                            .shared
                            .remote_session
                            .lock()
                            .expect("remote_session")
                            .as_ref()
                            .is_some_and(|s| s.role.is_receiving()),
                        "four-finger swipe ignored: this node is the controlled side"
                    );
                }
                return Ok(());
            }
            // Resolve through the gesture map: gesture → intent → raw VK
            // sequence for the target platform. Sequences are deliberately
            // OUTSIDE the keymap (mac_to_windows remaps Meta→Ctrl, which
            // would turn Win+Tab into a browser tab switch). Empty result =
            // the gesture is mapped to `none`; that is a configured no-op,
            // not an error.
            let target_platform = self.input_peer_platform().unwrap_or_default();
            let map = self.gesture_map();
            let action = map.action_for(gesture);
            let steps = map.sequence_for(gesture, &target_platform);
            let base = server.next_seq();
            let events: Vec<InputEvent> = steps
                .iter()
                .enumerate()
                .map(|(i, s)| InputEvent::key(base + i as u64, s.code, s.down, 0))
                .collect();
            let len = events.len();
            if len > 0 {
                server.note_seq(base + len as u64 - 1);
            } else {
                server.note_seq(base);
            }
            tracing::info!(
                gesture = gesture.as_str(),
                action = action.as_str(),
                target_platform = %target_platform,
                events = len,
                "four-finger swipe resolved via gesture map"
            );
            events
        };
        let len = events.len();
        if len == 0 {
            return Ok(());
        }
        if let Err(e) = self.send_input_events(events, "gesture_app_switch") {
            tracing::warn!(direction, error = %e, "four-finger swipe gesture failed to send");
            return Err(e);
        }
        tracing::info!(
            direction,
            events = len,
            "four-finger swipe forwarded as remote gesture"
        );
        Ok(())
    }

    /// The active gesture→intent map. Config override when the user (or the
    /// UI) has saved one, otherwise the defaults that mirror macOS trackpad
    /// semantics.
    fn gesture_map(&self) -> tetherly_core::gesture_map::GestureMap {
        self.shared.gesture_map.lock().expect("gesture_map").clone()
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

    /// ADR-138: is a remote desktop session active on this node (either side)?
    /// While true, roaming takeover is suppressed and the watcher must not
    /// auto-attach — the session owns the single outbound input channel.
    pub fn remote_session_active(&self) -> bool {
        self.shared
            .remote_session
            .lock()
            .expect("remote_session")
            .is_some()
    }

    /// Physical movement a mouse must accumulate (px, Manhattan over
    /// coordinate deltas) after its last idle gap before an edge-push may
    /// take over. ~1/4 of a 1080p screen height: real use from anywhere on
    /// the screen crosses it, plug-in jitter (a few px) never does.
    const REVERSE_ARM_MIN_PX: i64 = 300;
    /// Idle gap that resets the movement accumulator (and absorbs the
    /// plug-in/unplug position jump).
    const REVERSE_RESET_AFTER_MS: u64 = 2000;
    /// Dwell required when the distance gate IS armed (real traversal
    /// happened): the topology `delay_ms` floor, ~100ms.
    const REVERSE_DWELL_ARMED_MS: u64 = 100;
    /// Dwell required when the distance gate is NOT armed. Production
    /// 2026-11-12 (v0.2.23): the entry placement parks the cursor ON the
    /// shared edge, so the most natural reverse gesture — push down from
    /// where the cursor already sits — accumulates near-zero coordinate
    /// travel (the OS clamps at height-1, dy collapses) and the 300px gate
    /// never armed: every push died in the blocked branch with accum_px≈0,
    /// silently. Instead of a hard veto there, an un-armed push must now
    /// dwell CONTIGUOUSLY (gaps <= TOUCH_GAP_MS) for this long to fire. A
    /// real hand holds the push far longer; plug-in jitter / desk bumps are
    /// isolated spikes that reset the dwell long before 600ms.
    const REVERSE_DWELL_UNARMED_MS: u64 = 600;
    /// Cooldown between automatic flips while the controller's cursor may
    /// be parked on this screen (storm fence).
    const REVERSE_COOLDOWN_MS: u128 = 3000;
    /// v0.2.33: cooldown when the local user owns the cursor — only an
    /// anti-spam fence for the in-flight attach, so a quick back-and-forth
    /// is not swallowed for 3 s.
    const REVERSE_COOLDOWN_LOCAL_OWNER_MS: u128 = 1000;

    /// Cooldown that applies to the current Target session.
    fn reverse_cooldown_ms(&self) -> u128 {
        if self.shared.reverse_local_owner.load(Ordering::SeqCst) {
            Self::REVERSE_COOLDOWN_LOCAL_OWNER_MS
        } else {
            Self::REVERSE_COOLDOWN_MS
        }
    }

    /// Short dwell (the topology `delay_ms`, floored at
    /// `REVERSE_DWELL_ARMED_MS`).
    fn reverse_short_dwell_ms(&self) -> u64 {
        Self::REVERSE_DWELL_ARMED_MS.max(
            self.shared
                .topology_layout
                .lock()
                .expect("topology_layout")
                .config
                .delay_ms as u64,
        )
    }

    /// Deskflow "switch at the shared edge" for the controlled side.
    ///
    /// While this node is the Target, a sustained physical push through its
    /// own configured edge takes over as controller automatically — the
    /// reverse direction (e.g. Windows -> Mac) needs no UI click. Accidental
    /// flips are fenced by three gates:
    ///
    /// 1. a two-tier distance gate: a mouse with 300px of real traversal
    ///    behind it fires after the short dwell (`REVERSE_DWELL_ARMED_MS`);
    ///    a mouse with zero travel — the entry placement parks it ON the
    ///    shared edge — must instead hold the long contiguous dwell
    ///    (`REVERSE_DWELL_UNARMED_MS`); the engine already mirrors the
    ///    declared edge (`set_edge` on accept and policy change), so it is
    ///    read lock-only here — no policy clone on the per-move hot path;
    /// 2. gap-continuity dwell: boundary-touching events must keep arriving
    ///    (gap <= 150ms) for the tier's dwell length. A
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
        let local_owner = self.shared.reverse_local_owner.load(Ordering::SeqCst);
        let now = std::time::Instant::now();
        {
            let last_flip = self.shared.last_auto_reverse.lock().expect("last_flip");
            if let Some(t) = *last_flip {
                if now.duration_since(t).as_millis() < self.reverse_cooldown_ms() {
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
                // Cursor away from the edge zone: the push is over.
                // v0.2.27 zone gate: this event also PROVES the cursor has
                // been outside the reverse-trigger zone during this Target
                // session — the user has entered the screen, so a later
                // deliberate push back to the shared edge may arm the dwell.
                self.shared.reverse_zone_dirty.store(true, Ordering::SeqCst);
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

        // v0.2.33 local-owner fast path (controller parity): no controller
        // cursor is parked on this screen, so none of the parked-entry
        // fences below has anything to protect. A fast into-edge push
        // (>= the topology `fast_push_delta_px`, the same threshold the
        // controller-side band gate uses) hands over at once; slower or
        // pinned pushes fall through to the short dwell below.
        if local_owner && crossed {
            let into = match edge {
                tetherly_core::ScreenEdge::Right => dx,
                tetherly_core::ScreenEdge::Left => -dx,
                tetherly_core::ScreenEdge::Bottom => dy,
                tetherly_core::ScreenEdge::Top => -dy,
            };
            let fast_push = self
                .shared
                .topology_layout
                .lock()
                .expect("topology_layout")
                .config
                .fast_push_delta_px
                .max(1) as i32;
            if into >= fast_push {
                *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
                *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
                *self
                    .shared
                    .reverse_pinned_touch
                    .lock()
                    .expect("reverse_pinned") = None;
                tracing::info!(into, fast_push, %edge, "auto-reverse: local-owner fast push");
                self.trace_input("reverse_fast", format!("push into={into} edge={edge}"));
                self.fire_reverse_takeover(edge, x, y, width, height);
                return;
            }
        }

        // v0.2.27 zone gate: the entry placement parks the incoming cursor
        // ON the shared edge — inside the reverse-trigger zone — so from the
        // parked position a single micro jitter toward the edge would arm
        // the dwell and the pinned heartbeat (v0.2.25) would carry it to
        // completion, bouncing the session back within seconds of entry
        // (production 18:16: entry parked at py=0, accum_px=0, control
        // snapped back to Windows 7s after the push had landed). A session
        // whose cursor never left the trigger zone has no "into the screen"
        // yet, hence no "push back" — gate the dwell until an event proves
        // interior presence. Applies regardless of the distance tier: a
        // slide ALONG the shared edge accumulates arming distance without
        // ever leaving the zone.
        if !self.shared.reverse_zone_dirty.load(Ordering::SeqCst) {
            static LAST_ZONE_GATE_LOG: std::sync::Mutex<Option<std::time::Instant>> =
                std::sync::Mutex::new(None);
            let mut last = LAST_ZONE_GATE_LOG.lock().expect("zonelog");
            if last
                .map(|t| t.elapsed() >= std::time::Duration::from_millis(1000))
                .unwrap_or(true)
            {
                *last = Some(std::time::Instant::now());
                tracing::info!(
                    %edge,
                    "auto-reverse: cursor never entered the screen interior; dwell gated"
                );
            }
            return;
        }

        // Distance gate, two-tier (production 2026-11-12, v0.2.23): an
        // ARMED mouse (300px of real traversal) keeps the short delay_ms
        // dwell. An UN-armed mouse no longer gets a hard veto — the entry
        // placement parks the cursor ON the shared edge, so the natural
        // "push from where it sits" gesture accumulates ~0px and was
        // structurally dead — it instead needs the long contiguous dwell
        // below. The blocked log stays, once per second, for attribution.
        if !distance_armed && !local_owner {
            static LAST_BLOCK_LOG: std::sync::Mutex<Option<std::time::Instant>> =
                std::sync::Mutex::new(None);
            let mut last = LAST_BLOCK_LOG.lock().expect("blocklog");
            if last
                .map(|t| t.elapsed() >= std::time::Duration::from_millis(1000))
                .unwrap_or(true)
            {
                *last = Some(std::time::Instant::now());
                tracing::info!(
                    accum_px = self.shared.reverse_move_accum_px.load(Ordering::SeqCst),
                    gate_px = Self::REVERSE_ARM_MIN_PX,
                    %edge,
                    "auto-reverse: distance gate not armed; requiring long dwell"
                );
            }
        }

        // Gap-continuity dwell: touch events must keep arriving within
        // TOUCH_GAP_MS of each other for `delay_ms` before the push counts
        // as sustained. Real mice report at 8-16ms intervals, so 150ms
        // means the pushing hand actually paused; two isolated accidental
        // bumps (typically >=300ms apart) reset the dwell instead. Leaving
        // the edge for longer than TOUCH_GAP_MS disarms the dwell. The
        // constant now lives in `reverse_dwell_advance` (shared with the
        // pinned-dwell heartbeat).
        let now = std::time::Instant::now();
        // Direction discipline (v0.1.97): the event must push INTO the edge
        // to sustain the dwell. The entry placement parks the cursor ON the
        // shared edge; horizontal drift along that edge (dy==0 for a
        // top/bottom edge) used to keep the dwell alive and the target
        // snatched control back seconds after every takeover. Only deltas
        // with an into-the-edge component count as "still pushing".
        //
        // Zero-delta leniency (win→mac reverse takeover fix): the Windows
        // hook may still report dx=dy=0 at the boundary — the OS clamps the
        // cursor to height-1 so consecutive hook positions stop changing
        // even while the hand keeps pushing (the delta-deriving fix removed
        // most of these, but clamped tails remain). A boundary-touching
        // event that arrived CONTIGUOUSLY during an active dwell continues
        // it; only when no dwell is armed does a zero delta need positive
        // direction proof. Without this, `pushing_in` was always false on
        // Windows and the takeover could never fire (silent, no log).
        let dwell_active = self
            .shared
            .reverse_arm_since
            .lock()
            .expect("reverse_arm")
            .is_some();
        // Pinned-open leniency (v0.2.24): the entry placement parks the
        // cursor ON the shared edge, and the OS clamps it there — pushing
        // straight into the edge from the parked spot produces a dy==0
        // event stream (positions never change; the hook's derived delta is
        // zero), so requiring dy>0 to OPEN the dwell made the natural
        // gesture dead (production: the only reverse log lines were
        // "distance gate not armed"). An un-armed mouse at the exact clamp
        // boundary may open the dwell on a zero-delta touch; the long
        // un-armed dwell is the misfire fence. The armed tier keeps the
        // v0.1.97 discipline: dy>0 opens, dy==0 only sustains.
        let pinned_at_clamp = match edge {
            tetherly_core::ScreenEdge::Right => x >= width.saturating_sub(2),
            tetherly_core::ScreenEdge::Bottom => y >= height.saturating_sub(2),
            tetherly_core::ScreenEdge::Top => y <= 2,
            tetherly_core::ScreenEdge::Left => x <= 2,
        };
        // A local-owner push may open from the clamp row too (v0.2.33): the
        // cursor typically already rests on the edge, so the OS reports
        // zero deltas while the hand pushes.
        let pinned_open = pinned_at_clamp && (!distance_armed || local_owner);
        let pushing_in = match edge {
            tetherly_core::ScreenEdge::Right => {
                dx > 0 || (dx == 0 && dwell_active) || (dx == 0 && pinned_open)
            }
            tetherly_core::ScreenEdge::Bottom => {
                dy > 0 || (dy == 0 && dwell_active) || (dy == 0 && pinned_open)
            }
            tetherly_core::ScreenEdge::Top => {
                dy < 0 || (dy == 0 && dwell_active) || (dy == 0 && pinned_open)
            }
            tetherly_core::ScreenEdge::Left => {
                dx < 0 || (dx == 0 && dwell_active) || (dx == 0 && pinned_open)
            }
        };
        let fired = if !crossed || !pushing_in {
            // Left the edge (or slid along it without pushing in): the push
            // is over, reset the dwell — and the pinned-heartbeat anchor with
            // it, or the heartbeat would keep feeding a dead push.
            *self.shared.reverse_arm_since.lock().expect("reverse_arm") = None;
            *self.shared.reverse_last_tick.lock().expect("reverse_tick") = None;
            *self
                .shared
                .reverse_pinned_touch
                .lock()
                .expect("reverse_pinned") = None;
            false
        } else {
            let delay_ms = if distance_armed || local_owner {
                self.reverse_short_dwell_ms()
            } else {
                Self::REVERSE_DWELL_UNARMED_MS
            };
            self.reverse_dwell_advance(now, delay_ms, pinned_at_clamp, x, y)
        };
        if !fired {
            return;
        }
        if local_owner {
            self.trace_input("reverse_fast", format!("short dwell edge={edge}"));
        }
        self.fire_reverse_takeover(edge, x, y, width, height);
    }

    /// Advance the gap-continuity dwell state machine by one tick. Shared by
    /// the physical-event path (`maybe_auto_reverse`) and the pinned-dwell
    /// heartbeat (`spawn_reverse_pinned_heartbeat`) so both feed exactly the
    /// same arm/continuity rules. Returns true when the dwell completed and
    /// the takeover should fire. TOUCH_GAP_MS lives on the caller side (used
    /// in log wording there); the heartbeat passes the same constant.
    fn reverse_dwell_advance(
        &self,
        now: std::time::Instant,
        delay_ms: u64,
        record_anchor: bool,
        x: i32,
        y: i32,
    ) -> bool {
        const TOUCH_GAP_MS: u64 = 150;
        // v0.2.27 zone gate (also covers the pinned heartbeat, which feeds
        // this same state machine): a Target session whose cursor never
        // left the reverse-trigger zone cannot arm or sustain a dwell —
        // the parked entry position must not be able to bounce itself
        // back via heartbeat-completed jitter dwells.
        if !self.shared.reverse_zone_dirty.load(Ordering::SeqCst) {
            return false;
        }
        let mut arm = self.shared.reverse_arm_since.lock().expect("reverse_arm");
        let mut last_tick = self.shared.reverse_last_tick.lock().expect("reverse_tick");
        match (*arm, *last_tick) {
            (Some(started), Some(prev_tick)) => {
                let sustained = now.duration_since(started).as_millis() as u64 >= delay_ms;
                let contiguous = now.duration_since(prev_tick).as_millis() as u64 <= TOUCH_GAP_MS;
                if sustained && contiguous {
                    *arm = None;
                    *last_tick = None;
                    *self
                        .shared
                        .reverse_pinned_touch
                        .lock()
                        .expect("reverse_pinned") = None;
                    true
                } else if contiguous {
                    // Still pushing: keep accumulating towards delay_ms.
                    // Refresh the continuity anchor — otherwise the gap
                    // check keeps measuring from the very first touch and
                    // any dwell longer than TOUCH_GAP_MS resets forever.
                    *last_tick = Some(now);
                    // v0.2.36: a real sweep opens the dwell one row SHORT
                    // of the clamp (y in the promotion band, dy>0) and only
                    // the NEXT event lands on the clamp row — after which
                    // the OS goes silent. Recording the anchor only on the
                    // first touch left that dwell with no heartbeat to
                    // carry it (production 2026-11-13 17:52–17:56: the Mac
                    // cursor parked here, the Windows hand pushed into the
                    // edge repeatedly, zero reverse log lines). Adopt the
                    // anchor on the first clamped tick of an active dwell.
                    if record_anchor {
                        let mut pinned = self
                            .shared
                            .reverse_pinned_touch
                            .lock()
                            .expect("reverse_pinned");
                        if pinned.is_none() {
                            *pinned = Some((now, x, y));
                        }
                    }
                    false
                } else {
                    // Gap too long: a fresh push, not a continuation.
                    *arm = Some(now);
                    *last_tick = Some(now);
                    if record_anchor {
                        *self
                            .shared
                            .reverse_pinned_touch
                            .lock()
                            .expect("reverse_pinned") = Some((now, x, y));
                    }
                    false
                }
            }
            _ => {
                // First touch of a new push. When the cursor sits in the
                // clamp band, record the heartbeat anchor: the OS will
                // stop reporting moves while the hand keeps pushing
                // (Windows LL hooks fire only on cursor movement), and
                // the heartbeat task must then carry the dwell.
                if record_anchor {
                    *self
                        .shared
                        .reverse_pinned_touch
                        .lock()
                        .expect("reverse_pinned") = Some((now, x, y));
                }
                *arm = Some(now);
                *last_tick = Some(now);
                false
            }
        }
    }

    /// Pinned-dwell heartbeat (v0.2.25): Windows LL mouse hooks fire only
    /// when the cursor MOVES. The entry placement parks the cursor ON the
    /// shared edge and the OS clamps it there, so a continued physical push
    /// into the edge produces NO further WM_MOUSEMOVE after the first
    /// boundary touch — the gap-continuity dwell starves on its first tick
    /// and no dwell target beyond that single touch is reachable (production
    /// 2026-11-12 09:34: each push logged exactly one "requiring long dwell"
    /// line; the 600ms gate never had a chance to accumulate). This task
    /// carries the dwell from the heartbeat anchor: while the anchor is
    /// fresh (TOUCH_GAP_MS since the last heartbeat tick — i.e. "the hand
    /// never demonstrably left"), it re-runs the same dwell state machine
    /// every HEARTBEAT_PERIOD_MS. Any physical event that moves the cursor
    /// off the clamp band re-enters maybe_auto_reverse with
    /// !crossed/!pushing_in and clears the anchor; the heartbeat also stops
    /// once the dwell fires or the anchor ages out (PUSH_ABANDON_MS).
    pub fn spawn_reverse_pinned_heartbeat(&self) {
        const HEARTBEAT_PERIOD_MS: u64 = 30;
        const PUSH_ABANDON_MS: u64 = 3000;
        let node = self.clone_handle();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(HEARTBEAT_PERIOD_MS)).await;
                let now = std::time::Instant::now();
                // Feed only while an anchor exists and is fresh enough to
                // still plausibly represent "the hand is holding the push".
                // A hand-held push leaves the cursor pinned, so the anchor
                // position must also match where the cursor was last seen.
                let anchor = *node
                    .shared
                    .reverse_pinned_touch
                    .lock()
                    .expect("reverse_pinned");
                let Some((touched_at, ax, ay)) = anchor else {
                    continue;
                };
                if now.duration_since(touched_at).as_millis() as u64 > PUSH_ABANDON_MS {
                    // Stale anchor: the push was abandoned (hand lifted; any
                    // new push would re-record a fresh anchor). Clear it.
                    *node
                        .shared
                        .reverse_pinned_touch
                        .lock()
                        .expect("reverse_pinned") = None;
                    continue;
                }
                // Same role gate as maybe_auto_reverse: the heartbeat only
                // exists to complete a Target-side reverse takeover.
                if node.active_tunnel_role() != Some(tetherly_core::InputTunnelRole::Target) {
                    continue;
                }
                // Cooldown gate mirrors the physical path.
                {
                    let cooldown = node.reverse_cooldown_ms();
                    let last_flip = node.shared.last_auto_reverse.lock().expect("last_flip");
                    if let Some(t) = *last_flip {
                        if now.duration_since(t).as_millis() < cooldown {
                            continue;
                        }
                    }
                }
                let distance_armed = node.shared.reverse_move_accum_px.load(Ordering::SeqCst)
                    >= Node::REVERSE_ARM_MIN_PX;
                let local_owner = node.shared.reverse_local_owner.load(Ordering::SeqCst);
                let delay_ms = if distance_armed || local_owner {
                    node.reverse_short_dwell_ms()
                } else {
                    Node::REVERSE_DWELL_UNARMED_MS
                };
                // Anchor is NOT re-recorded from the heartbeat (record_anchor
                // = false): the physical first-touch event owns the anchor;
                // the heartbeat only advances the clock.
                if node.reverse_dwell_advance(now, delay_ms, false, ax, ay) {
                    let edge = {
                        let server = node.shared.input_engine.lock().expect("engine");
                        server.edge()
                    };
                    info!(
                        "auto-reverse: pinned heartbeat completed the dwell; requesting takeover"
                    );
                    node.trace_input(
                        "reverse_heartbeat",
                        format!("pinned dwell done at ({ax},{ay}) local_owner={local_owner}"),
                    );
                    node.fire_reverse_takeover(edge, ax, ay, 0, 0);
                }
            }
        });
    }

    /// The takeover half of maybe_auto_reverse (post-dwell): cooldown stamp,
    /// peer resolution, proportional entry mapping, and the polite-attach
    /// spawn. Split out so the pinned heartbeat can fire the identical path.
    fn fire_reverse_takeover(
        &self,
        edge: tetherly_core::ScreenEdge,
        x: i32,
        y: i32,
        _width: i32,
        _height: i32,
    ) {
        let now = std::time::Instant::now();
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
        // Deskflow proportional entry: map the edge-push position onto the
        // controller-side screen being taken over, so its cursor starts
        // where the push happened instead of at a stale resting spot.
        // Same mapping the forward auto_link crossing uses.
        let push_pos = (x, y);
        let mapped_peer = peer.clone();
        fn mapped_entry(
            node: &Node,
            mapped_peer: &str,
            display_edge: tetherly_core::DisplayEdge,
            push_pos: (i32, i32),
        ) -> Option<(i32, i32)> {
            let layout = node.shared.topology_layout.lock().expect("topology_layout");
            let neighbor = layout
                .neighbors
                .iter()
                .find(|(_, n)| n.peer_id == mapped_peer)
                .map(|(_, n)| n.clone());
            let local_display = layout.display;
            let remote = node
                .shared
                .peer_topologies
                .lock()
                .expect("peer_topologies")
                .get(mapped_peer)
                .map(|p| p.display);
            neighbor.zip(remote).map(|(n, remote_display)| {
                map_edge_exit_to_entry(
                    &local_display,
                    display_edge,
                    push_pos,
                    &remote_display,
                    n.peer_edge,
                    n.offset,
                )
            })
        }
        let entry = mapped_entry(self, &mapped_peer, display_edge, push_pos);
        let (entry_x, entry_y) = entry
            .map(|(ex, ey)| (Some(ex), Some(ey)))
            .unwrap_or((None, None));
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        if let Some(handle) = rt {
            handle.spawn(async move {
                if let Err(e) = node
                    .input_tunnel_attach(&peer, Some(display_edge), entry_x, entry_y)
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
        self.shared.input_tx_buttons.fetch_add(1, Ordering::SeqCst);
        self.trace_input(
            "push_button",
            format!(
                "mask={mask} down={down} seq={}",
                events.first().map(|e| e.seq).unwrap_or(0)
            ),
        );
        self.send_input_events(events, "drive_button")
    }

    /// Push a physical wheel observation to the remote seat.
    pub fn drive_input_wheel(&self, dx: i16, dy: i16) -> Result<(), CoreError> {
        let events = {
            let mut server = self.shared.input_engine.lock().expect("engine");
            if server.seat() != CursorSeat::Remote {
                return Ok(());
            }
            self.note_reverse_physical_activity();
            let speed = self.policy_wheel_speed();
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
        self.send_input_events(events, "drive_wheel")
    }

    pub fn input_engine_seat(&self) -> CursorSeat {
        self.shared.input_engine.lock().expect("engine").seat()
    }

    /// Per-batch roaming recenter decision for the capture worker (Deskflow
    /// MSWindowsComputer::onMouseMove alignment): while roaming the Remote
    /// seat, every consumed motion batch pulls the physical cursor back to
    /// screen center. The delta chain — anchored at the warp target by the
    /// echo gate — then always measures genuine hand motion from center and
    /// can never saturate at a screen boundary: the controller's hand and
    /// the remote cursor own independent positions. Seat-scoped: Local
    /// movement owns the whole screen and is never recentred. The caller
    /// executed the warp through `warp_mouse_position`, whose echo is
    /// swallowed with the baseline anchored at the target (replayed in the
    /// platform warp-echo tests). Production callers are Windows-only:
    /// macOS needs no recenter, its capture is dissociated from the
    /// hardware mouse (`CGAssociateMouseAnd...`) and reads raw CGEvent
    /// deltas — the method stays seat-agnostic so the decision logic is
    /// unit-testable on any platform.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn roaming_recenter_target(&self, width: i32, height: i32) -> Option<(i32, i32)> {
        if self.input_engine_seat() != CursorSeat::Remote {
            return None;
        }
        Some(crate::platform::roaming_recenter_target(width, height))
    }

    /// The engine's parked local cursor position (pixels). Used by tests to
    /// assert the Deskflow-style proportional return landing.
    pub fn input_engine_local_pos(&self) -> (i32, i32) {
        self.shared.input_engine.lock().expect("engine").local_pos()
    }

    pub(super) fn send_input_events(
        &self,
        events: Vec<InputEvent>,
        source: &'static str,
    ) -> Result<(), CoreError> {
        if events.is_empty() {
            return Ok(());
        }
        // Operator activity: every attempted delivery (keys, moves, buttons,
        // wheel, gestures) keeps the roaming tunnel's idle clock fresh. Only
        // mouse moves used to count (drive_input_delta touched the slot
        // directly), so a pure typing stretch looked idle and the watcher's
        // `input_tunnel_status()` sweep reaped the tunnel mid-use — the same
        // 1s watcher tick then rescued the now role-less Remote seat and the
        // keyboard snapped back to the local machine (production 2026-11-12:
        // 86 `input_tunnel_idle_timeout` reaps in one day under
        // `idle_timeout_secs=10`, each one an interrupted typing stint).
        if let Ok(mut guard) = self.shared.active_input_tunnel.lock() {
            if let Some(ref mut session) = *guard {
                session.last_activity_ms = self.shared.clock.unix_ms();
            }
        }
        // Diagnostic (v0.1.83): name every push batch — enter-carrying ones
        // AND lone edge-clamp Moves (rem_y==1000) — so the ghost
        // re-placement frame's producer is attributable.
        for (idx, ev) in events.iter().enumerate() {
            match &ev.kind {
                tetherly_core::InputKind::Enter => {
                    if let Some(tetherly_core::InputEvent {
                        kind: tetherly_core::InputKind::Move { x, y },
                        ..
                    }) = events.get(idx + 1)
                    {
                        tracing::info!(source, rem_x = x, rem_y = y, "push: enter+move");
                        self.trace_input("push_enter", format!("rem=({x},{y}) src={source}"));
                    } else {
                        tracing::info!(source, "push: enter");
                        self.trace_input("push_enter", format!("no move src={source}"));
                    }
                }
                tetherly_core::InputKind::Move { x, y }
                    if *y == tetherly_core::SCREEN_UNITS && idx == 0 =>
                {
                    tracing::info!(
                        source,
                        rem_x = x,
                        seq = ev.seq,
                        "push: lone edge-clamp move"
                    );
                    self.trace_input(
                        "push_clamp",
                        format!("rem_x={x} seq={} src={source}", ev.seq),
                    );
                }
                _ => {}
            }
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
                let frames = engine
                    .drain()
                    .into_iter()
                    .filter_map(|ev| ev.to_frame().ok())
                    .collect::<Vec<_>>();
                // Diagnostic (v0.1.82): the resume replay is the prime
                // suspect for stale re-placement frames — record what rem it
                // actually pushed.
                if let Some(f) = frames.get(1) {
                    if let Ok(ev) = tetherly_core::InputEvent::from_frame(f) {
                        if let tetherly_core::InputKind::Move { x, y } = ev.kind {
                            tracing::info!(
                                rem_x = x,
                                rem_y = y,
                                was_in_grace,
                                is_remote,
                                "resume_remote replayed move"
                            );
                        }
                    }
                }
                frames
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
            // Diagnostic (v0.1.83): replayed buffer frames are the prime
            // suspect for stale re-placement — log any edge-clamp Move they
            // carry with its rem.
            for f in &buffered {
                if let Ok(ev) = tetherly_core::InputEvent::from_frame(f) {
                    if let tetherly_core::InputKind::Move { x, y } = ev.kind {
                        if y == tetherly_core::SCREEN_UNITS {
                            tracing::info!(rem_x = x, rem_y = y, "buffer flush: edge-clamp move");
                        }
                    }
                }
            }
            for f in buffered {
                let _ = tx.try_send(f);
            }
            tracing::info!(peer = %peer_id, buf_count, "input session resumed; flushed buffered frames");
        }

        let channel_gen = INPUT_CHANNEL_SEQ.fetch_add(1, Ordering::SeqCst);
        *self.shared.input_out.lock().expect("input_out") = Some((channel_gen, tx));
        *self.shared.input_peer.lock().expect("input_peer") = Some(peer_id.clone());
        self.trace_input(
            "channel_open",
            format!("input channel to {peer_id} gen={channel_gen}"),
        );
        let current_seat = self.input_seat_str();
        self.emit(UiEvent::input_seat_changed(
            current_seat,
            true,
            Some(peer_id.to_string()),
        ));
        let mut stop = self.shared.shutdown.subscribe();
        let node = self.clone_handle();
        // Epoch the grant slot had when THIS pump took over: the teardown
        // below may only release a grant this pump actually served. A newer
        // pre-arm (takeover attach overwrites the slot synchronously) must
        // survive this pump's death — the peer's ACCEPTED resp matches it
        // (CI m3_11 / production 2026-11-11).
        let grant_epoch = self
            .shared
            .active_input_tunnel
            .lock()
            .expect("active_input_tunnel")
            .as_ref()
            .map(|s| s.pre_arm_epoch);
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
                                    node.trace_input(
                                        "pump_exit",
                                        format!("controller pump send failed peer={peer_id}"),
                                    );
                                    break;
                                }
                                node.shared.telemetry.record_input_tx(f_len);
                            }
                            None => {
                                node.trace_input(
                                    "pump_exit",
                                    format!("controller pump outbound slot dropped peer={peer_id}"),
                                );
                                break;
                            }
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
                                        node.release_input_tunnel_local("target returned to its local seat");
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
                                node.trace_input(
                                    "pump_exit",
                                    format!("controller pump recv error peer={peer_id}: {err_s}"),
                                );
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
                let (role_now, epoch_now) = {
                    let guard = node
                        .shared
                        .active_input_tunnel
                        .lock()
                        .expect("active_input_tunnel");
                    let role = guard
                        .as_ref()
                        .map(|s| s.role)
                        // Bare input channels (no tunnel session) keep the legacy
                        // controller-side cleanup; only an actual demotion
                        // (role = Target) suppresses it.
                        .unwrap_or(InputTunnelRole::Controller);
                    (role, guard.as_ref().map(|s| s.pre_arm_epoch))
                };
                let peer_now = node.shared.input_peer.lock().expect("input_peer").clone();
                role_now == InputTunnelRole::Controller
                    && epoch_now == grant_epoch
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
                        // Do not gate on input_channel_open(): a slot that
                        // is Some but dead (half-open TCP, dead reader)
                        // would spin here forever reporting "re-established".
                        // open_input probes the queue and rebuilds it when
                        // dead; on a healthy channel it returns Ok at once.
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
                        if retry_node.open_input(&target_peer).await.is_ok() {
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
                        // Transient, NOT a session end: a pending takeover's grant must
                        // survive the grace window (its ACCEPT resp re-opens the channel).
                        retry_node.rescue_seat_to_local(false);
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
                node.release_input_tunnel_local("peer session gone");
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
            let entry_moves_logged = std::sync::atomic::AtomicU32::new(0);
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
                                node.note_inbound_tunnel_activity(peer_id.as_str());
                                if let Ok(ev) = InputEvent::from_frame(&frame) {
                                    // Diagnostic (v0.1.81): log the entry placement
                                    // moment — Enter and the first Move after it —
                                    // so a mis-placed crossing cursor is attributable
                                    // to the received rem coords vs the sink math.
                                    // Low volume: one line per crossing + 2.
                                    match &ev.kind {
                                        InputKind::Enter => {
                                            tracing::info!(
                                                "apply: enter (crossing placement begins)"
                                            );
                                            node.trace_input(
                                                "apply_enter",
                                                format!("controller {peer_id} entered this screen"),
                                            );
                                            node.note_controller_entered();
                                            entry_moves_logged.store(0, Ordering::SeqCst);
                                        }
                                        InputKind::Button { mask, down } => {
                                            node.shared
                                                .input_rx_buttons
                                                .fetch_add(1, Ordering::SeqCst);
                                            node.trace_input(
                                                "apply_button",
                                                format!(
                                                    "mask={mask} down={down} seq={} focused={}",
                                                    ev.seq,
                                                    client.focused()
                                                ),
                                            );
                                        }
                                        InputKind::Wheel { .. } => {
                                            node.shared
                                                .input_rx_wheel
                                                .fetch_add(1, Ordering::SeqCst);
                                        }
                                        InputKind::Key { .. } => {
                                            node.shared
                                                .input_rx_keys
                                                .fetch_add(1, Ordering::SeqCst);
                                        }
                                        InputKind::Move { x, y }
                                            if entry_moves_logged.fetch_add(1, Ordering::SeqCst)
                                                < 2
                                            => {
                                                let (rx, ry) = (*x, *y);
                                                let (w, h) = crate::platform::screen_size();
                                                let px = ((rx
                                                    .clamp(0, tetherly_core::SCREEN_UNITS)
                                                    as i64
                                                    * (w.saturating_sub(1) as i64)
                                                    + tetherly_core::SCREEN_UNITS as i64 / 2)
                                                    / (tetherly_core::SCREEN_UNITS as i64))
                                                    as i32;
                                                let py = ((ry
                                                    .clamp(0, tetherly_core::SCREEN_UNITS)
                                                    as i64
                                                    * (h.saturating_sub(1) as i64)
                                                    + tetherly_core::SCREEN_UNITS as i64 / 2)
                                                    / (tetherly_core::SCREEN_UNITS as i64))
                                                    as i32;
                                                tracing::info!(
                                                    rem_x = rx, rem_y = ry, px, py,
                                                    "apply: entry move placed"
                                                );
                                            }
                                        _ => {}
                                    }
                                    // Mirror-fast path: Move frames dominate input
                                    // traffic (up to 1000 Hz). Cloning the whole
                                    // 256-entry trace per frame would cost dozens
                                    // of heap allocs under the sink lock, so only
                                    // the cursor scalars are synced; the trace is
                                    // refreshed on every other event kind.
                                    let is_move = matches!(ev.kind, InputKind::Move { .. });
                                    let is_leave = matches!(ev.kind, InputKind::Leave);
                                    let ev_seq = ev.seq;
                                    let apply_res = client.apply(ev);
                                    // v0.2.31: mirror the client's counters so
                                    // `GET /api/input` shows applied vs dropped.
                                    node.shared
                                        .input_rx_applied
                                        .store(client.applied(), Ordering::SeqCst);
                                    node.shared
                                        .input_rx_dropped
                                        .store(client.dropped(), Ordering::SeqCst);
                                    if let Ok(leave_opt) = apply_res {
                                        if is_leave {
                                            // The controller ended the roaming stint
                                            // (its sustained-push return ships a
                                            // Leave). The tunnel grant must end with
                                            // it — a stale Target role routes every
                                            // local mouse event into the
                                            // reverse-arming branch and blocks the
                                            // target's own edge crossings
                                            // (production 2026-11-11).
                                            node.release_input_tunnel_local("controller left roaming");
                                            node.note_controller_left();
                                            node.park_target_cursor_after_leave(
                                                Node::TARGET_PARK_ON_LEAVE,
                                                crate::platform::screen_size(),
                                            );
                                        }
                                        if let Some(leave) = leave_opt {
                                            if let Ok(f) = leave.to_frame() {
                                                let l_len = (f.payload.len() + 12) as u64;
                                                if sess.send_bounded(f).await.is_ok() {
                                                    node.shared.telemetry.record_input_tx(l_len);
                                                }
                                            }
                                        }
                                        if is_move {
                                            let (x, y) = client.sink().memory().cursor;
                                            let mut sink =
                                                node.shared.input_sink.lock().expect("sink");
                                            sink.cursor = (x, y);
                                            // The Move fast-path skips the full
                                            // memory clone, so the shared trace
                                            // would never see move-only sequences
                                            // (a session Enter+Move is exactly
                                            // that — Enter is engine-internal and
                                            // leaves no sink record). Append the
                                            // single record with the same bounded
                                            // ring accounting the memory observer
                                            // uses; a later full clone stays
                                            // consistent.
                                            if sink.applied.len()
                                                >= tetherly_core::INPUT_SINK_TRACE_MAX
                                            {
                                                sink.applied.pop_front();
                                            }
                                            sink.applied.push_back(
                                                tetherly_core::InputKind::Move { x, y },
                                            );
                                        } else {
                                            *node.shared.input_sink.lock().expect("sink") =
                                                client.sink().memory().clone();
                                        }
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
                                        node.trace_input(
                                            "apply_reject",
                                            format!("seq={ev_seq} dropped={}", client.dropped()),
                                        );
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
                                node.trace_input(
                                    "pump_exit",
                                    format!("target pump recv error peer={peer_id}: {err_s}"),
                                );
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

    /// Append one decision to the input-engine trace ring (drops the oldest
    /// past [`INPUT_TRACE_CAP`]). Cheap: one short lock, no allocation
    /// beyond the entry itself; callers are per-decision, never per-frame.
    pub(crate) fn trace_input(&self, kind: &'static str, detail: impl Into<String>) {
        let entry = InputTraceEntry {
            ts_ms: self.shared.clock.unix_ms(),
            kind: kind.to_string(),
            detail: detail.into(),
        };
        let mut ring = self.shared.input_trace.lock().expect("input_trace");
        while ring.len() >= INPUT_TRACE_CAP {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    /// Snapshot of the input-engine trace ring, oldest first.
    pub fn input_trace_snapshot(&self) -> Vec<InputTraceEntry> {
        self.shared
            .input_trace
            .lock()
            .expect("input_trace")
            .iter()
            .cloned()
            .collect()
    }

    /// `GET /api/input/trace` report.
    pub async fn input_trace_report(&self) -> InputTraceReport {
        let channel_open = self.input_channel_open().await;
        InputTraceReport {
            ok: true,
            seat: self.input_seat_str().to_string(),
            channel_open,
            capacity: INPUT_TRACE_CAP,
            entries: self.input_trace_snapshot(),
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
            applied: self.shared.input_rx_applied.load(Ordering::SeqCst),
            dropped: self.shared.input_rx_dropped.load(Ordering::SeqCst),
            rx_buttons: self.shared.input_rx_buttons.load(Ordering::SeqCst),
            rx_wheel: self.shared.input_rx_wheel.load(Ordering::SeqCst),
            rx_keys: self.shared.input_rx_keys.load(Ordering::SeqCst),
            tx_buttons: self.shared.input_tx_buttons.load(Ordering::SeqCst),
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

    /// v0.2.30 input-engine trace ring: bounded at `INPUT_TRACE_CAP`
    /// (oldest dropped), timestamps from the node clock, and the seat
    /// rescue path records itself so a stranded-seat triage no longer
    /// needs the daemon log.
    #[tokio::test]
    async fn input_trace_ring_is_bounded_and_records_rescue() {
        let (node, _dir, clock) =
            crate::runtime::clipboard_policy_tests::temp_test_node_manual_clock(
                "input-trace",
                10_000,
            );
        node.shared
            .input_capture_active
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let empty = node.input_trace_report().await;
        assert!(empty.ok);
        assert_eq!(empty.capacity, INPUT_TRACE_CAP);
        assert_eq!(empty.seat, "local");
        assert!(empty.entries.is_empty());

        for i in 0..(INPUT_TRACE_CAP + 6) {
            clock.set(10_000 + i as u64);
            node.trace_input("seat", format!("entry {i}"));
        }
        let snap = node.input_trace_snapshot();
        assert_eq!(snap.len(), INPUT_TRACE_CAP, "ring must drop the oldest");
        assert_eq!(snap[0].detail, "entry 6");
        assert_eq!(snap[0].ts_ms, 10_006);
        assert_eq!(
            snap[INPUT_TRACE_CAP - 1].detail,
            format!("entry {}", INPUT_TRACE_CAP + 5)
        );

        clock.set(20_000);
        node.rescue_seat_to_local(false);
        let snap = node.input_trace_snapshot();
        let last = snap.last().expect("rescue entry");
        assert_eq!(last.kind, "rescue");
        assert_eq!(last.ts_ms, 20_000);
        assert!(
            last.detail.contains("release_grant=false"),
            "rescue detail must name the grant decision, got {:?}",
            last.detail
        );
        assert_eq!(snap.len(), INPUT_TRACE_CAP);
    }

    /// Regression (production 2026-11-12): while the operator typed on the
    /// remote machine, the roaming tunnel's `last_activity_ms` was refreshed
    /// only by mouse moves (the touch lived at the tail of
    /// `drive_input_delta`), so a pure typing stretch looked idle. The 1s
    /// link watcher calls `input_tunnel_status()` every tick, whose inline
    /// sweep reaps the tunnel once `idle_timeout_secs` passes without a
    /// MOUSE event — and the same tick's `reconcile_stranded_remote_seat()`
    /// then finds seat==Remote with no Controller grant and rescues the seat
    /// to Local: the keyboard snaps back to the local machine mid-typing
    /// (86 `input_tunnel_idle_timeout` reaps recorded in one production day
    /// under `idle_timeout_secs=10`; watcher tick phase aligned with every
    /// reap timestamp). Typing — and every other delivered input kind — must
    /// count as tunnel activity.
    #[tokio::test]
    async fn typing_keeps_the_roaming_tunnel_alive_through_the_watcher_idle_sweep() {
        let (node, _dir, clock) =
            crate::runtime::clipboard_policy_tests::temp_test_node_manual_clock(
                "typing-idle",
                10_000,
            );
        node.update_input_policy(InputTunnelPolicyUpdate {
            // The sanitizer floors this at 10s (clamp(10, 86400)) — the same
            // aggressive value the production node runs.
            idle_timeout_secs: Some(10),
            ..Default::default()
        })
        .unwrap();
        // Never auto-arm the real input-capture hooks inside a test host
        // (this machine may have accessibility granted).
        node.shared
            .input_capture_active
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Enter the Remote seat the way the drive path does.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            let crossed = engine.local_move_delta(1919, 500, 4, 0);
            assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Right));
            engine.drain();
        }
        // Arm the controller grant the way the attach path does.
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_typing".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: clock.unix_ms(),
            last_activity_ms: clock.unix_ms(),
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        // A live outbound channel so the keystrokes are really delivered.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<tetherly_core::InnerFrame>(64);
        *node.shared.input_out.lock().unwrap() = Some((0, tx));

        // Type: down/up keystrokes with the clock advancing between them.
        for ms in [10_200u64, 10_600, 11_000] {
            clock.set(ms);
            node.drive_input_key(0x04, true).unwrap();
            node.drive_input_key(0x04, false).unwrap();
        }
        // The keystrokes really crossed the outbound channel.
        assert!(rx.try_recv().is_ok(), "keystrokes must be delivered");

        // The last keystroke refreshed the slot to the clock's now.
        let last = node
            .shared
            .active_input_tunnel
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .last_activity_ms;
        assert_eq!(last, 11_000, "typing must refresh the tunnel's idle clock");

        // Advance 9.5s past the last keystroke (inside the 10s window, but
        // 10.5s past the arm time) and run the watcher's exact tick:
        // `input_tunnel_status()` (sweep) then the stranded-seat reconcile.
        // Without the typing refresh the sweep reaps here and the reconcile
        // rescues the seat — the production interruption, deterministically.
        clock.set(20_500);
        let status = node.input_tunnel_status();
        assert!(
            status.session.is_some(),
            "a tunnel kept alive by typing must survive the idle sweep"
        );
        assert!(
            !node.reconcile_stranded_remote_seat(),
            "an actively-typed roaming stint must not be rescued"
        );
        assert_eq!(node.input_engine_seat(), CursorSeat::Remote);
    }

    /// The roaming TARGET has no heartbeat keepalive (heartbeats are
    /// ADR-138-session-only), so inbound input frames are its only activity
    /// signal. Without the pump touch its slot goes stale during active
    /// control and the target's own watcher sweep reaps it mid-use — the
    /// reconcile no longer sees the receiving role and the controlled side
    /// self-rescues while the operator is typing.
    #[tokio::test]
    async fn inbound_frames_keep_the_target_tunnel_alive_through_the_idle_sweep() {
        let (node, _dir, clock) =
            crate::runtime::clipboard_policy_tests::temp_test_node_manual_clock(
                "target-idle",
                10_000,
            );
        node.update_input_policy(InputTunnelPolicyUpdate {
            // The sanitizer floors this at 10s (clamp(10, 86400)).
            idle_timeout_secs: Some(10),
            ..Default::default()
        })
        .unwrap();
        node.shared
            .input_capture_active
            .store(true, std::sync::atomic::Ordering::SeqCst);

        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_target".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Target,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 10_000,
            last_activity_ms: 10_000,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        // Frames flowing (the controller is actively typing) must refresh
        // the slot even though this side sends nothing.
        clock.set(11_600);
        node.note_inbound_tunnel_activity("tdev_peer");
        clock.set(18_000);
        let status = node.input_tunnel_status();
        assert!(
            status.session.is_some(),
            "an actively driven target tunnel must survive the idle sweep"
        );

        // Control: with NO inbound activity the same window reaps the slot
        // (the governance semantics are unchanged — idle still releases).
        clock.set(25_000);
        let status = node.input_tunnel_status();
        assert!(
            status.session.is_none(),
            "a silent tunnel is reaped by design"
        );
    }

    /// Regression (production 2026-11-13, v0.2.27): the idle sweep reaped a
    /// ROAMING stint whose operator simply paused — the 1s watcher then
    /// re-armed a fresh session every ~11s and each re-arm's same-peer
    /// takeover request demoted the live stint on the target (keyboard dead
    /// mid-control; audit showed a perpetual started→idle_timeout→started
    /// loop). A Remote engine seat IS active use: the sweep must skip it and
    /// let the roaming machinery end the stint instead.
    #[tokio::test]
    async fn roaming_stint_survives_the_idle_sweep_while_seat_is_remote() {
        let (node, _dir, clock) =
            crate::runtime::clipboard_policy_tests::temp_test_node_manual_clock(
                "roaming-idle",
                10_000,
            );
        node.update_input_policy(InputTunnelPolicyUpdate {
            idle_timeout_secs: Some(10),
            ..Default::default()
        })
        .unwrap();
        node.shared
            .input_capture_active
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Roaming: engine seat Remote, controller grant armed.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            let crossed = engine.local_move_delta(1919, 500, 4, 0);
            assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Right));
            engine.drain();
        }
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_roaming".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 10_000,
            last_activity_ms: 10_000,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        // 11s of TOTAL silence (no events at all) — the seat is still Remote.
        clock.set(21_000);
        let status = node.input_tunnel_status();
        assert!(
            status.session.is_some(),
            "a roaming stint must survive the idle sweep no matter how idle it is"
        );

        // The stint ends through the roaming machinery (engine return) —
        // afterwards the same stale slot is sweepable again.
        node.shared.input_engine.lock().unwrap().force_local();
        let status = node.input_tunnel_status();
        assert!(
            status.session.is_none(),
            "once the seat is back Local the stale slot is reaped by design"
        );
    }

    /// Regression (production 2026-11-11): the target-side input tunnel
    /// survived the control session's end. With a stale
    /// `active_input_tunnel` of role Target, every local drive event was
    /// routed into the reverse-takeover arming branch — the normal edge
    /// crossing never ran and Windows could not cross into the Mac until
    /// the tunnel was manually released. Every session end (operator stop
    /// here; engine return / inbound Leave / peer gone elsewhere) must
    /// release the grant.
    /// Regression (production 2026-11-11): the drive path crossed into
    /// Remote while this node owned no outbound channel; with no further
    /// local events the seat sat Remote and MOUSE_RELAY kept swallowing the
    /// physical mouse. The watcher reconciliation must force the seat back
    /// to local — and must never rescue a genuinely controlled target.
    #[tokio::test]
    async fn stranded_remote_seat_is_reconciled_without_user_events() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("stranded-seat");

        // Cross into Remote the way the drive path does.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            let crossed = engine.local_move_delta(1919, 500, 4, 0);
            assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Right));
            engine.drain();
        }
        assert_eq!(node.input_engine_seat(), CursorSeat::Remote);
        assert!(
            !node.input_channel_open_sync(),
            "no channel in the stranded case"
        );

        // A genuinely controlled target must be spared.
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_ctl".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Target,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 0,
            last_activity_ms: 0,
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        assert!(
            !node.reconcile_stranded_remote_seat(),
            "a controlled target's remote seat is legitimate"
        );
        assert_eq!(node.input_engine_seat(), CursorSeat::Remote);

        // Grant gone (the stale-tunnel scenario): reconcile rescues the seat.
        *node.shared.active_input_tunnel.lock().unwrap() = None;
        assert!(
            node.reconcile_stranded_remote_seat(),
            "stranded remote seat must be rescued"
        );
        assert_eq!(node.input_engine_seat(), CursorSeat::Local);
        assert_eq!(
            *node.shared.input_seat.lock().unwrap(),
            Some(CursorSeat::Local)
        );
        assert!(!node.is_keyboard_armed(), "arming must be released");
        // Idempotent: nothing left to rescue.
        assert!(!node.reconcile_stranded_remote_seat());
    }

    /// Regression (production 2026-11-11 23:47, v0.2.14): both engines sat
    /// Remote, frames flowed over the persistent channel, and NO session
    /// existed on either side — the reconcile guard used "channel open" as
    /// the skip proxy, and the persistent channel survives session releases,
    /// so the rescue never fired and both physical mice froze until an
    /// operator stop. "Channel open" is not authorization: only a live
    /// Controller grant is.
    #[tokio::test]
    async fn ghost_roam_remote_seat_is_reconciled_despite_open_channel() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("ghost-seat");

        // Cross into Remote the way the drive path does.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            let crossed = engine.local_move_delta(1919, 500, 4, 0);
            assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Right));
            engine.drain();
        }
        assert_eq!(node.input_engine_seat(), CursorSeat::Remote);

        // The persistent channel survives a session release: simulate it
        // with a live queue so `input_channel_open_sync()` is true while no
        // grant exists at all — the exact ghost-roam state.
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let mut rx = rx;
            while rx.recv().await.is_some() {}
        });
        *node.shared.input_out.lock().unwrap() = Some((1, tx));

        // A live Controller grant is legitimate roaming: spare the seat.
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_ctl".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 0,
            last_activity_ms: 0,
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        assert!(
            !node.reconcile_stranded_remote_seat(),
            "a live Controller grant must spare the seat"
        );

        // Grant gone: channel still open, no session anywhere. This is the
        // frozen state the watcher tick must rescue.
        *node.shared.active_input_tunnel.lock().unwrap() = None;
        assert!(
            node.reconcile_stranded_remote_seat(),
            "ghost roam (open channel, no grant) must be rescued"
        );
        assert_eq!(node.input_engine_seat(), CursorSeat::Local);
        assert!(!node.reconcile_stranded_remote_seat(), "idempotent");
    }

    /// Regression (production 2026-11-11 23:47:46, v0.2.14): with the
    /// persistent channel open but no Controller grant, the drive path
    /// crossed into Remote anyway — physical mouse dissociated, deltas
    /// shipped to the peer, no session anywhere. Entering remote roaming
    /// requires a grant; without one the crossing must stay local.
    #[tokio::test]
    async fn drive_crossing_without_controller_grant_stays_local() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("ghost-drive");

        // Persistent channel open (session released underneath it), no
        // grant, no grace deadline. Pair the bottom edge like the incident
        // (Windows bottom <-> Mac top); fast pushes (|dy| >= 6) cross the
        // redundancy band immediately.
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            let mut rx = rx;
            while rx.recv().await.is_some() {}
        });
        *node.shared.input_out.lock().unwrap() = Some((1, tx));
        node.shared.input_policy.lock().unwrap().edge = Some("bottom".to_string());

        // Mirror the incident: the user pushes toward the paired edge.
        let _ = node
            .drive_input_delta(960, 1080, 2, 10, 1920, 1080)
            .unwrap();

        assert_eq!(
            node.input_engine_seat(),
            CursorSeat::Local,
            "no grant, no remote seat"
        );
        assert_ne!(
            *node.shared.input_seat.lock().unwrap(),
            Some(CursorSeat::Remote),
            "shared seat must not read remote without a grant"
        );
        assert!(
            !node.is_keyboard_armed(),
            "capture must stay disarmed without a grant"
        );

        // Now arm a Controller grant and push again: roaming works.
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_ctl".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: Some(tetherly_core::DisplayEdge::Bottom),
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 0,
            last_activity_ms: 0,
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        let _ = node
            .drive_input_delta(960, 1080, 2, 10, 1920, 1080)
            .unwrap();
        assert_eq!(
            node.input_engine_seat(),
            CursorSeat::Remote,
            "with a grant the push roams"
        );
    }

    #[tokio::test]
    async fn session_end_releases_stale_input_tunnel_grant() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("tunnel-release");

        let stale = ActiveInputTunnelSession {
            session_id: "tun_test".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Target,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 0,
            last_activity_ms: 0,
            pre_arm_epoch: 1,
        };
        *node.shared.active_input_tunnel.lock().unwrap() = Some(stale);

        // Mid-screen motion (no edge involved) must NOT touch the
        // reverse-arming accumulator on the normal local path. Two events:
        // the arming accumulator diffs against the previous event position,
        // so the first event after a reset contributes nothing.
        let _ = node.drive_input_delta(956, 540, 4, 0, 1920, 1080).unwrap();
        let _ = node.drive_input_delta(960, 540, 4, 0, 1920, 1080).unwrap();
        let hijacked = node
            .shared
            .reverse_move_accum_px
            .load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            hijacked > 0,
            "stale Target tunnel must be demonstrably hijacking the drive path"
        );

        // Session end: the operator-ordered stop must clear the grant.
        node.input_stop().await.unwrap();
        assert!(
            node.active_tunnel_role().is_none(),
            "input_stop must release the input tunnel grant"
        );
        assert!(
            node.release_input_tunnel_local("idempotent second call")
                .is_none(),
            "releasing an already-cleared tunnel is a no-op"
        );

        // With the grant gone, local motion runs the normal path again —
        // the accumulator stays untouched.
        let _ = node.drive_input_delta(956, 540, 4, 0, 1920, 1080).unwrap();
        let _ = node.drive_input_delta(960, 540, 4, 0, 1920, 1080).unwrap();
        let after = node
            .shared
            .reverse_move_accum_px
            .load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            after, 0,
            "without a Target tunnel, local motion must not feed reverse arming"
        );
    }

    use tetherly_core::{
        EdgeNeighbor, TopologyStateFrame, TrustStore, TrustedPeer, TYPE_INPUT_CLIP_HINT,
    };

    #[tokio::test]
    async fn test_input_grace_period_buffering() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("input-grace");

        // 1. Initially local seat: send_input_events errors if channel closed
        let ev = InputEvent::move_to(1, 100, 200);
        assert!(node.send_input_events(vec![ev.clone()], "test").is_err());
        assert_eq!(node.shared.input_buffered_frames.lock().unwrap().len(), 0);

        // 2. Set engine seat to Remote:
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(1920, 540, 10, 0); // cross right edge
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }

        // 3. Now send_input_events should enter grace period and buffer the frames
        let res = node.send_input_events(vec![ev], "test");
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

    // Regression: a takeover demolished the peer side of the old channel but
    // `input_out` still held the dead sender, so `open_input` early-returned
    // and every later Enter+Move landed in a nobody-read queue — the
    // production "black hole". The probe must clear the dead slot so the
    // dial proceeds.
    #[tokio::test]
    async fn test_open_input_probe_clears_dead_channel() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("probe-dead");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("probe-dead-peer");
        let other_id = other.identity().device_id().clone();

        let (tx, rx) = tokio::sync::mpsc::channel(8);
        drop(rx);
        *node.shared.input_out.lock().unwrap() = Some((999, tx));

        // Unknown peer makes the post-probe dial fail; the assertion is that
        // the dead slot was cleared on the way there.
        assert!(node.open_input(&other_id).await.is_err());
        assert!(
            node.shared.input_out.lock().unwrap().is_none(),
            "dead channel slot must be cleared by the open_input probe"
        );
    }

    // The probe must leave a HEALTHY channel alone: a queued ClipHint means
    // a live reader, so open_input early-returns without churning a dial.
    #[tokio::test]
    async fn test_open_input_probe_keeps_healthy_channel() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("probe-alive");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("probe-alive-peer");
        let other_id = other.identity().device_id().clone();

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        *node.shared.input_out.lock().unwrap() = Some((999, tx));

        assert!(node.open_input(&other_id).await.is_ok());
        {
            let guard = node.shared.input_out.lock().unwrap();
            assert!(guard.is_some(), "healthy channel must be kept");
        }
        let frame = rx.try_recv().expect("probe frame queued");
        assert_eq!(frame.ty, TYPE_INPUT_CLIP_HINT);
    }

    // Regression: the target never applied the controller's mapped entry,
    // so its cursor stayed parked wherever the previous session left it.
    // A tunnel request carrying entry must place the cursor there.
    #[tokio::test]
    async fn test_tunnel_request_applies_proportional_entry() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("entry-apply");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("entry-peer");
        let peer_id = other.identity().device_id().clone();

        // Target display 1920x1080; the entry lands on the bottom edge.
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);

        let frame = InputTunnelRequestFrame {
            session_id: "tun_entry_test".into(),
            client_device_id: peer_id.to_string(),
            requested_edge: Some(DisplayEdge::Top),
            entry_x: Some(960),
            entry_y: Some(2000), // out of range: must clamp to 1079
            timestamp_ms: 1,
        };
        node.receive_input_tunnel_request(&peer_id, frame).await;

        {
            let guard = node.shared.active_input_tunnel.lock().unwrap();
            let session = guard.as_ref().expect("tunnel session accepted");
            assert_eq!(session.role, InputTunnelRole::Target);
            assert_eq!(session.entry_x, Some(960));
            assert_eq!(session.entry_y, Some(1079), "entry must clamp to display");
        }
        let (sw, sh) = crate::platform::screen_size();
        let (exp_x, exp_y) = (
            960.clamp(0, sw.saturating_sub(1)),
            1079.clamp(0, sh.saturating_sub(1)),
        );
        assert_eq!(
            node.shared.input_sink.lock().unwrap().cursor,
            (exp_x, exp_y),
            "cursor must be placed at the controller-mapped entry"
        );
    }

    // Regression (production 2026-11-12, v0.2.18): the TARGET side never
    // told the engine which shared edge the session sits on. A Windows box
    // with no input_policy.json kept the engine's default Right edge while
    // the real pair is bottom — the user's sustained push at the bottom
    // edge matched no zone in `maybe_auto_reverse` (which reads
    // `engine.edge()`) and was silently swallowed: zero crossing logs, zero
    // takeover attempts. The accept path must arm the engine with the
    // session's edge, so a sustained push at that edge fires the reverse
    // takeover.
    #[tokio::test]
    async fn test_tunnel_accept_sets_engine_edge_for_reverse_takeover() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("target-edge");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("target-edge-peer");
        let peer_id = other.identity().device_id().clone();

        // Windows production shape: 1920x1080, NO policy edge, topology
        // neighbor on the BOTTOM edge (win's neighbor below is the mac).
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        assert!(
            node.input_policy().edge.is_none(),
            "test models the no-policy-edge production box"
        );

        // Seed the peer as live so the takeover attach can resolve it
        // (unreachable addr is fine — the Controller flip in the pre-arm
        // happens before any wire I/O).
        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45719),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );

        // The mac's takeover request names the edge IT crossed — its own
        // Top (the mac sits below windows; crossing Top enters windows).
        // The accept path must INVERT it: windows' shared edge is Bottom,
        // the opposite side of the same physical boundary (production
        // 2026-11-12: v0.2.22 armed the engine with Top as sent and the
        // bottom-edge reverse takeover could never fire).
        let frame = InputTunnelRequestFrame {
            session_id: "tun_target_edge".into(),
            client_device_id: peer_id.to_string(),
            requested_edge: Some(DisplayEdge::Top),
            entry_x: None,
            entry_y: None,
            timestamp_ms: 1,
        };
        node.receive_input_tunnel_request(&peer_id, frame).await;

        {
            let guard = node.shared.active_input_tunnel.lock().unwrap();
            let session = guard.as_ref().expect("tunnel session accepted");
            assert_eq!(session.role, InputTunnelRole::Target);
        }
        // The engine edge must now be the INVERTED request edge (Bottom) —
        // NOT the request as sent (Top) and NOT the default Right.
        {
            let engine = node.shared.input_engine.lock().unwrap();
            assert_eq!(
                engine.edge(),
                tetherly_core::ScreenEdge::Bottom,
                "accept must arm the engine with the target-side shared edge (request inverted)"
            );
        }
        assert_eq!(
            node.shared.input_peer.lock().unwrap().as_ref(),
            Some(&peer_id),
            "input_peer must be seeded so the drive-path topology fallback resolves"
        );

        // End-to-end drive check: with the engine armed for Bottom, a
        // sustained push into the BOTTOM edge must fire the reverse
        // takeover. Distance arming first (~1/4 screen of traversal, the
        // same sequence a real hand produces), then a dwell at the boundary
        // — one into-edge event opens the dwell, the OS-clamped zero-delta
        // tail continues it (win hook clamp behavior).
        let mut positions: Vec<(i32, i32)> = (350..=700).rev().map(|y| (960, y)).collect();
        positions.push((960, 1070)); // boundary approach
        positions.push((960, 1079)); // into-edge event (dy>0): opens the dwell
        for _ in 0..20 {
            positions.push((960, 1079)); // clamped tail: zero-delta, dwell continues
        }
        let mut prev: Option<(i32, i32)> = None;
        for (i, (x, y)) in positions.iter().enumerate() {
            let (dx, dy) = match prev {
                Some((lx, ly)) => (x - lx, y - ly),
                None => (0, 0),
            };
            prev = Some((*x, *y));
            let _ = node
                .drive_input_delta(*x, *y, dx, dy, 1920, 1080)
                .expect("reverse path must not error");
            // Pace the clamped tail so the dwell crosses delay_ms (100ms
            // default) while every gap stays inside TOUCH_GAP_MS (150ms).
            if i >= positions.len() - 22 {
                std::thread::sleep(std::time::Duration::from_millis(12));
            }
        }
        // The takeover must have FIRED (same observable as the win→mac
        // zero-delta regression): the stamp happens the instant the dwell
        // completes. On the pre-fix code the engine edge was Right, the
        // bottom push matched no zone, and the dwell never armed.
        let fired = std::iter::repeat_n((), 100)
            .find(|_| node.shared.last_auto_reverse.lock().unwrap().is_some())
            .is_some();
        assert!(
            fired,
            "with the accept-armed edge, the sustained bottom-edge push must fire the reverse takeover"
        );
    }

    // Production 2026-11-12 (v0.2.23): the entry placement parks the cursor
    // ON the shared edge, so the natural reverse gesture — push straight
    // down from where the cursor already sits — accumulates ~0 coordinate
    // travel (the OS clamps at height-1; dy collapses to zero) and the 300px
    // distance gate never armed. The old code hard-vetoed there AND reset
    // the dwell: every push died in the blocked branch, silently. The
    // two-tier gate lets an UN-ARMED mouse fire after a LONG contiguous
    // dwell, so this exact production shape must now trigger.
    #[tokio::test]
    async fn test_auto_reverse_fires_unarmed_long_dwell_from_parked_edge() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("rev-unarmed");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("rev-unarmed-peer");
        let peer_id = other.identity().device_id().clone();

        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45719),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.set_input_policy(crate::store::InputTunnelPolicy {
            edge: Some("bottom".into()),
            ..crate::store::InputTunnelPolicy::default()
        })
        .expect("policy set");
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        let now = node.shared.clock.unix_ms();
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_unarmed".into(),
            peer_id: peer_id.to_string(),
            role: InputTunnelRole::Target,
            requested_edge: Some(DisplayEdge::Top),
            entry_x: Some(960),
            entry_y: Some(1079),
            capture_width: Some(1920),
            capture_height: Some(1080),
            started_at_ms: now,
            last_activity_ms: now,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        // Zone warmup (v0.2.27): one interior event proves the cursor
        // entered the screen — the fixed production flow is enter → move
        // into the screen → push back to the edge. Without it the zone
        // gate would (correctly) hold the dwell disarmed. The accumulator
        // is zeroed afterwards so the gesture stays in the UN-ARMED tier
        // the test exists to pin.
        let _ = node
            .drive_input_delta(960, 400, 0, -679, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.reverse_zone_dirty.load(Ordering::SeqCst),
            "an interior event must open the zone gate"
        );
        node.shared.reverse_move_accum_px.store(0, Ordering::SeqCst);
        // The warmup's 679px jump also lands in the position accumulator;
        // drop it so the boundary push below stays in the UN-ARMED tier
        // this test pins (a fresh session carries no traversal).
        *node.shared.reverse_last_pos.lock().unwrap() = None;

        // Phase 1 — production dead shape: push at the parked edge with NO
        // traversal first. Short burst (4 events, ~40ms): under the old
        // code this died in the blocked branch; under the new code the
        // un-armed dwell needs 600ms, so the takeover must NOT fire yet.
        let mut prev: Option<(i32, i32)> = None;
        for (x, y) in [(960, 1079), (960, 1079), (960, 1079), (960, 1079)] {
            let (dx, dy) = match prev {
                Some((lx, ly)) => (x - lx, y - ly),
                None => (0, 0),
            };
            prev = Some((x, y));
            let _ = node
                .drive_input_delta(x, y, dx, dy, 1920, 1080)
                .expect("reverse path must not error");
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_none(),
            "a brief un-armed push must not fire the takeover (long-dwell gate)"
        );

        // Phase 2 — same gesture, held: keep the contiguous boundary
        // stream going until the 600ms un-armed dwell completes.
        for _ in 0..70 {
            let (x, y) = (960, 1079);
            let (dx, dy) = match prev {
                Some((lx, ly)) => (x - lx, y - ly),
                None => (0, 0),
            };
            prev = Some((x, y));
            let _ = node
                .drive_input_delta(x, y, dx, dy, 1920, 1080)
                .expect("reverse path must not error");
            std::thread::sleep(std::time::Duration::from_millis(12));
            if node.shared.last_auto_reverse.lock().unwrap().is_some() {
                return;
            }
        }
        panic!(
            "a sustained un-armed push from the parked edge must fire the takeover after the long dwell"
        );
    }

    // Production dead shape (2026-11-12, v0.2.24 postmortem): the Windows
    // LL mouse hook fires only when the cursor MOVES. Entry placement parks
    // the cursor ON the shared edge, the OS clamps it there, and a continued
    // physical push produces NO further events — every push logged exactly
    // one dwell tick ("requiring long dwell") and the 600ms gate starved.
    // The v0.2.24 test above drives a synthetic event stream, which the real
    // OS never delivers; THIS test reproduces the true shape: ONE physical
    // event opens the dwell and records the anchor, then total silence from
    // the OS. The pinned heartbeat must carry the dwell to completion alone.
    #[tokio::test]
    async fn test_auto_reverse_pinned_heartbeat_fires_from_single_pinned_touch() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("rev-hb");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("rev-hb-peer");
        let peer_id = other.identity().device_id().clone();

        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45720),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.set_input_policy(crate::store::InputTunnelPolicy {
            edge: Some("bottom".into()),
            ..crate::store::InputTunnelPolicy::default()
        })
        .expect("policy set");
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        let now = node.shared.clock.unix_ms();
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_hb".into(),
            peer_id: peer_id.to_string(),
            role: InputTunnelRole::Target,
            requested_edge: Some(DisplayEdge::Top),
            entry_x: Some(960),
            entry_y: Some(1079),
            capture_width: Some(1920),
            capture_height: Some(1080),
            started_at_ms: now,
            last_activity_ms: now,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        // temp_test_node never runs run(), so the heartbeat must be started
        // exactly as the runtime starts it.
        node.spawn_reverse_pinned_heartbeat();

        // Zone warmup (v0.2.27): one interior event proves the cursor
        // entered the screen (the fixed production flow: enter → move into
        // the screen → push back). Without it the zone gate would
        // (correctly) keep the heartbeat from completing the dwell.
        let _ = node
            .drive_input_delta(960, 400, 0, -679, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.reverse_zone_dirty.load(Ordering::SeqCst),
            "an interior event must open the zone gate"
        );
        // The warmup's 679px jump would arm the distance gate and kill the
        // zero-delta pinned leniency at the clamp; drop the traversal so
        // the pinned touch below stays in the UN-ARMED tier this test pins.
        node.shared.reverse_move_accum_px.store(0, Ordering::SeqCst);
        *node.shared.reverse_last_pos.lock().unwrap() = None;

        // Phase 1 — the production shape verbatim: ONE pinned boundary event
        // (dy=0 inside the clamp band). It opens the dwell and records the
        // heartbeat anchor; the takeover must not fire yet.
        let _ = node
            .drive_input_delta(960, 1079, 0, 0, 1920, 1080)
            .expect("reverse path must not error");
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_none(),
            "a single pinned touch must not fire immediately (dwell gate)"
        );

        // Phase 2 — OS silence: the cursor is clamped, NO further physical
        // events exist. Only the 30ms heartbeat keeps the dwell contiguous
        // across the 600ms un-armed gate; it must fire the takeover from
        // the anchor. (Async sleep so the current-thread runtime keeps
        // polling the heartbeat task — the production task runs beside the
        // hook thread the same way.)
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if node.shared.last_auto_reverse.lock().unwrap().is_some() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the pinned heartbeat must complete the dwell with no further physical events"
            );
        }
    }

    // v0.2.27 regression (production 18:16): the entry placement parks the
    // incoming cursor ON the shared edge — inside the reverse-trigger zone.
    // Micro hand jitter at the parked position armed the gap-continuity
    // dwell and the pinned heartbeat (v0.2.25) carried it to completion,
    // bouncing the session back to Windows 7s after the user's push had
    // landed (accum_px=0). The zone gate must hold the dwell disarmed for
    // the whole un-armed dwell window when the cursor never entered the
    // screen — jitter stays inside the y>=height-2 clamp core here, so no
    // event ever proves interior presence.
    #[tokio::test]
    async fn test_entry_parked_edge_micro_jitter_cannot_arm_reverse_dwell() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("rev-jitter");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("rev-jitter-peer");
        let peer_id = other.identity().device_id().clone();

        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45721),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.set_input_policy(crate::store::InputTunnelPolicy {
            edge: Some("bottom".into()),
            ..crate::store::InputTunnelPolicy::default()
        })
        .expect("policy set");
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        let now = node.shared.clock.unix_ms();
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_jitter".into(),
            peer_id: peer_id.to_string(),
            role: InputTunnelRole::Target,
            requested_edge: Some(DisplayEdge::Top),
            entry_x: Some(960),
            entry_y: Some(1079),
            capture_width: Some(1920),
            capture_height: Some(1080),
            started_at_ms: now,
            last_activity_ms: now,
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        node.spawn_reverse_pinned_heartbeat();

        // Sustained ±1px jitter at the parked clamp point (1079 stays in the
        // y>=height-2 core on every event), paced past the 600ms un-armed
        // dwell. Async sleep so the heartbeat task keeps ticking alongside.
        let jitter = [(0, 0), (0, -1), (0, 1), (0, -1), (0, 1), (0, -1), (0, 1)];
        for _ in 0..8 {
            for (jx, jy) in jitter {
                let (x, y) = (960 + jx, (1079 + jy).clamp(0, 1079));
                let _ = node
                    .drive_input_delta(x, y, 0, jy, 1920, 1080)
                    .expect("reverse path must not error");
                tokio::time::sleep(std::time::Duration::from_millis(12)).await;
            }
            assert!(
                node.shared.last_auto_reverse.lock().unwrap().is_none(),
                "jitter at the parked entry position must never arm the reverse dwell"
            );
        }
        assert!(
            !node.shared.reverse_zone_dirty.load(Ordering::SeqCst),
            "no event left the trigger zone, so the gate must still be closed"
        );
    }

    // Inversion symmetry (production 2026-11-12, v0.2.22): the accept path
    // must arm the engine with the request's OPPOSITE edge, whichever
    // direction the pair runs. The mac→win regression above covers Top
    // request → Bottom target; this one covers the mirrored Bottom request
    // → Top target (the reverse takeover's own attach: windows crossed its
    // Bottom into the mac, whose shared edge is Top). With the request
    // applied as sent the mac's engine would sit on Bottom and the mac's
    // own top-edge return push would silently match no zone — the same
    // freeze one level flipped.
    #[tokio::test]
    async fn test_tunnel_accept_inverts_request_edge_bottom_request() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("edge-inv-b");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("edge-inv-b-peer");
        let peer_id = other.identity().device_id().clone();

        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Top,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Bottom,
                    offset: 0,
                },
            );
        assert!(
            node.input_policy().edge.is_none(),
            "test models the no-policy-edge production box"
        );

        // The takeover's request: the controller (windows) crossed ITS
        // Bottom edge into the mac.
        let frame = InputTunnelRequestFrame {
            session_id: "tun_edge_inv".into(),
            client_device_id: peer_id.to_string(),
            requested_edge: Some(DisplayEdge::Bottom),
            entry_x: None,
            entry_y: None,
            timestamp_ms: 1,
        };
        node.receive_input_tunnel_request(&peer_id, frame).await;

        {
            let engine = node.shared.input_engine.lock().unwrap();
            assert_eq!(
                engine.edge(),
                tetherly_core::ScreenEdge::Top,
                "accept must invert the request: Bottom crossed by the controller = Top on this side"
            );
        }
    }

    // Wire-level regression: the attach frame must carry the entry coords to
    // the target's session (forward auto_link path sends them; auto-reverse
    // now computes them too — see maybe_auto_reverse).
    #[tokio::test]
    async fn test_tunnel_attach_transmits_entry_to_target() {
        use crate::runtime::clipboard_policy_tests::temp_test_node;

        let (a, _dir_a) = temp_test_node("attach_entry_a");
        let (b, _dir_b) = temp_test_node("attach_entry_b");

        let now = 1u64;
        for (node, peer) in [(&a, &b), (&b, &a)] {
            let tp = TrustedPeer {
                device_id: peer.identity().device_id().clone(),
                id_pk: *peer.identity().id_pk(),
                n_pk: *peer.identity().n_pk(),
                alias: "peer".into(),
                paired_at: now,
                revoked: false,
            };
            node.shared
                .trust
                .lock()
                .expect("trust")
                .put(tp)
                .expect("seed trust");
        }

        b.spawn_listeners().await.expect("spawn b listeners");
        let b_addr = b.control_addr();
        a.dial(b_addr).await.expect("a dials b");
        b.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);

        let b_id = b.identity().device_id().to_string();
        a.input_tunnel_attach(&b_id, Some(DisplayEdge::Bottom), Some(123), Some(456))
            .await
            .expect("attach accepted");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let session = b.shared.active_input_tunnel.lock().unwrap().clone();
            match session {
                Some(s) => {
                    assert_eq!(s.role, InputTunnelRole::Target);
                    assert_eq!(s.entry_x, Some(123), "entry x must survive the wire");
                    assert_eq!(s.entry_y, Some(456), "entry y must survive the wire");
                    break;
                }
                None => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "target never recorded the tunnel session with entry"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
    }

    // Regression (root-fix D): with the channel closed and no grace, an
    // edge push must SELF-HEAL — kick a dial via reconnect_target_peer —
    // instead of silently eating the event until the watcher tick.
    #[tokio::test]
    async fn test_crossing_guard_self_heal_dials() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("guard-heal");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("guard-peer");
        let peer_id = other.identity().device_id().clone();

        // Seed the peer as live (unreachable addr is fine: the spawned dial
        // fails asynchronously; the assertion is that the heal fired).
        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45719),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );

        assert!(!node.input_channel_open_sync());
        let res = node
            .drive_input_delta(10, 10, 1, 0, 100, 50)
            .expect("guard must not error");
        assert_eq!(res, None, "no crossing while channel closed");
        // The heal dial CAS must be held (or already finished) — poll briefly.
        let saw_dialing = std::iter::repeat_n((), 50)
            .find(|_| node.shared.input_dialing.load(Ordering::SeqCst))
            .is_some();
        assert!(
            saw_dialing || !node.input_channel_open_sync(),
            "guard must trigger a heal dial (or keep the channel closed, never wedge)"
        );
    }

    // Regression (v0.2.1 field failure): the gesture-forwarding guard required
    // an active tunnel/remote-session slot, but a BARE input channel
    // controller (edge-crossing roam; deploy-window peer-gone cleared the
    // slot) owns no slot — its swipe was permanently rejected while the
    // mouse/keyboard kept working. The source of truth is the ENGINE SEAT;
    // only the controlled side (tunnel Target / receiving session) is
    // suppressed.
    #[tokio::test]
    async fn test_gesture_swipe_forwards_on_bare_channel_control() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gesture-bare");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("gesture-peer");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        *node.shared.input_out.lock().unwrap() = Some((1, tx));
        *node.shared.input_peer.lock().unwrap() = Some(other.identity().device_id().clone());

        // Bare-channel controller: engine seat Remote, NO tunnel slot, NO
        // remote_session slot. The old guard rejected here — must not.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(100, 50, 2, 0); // cross right edge -> Remote
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }
        assert!(node.active_tunnel_role().is_none());
        assert!(!node.remote_session_active());

        node.drive_gesture_swipe(1)
            .expect("controller swipe must forward");

        // direction>0 = Shift+Alt+Tab: Shift down is the first event.
        let key_down = rx.recv().await.expect("Shift down queued");
        let ev = InputEvent::from_frame(&key_down).expect("frame is an input event");
        assert!(
            matches!(
                ev.kind,
                InputKind::Key {
                    code: 0xA0,
                    down: true,
                    ..
                }
            ),
            "first event of a forwarded right-swipe must be Shift down, got {:?}",
            ev.kind
        );
        let alt_down = rx.recv().await.expect("Alt down queued");
        let ev = InputEvent::from_frame(&alt_down).expect("frame is an input event");
        assert!(
            matches!(
                ev.kind,
                InputKind::Key {
                    code: 0xA4,
                    down: true,
                    ..
                }
            ),
            "second event must be Alt down, got {:?}",
            ev.kind
        );
    }

    // Vertical swipe (direction 0) forwards Win+Tab — Task View, the Mission
    // Control counterpart. The raw VK pair must survive verbatim: it is
    // deliberately NOT routed through the keymap (mac_to_windows would remap
    // Meta->Ctrl, turning it into a browser tab switch).
    #[tokio::test]
    async fn test_gesture_swipe_vertical_forwards_win_tab() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gesture-vswipe");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("gesture-vpeer");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        *node.shared.input_out.lock().unwrap() = Some((1, tx));
        *node.shared.input_peer.lock().unwrap() = Some(other.identity().device_id().clone());

        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(100, 50, 2, 0);
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }

        node.drive_gesture_swipe(0)
            .expect("vertical swipe must forward");

        let win_down = rx.recv().await.expect("Win down queued");
        let ev = InputEvent::from_frame(&win_down).expect("frame is an input event");
        assert!(
            matches!(
                ev.kind,
                InputKind::Key {
                    code: 0x5B,
                    down: true,
                    ..
                }
            ),
            "first event of a vertical swipe must be Win down, got {:?}",
            ev.kind
        );
        let tab_down = rx.recv().await.expect("Tab down queued");
        let ev = InputEvent::from_frame(&tab_down).expect("frame is an input event");
        assert!(
            matches!(
                ev.kind,
                InputKind::Key {
                    code: 0x09,
                    down: true,
                    ..
                }
            ),
            "second event must be Tab down, got {:?}",
            ev.kind
        );
    }

    // Rewriting the gesture map changes what a swipe injects: remapping the
    // vertical swipe to `none` must silence it (no frames), and remapping a
    // horizontal swipe to `desktop_right` must emit Win+Ctrl+Right.
    #[tokio::test]
    async fn test_gesture_map_remap_changes_injected_sequence() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gesture-remap");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("gesture-rpeer");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        *node.shared.input_out.lock().unwrap() = Some((1, tx));
        *node.shared.input_peer.lock().unwrap() = Some(other.identity().device_id().clone());

        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(100, 50, 2, 0);
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }

        // Silenced: map the vertical swipe to `none`.
        {
            let mut map = node.shared.gesture_map.lock().unwrap();
            map.swipe4_up = tetherly_core::gesture_map::GestureAction::None;
        }
        node.drive_gesture_swipe(0)
            .expect("no-op swipe is not an error");
        assert!(
            rx.try_recv().is_err(),
            "a `none`-mapped gesture must inject nothing"
        );

        // Remapped: map the right-swipe to desktop_right (Win+Ctrl+Right).
        {
            let mut map = node.shared.gesture_map.lock().unwrap();
            map.swipe4_right = tetherly_core::gesture_map::GestureAction::DesktopRight;
        }
        node.drive_gesture_swipe(1)
            .expect("remapped swipe must forward");
        let first = rx.recv().await.expect("Win down queued");
        let ev = InputEvent::from_frame(&first).expect("frame is an input event");
        assert!(
            matches!(
                ev.kind,
                InputKind::Key {
                    code: 0x5B,
                    down: true,
                    ..
                }
            ),
            "desktop_right must start with Win down, got {:?}",
            ev.kind
        );
    }

    // The controlled side must never inject an app switch into its own
    // controller: tunnel role Target is suppressed even in the Remote seat.
    #[tokio::test]
    async fn test_gesture_swipe_suppressed_on_tunnel_target() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gesture-tgt");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        *node.shared.input_out.lock().unwrap() = Some((1, tx));

        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(100, 50, 2, 0);
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }
        let now = node.shared.clock.unix_ms();
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_test".into(),
            peer_id: "tdev_peer".into(),
            role: InputTunnelRole::Target,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: now,
            last_activity_ms: now,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        node.drive_gesture_swipe(1)
            .expect("suppression is not an error");
        assert!(
            rx.try_recv().is_err(),
            "target-side swipe must not queue any injected key"
        );
    }

    // An ADR-138 session that is DRIVING this node (SessionTarget) is the
    // same controlled side: suppressed.
    #[tokio::test]
    async fn test_gesture_swipe_suppressed_on_receiving_session() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gesture-recv");

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        *node.shared.input_out.lock().unwrap() = Some((1, tx));

        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.local_move_delta(100, 50, 2, 0);
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }
        let now = node.shared.clock.unix_ms();
        let mut sm = RemoteSessionStateMachine::new();
        let _ = sm.request_received();
        let _ = sm.accepted();
        *node.shared.remote_session.lock().unwrap() = Some(RemoteSessionState {
            state: ActiveRemoteSessionState {
                session_id: "ses_test".into(),
                peer_id: "tdev_peer".into(),
                role: InputTunnelRole::SessionTarget,
                screen_width: 1920,
                screen_height: 1080,
                started_at_ms: now,
                last_activity_ms: now,
            },
            machine: sm,
        });

        node.drive_gesture_swipe(-1)
            .expect("suppression is not an error");
        assert!(
            rx.try_recv().is_err(),
            "receiving-session swipe must not queue any injected key"
        );
    }

    // Regression (win→mac reverse takeover dead): the Windows low-level mouse
    // hook reports WM_MOUSEMOVE with dx=0/dy=0 (MSLLHOOKSTRUCT carries no
    // deltas), while `maybe_auto_reverse`'s direction discipline requires an
    // into-the-edge delta (`Bottom => dy > 0`) and its boundary leniency
    // requires `dy > 0` outside the last 2px. With both fed zeros the dwell
    // resets on every boundary event and the takeover never fires — silently.
    // The hook now derives real deltas from consecutive hook positions, so a
    // sustained downward push into the bottom edge must arm, dwell, and fire
    // the polite takeover toward the topology neighbor.
    #[tokio::test]
    async fn test_auto_reverse_fires_with_zero_deltas_via_position_series() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("rev-zero-d");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("rev-zero-peer");
        let peer_id = other.identity().device_id().clone();

        // Seed the peer as live so resolve_peer inside the takeover's
        // input_tunnel_attach can find it (unreachable addr is fine: the
        // attach's send fails asynchronously, but the Controller flip
        // happens before any wire I/O).
        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45719),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );

        // Windows-like setup: 1920x1080, neighbor on the BOTTOM edge (win's
        // topology points down at the mac) and the node's own edge policy
        // "bottom" — exactly what the production Windows input_policy.json
        // carries. maybe_auto_reverse reads the ENGINE edge, which
        // set_input_policy mirrors.
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.set_input_policy(crate::store::InputTunnelPolicy {
            edge: Some("bottom".into()),
            ..crate::store::InputTunnelPolicy::default()
        })
        .expect("policy set");
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        node.shared.peer_topologies.lock().unwrap().insert(
            peer_id.to_string(),
            TopologyStateFrame {
                device_id: peer_id.to_string(),
                display: DisplayRect::new(1920, 1080, 1.0),
                neighbors: HashMap::new(),
                config: tetherly_core::EdgeCrossingConfig::default(),
                timestamp_ms: 0,
            },
        );

        // A live Controller tunnel session toward that peer (exactly the
        // state left behind after a mac→win forward roam: session survives
        // the peer's Leave on this side).
        let now = node.shared.clock.unix_ms();
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_rev_test".into(),
            peer_id: peer_id.to_string(),
            role: InputTunnelRole::Target,
            requested_edge: Some(DisplayEdge::Top),
            entry_x: Some(960),
            entry_y: Some(1079),
            capture_width: Some(1920),
            capture_height: Some(1080),
            started_at_ms: now,
            last_activity_ms: now,
            pre_arm_epoch: next_pre_arm_epoch(),
        });

        // Distance arming: ~1/4 screen of real traversal first (the same
        // sequence a real push produces), THEN dwell at the boundary. The
        // approach models the FIXED Windows hook: deltas derived from
        // consecutive hook positions (dy>0 while descending). The clamped
        // tail at height-1 models the OS clamp: the position stops so the
        // derived delta collapses to ZERO even though the hand keeps
        // pushing — those zero-delta boundary events must still sustain the
        // dwell via the engine-layer leniency (`dy == 0 && dwell_active`).
        // Before both fixes the hook fed hard zeros: pushing_in was always
        // false and the takeover never fired (silent, no log).
        let positions = [
            (960, 700), // far from bottom edge; arms the accumulator
            (960, 650),
            (960, 600),
            (960, 550),
            (960, 500),
            (960, 450),
            (960, 400),
            (960, 350),  // 350px total — REVERSE_ARM_MIN_PX reached
            (960, 1070), // boundary approach (not yet in the 2px gate)
            (960, 1078), // inside the y >= height-2 gate with zero delta
            (960, 1079), // OS clamp point
            (960, 1079), // clamped: position stops, hand keeps pushing
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
            (960, 1079),
        ];
        let mut prev: Option<(i32, i32)> = None;
        for (i, (x, y)) in positions.iter().enumerate() {
            let (dx, dy) = match prev {
                Some((lx, ly)) => (x - lx, y - ly),
                None => (0, 0),
            };
            prev = Some((*x, *y));
            let _ = node
                .drive_input_delta(*x, *y, dx, dy, 1920, 1080)
                .expect("reverse path must not error");
            // The dwell needs `delay_ms` (topology config; min 50 — the
            // default layout carries 100) of CONTIGUOUS boundary events.
            // Pacing the clamped tail at 12ms lets the elapsed time cross
            // 100ms while every inter-event gap stays well inside the
            // 150ms TOUCH_GAP_MS continuity window.
            if i >= 10 {
                std::thread::sleep(std::time::Duration::from_millis(12));
            }
        }

        // The takeover must have FIRED: `last_auto_reverse` is stamped at
        // the exact moment the dwell completes, before the (network-
        // dependent) attach attempt. Watching the tunnel slot for a role
        // flip would test the network, not the bug: with an unreachable
        // test peer the attach's send_to legitimately errors, but the
        // pre-fix code never even reached this stamp because the dwell
        // reset on every zero-delta boundary event.
        let fired = std::iter::repeat_n((), 100)
            .find(|_| node.shared.last_auto_reverse.lock().unwrap().is_some())
            .is_some();
        assert!(
            fired,
            "sustained boundary push must fire the auto-reverse takeover (dwell must survive the zero-delta clamped tail)"
        );
    }

    /// Takeover demotion must immediately release the REMOTE-seat capture
    /// arming. On macOS `arm_input_capture` disassociated the hardware mouse
    /// and hid the cursor behind shields; `input_stop` used to leave that
    /// armed state in place, so a demoted controller stayed "controlling" —
    /// hidden cursor, unassociated mouse — while receiving inbound control.
    /// The capture worker only realigns armed-state on the next LOCAL
    /// physical event, which injected remote moves never produce (they pass
    /// the tap via the injected magic). Regression: after a takeover
    /// handoff, `input_stop` must drive the armed flag down right away.
    #[tokio::test]
    async fn test_input_stop_releases_remote_capture_arming() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("stop-disarm");

        // Arm state without a real hook: HookControl::set_keyboard on the
        // stub platform is a no-op, so simulate the macOS armed state the
        // runtime actually tracks: the seat flipped Remote arms capture
        // through the worker tail; here we assert the demotion contract at
        // the observable boundary — engine seat, shared seat and the armed
        // query agreeing on "local, disarmed" after input_stop.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            engine.local_move_delta(1920, 540, 10, 0); // cross right edge -> Remote
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }
        *node.shared.input_seat.lock().unwrap() = Some(CursorSeat::Remote);

        let stopped = node.input_stop().await;
        assert!(stopped.is_ok());
        assert_eq!(
            node.input_engine_seat(),
            CursorSeat::Local,
            "engine must be forced local by input_stop"
        );
        assert_eq!(
            *node.shared.input_seat.lock().unwrap(),
            Some(CursorSeat::Local),
            "shared seat must return to local on takeover demotion"
        );
        assert!(
            !node.is_keyboard_armed(),
            "remote-seat arming must be released immediately by input_stop (no cursor-hidden/mouse-unassociated residue)"
        );
    }

    /// Deskflow MSWindowsComputer::onMouseMove alignment: while roaming the
    /// Remote seat, the capture worker must pull the physical cursor back to
    /// screen center after every consumed motion batch — the delta chain is
    /// anchored at the warp target by the echo gate, so it always measures
    /// genuine hand motion from center and never saturates at a screen
    /// boundary (the controller hand and the remote cursor own independent
    /// positions). The method returns the decision; the caller executes the
    /// warp through the echo-gated `warp_mouse_position` (whose hook-side
    /// contract is replayed in platform tests). Asserted at the decision
    /// boundary so the test never warps a real cursor.
    #[tokio::test]
    async fn test_roaming_recenter_decision_is_center_in_remote_seat() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("roam-recenter");

        // Local seat: never decide — local movement owns the whole screen.
        assert_eq!(
            node.roaming_recenter_target(1920, 1080),
            None,
            "local seat owns its edges — no recenter decision"
        );

        // Enter Remote through the Right edge (crossing event x >= width).
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            engine.local_move_delta(1920, 540, 10, 0);
            assert_eq!(engine.seat(), CursorSeat::Remote);
        }

        // Roaming: every consumed batch decides a center pull — the target
        // is exactly the screen center and is INDEPENDENT of the hook
        // position (the decision consumes no position at all: entry-edge,
        // interior and clamped-corner states all converge to the same
        // steady-state center park).
        assert_eq!(
            node.roaming_recenter_target(1920, 1080),
            Some((960, 540)),
            "roaming batch must recenter to screen center"
        );
        assert_eq!(
            node.roaming_recenter_target(2560, 1440),
            Some((1280, 720)),
            "decision scales with the engine's screen geometry"
        );

        // After the seat returns Local, the decision must go silent again.
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.force_local();
        }
        assert_eq!(
            node.roaming_recenter_target(1920, 1080),
            None,
            "returned local seat must not recenter"
        );
    }

    /// v0.2.31 (production 2026-11-13 22:28, 8 s first entry): a pre-arm
    /// accept carries no entry placement, so the cursor on this screen is
    /// still the local user's and the zone gate must stay OPEN — a push at
    /// the shared edge right after the peer returned home must be able to
    /// arm the takeover without an interior detour. An accept WITH entry
    /// placement (a real crossing) keeps the v0.2.27 closed gate.
    #[tokio::test]
    async fn pre_arm_accept_without_entry_keeps_zone_gate_open() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("prearm-gate");
        let (other, _dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node("prearm-gate-peer");
        let peer_id = other.identity().device_id().clone();
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);

        node.receive_input_tunnel_request(
            &peer_id,
            InputTunnelRequestFrame {
                session_id: "tun_prearm".into(),
                client_device_id: peer_id.to_string(),
                requested_edge: None,
                entry_x: None,
                entry_y: None,
                timestamp_ms: 1,
            },
        )
        .await;
        assert_eq!(
            node.shared
                .active_input_tunnel
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.role),
            Some(InputTunnelRole::Target)
        );
        assert!(
            node.shared.reverse_zone_dirty.load(Ordering::SeqCst),
            "pre-arm accept (no entry placement) must leave the zone gate open"
        );
        assert!(
            node.input_trace_snapshot()
                .iter()
                .any(|e| e.kind == "zone_gate" && e.detail.starts_with("open: pre-arm")),
            "the gate decision must be traced"
        );

        // A real crossing accept (entry placement present) closes it again.
        node.receive_input_tunnel_request(
            &peer_id,
            InputTunnelRequestFrame {
                session_id: "tun_cross".into(),
                client_device_id: peer_id.to_string(),
                requested_edge: Some(DisplayEdge::Top),
                entry_x: Some(960),
                entry_y: Some(1079),
                timestamp_ms: 2,
            },
        )
        .await;
        assert!(
            !node.shared.reverse_zone_dirty.load(Ordering::SeqCst),
            "an accept with entry placement parks the cursor on the edge: gate closed"
        );
    }

    /// v0.2.31: the controller's applied Enter parks the cursor (gate
    /// closes, arming state restarts); its applied Leave hands the cursor
    /// back to the local user (gate re-opens).
    #[tokio::test]
    async fn controller_enter_closes_zone_gate_and_leave_reopens_it() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("gate-enter");
        node.shared.reverse_zone_dirty.store(true, Ordering::SeqCst);
        node.shared
            .reverse_move_accum_px
            .store(500, Ordering::SeqCst);

        node.note_controller_entered();
        assert!(!node.shared.reverse_zone_dirty.load(Ordering::SeqCst));
        assert_eq!(
            node.shared.reverse_move_accum_px.load(Ordering::SeqCst),
            0,
            "Enter restarts every arming state"
        );

        node.note_controller_left();
        assert!(node.shared.reverse_zone_dirty.load(Ordering::SeqCst));
        let kinds: Vec<String> = node
            .input_trace_snapshot()
            .iter()
            .filter(|e| e.kind == "zone_gate")
            .map(|e| e.detail.clone())
            .collect();
        assert_eq!(
            kinds,
            vec!["close: controller entered", "open: controller left"]
        );
    }

    /// v0.2.31 instrumentation: a Button pushed from the Remote seat is
    /// counted and traced (`push_button`) so a "left click does nothing on
    /// the mac" report can be split into controller-never-sent vs
    /// target-never-applied without RUST_LOG.
    #[tokio::test]
    async fn drive_input_button_counts_and_traces_push() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("push-button");
        node.shared
            .input_capture_active
            .store(true, std::sync::atomic::Ordering::SeqCst);
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Right);
            assert_eq!(
                engine.local_move_delta(1919, 500, 4, 0),
                Some(tetherly_core::ScreenEdge::Right)
            );
            engine.drain();
        }
        assert_eq!(node.input_engine_seat(), CursorSeat::Remote);

        let _ = node.drive_input_button(tetherly_core::BTN_LEFT, true);
        let _ = node.drive_input_button(tetherly_core::BTN_LEFT, false);
        assert_eq!(node.shared.input_tx_buttons.load(Ordering::SeqCst), 2);
        let pushes: Vec<String> = node
            .input_trace_snapshot()
            .iter()
            .filter(|e| e.kind == "push_button")
            .map(|e| e.detail.clone())
            .collect();
        assert_eq!(pushes.len(), 2);
        assert!(pushes[0].starts_with("mask=1 down=true seq="), "{pushes:?}");
        assert!(
            pushes[1].starts_with("mask=1 down=false seq="),
            "{pushes:?}"
        );

        // Local seat: nothing is pushed, nothing is counted.
        node.shared.input_engine.lock().unwrap().force_local();
        let _ = node.drive_input_button(tetherly_core::BTN_LEFT, true);
        assert_eq!(node.shared.input_tx_buttons.load(Ordering::SeqCst), 2);
    }

    /// v0.2.31: `GET /api/input` exposes the apply counters mirrored from
    /// the target pump plus the controller's button pushes.
    #[tokio::test]
    async fn input_status_report_exposes_apply_counters() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("status-ctr");
        node.shared.input_rx_applied.store(40, Ordering::SeqCst);
        node.shared.input_rx_dropped.store(2, Ordering::SeqCst);
        node.shared.input_rx_buttons.store(6, Ordering::SeqCst);
        node.shared.input_rx_wheel.store(3, Ordering::SeqCst);
        node.shared.input_rx_keys.store(9, Ordering::SeqCst);
        node.shared.input_tx_buttons.store(1, Ordering::SeqCst);

        let rep = node.input_status_report().await;
        assert_eq!(
            (
                rep.applied,
                rep.dropped,
                rep.rx_buttons,
                rep.rx_wheel,
                rep.rx_keys,
                rep.tx_buttons
            ),
            (40, 2, 6, 3, 9, 1)
        );
        let json = serde_json::to_value(&rep).unwrap();
        assert_eq!(json["applied"], 40);
        assert_eq!(json["dropped"], 2);
        // Older CLIs deserialize reports without the new fields.
        let legacy: InputStatusReport =
            serde_json::from_str(r#"{"ok":true,"channel_open":false,"seat":"local"}"#).unwrap();
        assert_eq!(legacy.applied, 0);
        assert_eq!(legacy.tx_buttons, 0);
    }

    /// Shared setup for the v0.2.33 local-owner reverse tests: a node with a
    /// live peer below its bottom edge and an accepted PRE-ARM Target
    /// session (no entry placement — the controller has not crossed, the
    /// cursor on this screen is the local user's).
    async fn local_owner_target_fixture(
        name: &str,
    ) -> (Node, std::path::PathBuf, std::path::PathBuf) {
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node(name);
        let (other, dir_other) =
            crate::runtime::clipboard_policy_tests::temp_test_node(&format!("{name}-peer"));
        let peer_id = other.identity().device_id().clone();
        node.shared.live.lock().unwrap().insert(
            peer_id.clone(),
            crate::runtime::trust::LivePeer {
                device_id: peer_id.clone(),
                name: "peer".into(),
                platform: "test".into(),
                addr: "127.0.0.1:1".parse().unwrap(),
                handshake_hash: [0u8; 32],
                file_port: None,
                input_port: Some(45719),
                screen_port: None,
                caps: Vec::new(),
                path: tetherly_core::PathKind::Lan,
                rtt_ms: None,
                last_seen_ms: 0,
            },
        );
        node.shared.topology_layout.lock().unwrap().display = DisplayRect::new(1920, 1080, 1.0);
        node.set_input_policy(crate::store::InputTunnelPolicy {
            edge: Some("bottom".into()),
            ..crate::store::InputTunnelPolicy::default()
        })
        .expect("policy set");
        node.shared
            .topology_layout
            .lock()
            .unwrap()
            .neighbors
            .insert(
                DisplayEdge::Bottom,
                EdgeNeighbor {
                    peer_id: peer_id.to_string(),
                    peer_edge: DisplayEdge::Top,
                    offset: 0,
                },
            );
        node.receive_input_tunnel_request(
            &peer_id,
            InputTunnelRequestFrame {
                session_id: "tun_prearm_local".into(),
                client_device_id: peer_id.to_string(),
                requested_edge: None,
                entry_x: None,
                entry_y: None,
                timestamp_ms: 1,
            },
        )
        .await;
        assert_eq!(
            node.active_tunnel_role(),
            Some(InputTunnelRole::Target),
            "fixture: pre-arm accept must make this node the Target"
        );
        (node, dir, dir_other)
    }

    /// v0.2.33 (production 2026-11-13 13:05:07 → 13:05:10.70, 3.6 s first
    /// entry): the peer had just returned home and re-pre-armed, so this
    /// node was Target while its own user held the cursor. A deliberate
    /// push into the shared edge took the 600 ms UN-armed dwell, which
    /// every 1px pull-back reset. With no controller on this screen, the
    /// push is the same gesture as a controller-side crossing: a fast
    /// into-edge delta must hand over at once.
    #[tokio::test]
    async fn local_owner_target_fast_push_reverses_immediately() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-local-fast").await;
        let _ = node
            .drive_input_delta(960, 1079, 0, 12, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_some(),
            "a fast into-edge push while the local user owns the cursor must reverse at once"
        );
        assert!(
            node.input_trace_snapshot()
                .iter()
                .any(|e| e.kind == "reverse_fast"),
            "the fast-path decision must be traced"
        );
    }

    /// Slow / pinned variant: the cursor already rests on the clamp row, so
    /// the OS reports zero deltas while the hand pushes. The short
    /// controller-parity dwell must complete well before the 600 ms
    /// un-armed dwell would.
    #[tokio::test]
    async fn local_owner_target_pinned_push_reverses_after_short_dwell() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-local-pinned").await;
        let started = std::time::Instant::now();
        for _ in 0..40 {
            let _ = node
                .drive_input_delta(960, 1079, 0, 0, 1920, 1080)
                .expect("reverse path must not error");
            if node.shared.last_auto_reverse.lock().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        let fired_after = started.elapsed();
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_some(),
            "a held pinned push while the local user owns the cursor must reverse"
        );
        assert!(
            fired_after < std::time::Duration::from_millis(400),
            "the local-owner dwell must be the short controller-parity one, took {fired_after:?}"
        );
    }

    #[tokio::test]
    async fn controller_enter_restores_a_visible_cursor() {
        let (node, _d1, _d2) = local_owner_target_fixture("enter-reveal").await;
        node.note_controller_entered();
        assert!(
            node.input_trace_snapshot()
                .iter()
                .any(|e| e.kind == "cursor_reveal"),
            "a controller entering this screen must force a visible arrow cursor"
        );
    }

    /// Regression (production 2026-11-13 18:34:53, v0.2.36): the Mac was
    /// roaming Windows (cursor blanked + hidden) when the Windows takeover
    /// demoted it. The demotion released the grant without the cursor
    /// restore the edge return performs, so the Windows-driven cursor
    /// entered the Mac menu bar invisible and the user bounced back 8x.
    #[tokio::test]
    async fn demoted_roaming_controller_restores_the_cursor() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("demote-restore");
        {
            let mut engine = node.shared.input_engine.lock().unwrap();
            engine.set_edge(tetherly_core::ScreenEdge::Top);
            let crossed = engine.local_move_delta(700, 0, 0, -40);
            assert_eq!(crossed, Some(tetherly_core::ScreenEdge::Top));
            engine.drain();
        }
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_demote".to_string(),
            peer_id: "tdev_peer".to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: 0,
            last_activity_ms: 0,
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        let _ = node.input_stop().await;
        assert!(
            node.input_trace_snapshot()
                .iter()
                .any(|e| e.kind == "cursor_restore"),
            "releasing a controller grant must restore the local cursor"
        );
    }

    /// v0.2.36 (production 2026-11-13 17:52:17 → 17:56:03): while the
    /// Mac's cursor sat on this Windows Target, the user moved the
    /// Windows mouse down into the shared edge and nothing happened for
    /// minutes — no reverse log line at all. The realistic gesture is a
    /// sweep from the interior whose last pre-clamp event lands inside the
    /// promotion band (y >= height-8) but NOT on the clamp row, then one
    /// clamped event, then OS silence (LL hooks only fire on movement).
    /// That push must still hand over promptly.
    #[tokio::test]
    async fn controller_on_screen_physical_sweep_into_edge_reverses() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-sweep").await;
        node.spawn_reverse_pinned_heartbeat();
        node.note_controller_entered();
        let mut y = 600;
        while y < 1072 {
            let _ = node
                .drive_input_delta(900, y, 0, 24, 1920, 1080)
                .expect("reverse path must not error");
            y += 24;
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        }
        let _ = node
            .drive_input_delta(900, 1074, 0, 24, 1920, 1080)
            .expect("reverse path must not error");
        tokio::time::sleep(std::time::Duration::from_millis(8)).await;
        let _ = node
            .drive_input_delta(900, 1079, 0, 5, 1920, 1080)
            .expect("reverse path must not error");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        while node.shared.last_auto_reverse.lock().unwrap().is_none()
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_some(),
            "a physical sweep into the shared edge must reverse even while the controller's cursor is on screen"
        );
    }

    /// Fence: once the controller's Enter lands (its cursor is parked on
    /// this screen) the misfire protections are back — a single fast push
    /// must NOT bounce the session. The Leave hands the cursor back to the
    /// local user and re-enables the fast path.
    #[tokio::test]
    async fn controller_on_screen_disables_fast_reverse_until_leave() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-local-fence").await;
        node.note_controller_entered();
        // Prove interior presence so only the dwell (not the zone gate)
        // stands between the push and a takeover.
        let _ = node
            .drive_input_delta(960, 400, 0, -679, 1920, 1080)
            .expect("reverse path must not error");
        node.shared.reverse_move_accum_px.store(0, Ordering::SeqCst);
        *node.shared.reverse_last_pos.lock().unwrap() = None;
        let _ = node
            .drive_input_delta(960, 1079, 0, 12, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_none(),
            "with the controller on screen a single push must not reverse"
        );

        node.note_controller_left();
        let _ = node
            .drive_input_delta(960, 1079, 0, 12, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_some(),
            "after the controller's Leave the local user's push must reverse at once"
        );
    }

    /// v0.2.33 (a): on Windows the center park warp ran ~6 ms BEFORE the
    /// 14-call SetSystemCursor hide, so the arrow flashed at screen center
    /// on every entry (production 13:05:10.998 warp, 13:05:11.004 hidden).
    /// With `hide_first` the hide step precedes the warp; on return with
    /// `restore_last` the cursor is moved home before it is shown again.
    #[tokio::test]
    async fn remote_seat_capture_orders_hide_and_restore_around_warps() {
        let (node, _dir) = crate::runtime::clipboard_policy_tests::temp_test_node("cursor-order");
        node.enter_remote_seat_capture(960, 540, true);
        node.leave_remote_seat_capture(900, 1079, true);
        let kinds: Vec<String> = node
            .input_trace_snapshot()
            .iter()
            .map(|e| e.kind.clone())
            .filter(|k| {
                matches!(
                    k.as_str(),
                    "cursor_hide" | "park_warp" | "return_warp" | "cursor_restore"
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["cursor_hide", "park_warp", "return_warp", "cursor_restore"]
        );

        // macOS keeps its proven order: warp, then arm (dissociate + hide);
        // disarm (re-associate) then warp home.
        let (node, _dir) =
            crate::runtime::clipboard_policy_tests::temp_test_node("cursor-order-mac");
        node.enter_remote_seat_capture(960, 540, false);
        node.leave_remote_seat_capture(900, 1079, false);
        let kinds: Vec<String> = node
            .input_trace_snapshot()
            .iter()
            .map(|e| e.kind.clone())
            .filter(|k| {
                matches!(
                    k.as_str(),
                    "cursor_hide" | "park_warp" | "return_warp" | "cursor_restore"
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["park_warp", "cursor_hide", "cursor_restore", "return_warp"]
        );
    }

    /// v0.2.34 (production 2026-11-13 16:01:49.6 → 16:01:51.14): the
    /// controller left this Target through the shared edge and its cursor
    /// stayed parked ON that edge — a visible leftover pointer, and a
    /// pinned clamp-band position from which one stray zero-delta touch
    /// let the local-owner short dwell steal the controller role (the
    /// peer was then a Target and its next crossing needed a slow reverse).
    /// The Leave must park the cursor off the edge and clear the pinned
    /// anchor; a deliberate push to the edge must still hand over at once.
    #[tokio::test]
    async fn controller_leave_parks_target_cursor_off_the_shared_edge() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-leave-park").await;
        node.note_controller_entered();
        // The controller's last moves pinned the cursor on the shared edge.
        let _ = node
            .drive_input_delta(960, 1079, 0, 0, 1920, 1080)
            .expect("reverse path must not error");
        node.note_controller_left();
        let parked = node.park_target_cursor_after_leave(true, (1920, 1080));
        assert_eq!(parked, Some((960, 540)), "leave parks at screen center");
        assert!(
            node.shared.reverse_pinned_touch.lock().unwrap().is_none(),
            "the leave park must drop any pinned-edge anchor"
        );
        assert!(
            node.input_trace_snapshot()
                .iter()
                .any(|e| e.kind == "target_park"),
            "the leave park must be traced"
        );
        for _ in 0..25 {
            let _ = node
                .drive_input_delta(960, 540, 0, 0, 1920, 1080)
                .expect("reverse path must not error");
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_none(),
            "stray zero-delta events at the parked point must never reverse"
        );
        let _ = node
            .drive_input_delta(960, 1079, 0, 12, 1920, 1080)
            .expect("reverse path must not error");
        assert!(
            node.shared.last_auto_reverse.lock().unwrap().is_some(),
            "a deliberate push to the edge after the park must still reverse at once"
        );
    }

    /// Platforms without the leave park (flag off) keep the cursor where
    /// the controller left it and emit no park trace.
    #[tokio::test]
    async fn controller_leave_park_disabled_is_a_no_op() {
        let (node, _d1, _d2) = local_owner_target_fixture("rev-leave-nopark").await;
        node.note_controller_left();
        assert_eq!(
            node.park_target_cursor_after_leave(false, (1920, 1080)),
            None
        );
        assert!(!node
            .input_trace_snapshot()
            .iter()
            .any(|e| e.kind == "target_park"));
    }
}

#[cfg(test)]
mod remote_screen_tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// v0.2.32: the tunnel ACCEPT already carries the target's screen size
    /// (`capture_width/height`); the controller must feed it into the engine
    /// so remote deltas normalize 1:1 against the PEER screen, and the
    /// grant's end must clear it (the next peer may differ). The persisted
    /// `remote_speed_percent` is mirrored into the engine on the drive
    /// path like the other edge-crossing knobs.
    #[tokio::test]
    async fn tunnel_accept_feeds_peer_screen_into_engine_and_release_clears_it() {
        let (node, _dir, clock) =
            crate::runtime::clipboard_policy_tests::temp_test_node_manual_clock(
                "peer-screen",
                10_000,
            );
        node.shared
            .input_capture_active
            .store(true, Ordering::SeqCst);
        let peer = DeviceId::from_id_pk(&[44u8; 32]);

        // Pre-armed controller grant (the attach path) and a live outbound
        // channel so the accept path never dials.
        *node.shared.active_input_tunnel.lock().unwrap() = Some(ActiveInputTunnelSession {
            session_id: "tun_screen".to_string(),
            peer_id: peer.to_string(),
            role: InputTunnelRole::Controller,
            requested_edge: None,
            entry_x: None,
            entry_y: None,
            capture_width: None,
            capture_height: None,
            started_at_ms: clock.unix_ms(),
            last_activity_ms: clock.unix_ms(),
            pre_arm_epoch: next_pre_arm_epoch(),
        });
        let (tx, _rx) = tokio::sync::mpsc::channel::<tetherly_core::InnerFrame>(64);
        *node.shared.input_out.lock().unwrap() = Some((0, tx));

        node.receive_input_tunnel_resp(
            &peer,
            InputTunnelRespFrame {
                session_id: "tun_screen".to_string(),
                accepted: true,
                reject_reason: None,
                capture_width: Some(1470),
                capture_height: Some(956),
                timestamp_ms: clock.unix_ms(),
            },
        )
        .await;
        assert_eq!(
            node.shared.input_engine.lock().unwrap().remote_screen(),
            Some((1470, 956)),
            "accept must hand the peer screen to the engine"
        );

        node.topology_update_config(
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(150),
        )
        .await
        .unwrap();
        // An interior local move runs the config mirror without crossing.
        assert_eq!(
            node.drive_input_delta(100, 100, 1, 0, 1920, 1080).unwrap(),
            None
        );
        assert_eq!(
            node.shared
                .input_engine
                .lock()
                .unwrap()
                .remote_speed_percent(),
            150,
            "drive path must mirror remote_speed_percent into the engine"
        );

        node.release_input_tunnel_local("test");
        assert_eq!(
            node.shared.input_engine.lock().unwrap().remote_screen(),
            None,
            "the grant's end must forget the peer screen"
        );
    }
}

#[cfg(test)]
mod attach_channel_tests {
    use super::*;
    use tetherly_core::{TrustStore, TrustedPeer};

    /// v0.2.32: `input_tunnel_attach` must drop the previous outbound input
    /// channel BEFORE the request hits the wire. Production (v0.2.31 trace):
    /// the peer's ACCEPT resp was processed while attach was still inside
    /// `send_to`, so the resp dialed channel gen N, attach then nulled
    /// `input_out` and the next drive dialed gen N+1 — two dials per
    /// takeover and the first Mac entry stalled on the dead gen N. Modelled
    /// with a peer whose `send_to` fails: the stale channel installed before
    /// attach must be gone even though the request never left, which only
    /// holds when the drop precedes the send.
    #[tokio::test]
    async fn attach_drops_stale_outbound_channel_before_the_request_hits_the_wire() {
        use crate::runtime::clipboard_policy_tests::temp_test_node;

        let (node, _dir) = temp_test_node("attach_channel_drop");
        let peer = DeviceId::from_id_pk(&[77u8; 32]);
        node.shared
            .trust
            .lock()
            .expect("trust")
            .put(TrustedPeer {
                device_id: peer.clone(),
                id_pk: [77u8; 32],
                n_pk: [78u8; 32],
                alias: "unreachable".into(),
                paired_at: 1,
                revoked: false,
            })
            .expect("seed trust");

        let (tx, _rx) = tokio::sync::mpsc::channel::<tetherly_core::InnerFrame>(4);
        *node.shared.input_out.lock().unwrap() = Some((41, tx));

        let err = node
            .input_tunnel_attach(&peer.to_string(), Some(DisplayEdge::Bottom), None, None)
            .await
            .expect_err("peer has no session: send must fail");
        assert!(
            err.to_string().contains("not connected"),
            "unexpected failure: {err}"
        );
        assert!(
            node.shared.input_out.lock().unwrap().is_none(),
            "stale outbound channel must be dropped before send_to, not after"
        );
    }
}
