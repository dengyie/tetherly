// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ADR-138 remote desktop session orchestration (controller side).
//!
//! `remote_session_start` chains the six steps the ADR freezes: polite
//! release of any roaming controller tunnel → session request/resp → screen
//! channel open + Start → Session-mode Enter (done by the resp handler) →
//! viewer subprocess spawn → heartbeat loop. The target-side receive path
//! lives in `runtime/input.rs`; this module owns the caller side.

use super::input::{ActiveRemoteSessionState, RemoteSessionState};
use super::*;
use tetherly_core::frame::InnerFrame;
use tetherly_core::{
    CoreError, DeviceId, InputTunnelRole as CoreRole, RemoteSessionPhase, RemoteSessionReason,
    RemoteSessionRequestFrame, RemoteSessionStateMachine,
};

/// Report returned by `POST /api/remote/session/start`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RemoteSessionStartReport {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    /// Host physical resolution — the viewer viewport mapping baseline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_pid: Option<u32>,
    pub message: String,
    pub timestamp_ms: u64,
}

impl Node {
    /// Controller side: open a remote desktop control session to `peer`.
    ///
    /// Steps (ADR-138 §2.6):
    /// 1. governance — if this node roams as tunnel Controller, polite-release first;
    /// 2. signaling — `RemoteSessionRequest` → `RemoteSessionResp` (resolution + keymap);
    /// 3. screen flow — `open_screen` + `screen_start` (existing path);
    /// 4. focus — the resp handler sends the Session Enter (`enter_session`);
    /// 5. viewer — spawn `tetherly-viewer`, its PID goes into the report;
    /// 6. heartbeat — `TYPE_INPUT_TUNNEL_HEARTBEAT` reuses `idle_timeout_secs`.
    pub async fn remote_session_start(
        &self,
        peer_query: &str,
        reason: RemoteSessionReason,
    ) -> Result<RemoteSessionStartReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();

        // Single-session invariant: one session per node, either side.
        if self.remote_session_active() {
            return Err(CoreError::InvalidPayload(
                "a remote desktop session is already active".to_string(),
            ));
        }

        // 1. Polite takeover (ADR-136 invariant): never hold two outbound
        //    input channels. A target role is untouched — the session will
        //    replace it on the peer's accept via close.
        if self.active_tunnel_role() == Some(CoreRole::Controller) {
            let _ = self
                .input_tunnel_release(Some("remote session taking over"))
                .await;
            self.record_audit(
                AuditCategory::Config,
                AuditSeverity::Info,
                "remote_session_takeover_released",
                Some(peer_str.clone()),
                "Released local controller tunnel: remote session requested".to_string(),
                true,
            );
        }

        // 2. Signaling. Optimistically register as SessionController so a
        //    racing request from the peer sees Busy; the resp or the close
        //    path clears the slot.
        let session_id = format!("rs_{}", now_ms);
        let my_id = self.shared.identity.device_id().to_string();
        {
            let mut sm = RemoteSessionStateMachine::new();
            sm.request_sent()
                .map_err(|e| CoreError::InvalidPayload(e.to_string()))?;
            *self.shared.remote_session.lock().expect("remote_session") =
                Some(RemoteSessionState {
                    state: ActiveRemoteSessionState {
                        session_id: session_id.clone(),
                        peer_id: peer_str.clone(),
                        role: CoreRole::SessionController,
                        screen_width: 0,
                        screen_height: 0,
                        started_at_ms: now_ms,
                        last_activity_ms: now_ms,
                    },
                    machine: sm,
                });
        }
        let req = RemoteSessionRequestFrame {
            session_id: session_id.clone(),
            client_device_id: my_id,
            reason,
            timestamp_ms: now_ms,
        };
        let req_frame: InnerFrame = req.to_frame()?;
        if let Err(e) = self.send_to(&peer_id, req_frame).await {
            *self.shared.remote_session.lock().expect("remote_session") = None;
            // Never leave a stale auto-allow latch from an earlier session
            // latched across unrelated attempts.
            self.shared
                .remote_session_auto_allowed
                .store(false, std::sync::atomic::Ordering::Relaxed);
            return Err(CoreError::InvalidPayload(format!(
                "failed to send remote session request: {e}"
            )));
        }

        // 3. Input channel. The session Enter (step 4, sent by the resp
        //    handler) rides the existing 45719 wire; without a live channel
        //    the Enter lands in the grace buffer and the host's focused gate
        //    would drop every frame. Same retry shape as `input_tunnel_attach`:
        //    a takeover's teardown races this dial.
        if let Err(e) = self.open_input(&peer_id).await {
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
                self.teardown_session_silent(&session_id).await;
                return Err(CoreError::InvalidPayload(format!(
                    "input channel open for session failed: {e}"
                )));
            }
        }

        // 4. Wait for the host's resp. The resp handler sends the session
        //    Enter through 45719 and flips this node's phase to Controlling.
        //    Ordering note: the 45720 screen dial MUST wait until the host
        //    has *processed* the request — its accept handler auto-allows
        //    screen consent only from ScreenState::Idle, and this dial itself
        //    moves the host to Requested (on_peer_connected). Dialing first
        //    races the handler and permanently wedges consent at Requested
        //    (production trace: host stuck Requested, viewer PingTimeout).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let (resp_state, _resp_phase) = loop {
            let snap = self
                .shared
                .remote_session
                .lock()
                .expect("remote_session")
                .as_ref()
                .map(|s| (s.state.clone(), s.machine.phase));
            match snap {
                None => {
                    return Err(CoreError::InvalidPayload(format!(
                        "remote session rejected or closed by peer {peer_str}"
                    )));
                }
                Some((state, phase)) => {
                    if phase == RemoteSessionPhase::Controlling {
                        break (state, phase);
                    }
                }
            }
            if std::time::Instant::now() > deadline {
                self.teardown_session_silent(&session_id).await;
                return Err(CoreError::PingTimeout);
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };

        // 5. Screen flow: dial the host's 45720 and ask it to stream. The
        //    host's accept already opened consent (auto-allow in the request
        //    handler), so Start proceeds without a manual allow click.
        //    Reuse a channel that is already open to this peer (an operator
        //    pre-dial, an earlier view) instead of stacking a second 45720
        //    connection: the replacement dial makes the old host-side loop
        //    tear down, and its exit can land between our allow and the new
        //    attach, wiping operator consent and refusing the session's Start
        //    (m6_7 part-2 CI race). A channel to a DIFFERENT peer is replaced
        //    — the session must ride a channel to its own host.
        {
            // Only reuse a channel this machine holds as the VIEWER for this
            // exact peer. `screen_peer` is also written by the server-side
            // attach (someone watching us), and `send_screen_control` falls
            // back to the server channel when `screen_ctl` is missing — a
            // role-blind check here would send the session's Start into the
            // peer's viewer pump, where it is ignored, and the start would
            // silently time out into a black screen.
            let open_peer = self.shared.screen_peer.lock().expect("screen_peer").clone();
            let ctl_open = self.shared.screen_ctl.lock().await.is_some();
            eprintln!(
                "[m6dbg] reuse check: open_peer={:?} ctl_open={} (reuse={})",
                open_peer.as_ref().map(|p| p.as_str()),
                ctl_open,
                ctl_open && open_peer.as_ref() == Some(&peer_id)
            );
            if !(ctl_open && open_peer.as_ref() == Some(&peer_id)) {
                if let Err(e) = self.open_screen(&peer_id).await {
                    self.teardown_session_silent(&session_id).await;
                    return Err(CoreError::InvalidPayload(format!("screen open: {e}")));
                }
            }
        }
        if let Err(e) = self.screen_start().await {
            self.teardown_session_silent(&session_id).await;
            return Err(CoreError::InvalidPayload(format!("screen start: {e}")));
        }

        let viewer_pid = self.spawn_session_viewer(&peer_id);
        self.spawn_session_heartbeat(&session_id);
        Ok(RemoteSessionStartReport {
            ok: true,
            session_id: Some(resp_state.session_id),
            peer_id: Some(resp_state.peer_id),
            screen_width: Some(resp_state.screen_width),
            screen_height: Some(resp_state.screen_height),
            viewer_pid,
            message: "remote session controlling".to_string(),
            timestamp_ms: self.shared.clock.unix_ms(),
        })
    }

    /// Controller side: gracefully end the active session (window close, Esc,
    /// REST stop). Idempotent — no session is a no-op report.
    pub async fn remote_session_stop(
        &self,
        reason: &str,
    ) -> Result<RemoteSessionStartReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let state = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .clone();
        let Some(state) = state else {
            return Ok(RemoteSessionStartReport {
                ok: true,
                session_id: None,
                peer_id: None,
                screen_width: None,
                screen_height: None,
                viewer_pid: None,
                message: "no active remote session".to_string(),
                timestamp_ms: now_ms,
            });
        };
        let peer_id = DeviceId::parse(&state.peer_id).ok();
        *self.shared.remote_session.lock().expect("remote_session") = None;
        *self.shared.remote_session_keymap.lock().expect("rs_keymap") = None;
        self.shared
            .remote_session_auto_allowed
            .store(false, std::sync::atomic::Ordering::Relaxed);

        if let Some(peer) = peer_id {
            let close = tetherly_core::RemoteSessionCloseFrame {
                session_id: state.session_id.clone(),
                reason: reason.to_string(),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = close.to_frame() {
                let _ = self.send_to(&peer, inner).await;
            }
        }
        // Controller side: stop the screen flow and the session's 45719
        // channel; the peer's close handler flushed its held inputs via
        // exit_session.
        let _ = self.screen_stop().await;
        let _ = self.input_stop().await;

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "remote_session_closed",
            Some(state.peer_id.clone()),
            format!("Remote session {} stopped: {reason}", state.session_id),
            true,
        );
        Ok(RemoteSessionStartReport {
            ok: true,
            session_id: Some(state.state.session_id.clone()),
            peer_id: Some(state.state.peer_id.clone()),
            screen_width: Some(state.state.screen_width),
            screen_height: Some(state.state.screen_height),
            viewer_pid: None,
            message: format!("remote session stopped: {reason}"),
            timestamp_ms: now_ms,
        })
    }

    /// Pause/resume the controller-side session heartbeat loop. The heartbeat
    /// is what refreshes the host's session slot; pausing it is how "real
    /// silence" is simulated in tests (the idle-timeout floor is 10s, so a
    /// silent-but-refreshing session can never expire).
    pub fn set_session_heartbeat_paused(&self, paused: bool) {
        self.shared
            .session_heartbeat_paused
            .store(paused, std::sync::atomic::Ordering::Relaxed);
    }

    /// Status snapshot for `GET /api/remote/session`.
    pub fn remote_session_report(&self) -> RemoteSessionStartReport {
        let now_ms = self.shared.clock.unix_ms();
        match self.remote_session_status() {
            Some(state) => {
                let phase = self
                    .shared
                    .remote_session
                    .lock()
                    .expect("remote_session")
                    .as_ref()
                    .map(|s| s.machine.phase)
                    .unwrap_or(RemoteSessionPhase::Idle);
                RemoteSessionStartReport {
                    ok: true,
                    session_id: Some(state.session_id),
                    peer_id: Some(state.peer_id),
                    screen_width: Some(state.screen_width),
                    screen_height: Some(state.screen_height),
                    viewer_pid: None,
                    message: phase.as_str().to_string(),
                    timestamp_ms: now_ms,
                }
            }
            None => RemoteSessionStartReport {
                ok: true,
                session_id: None,
                peer_id: None,
                screen_width: None,
                screen_height: None,
                viewer_pid: None,
                message: RemoteSessionPhase::Idle.as_str().to_string(),
                timestamp_ms: now_ms,
            },
        }
    }

    /// Clear the session slot on controller-side start failures (used on
    /// screen/input failures). "Silent" means no report to the REST caller —
    /// it still notifies the peer with a Close so the host's single slot is
    /// freed; otherwise the host would stay Busy until its idle sweep.
    async fn teardown_session_silent(&self, session_id: &str) {
        let guard = self
            .shared
            .remote_session
            .lock()
            .expect("remote_session")
            .clone();
        if let Some(state) = guard {
            if state.session_id == session_id {
                *self.shared.remote_session.lock().expect("remote_session") = None;
                *self.shared.remote_session_keymap.lock().expect("rs_keymap") = None;
                self.shared
                    .remote_session_auto_allowed
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                if let Ok(peer) = DeviceId::parse(&state.peer_id) {
                    let close = tetherly_core::RemoteSessionCloseFrame {
                        session_id: state.session_id.clone(),
                        reason: "viewer-side setup failed".to_string(),
                        timestamp_ms: self.shared.clock.unix_ms(),
                    };
                    if let Ok(inner) = close.to_frame() {
                        let _ = self.send_to(&peer, inner).await;
                    }
                }
                let _ = self.screen_stop().await;
                let _ = self.input_stop().await;
            }
        }
    }

    /// Spawn `tetherly-viewer` for the session window. Returns its PID when
    /// the platform ships the AppKit viewer subprocess (macOS); CI/headless
    /// hosts report `None` and the session still works wire-side.
    fn spawn_session_viewer(&self, _peer: &DeviceId) -> Option<u32> {
        match crate::platform::spawn_remote_session_viewer() {
            Ok(pid) => Some(pid),
            Err(e) => {
                tracing::warn!("session viewer unavailable: {e}");
                None
            }
        }
    }

    /// Heartbeat loop: reuse `TYPE_INPUT_TUNNEL_HEARTBEAT` for session
    /// liveness. `idle_timeout_secs` (InputTunnelPolicy) governs both sides;
    /// the target's `remote_session_idle_check` force-closes on silence, and
    /// the heartbeat keeps both the target's session slot and this side's
    /// tunnel table fresh (`receive_input_tunnel_heartbeat` refreshes both).
    /// Consecutive send failures mean the peer link is gone and the target's
    /// own sweep may never reach us (its Close can be lost with the link), so
    /// after `HEARTBEAT_FAIL_LIMIT` failures the controller tears itself down
    /// instead of holding the slot (and the suppressed watcher) forever.
    fn spawn_session_heartbeat(&self, session_id: &str) {
        const HEARTBEAT_FAIL_LIMIT: u32 = 3;
        let node = self.clone_handle();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut send_failures: u32 = 0;
            loop {
                ticker.tick().await;
                let alive = node
                    .shared
                    .remote_session
                    .lock()
                    .expect("remote_session")
                    .as_ref()
                    .is_some_and(|s| s.session_id == session_id);
                if !alive {
                    return;
                }
                if node
                    .shared
                    .session_heartbeat_paused
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    // Paused (tests simulate silence this way): skip the
                    // self-refresh AND the wire heartbeat so the target's
                    // idle sweep observes real silence.
                    continue;
                }
                node.touch_remote_session(&session_id);
                let hb = tetherly_core::InputTunnelHeartbeatFrame {
                    session_id: session_id.clone(),
                    seq: 0,
                    timestamp_ms: node.shared.clock.unix_ms(),
                };
                if let Ok(inner) = hb.to_frame() {
                    let peer = {
                        let guard = node.shared.remote_session.lock().expect("remote_session");
                        match guard.as_ref() {
                            Some(s) if s.session_id == session_id => {
                                DeviceId::parse(&s.peer_id).ok()
                            }
                            _ => None,
                        }
                    };
                    if let Some(peer) = peer {
                        match node.send_to(&peer, inner).await {
                            Ok(()) => send_failures = 0,
                            Err(e) => {
                                send_failures += 1;
                                tracing::warn!(
                                    "session {session_id}: heartbeat #{send_failures} to {peer} failed: {e}"
                                );
                                if send_failures >= HEARTBEAT_FAIL_LIMIT {
                                    tracing::warn!(
                                        "session {session_id}: peer unreachable after \
                                         {HEARTBEAT_FAIL_LIMIT} heartbeats — self-teardown"
                                    );
                                    node.teardown_session_silent(&session_id).await;
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}
