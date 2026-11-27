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
use tracing::{info, warn};

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

    pub(crate) fn drag_peer_label_pub(&self, peer: &DeviceId) -> String {
        self.drag_peer_label(peer)
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
                if self.native_drag_refresh(drag_id, file_count) {
                    return; // v0.3.0: the pulled handoff's refreshed enter
                }
                if self.native_drag_is_stale_enter(drag_id) {
                    // v0.3.2: a pull that raced the return re-announced a
                    // carry that is already over; taking it over again
                    // started a second helper (E_FAIL, Explorer popped).
                    info!(peer = %peer, drag_id, "carry enter for a returned drag ignored");
                    return;
                }
                let entered = {
                    let mut target = self.shared.drag_target.lock().expect("drag_target");
                    target.handle_drag_enter(
                        drag_id,
                        file_count,
                        total_bytes,
                        first_file_name.clone(),
                        norm_x,
                        norm_y,
                        has_directory,
                        now,
                    )
                };
                match entered {
                    Ok(()) => {
                        self.emit(UiEvent::DragCarry {
                            active: true,
                            file_count,
                            first_file_name,
                        });
                        // v0.3.0: hand the carry to a real OS drag here.
                        self.native_drag_begin(peer, drag_id, file_count, has_directory)
                            .await;
                    }
                    Err(e) => warn!(peer = %peer, drag_id, error = %e, "drag enter rejected"),
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
                if self.native_drag_cancelled(drag_id, reason) {
                    return;
                }
                let mut target = self.shared.drag_target.lock().expect("drag_target");
                if let Err(e) = target.handle_drag_cancel(drag_id, reason) {
                    warn!(peer = %peer, drag_id, error = %e, "drag cancel rejected");
                }
                drop(target);
                self.emit(UiEvent::DragCarry {
                    active: false,
                    file_count: 0,
                    first_file_name: String::new(),
                });
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
                if self.is_outgoing_drag_echo(drag_id) {
                    // v0.2.89: the receiver echoed our own handoff's confirm;
                    // the files are already being offered.
                    return;
                }
                if self.native_drag_confirmed(drag_id) {
                    // v0.3.0: released before the pull was honoured; the
                    // drop was registered at the enter and the helper
                    // decides the outcome. No echo: the controller offers
                    // the files right after its confirm.
                    return;
                }
                // The controller tells us the user released LMB on our screen.
                // Advance the target state machine to Dropping and record the
                // drag_id so the first inbound FileOffer of this drop can claim
                // it and the completion hook can settle MOTW + reveal. The
                // transfer itself is initiated by the controller pushing a
                // FileOffer; we do not pull.
                let confirm_echo = {
                    let mut target = self.shared.drag_target.lock().expect("drag_target");
                    let file_count = target
                        .active_session()
                        .filter(|s| s.drag_id == drag_id)
                        .map(|s| s.file_count)
                        .unwrap_or(1)
                        .max(1);
                    match target.handle_drop_confirm(drag_id, drop_norm_x, drop_norm_y, now) {
                        Ok(echo) => Some((echo, file_count)),
                        Err(e) => {
                            warn!(peer = %peer, drag_id, error = %e, "drop confirm rejected");
                            None
                        }
                    }
                };
                // Released on this screen: the badge gives way to the
                // transfer (and the reveal once it lands).
                self.emit(UiEvent::DragCarry {
                    active: false,
                    file_count: 0,
                    first_file_name: String::new(),
                });
                if let Some((_, file_count)) = &confirm_echo {
                    self.shared
                        .pending_drag_drops
                        .lock()
                        .expect("pending_drag_drops")
                        .insert(
                            drag_id,
                            crate::runtime::PendingDragDrop {
                                peer: peer.clone(),
                                registered_ms: now,
                                remaining: *file_count,
                            },
                        );
                }
                // Echo the confirm back so the controller proceeds to stream.
                if let Some((echo, _)) = confirm_echo {
                    if let Err(e) = self.send_drag_control(peer, &echo).await {
                        warn!(peer = %peer, drag_id, error = %e, "drop confirm echo failed");
                    }
                }
            }
            DragControlMsg::RemoteDragPull { drag_id } => {
                // Controller side: the receiver runs a native drag and wants
                // the carried files now, while the button is still held.
                self.drag_handoff_on_pull(peer, drag_id);
            }
        }
    }

    /// Bind the first inbound `FileOffer` of a drag-originated drop to its
    /// drag session. Called from the receiver offer handler; claims at most
    /// one pending drag per peer (FIFO by insertion is not required — a peer
    /// has at most one active drag at a time). Pending entries older than
    /// [`PENDING_DRAG_TTL_MS`] are purged first: a confirm whose transfer
    /// never arrived (crash, dropped connection) must not mis-claim an
    /// unrelated later offer from the same peer.
    pub(crate) fn claim_drag_offer(&self, peer: &DeviceId, transfer_id: &str) -> bool {
        const PENDING_DRAG_TTL_MS: u64 = 30_000;
        let now = self.now_ms();
        let claimed = {
            let mut pending = self
                .shared
                .pending_drag_drops
                .lock()
                .expect("pending_drag_drops");
            pending.retain(|_, p| now.saturating_sub(p.registered_ms) <= PENDING_DRAG_TTL_MS);
            // A peer has at most one active drag at a time; every file offer
            // of that drop claims it until its file count is used up.
            let found = pending
                .iter()
                .find(|(_, p)| p.peer == *peer)
                .map(|(id, _)| *id);
            found.and_then(|id| {
                let entry = pending.get_mut(&id)?;
                entry.remaining = entry.remaining.saturating_sub(1);
                let claim = (id, entry.peer.clone());
                if entry.remaining == 0 {
                    pending.remove(&id);
                }
                Some(claim)
            })
        };
        let Some((drag_id, source)) = claimed else {
            return false;
        };
        // Source attribution rides with the claim so settlement can label
        // MOTW with the real peer.
        self.shared
            .transfer_drag_ids
            .lock()
            .expect("transfer_drag_ids")
            .insert(transfer_id.to_string(), (source, drag_id));
        true
    }

    /// Settle a delivered drag payload when an inbound transfer lands on disk:
    /// apply MOTW and reveal in the file manager. No-op for non-drag transfers.
    pub(crate) fn settle_drag_for_transfer(&self, transfer_id: &str, saved_path: &std::path::Path) {
        let claimed = self
            .shared
            .transfer_drag_ids
            .lock()
            .expect("transfer_drag_ids")
            .remove(transfer_id);
        let Some((peer, drag_id)) = claimed else {
            return;
        };
        let label = self.drag_peer_label(&peer);
        if self.native_drag_delivered(transfer_id) {
            // v0.3.0: the drag source reads it; no file-manager window.
            if let Err(e) = super::shell_reveal::apply_motw(saved_path, &label) {
                warn!(drag_id, error = %e, "native drag MOTW failed");
            }
            return;
        }
        let now = self.now_ms();
        let mut target = self.shared.drag_target.lock().expect("drag_target");
        if target.active_session().map(|s| s.drag_id) == Some(drag_id) {
            if let Err(e) = target.on_delivery_completed(drag_id, saved_path, &label, true, now) {
                warn!(drag_id, error = %e, "drag delivery settlement failed");
            }
        } else if let Err(e) = super::shell_reveal::apply_motw(saved_path, &label) {
            // Later files of a multi-file drop: the session already settled
            // (and revealed the first file); tag the rest without opening one
            // file-manager window per file.
            warn!(drag_id, error = %e, "drag delivery MOTW failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use tetherly_core::drag::{DragControlMsg, DragPhase};
    use tetherly_core::ports::TrustStore;
    use tetherly_core::TrustedPeer;

    /// The target surfaces the carried files while the cursor is over it
    /// (the shell draws a badge on `drag_carry`) and drops the badge on
    /// release or cancel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drag_enter_announces_the_carried_files_until_drop_or_cancel() {
        use crate::events::UiEvent;
        use crate::runtime::clipboard_policy_tests::temp_test_node;
        let (node, _dir) = temp_test_node("drag_carry");
        let mut rx = node.shared.events.subscribe();
        let peer = tetherly_core::DeviceId::from_id_pk(&[9u8; 32]);
        let carry = |rx: &mut tokio::sync::broadcast::Receiver<UiEvent>| loop {
            match rx.try_recv() {
                Ok(UiEvent::DragCarry {
                    active,
                    file_count,
                    first_file_name,
                }) => return Some((active, file_count, first_file_name)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        };
        node.receive_drag_control(
            &peer,
            DragControlMsg::RemoteDragEnter {
                drag_id: 7,
                file_count: 2,
                total_bytes: 10,
                first_file_name: "a.txt".into(),
                norm_x: 0.0,
                norm_y: 0.5,
                has_directory: false,
            },
        )
        .await;
        assert_eq!(carry(&mut rx), Some((true, 2, "a.txt".into())));
        node.receive_drag_control(
            &peer,
            DragControlMsg::RemoteDragCancel {
                drag_id: 7,
                reason: tetherly_core::drag::DragCancelReason::DraggedBackToLocal,
            },
        )
        .await;
        assert_eq!(carry(&mut rx).map(|c| c.0), Some(false));
    }

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

    /// Regression for the v0.1.76 rollout gap: the receiver-side settlement
    /// chain (pending_drag_drops registration → claim on FileOffer → settle on
    /// FileDone → MOTW + reveal) must fire when — and only when — the
    /// controller's confirm REST handler transmits `RemoteDropConfirm` before
    /// streaming. Exercises the exact in-process order the confirm handler
    /// performs (signal → confirm_drop → offer-claim → FileDone-settle).
    #[tokio::test]
    async fn test_confirm_signal_settles_delivery_with_motw() {
        use crate::runtime::clipboard_policy_tests::temp_test_node;
        use tetherly_core::ports::TrustStore;
        use tetherly_core::TrustedPeer;

        let (a, _dir_a) = temp_test_node("drag_settle_a");
        let (b, _dir_b) = temp_test_node("drag_settle_b");

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
        a.dial(b.control_addr()).await.expect("dial");

        let b_id = b.identity().device_id().clone();

        // The receiver side of the drop: B's DragTarget has an active remote
        // drag session (as after RemoteDragEnter), mirroring production order.
        {
            let mut target = b.drag_target().lock().expect("t");
            target
                .handle_drag_enter(777, 1, 4, "settle.txt".into(), 0.5, 0.5, false, 100)
                .expect("enter");
        }

        // Controller handler order (post-fix): transmit the confirm signal on
        // the wire FIRST. On B this advances the target to Dropping and
        // registers the pending delivery keyed by drag_id.
        let confirm = DragControlMsg::RemoteDropConfirm {
            drag_id: 777,
            drop_norm_x: 0.5,
            drop_norm_y: 0.5,
        };
        a.send_drag_control(&b_id, &confirm)
            .await
            .expect("send confirm");

        let mut tries = 0;
        loop {
            let phase = b.drag_target().lock().expect("t").current_phase();
            let pending = b
                .shared
                .pending_drag_drops
                .lock()
                .expect("pending")
                .contains_key(&777);
            if phase == DragPhase::Dropping && pending {
                break;
            }
            tries += 1;
            assert!(
                tries < 100,
                "receiver must reach Dropping with a pending delivery after the confirm signal"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // The inbound FileOffer claims the pending drop (what the offer
        // handler does), then the completion hook settles it with MOTW.
        let a_id = a.identity().device_id().clone();
        b.claim_drag_offer(&a_id, "tr_settle_regression");
        assert!(
            b.shared
                .transfer_drag_ids
                .lock()
                .expect("ids")
                .contains_key("tr_settle_regression"),
            "offer must claim the pending drag drop"
        );

        let inbox = b.shared.cfg.data_dir.join("inbox");
        std::fs::create_dir_all(&inbox).expect("mkdir inbox");
        let saved = inbox.join("settle.txt");
        std::fs::write(&saved, b"data").expect("write saved file");

        b.settle_drag_for_transfer("tr_settle_regression", &saved);

        // Settlement drives the target back to Idle and clears both maps.
        assert_eq!(
            b.drag_target().lock().expect("t").current_phase(),
            DragPhase::Idle,
            "delivery completion must settle the drag session"
        );
        assert!(
            b.shared.transfer_drag_ids.lock().expect("ids").is_empty(),
            "transfer binding must be consumed by settlement"
        );
        assert!(
            b.shared
                .pending_drag_drops
                .lock()
                .expect("pending")
                .is_empty(),
            "pending drop must be consumed by the offer claim"
        );
        let _ = std::fs::remove_file(&saved);
    }

    /// v0.2.89: a multi-file drag registers one pending drop that every file
    /// offer of that drop claims (not just the first), and the claim tells
    /// the offer handler to auto-accept regardless of the auto-accept policy.
    #[test]
    fn multi_file_drag_drop_claims_every_offer_then_expires() {
        let (node, _dir) =
            crate::runtime::clipboard_policy_tests::temp_test_node("drag-multi-claim");
        let peer = tetherly_core::DeviceId::from_id_pk(&[7u8; 32]);
        node.shared.pending_drag_drops.lock().unwrap().insert(
            42,
            crate::runtime::PendingDragDrop {
                peer: peer.clone(),
                registered_ms: node.now_ms(),
                remaining: 2,
            },
        );
        assert!(node.claim_drag_offer(&peer, "tr_a"));
        assert!(node.claim_drag_offer(&peer, "tr_b"));
        assert!(
            !node.claim_drag_offer(&peer, "tr_c"),
            "a drop of two files must not claim a third, unrelated offer"
        );
        let ids = node.shared.transfer_drag_ids.lock().unwrap();
        assert_eq!(ids.get("tr_a").map(|v| v.1), Some(42));
        assert_eq!(ids.get("tr_b").map(|v| v.1), Some(42));
        assert!(!ids.contains_key("tr_c"));
    }
}
