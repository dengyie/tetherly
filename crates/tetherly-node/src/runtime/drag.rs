// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Drag & drop session-channel dispatcher (Engine II wire glue).
//!
//! Bridges `DragControlMsg` (domain, `tetherly-core/src/drag.rs`) onto the
//! Noise-encrypted session channel (TCP 45717) and routes inbound signals to
//! the receiver-side [`DragTarget`]. Also owns the controller-side transmit
//! path used by the loopback REST API.
//!
//! INVARIANTS:
//! - Every signal is length-bounded by `DRAG_CONTROL_MSG_MAX` on both encode
//!   and decode; a hostile or buggy peer cannot force large allocations.
//! - The receiver never auto-executes a delivered payload: settlement only
//!   applies MOTW and reveals the file in the native file manager.
//! - Peer-facing strings used for MOTW are sanitized by the shell layer.

use super::*;
use tetherly_core::drag::DragControlMsg;
use tetherly_core::{CoreError, DeviceId};
use tracing::warn;

impl Node {
    /// Transmit a drag control signal to a connected peer over the session
    /// channel. Returns `CoreError` when the peer is not connected.
    pub async fn send_drag_control(
        &self,
        peer: &DeviceId,
        msg: &DragControlMsg,
    ) -> Result<(), CoreError> {
        let frame = msg.to_frame()?;
        self.send_to(peer, frame).await
    }

    /// Resolve the human-facing name/alias of a peer for MOTW attribution.
    /// Falls back to the raw device id string when no friendly name is known.
    fn drag_peer_label(&self, peer: &DeviceId) -> String {
        self.peer_alias(peer)
            .or_else(|| {
                self.live_peers()
                    .into_iter()
                    .find(|p| &p.device_id == peer)
                    .map(|p| p.name)
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| peer.to_string())
    }

    /// Route an inbound drag control signal from a peer to the receiver-side
    /// state machine. This is the single dispatch point that activates the
    /// previously test-only [`DragTarget`] pipeline.
    pub async fn receive_drag_control(&self, peer: &DeviceId, msg: DragControlMsg) {
        let now = self.now_ms();
        match msg {
            DragControlMsg::RemoteDragEnter {
                drag_id,
                file_count,
                total_bytes,
                first_file_name,
                norm_x,
                norm_y,
                has_directory,
            } => {
                let mut target = self.shared.drag_target.lock().expect("drag_target");
                if let Err(e) = target.handle_drag_enter(
                    drag_id,
                    file_count,
                    total_bytes,
                    first_file_name,
                    norm_x,
                    norm_y,
                    has_directory,
                    now,
                ) {
                    warn!(peer = %peer, drag_id, error = %e, "drag enter rejected");
                }
            }
            DragControlMsg::RemoteDragMove {
                drag_id,
                norm_x,
                norm_y,
            } => {
                let mut target = self.shared.drag_target.lock().expect("drag_target");
                if let Err(e) = target.handle_drag_move(drag_id, norm_x, norm_y, now) {
                    warn!(peer = %peer, drag_id, error = %e, "drag move rejected");
                }
            }
            DragControlMsg::RemoteDragCancel { drag_id, reason } => {
                let mut target = self.shared.drag_target.lock().expect("drag_target");
                if let Err(e) = target.handle_drag_cancel(drag_id, reason) {
                    warn!(peer = %peer, drag_id, error = %e, "drag cancel rejected");
                }
                // A cancelled drop must not linger as a pending delivery.
                self.shared
                    .pending_drag_drops
                    .lock()
                    .expect("pending_drag_drops")
                    .remove(&drag_id);
            }
            DragControlMsg::RemoteDropConfirm {
                drag_id,
                drop_norm_x,
                drop_norm_y,
            } => {
                // The controller tells us the user released LMB on our screen.
                // Advance the target state machine to Dropping and record the
                // drag_id so the first inbound FileOffer of this drop can claim
                // it and the completion hook can settle MOTW + reveal. The
                // transfer itself is initiated by the controller pushing a
                // FileOffer; we do not pull.
                let confirm_echo = {
                    let mut target = self.shared.drag_target.lock().expect("drag_target");
                    match target.handle_drop_confirm(drag_id, drop_norm_x, drop_norm_y, now) {
                        Ok(echo) => Some(echo),
                        Err(e) => {
                            warn!(peer = %peer, drag_id, error = %e, "drop confirm rejected");
                            None
                        }
                    }
                };
                if confirm_echo.is_some() {
                    self.shared
                        .pending_drag_drops
                        .lock()
                        .expect("pending_drag_drops")
                        .insert(drag_id, peer.clone());
                }
                // Echo the confirm back so the controller proceeds to stream.
                if let Some(echo) = confirm_echo {
                    if let Err(e) = self.send_drag_control(peer, &echo).await {
                        warn!(peer = %peer, drag_id, error = %e, "drop confirm echo failed");
                    }
                }
            }
        }
    }

    /// Bind the first inbound `FileOffer` of a drag-originated drop to its
    /// drag session. Called from the receiver offer handler; claims at most
    /// one pending drag per peer (FIFO by insertion is not required — a peer
    /// has at most one active drag at a time).
    pub(crate) fn claim_drag_offer(&self, peer: &DeviceId, transfer_id: &str) {
        let drag_id = {
            let mut pending = self
                .shared
                .pending_drag_drops
                .lock()
                .expect("pending_drag_drops");
            // Find a pending drop from this peer and claim it.
            let found = pending.iter().find(|(_, p)| *p == peer).map(|(id, _)| *id);
            if let Some(id) = found {
                pending.remove(&id);
                Some(id)
            } else {
                None
            }
        };
        if let Some(drag_id) = drag_id {
            self.shared
                .transfer_drag_ids
                .lock()
                .expect("transfer_drag_ids")
                .insert(transfer_id.to_string(), drag_id);
        }
    }

    /// Settle a delivered drag payload when an inbound transfer lands on disk:
    /// apply MOTW and reveal in the file manager. No-op for non-drag transfers.
    pub(crate) fn settle_drag_for_transfer(&self, transfer_id: &str, saved_path: &std::path::Path) {
        let drag_id = self
            .shared
            .transfer_drag_ids
            .lock()
            .expect("transfer_drag_ids")
            .remove(transfer_id);
        let Some(drag_id) = drag_id else { return };
        let peer = self
            .shared
            .pending_drag_drops
            .lock()
            .expect("pending_drag_drops")
            .remove(&drag_id);
        let label = peer
            .as_ref()
            .map(|p| self.drag_peer_label(p))
            .unwrap_or_else(|| "peer".to_string());
        let now = self.now_ms();
        let mut target = self.shared.drag_target.lock().expect("drag_target");
        if let Err(e) = target.on_delivery_completed(drag_id, saved_path, &label, true, now) {
            warn!(drag_id, error = %e, "drag delivery settlement failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use tetherly_core::drag::{DragControlMsg, DragPhase};
    use tetherly_core::ports::TrustStore;
    use tetherly_core::TrustedPeer;

    /// Full wire round-trip: controller A signals over the Noise session
    /// channel (45717) into receiver B's `DragTarget`, and B's drop-confirm
    /// echo flows back onto A's wire. This exercises the exact dispatch path
    /// (`send_to` → frame → `recv_event` → `receive_drag_control`) that was
    /// previously dead code.
    #[tokio::test]
    async fn test_drag_signal_wire_roundtrip_two_nodes() {
        use crate::runtime::clipboard_policy_tests::temp_test_node;

        let (a, _dir_a) = temp_test_node("drag_wire_a");
        let (b, _dir_b) = temp_test_node("drag_wire_b");

        // Seed mutual trust so dial/accept take the ResumeNoise path (no PIN).
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

        // Bring up B's control listener, then dial it from A.
        b.spawn_listeners().await.expect("spawn b listeners");
        let b_addr = b.control_addr();
        a.dial(b_addr).await.expect("a dials b");

        let b_id = b.identity().device_id().clone();
        let a_id = a.identity().device_id().clone();

        // Wait until both ends register a live session.
        let mut tries = 0;
        loop {
            let a_live = a.live_peers().iter().any(|p| p.device_id == b_id);
            let b_live = b.live_peers().iter().any(|p| p.device_id == a_id);
            if a_live && b_live {
                break;
            }
            tries += 1;
            assert!(tries < 100, "session never established");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // Controller A sends RemoteDragEnter to B.
        let enter = DragControlMsg::RemoteDragEnter {
            drag_id: 4242,
            file_count: 1,
            total_bytes: 128,
            first_file_name: "wire.txt".into(),
            norm_x: 0.99,
            norm_y: 0.5,
            has_directory: false,
        };
        a.send_drag_control(&b_id, &enter)
            .await
            .expect("send enter");

        // B's DragTarget must transition to RemoteDragActive (proves the
        // dispatcher routed the inbound signal, not just that the wire moved).
        let mut tries = 0;
        loop {
            let phase = b.drag_target().lock().expect("t").current_phase();
            if phase == DragPhase::RemoteDragActive {
                break;
            }
            tries += 1;
            assert!(tries < 100, "receiver never entered RemoteDragActive");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // A high-frequency move also crosses and updates the receiver position.
        let mv = DragControlMsg::RemoteDragMove {
            drag_id: 4242,
            norm_x: 0.25,
            norm_y: 0.75,
        };
        a.send_drag_control(&b_id, &mv).await.expect("send move");
        let mut tries = 0;
        loop {
            let pos = b.drag_target().lock().expect("t").current_norm_pos();
            if pos == (0.25, 0.75) {
                break;
            }
            tries += 1;
            assert!(tries < 100, "receiver never applied drag move");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // The controller (A) releases LMB: it sends RemoteDropConfirm to the
        // receiver (B), which advances its own target to Dropping, registers a
        // pending delivery, and echoes the confirm back onto A's wire.
        let confirm = DragControlMsg::RemoteDropConfirm {
            drag_id: 4242,
            drop_norm_x: 0.5,
            drop_norm_y: 0.5,
        };
        a.send_drag_control(&b_id, &confirm)
            .await
            .expect("send confirm");

        // B reaches Dropping and records the pending drag so the file
        // completion hook can settle MOTW + reveal once bytes land.
        let mut tries = 0;
        loop {
            let phase = b.drag_target().lock().expect("t").current_phase();
            let pending = b
                .shared
                .pending_drag_drops
                .lock()
                .expect("pending")
                .contains_key(&4242);
            if phase == DragPhase::Dropping && pending {
                break;
            }
            tries += 1;
            if tries >= 100 {
                let visual = b.drag_target().lock().expect("t").visual_state().clone();
                panic!(
                    "receiver never reached Dropping; phase={phase:?} pending={pending} visual={visual:?}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // The echoed confirm returns to A's dispatcher without wedging it (A has
        // no drag_target session for this id; it must be tolerated, not panic).
        let cancel = DragControlMsg::RemoteDragCancel {
            drag_id: 4242,
            reason: tetherly_core::drag::DragCancelReason::UserEscPressed,
        };
        a.send_drag_control(&b_id, &cancel)
            .await
            .expect("send cancel");
        let mut tries = 0;
        loop {
            let phase = b.drag_target().lock().expect("t").current_phase();
            if phase == DragPhase::Idle {
                break;
            }
            tries += 1;
            assert!(tries < 100, "receiver never cancelled back to Idle");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
}
