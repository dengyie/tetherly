// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Controlling node drag controller (Engine II).
//!
//! Handles:
//! - Drag intent detection: LMB down + Euclidean displacement > 5px.
//! - Preflight packaging: extracts file metadata, cumulative size, and first file name.
//! - Emits `DragControlMsg::RemoteDragEnter` when border is breached.
//! - Manages outgoing drag session lifecycle, cancellation (ESC/drag back), and drop execution.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tetherly_core::drag::{DragCancelReason, DragControlMsg, DragPhase};
use tetherly_core::error::CoreError;

static DRAG_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Information about an active outgoing drag action on the controller.
#[derive(Debug, Clone)]
pub struct OutgoingDragSession {
    pub drag_id: u64,
    pub target_peer: String,
    pub file_paths: Vec<PathBuf>,
    pub file_count: u32,
    pub total_bytes: u64,
    pub first_file_name: String,
    pub has_directory: bool,
    pub norm_x: f32,
    pub norm_y: f32,
    pub phase: DragPhase,
    pub started_ms: u64,
}

/// Request to initiate high-speed file transfer over TCP 45718 upon drop confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingTransferRequest {
    pub drag_id: u64,
    pub target_peer: String,
    pub file_paths: Vec<PathBuf>,
    pub total_bytes: u64,
}

/// Controller coordinator managing outgoing roaming drag operations.
///
/// NOTE: there is intentionally no passive LMB-gesture `DragIntentDetector`.
/// Engine I (tray/edge popover) initiates drags explicitly via the loopback
/// REST API, so a passive detector would be dead code; it was removed per the
/// project's no-unreferenced-code rule.
#[derive(Debug, Default)]
pub struct DragController {
    active_session: Option<OutgoingDragSession>,
}

impl DragController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_session(&self) -> Option<&OutgoingDragSession> {
        self.active_session.as_ref()
    }

    /// Target peer of the active session, if any.
    pub fn active_target_peer(&self) -> Option<&str> {
        self.active_session.as_ref().map(|s| s.target_peer.as_str())
    }

    /// Initiate an outgoing roaming drag across the border with selected files.
    ///
    /// Returns the `RemoteDragEnter` to transmit, plus a `RemoteDragCancel` for
    /// any prior session it supersedes so the remote ghost overlay is never
    /// orphaned (it would otherwise linger until the 10s timeout).
    pub fn initiate_cross_border_drag(
        &mut self,
        target_peer: impl Into<String>,
        paths: Vec<PathBuf>,
        entry_norm_x: f32,
        entry_norm_y: f32,
        now_ms: u64,
    ) -> Result<(DragControlMsg, Option<DragControlMsg>), CoreError> {
        if paths.is_empty() {
            return Err(CoreError::File("dragged file list is empty".into()));
        }

        let mut total_bytes = 0u64;
        let mut has_directory = false;

        for p in &paths {
            if !p.exists() {
                return Err(CoreError::File(format!(
                    "dragged path does not exist: {}",
                    p.display()
                )));
            }
            if p.is_dir() {
                has_directory = true;
            } else if let Ok(meta) = std::fs::metadata(p) {
                total_bytes = total_bytes.saturating_add(meta.len());
            }
        }

        let first_file_name = paths[0]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let drag_id = (now_ms << 16) | (DRAG_ID_COUNTER.fetch_add(1, Ordering::Relaxed) & 0xFFFF);
        let file_count = paths.len() as u32;

        let session = OutgoingDragSession {
            drag_id,
            target_peer: target_peer.into(),
            file_paths: paths,
            file_count,
            total_bytes,
            first_file_name: first_file_name.clone(),
            has_directory,
            norm_x: entry_norm_x.clamp(0.0, 1.0),
            norm_y: entry_norm_y.clamp(0.0, 1.0),
            phase: DragPhase::CrossingBorder,
            started_ms: now_ms,
        };

        // Cancel any prior session so the receiver tears down its ghost overlay
        // instead of leaking it until timeout.
        let superseded_cancel = self
            .active_session
            .take()
            .map(|prev| DragControlMsg::RemoteDragCancel {
                drag_id: prev.drag_id,
                reason: DragCancelReason::UserEscPressed,
            });

        self.active_session = Some(session);

        Ok((
            DragControlMsg::RemoteDragEnter {
                drag_id,
                file_count,
                total_bytes,
                first_file_name,
                norm_x: entry_norm_x.clamp(0.0, 1.0),
                norm_y: entry_norm_y.clamp(0.0, 1.0),
                has_directory,
            },
            superseded_cancel,
        ))
    }

    /// Sync cursor movement on remote screen.
    pub fn sync_remote_move(
        &mut self,
        norm_x: f32,
        norm_y: f32,
    ) -> Option<DragControlMsg> {
        let session = self.active_session.as_mut()?;
        session.norm_x = norm_x.clamp(0.0, 1.0);
        session.norm_y = norm_y.clamp(0.0, 1.0);
        session.phase = DragPhase::RemoteDragActive;

        Some(DragControlMsg::RemoteDragMove {
            drag_id: session.drag_id,
            norm_x: session.norm_x,
            norm_y: session.norm_y,
        })
    }

    /// Peek at the transfer that a drop would commit, without consuming the
    /// session. Lets the caller validate peer reachability and preflight before
    /// any irreversible state change (F4: confirm-then-fail must be retryable).
    pub fn peek_transfer(&self, drag_id: u64) -> Option<OutgoingTransferRequest> {
        let session = self.active_session.as_ref()?;
        if session.drag_id != drag_id {
            return None;
        }
        Some(OutgoingTransferRequest {
            drag_id,
            target_peer: session.target_peer.clone(),
            file_paths: session.file_paths.clone(),
            total_bytes: session.total_bytes,
        })
    }

    /// Commit drop when receiver confirms release. Consumes the session and
    /// returns the request to transfer files. Pair with [`Self::abort_transfer`]
    /// to roll the session back if the subsequent stream fails.
    pub fn on_drop_confirm(
        &mut self,
        drag_id: u64,
        _drop_norm_x: f32,
        _drop_norm_y: f32,
    ) -> Option<OutgoingTransferRequest> {
        let session = self.active_session.take()?;
        if session.drag_id != drag_id {
            self.active_session = Some(session);
            return None;
        }

        Some(OutgoingTransferRequest {
            drag_id,
            target_peer: session.target_peer,
            file_paths: session.file_paths,
            total_bytes: session.total_bytes,
        })
    }

    /// Roll a committed-but-failed transfer back into an active session so the
    /// caller can retry the same `drag_id`. No-op if a session is already
    /// active (never clobber a newer drag).
    pub fn abort_transfer(&mut self, req: OutgoingTransferRequest) {
        if self.active_session.is_some() {
            return;
        }
        let started_ms = 0; // original start lost on rollback; refreshed below
        self.active_session = Some(OutgoingDragSession {
            drag_id: req.drag_id,
            target_peer: req.target_peer,
            file_count: req.file_paths.len() as u32,
            total_bytes: req.total_bytes,
            first_file_name: req
                .file_paths
                .first()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string(),
            has_directory: req.file_paths.iter().any(|p| p.is_dir()),
            norm_x: 0.0,
            norm_y: 0.0,
            file_paths: req.file_paths,
            phase: DragPhase::RemoteDragActive,
            started_ms,
        });
    }

    /// Cancel active outgoing drag. Returns cancel control message to send across wire.
    pub fn cancel(&mut self, reason: DragCancelReason) -> Option<DragControlMsg> {
        let session = self.active_session.take()?;

        Some(DragControlMsg::RemoteDragCancel {
            drag_id: session.drag_id,
            reason,
        })
    }

    /// Reset state machine (e.g. on peer disconnect).
    pub fn reset(&mut self) {
        self.active_session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_controller_lifecycle() {
        let mut ctl = DragController::new();
        let tmp = std::env::temp_dir().join("tetherly_test_controller_item.txt");
        std::fs::write(&tmp, b"sample content").expect("write temp");

        let (msg, superseded) = ctl
            .initiate_cross_border_drag("home-win", vec![tmp.clone()], 0.99, 0.5, 1000)
            .expect("initiate");
        assert!(superseded.is_none());

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
                assert_eq!(file_count, 1);
                assert_eq!(total_bytes, 14);
                assert_eq!(first_file_name, "tetherly_test_controller_item.txt");
                assert_eq!(norm_x, 0.99);
                assert_eq!(norm_y, 0.5);
                assert!(!has_directory);

                // Move sync
                let move_msg = ctl.sync_remote_move(0.5, 0.5).expect("move");
                assert_eq!(
                    move_msg,
                    DragControlMsg::RemoteDragMove {
                        drag_id,
                        norm_x: 0.5,
                        norm_y: 0.5,
                    }
                );

                // Drop confirm
                let xfer = ctl.on_drop_confirm(drag_id, 0.5, 0.5).expect("drop confirm");
                assert_eq!(xfer.target_peer, "home-win");
                assert_eq!(xfer.file_paths, vec![tmp.clone()]);
            }
            _ => panic!("unexpected msg"),
        }

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_reinitiation_emits_cancel_for_superseded_session() {
        let mut ctl = DragController::new();
        let tmp = std::env::temp_dir().join("tetherly_test_reinit.txt");
        std::fs::write(&tmp, b"first").expect("write temp");

        let (first_msg, no_cancel) = ctl
            .initiate_cross_border_drag("home-win", vec![tmp.clone()], 0.99, 0.5, 1000)
            .expect("first initiate");
        assert!(no_cancel.is_none());
        let first_id = match first_msg {
            DragControlMsg::RemoteDragEnter { drag_id, .. } => drag_id,
            _ => panic!("expected enter"),
        };

        // Re-initiate within the timeout window: must supersede with a cancel
        // naming the previous drag_id, never orphan the remote ghost overlay.
        let (_second_msg, cancel) = ctl
            .initiate_cross_border_drag("home-win", vec![tmp.clone()], 0.99, 0.5, 2000)
            .expect("second initiate");
        match cancel {
            Some(DragControlMsg::RemoteDragCancel { drag_id, .. }) => {
                assert_eq!(drag_id, first_id);
            }
            other => panic!("expected superseded cancel, got {other:?}"),
        }

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_confirm_then_abort_transfer_is_retryable() {
        let mut ctl = DragController::new();
        let tmp = std::env::temp_dir().join("tetherly_test_retry.txt");
        std::fs::write(&tmp, b"retry-payload").expect("write temp");

        let (msg, _) = ctl
            .initiate_cross_border_drag("home-win", vec![tmp.clone()], 0.99, 0.5, 1000)
            .expect("initiate");
        let drag_id = match msg {
            DragControlMsg::RemoteDragEnter { drag_id, .. } => drag_id,
            _ => panic!("expected enter"),
        };

        // Peek must not consume the session.
        assert!(ctl.peek_transfer(drag_id).is_some());
        assert!(ctl.active_session().is_some());

        // Commit consumes the session.
        let req = ctl.on_drop_confirm(drag_id, 0.5, 0.5).expect("confirm");
        assert!(ctl.active_session().is_none());

        // A failed stream rolls the session back, so the same drag_id retries.
        ctl.abort_transfer(req);
        assert!(ctl.active_session().is_some());
        assert!(ctl.peek_transfer(drag_id).is_some());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_confirm_with_wrong_drag_id_preserves_session() {
        let mut ctl = DragController::new();
        let tmp = std::env::temp_dir().join("tetherly_test_wrongid.txt");
        std::fs::write(&tmp, b"x").expect("write temp");

        let (msg, _) = ctl
            .initiate_cross_border_drag("home-win", vec![tmp.clone()], 0.99, 0.5, 1000)
            .expect("initiate");
        let real_id = match msg {
            DragControlMsg::RemoteDragEnter { drag_id, .. } => drag_id,
            _ => panic!("expected enter"),
        };

        // A mismatched drag_id must not consume the real session.
        assert!(ctl.on_drop_confirm(real_id.wrapping_add(1), 0.5, 0.5).is_none());
        assert!(ctl.active_session().is_some());

        let _ = std::fs::remove_file(&tmp);
    }
}
