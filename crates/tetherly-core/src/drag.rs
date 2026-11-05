// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-screen roaming file drag & drop domain primitives.
//!
//! Provides the protocol messages, coordinate normalization, and pure deterministic
//! state machine for Engine II (Roaming Drag Penetration / Deskflow+ semantics).
//!
//! INVARIANTS:
//! - Pure OS-free & safe Rust only (`#![forbid(unsafe_code)]` inherited from crate root).
//! - All remote coordinates are normalized into unit floats `[0.0, 1.0]` to eliminate
//!   Retina vs 4K Per-Monitor DPI scaling artifacts.
//! - Strict state machine guards against modal hang and orphaned drag sessions.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};

/// Maximum duration a drag session may remain active before timing out (10 seconds).
pub const DRAG_SESSION_TIMEOUT_MS: u64 = 10_000;

/// Maximum serialized size of any `DragControlMsg` on the wire (1 KiB).
/// Enforced on decode so a hostile or buggy peer cannot force large allocations.
pub const DRAG_CONTROL_MSG_MAX: usize = 1024;

/// Default normalized entry position when a drag intent omits coordinates:
/// the right edge, vertically centered — where a left→right roam crosses over.
pub const DEFAULT_EDGE_ENTRY_NORM: (f32, f32) = (0.99, 0.5);

/// Default normalized drop position when a drop confirm omits coordinates:
/// the center of the target screen.
pub const DEFAULT_DROP_NORM: (f32, f32) = (0.5, 0.5);

/// Reason for aborting a roaming drag session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragCancelReason {
    /// User pressed Escape key on either screen.
    UserEscPressed,
    /// Dragged back across the border to the local screen.
    DraggedBackToLocal,
    /// Session timed out without a drop confirmation.
    SessionTimeout,
    /// Underlying TCP connection disconnected.
    NetworkDisconnected,
}

/// Control-plane messages exchanged over TCP 45719 for roaming drag & drop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DragControlMsg {
    /// Preflight metadata signal: cursor carrying files crosses border into remote screen.
    RemoteDragEnter {
        /// Unique drag session identifier (timestamp + monotonic counter).
        drag_id: u64,
        /// Number of files included in this drag action.
        file_count: u32,
        /// Cumulative size in bytes across all files (for progress bar & disk check).
        total_bytes: u64,
        /// Name of the primary / first file (for ghost overlay rendering).
        first_file_name: String,
        /// Normalized entry X coordinate in `[0.0, 1.0]`.
        norm_x: f32,
        /// Normalized entry Y coordinate in `[0.0, 1.0]`.
        norm_y: f32,
        /// Whether this drag payload contains a directory / directory hierarchy.
        has_directory: bool,
    },

    /// High-frequency position sync to update remote ghost overlay position.
    RemoteDragMove {
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
    },

    /// Drag session aborted. Remote host must instantly destroy ghost overlay window.
    RemoteDragCancel {
        drag_id: u64,
        reason: DragCancelReason,
    },

    /// Left mouse button released (LMB Up) on remote screen: commit drop and initiate transfer.
    RemoteDropConfirm {
        drag_id: u64,
        drop_norm_x: f32,
        drop_norm_y: f32,
    },
}

impl DragControlMsg {
    /// Serialize into an inner frame for the session channel.
    pub fn to_frame(&self) -> Result<crate::frame::InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        if payload.len() > DRAG_CONTROL_MSG_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        crate::frame::InnerFrame::new(crate::frame::TYPE_DRAG_CONTROL, payload)
    }

    /// Decode from an inner frame payload, enforcing the 1 KiB wire budget.
    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() > DRAG_CONTROL_MSG_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        Ok(serde_json::from_slice(payload)?)
    }
}

/// Lifecycle phases of a cross-screen drag session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DragPhase {
    /// Initial idle state.
    Idle,
    /// Mouse down + initial movement on local screen (candidate for roaming).
    LocalCandidate,
    /// Cursor touching screen border with valid dragged files.
    CrossingBorder,
    /// Cursor is active on remote screen; ghost overlay rendered.
    RemoteDragActive,
    /// LMB released on remote screen; transfer in-flight.
    Dropping,
    /// File written to disk, verified, and highlighted in file manager.
    Delivered,
    /// Cancelled; cleaned up.
    Cancelled,
}

/// Active drag session descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DragSession {
    pub drag_id: u64,
    pub file_count: u32,
    pub total_bytes: u64,
    pub first_file_name: String,
    pub norm_x: f32,
    pub norm_y: f32,
    pub has_directory: bool,
    pub phase: DragPhase,
    pub started_ms: u64,
    pub updated_ms: u64,
    pub cancel_reason: Option<DragCancelReason>,
}

impl DragSession {
    /// Create a new session in `CrossingBorder` phase.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        drag_id: u64,
        file_count: u32,
        total_bytes: u64,
        first_file_name: impl Into<String>,
        norm_x: f32,
        norm_y: f32,
        has_directory: bool,
        now_ms: u64,
    ) -> Self {
        Self {
            drag_id,
            file_count,
            total_bytes,
            first_file_name: first_file_name.into(),
            norm_x: norm_x.clamp(0.0, 1.0),
            norm_y: norm_y.clamp(0.0, 1.0),
            has_directory,
            phase: DragPhase::CrossingBorder,
            started_ms: now_ms,
            updated_ms: now_ms,
            cancel_reason: None,
        }
    }

    /// Check if session has exceeded the maximum duration without updates.
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.updated_ms) > DRAG_SESSION_TIMEOUT_MS
    }
}

/// Normalize screen coordinates `(x, y)` to `[0.0, 1.0]`.
#[inline]
pub fn normalize_coord(x: i32, y: i32, screen_w: u32, screen_h: u32) -> (f32, f32) {
    let w = screen_w.max(1) as f32;
    let h = screen_h.max(1) as f32;
    let nx = (x as f32 / w).clamp(0.0, 1.0);
    let ny = (y as f32 / h).clamp(0.0, 1.0);
    (nx, ny)
}

/// Denormalize `(norm_x, norm_y)` to absolute target desktop coordinates.
#[inline]
pub fn denormalize_coord(
    norm_x: f32,
    norm_y: f32,
    screen_left: i32,
    screen_top: i32,
    screen_w: u32,
    screen_h: u32,
) -> (i32, i32) {
    let nx = norm_x.clamp(0.0, 1.0);
    let ny = norm_y.clamp(0.0, 1.0);
    let x = screen_left + (nx * screen_w as f32).round() as i32;
    let y = screen_top + (ny * screen_h as f32).round() as i32;
    (x, y)
}

/// Pure deterministic state machine for managing cross-screen drag sessions.
#[derive(Debug, Default, Clone)]
pub struct DragStateMachine {
    active_session: Option<DragSession>,
}

impl DragStateMachine {
    pub fn new() -> Self {
        Self {
            active_session: None,
        }
    }

    /// Return reference to the active session if one exists.
    pub fn active_session(&self) -> Option<&DragSession> {
        self.active_session.as_ref()
    }

    /// Return current phase or `DragPhase::Idle` if no session is active.
    pub fn current_phase(&self) -> DragPhase {
        self.active_session
            .as_ref()
            .map(|s| s.phase)
            .unwrap_or(DragPhase::Idle)
    }

    /// Transition to `RemoteDragActive` upon receiving `RemoteDragEnter`.
    #[allow(clippy::too_many_arguments)]
    pub fn on_remote_enter(
        &mut self,
        drag_id: u64,
        file_count: u32,
        total_bytes: u64,
        first_file_name: String,
        norm_x: f32,
        norm_y: f32,
        has_directory: bool,
        now_ms: u64,
    ) -> Result<&DragSession, CoreError> {
        let mut session = DragSession::new(
            drag_id,
            file_count,
            total_bytes,
            first_file_name,
            norm_x,
            norm_y,
            has_directory,
            now_ms,
        );
        session.phase = DragPhase::RemoteDragActive;
        self.active_session = Some(session);
        Ok(self.active_session.as_ref().unwrap())
    }

    /// Update remote position if drag_id matches active session.
    pub fn on_remote_move(
        &mut self,
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
        now_ms: u64,
    ) -> Result<&DragSession, CoreError> {
        let session = self
            .active_session
            .as_mut()
            .filter(|s| s.drag_id == drag_id)
            .ok_or(CoreError::UnknownTransfer)?;

        if session.phase != DragPhase::RemoteDragActive {
            return Err(CoreError::UnknownTransfer);
        }

        session.norm_x = norm_x.clamp(0.0, 1.0);
        session.norm_y = norm_y.clamp(0.0, 1.0);
        session.updated_ms = now_ms;
        Ok(session)
    }

    /// Confirm drop upon LMB release.
    pub fn on_drop_confirm(
        &mut self,
        drag_id: u64,
        drop_norm_x: f32,
        drop_norm_y: f32,
        now_ms: u64,
    ) -> Result<&DragSession, CoreError> {
        let session = self
            .active_session
            .as_mut()
            .filter(|s| s.drag_id == drag_id)
            .ok_or(CoreError::UnknownTransfer)?;

        if session.phase != DragPhase::RemoteDragActive {
            return Err(CoreError::UnknownTransfer);
        }

        session.phase = DragPhase::Dropping;
        session.norm_x = drop_norm_x.clamp(0.0, 1.0);
        session.norm_y = drop_norm_y.clamp(0.0, 1.0);
        session.updated_ms = now_ms;
        Ok(session)
    }

    /// Mark delivered after file transfer and disk verification completes.
    pub fn on_delivered(&mut self, drag_id: u64, now_ms: u64) -> Result<(), CoreError> {
        let session = self
            .active_session
            .as_mut()
            .filter(|s| s.drag_id == drag_id)
            .ok_or(CoreError::UnknownTransfer)?;

        session.phase = DragPhase::Delivered;
        session.updated_ms = now_ms;
        // Reset to Idle after delivery
        self.active_session = None;
        Ok(())
    }

    /// Abort session with a specific reason.
    pub fn on_cancel(
        &mut self,
        drag_id: u64,
        reason: DragCancelReason,
    ) -> Result<DragCancelReason, CoreError> {
        if let Some(session) = self.active_session.as_mut() {
            if session.drag_id == drag_id {
                session.phase = DragPhase::Cancelled;
                session.cancel_reason = Some(reason);
                self.active_session = None;
                return Ok(reason);
            }
        }
        Err(CoreError::UnknownTransfer)
    }

    /// Prune active session if timed out. Returns `true` if a session was pruned.
    pub fn prune_expired(&mut self, now_ms: u64) -> bool {
        if let Some(ref session) = self.active_session {
            if session.is_expired(now_ms) {
                self.active_session = None;
                return true;
            }
        }
        false
    }

    /// Unconditionally reset state machine to Idle (e.g. on disconnect).
    pub fn reset(&mut self) {
        self.active_session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_drag_control_msg_roundtrip_json() {
        let enter = DragControlMsg::RemoteDragEnter {
            drag_id: 1730773000,
            file_count: 3,
            total_bytes: 104_857_600,
            first_file_name: "archive.zip".into(),
            norm_x: 0.125,
            norm_y: 0.875,
            has_directory: false,
        };

        let encoded = serde_json::to_string(&enter).expect("json serialize");
        let decoded: DragControlMsg = serde_json::from_str(&encoded).expect("json deserialize");
        assert_eq!(enter, decoded);

        let cancel = DragControlMsg::RemoteDragCancel {
            drag_id: 1730773000,
            reason: DragCancelReason::UserEscPressed,
        };
        let enc_cancel = serde_json::to_string(&cancel).expect("cancel serialize");
        let dec_cancel: DragControlMsg =
            serde_json::from_str(&enc_cancel).expect("cancel deserialize");
        assert_eq!(cancel, dec_cancel);
    }

    #[test]
    fn test_coordinate_normalization_and_denormalization() {
        let (nx, ny) = normalize_coord(960, 540, 1920, 1080);
        assert!((nx - 0.5).abs() < 1e-4);
        assert!((ny - 0.5).abs() < 1e-4);

        // Denormalize on a target 4K screen with offset
        let (x, y) = denormalize_coord(nx, ny, 1920, 0, 3840, 2160);
        assert_eq!(x, 1920 + 1920);
        assert_eq!(y, 1080);

        // Clamping bounds
        let (cx, cy) = normalize_coord(-100, 5000, 1920, 1080);
        assert_eq!(cx, 0.0);
        assert_eq!(cy, 1.0);
    }

    #[test]
    fn test_drag_state_machine_lifecycle() {
        let mut sm = DragStateMachine::new();
        assert_eq!(sm.current_phase(), DragPhase::Idle);

        // Enter
        sm.on_remote_enter(42, 1, 2048, "report.pdf".into(), 0.1, 0.2, false, 1000)
            .expect("enter");
        assert_eq!(sm.current_phase(), DragPhase::RemoteDragActive);

        // Move
        sm.on_remote_move(42, 0.5, 0.6, 1100).expect("move");
        assert_eq!(sm.active_session().unwrap().norm_x, 0.5);

        // Drop confirm
        sm.on_drop_confirm(42, 0.5, 0.6, 1200).expect("confirm");
        assert_eq!(sm.current_phase(), DragPhase::Dropping);

        // Delivered
        sm.on_delivered(42, 1500).expect("delivered");
        assert_eq!(sm.current_phase(), DragPhase::Idle);
    }

    #[test]
    fn test_drag_state_machine_cancel_and_timeout() {
        let mut sm = DragStateMachine::new();
        sm.on_remote_enter(100, 2, 4096, "test.png".into(), 0.1, 0.1, false, 1000)
            .expect("enter");

        // Timeout check before 10s
        assert!(!sm.prune_expired(5000));
        assert_eq!(sm.current_phase(), DragPhase::RemoteDragActive);

        // Timeout check after 10s
        assert!(sm.prune_expired(12_000));
        assert_eq!(sm.current_phase(), DragPhase::Idle);

        // Re-enter and cancel with ESC
        sm.on_remote_enter(101, 1, 1024, "doc.txt".into(), 0.3, 0.3, false, 15_000)
            .expect("enter");
        let r = sm
            .on_cancel(101, DragCancelReason::UserEscPressed)
            .expect("cancel");
        assert_eq!(r, DragCancelReason::UserEscPressed);
        assert_eq!(sm.current_phase(), DragPhase::Idle);
    }

    #[test]
    fn test_drag_control_msg_frame_roundtrip_and_size_cap() {
        use crate::frame::TYPE_DRAG_CONTROL;
        let enter = DragControlMsg::RemoteDragEnter {
            drag_id: 7,
            file_count: 1,
            total_bytes: 10,
            first_file_name: "a.txt".into(),
            norm_x: 0.99,
            norm_y: 0.5,
            has_directory: false,
        };
        let frame = enter.to_frame().expect("to_frame");
        assert_eq!(frame.ty, TYPE_DRAG_CONTROL);
        let decoded = DragControlMsg::from_payload(&frame.payload).expect("from_payload");
        assert_eq!(enter, decoded);

        // Oversized payloads are rejected on decode.
        let huge = vec![b'x'; DRAG_CONTROL_MSG_MAX + 1];
        assert!(DragControlMsg::from_payload(&huge).is_err());

        // A message whose serialized form exceeds the cap is rejected on encode.
        let big = DragControlMsg::RemoteDragEnter {
            drag_id: 1,
            file_count: 1,
            total_bytes: 1,
            first_file_name: "n".repeat(DRAG_CONTROL_MSG_MAX),
            norm_x: 0.0,
            norm_y: 0.0,
            has_directory: false,
        };
        assert!(big.to_frame().is_err());
    }

    #[test]
    fn test_drag_phase_serde_snake_case_contract() {
        // The wire/API contract is snake_case; renaming a variant must not
        // silently change it (regression net for the Debug-format bug).
        assert_eq!(
            serde_json::to_string(&DragPhase::RemoteDragActive).unwrap(),
            "\"remote_drag_active\""
        );
        assert_eq!(serde_json::to_string(&DragPhase::Idle).unwrap(), "\"idle\"");
        assert_eq!(
            serde_json::to_string(&DragPhase::Dropping).unwrap(),
            "\"dropping\""
        );
    }
}
