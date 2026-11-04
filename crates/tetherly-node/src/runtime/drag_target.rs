// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Target / Receiver node drag coordinator (Engine II).
//!
//! Handles:
//! - Inbound `DragControlMsg` processing via `DragStateMachine`.
//! - Ghost Drag Overlay lifecycle: render, position tracking, spinner mode, and instant teardown.
//! - Post-transfer settlement: Mark of the Web (Zone.Identifier) and native file manager reveal.

use std::path::Path;
use tetherly_core::drag::{
    DragCancelReason, DragControlMsg, DragPhase, DragSession, DragStateMachine,
};
use tetherly_core::error::CoreError;
use tracing::{info, warn};

use super::shell_reveal::{apply_motw, reveal_in_file_manager};

/// Visual state of the remote ghost overlay.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", content = "payload", rename_all = "snake_case")]
pub enum GhostOverlayVisual {
    /// Ghost window is hidden.
    #[default]
    Hidden,
    /// Actively following cursor, displaying file icon, name, and total size.
    Active {
        first_file_name: String,
        file_count: u32,
        total_bytes: u64,
        has_directory: bool,
    },
    /// Drop confirmed, displaying transfer progress spinner.
    DroppingProgress {
        first_file_name: String,
    },
}

/// Target-side manager orchestrating incoming cross-screen drag events and overlay feedback.
#[derive(Debug, Default)]
pub struct DragTarget {
    state_machine: DragStateMachine,
    visual_state: GhostOverlayVisual,
    current_norm_pos: (f32, f32),
}

impl DragTarget {
    pub fn new() -> Self {
        Self {
            state_machine: DragStateMachine::new(),
            visual_state: GhostOverlayVisual::Hidden,
            current_norm_pos: (0.0, 0.0),
        }
    }

    pub fn current_phase(&self) -> DragPhase {
        self.state_machine.current_phase()
    }

    pub fn visual_state(&self) -> &GhostOverlayVisual {
        &self.visual_state
    }

    pub fn current_norm_pos(&self) -> (f32, f32) {
        self.current_norm_pos
    }

    pub fn active_session(&self) -> Option<&DragSession> {
        self.state_machine.active_session()
    }

    /// Handle incoming `RemoteDragEnter` signal from controller.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_drag_enter(
        &mut self,
        drag_id: u64,
        file_count: u32,
        total_bytes: u64,
        first_file_name: String,
        norm_x: f32,
        norm_y: f32,
        has_directory: bool,
        now_ms: u64,
    ) -> Result<(), CoreError> {
        self.state_machine.on_remote_enter(
            drag_id,
            file_count,
            total_bytes,
            first_file_name.clone(),
            norm_x,
            norm_y,
            has_directory,
            now_ms,
        )?;

        self.current_norm_pos = (norm_x.clamp(0.0, 1.0), norm_y.clamp(0.0, 1.0));
        self.visual_state = GhostOverlayVisual::Active {
            first_file_name,
            file_count,
            total_bytes,
            has_directory,
        };

        info!(
            drag_id,
            norm_x,
            norm_y,
            file_count,
            "ghost drag overlay activated on target"
        );

        Ok(())
    }

    /// Handle high-frequency cursor tracking signal.
    pub fn handle_drag_move(
        &mut self,
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
        now_ms: u64,
    ) -> Result<(), CoreError> {
        self.state_machine
            .on_remote_move(drag_id, norm_x, norm_y, now_ms)?;
        self.current_norm_pos = (norm_x.clamp(0.0, 1.0), norm_y.clamp(0.0, 1.0));
        Ok(())
    }

    /// Handle drop confirmation upon LMB release on target screen.
    pub fn handle_drop_confirm(
        &mut self,
        drag_id: u64,
        drop_norm_x: f32,
        drop_norm_y: f32,
        now_ms: u64,
    ) -> Result<DragControlMsg, CoreError> {
        let session = self
            .state_machine
            .on_drop_confirm(drag_id, drop_norm_x, drop_norm_y, now_ms)?;

        self.visual_state = GhostOverlayVisual::DroppingProgress {
            first_file_name: session.first_file_name.clone(),
        };

        info!(
            drag_id,
            drop_norm_x,
            drop_norm_y,
            "drop confirmed on target; requesting data stream"
        );

        Ok(DragControlMsg::RemoteDropConfirm {
            drag_id,
            drop_norm_x: drop_norm_x.clamp(0.0, 1.0),
            drop_norm_y: drop_norm_y.clamp(0.0, 1.0),
        })
    }

    /// Handle drag cancellation signal.
    pub fn handle_drag_cancel(
        &mut self,
        drag_id: u64,
        reason: DragCancelReason,
    ) -> Result<(), CoreError> {
        self.state_machine.on_cancel(drag_id, reason)?;
        self.visual_state = GhostOverlayVisual::Hidden;

        info!(
            drag_id,
            ?reason,
            "ghost drag overlay torn down due to cancellation"
        );

        Ok(())
    }

    /// Finalize drag session when file transfer completes and lands on disk.
    pub fn on_delivery_completed(
        &mut self,
        drag_id: u64,
        saved_file_path: &Path,
        source_peer: &str,
        reveal: bool,
        now_ms: u64,
    ) -> Result<(), CoreError> {
        self.state_machine.on_delivered(drag_id, now_ms)?;
        self.visual_state = GhostOverlayVisual::Hidden;

        // Apply Mark of the Web for audit and safe tagging
        if let Err(e) = apply_motw(saved_file_path, source_peer) {
            warn!(error = %e, "failed to apply Mark of the Web zone identifier");
        }

        // Focus & highlight file in native file manager
        if reveal {
            if let Err(e) = reveal_in_file_manager(saved_file_path) {
                warn!(error = %e, path = %saved_file_path.display(), "failed to reveal delivered file");
            }
        }

        info!(
            drag_id,
            path = %saved_file_path.display(),
            "drag session delivered and revealed in file manager"
        );

        Ok(())
    }

    /// Check and prune expired sessions.
    pub fn prune_expired(&mut self, now_ms: u64) {
        if self.state_machine.prune_expired(now_ms) {
            self.visual_state = GhostOverlayVisual::Hidden;
            warn!("ghost drag session timed out and was cleaned up");
        }
    }

    /// Reset target state completely.
    pub fn reset(&mut self) {
        self.state_machine.reset();
        self.visual_state = GhostOverlayVisual::Hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_target_lifecycle() {
        let mut target = DragTarget::new();
        assert_eq!(target.current_phase(), DragPhase::Idle);
        assert_eq!(*target.visual_state(), GhostOverlayVisual::Hidden);

        // Enter
        target
            .handle_drag_enter(
                555,
                2,
                10240,
                "project_spec.pdf".into(),
                0.2,
                0.3,
                false,
                1000,
            )
            .expect("enter");

        assert_eq!(target.current_phase(), DragPhase::RemoteDragActive);
        assert_eq!(
            *target.visual_state(),
            GhostOverlayVisual::Active {
                first_file_name: "project_spec.pdf".into(),
                file_count: 2,
                total_bytes: 10240,
                has_directory: false,
            }
        );
        assert_eq!(target.current_norm_pos(), (0.2, 0.3));

        // Move
        target.handle_drag_move(555, 0.4, 0.5, 1100).expect("move");
        assert_eq!(target.current_norm_pos(), (0.4, 0.5));

        // Drop confirm
        let reply = target.handle_drop_confirm(555, 0.4, 0.5, 1200).expect("confirm");
        assert_eq!(
            reply,
            DragControlMsg::RemoteDropConfirm {
                drag_id: 555,
                drop_norm_x: 0.4,
                drop_norm_y: 0.5,
            }
        );
        assert_eq!(target.current_phase(), DragPhase::Dropping);
        assert_eq!(
            *target.visual_state(),
            GhostOverlayVisual::DroppingProgress {
                first_file_name: "project_spec.pdf".into(),
            }
        );

        // Delivery
        let tmp = std::env::temp_dir().join("tetherly_test_target_delivery.pdf");
        std::fs::write(&tmp, b"delivered bytes").expect("write temp");
        target
            .on_delivery_completed(555, &tmp, "mac-controller", false, 1500)
            .expect("delivered");

        assert_eq!(target.current_phase(), DragPhase::Idle);
        assert_eq!(*target.visual_state(), GhostOverlayVisual::Hidden);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_drag_target_cancellation() {
        let mut target = DragTarget::new();
        target
            .handle_drag_enter(888, 1, 512, "draft.txt".into(), 0.1, 0.1, false, 1000)
            .expect("enter");
        assert_eq!(target.current_phase(), DragPhase::RemoteDragActive);

        target
            .handle_drag_cancel(888, DragCancelReason::UserEscPressed)
            .expect("cancel");
        assert_eq!(target.current_phase(), DragPhase::Idle);
        assert_eq!(*target.visual_state(), GhostOverlayVisual::Hidden);
    }
}
