// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote desktop control session domain primitives (ADR-138).
//!
//! Sunlogin/RustDesk semantics: a viewer explicitly opens a session window and
//! gains full keyboard/mouse control over the peer desktop, distinct from the
//! edge-roaming input tunnel (ADR-136). This module carries the three control
//! frames and the pure session state machine; the orchestration that drives
//! them lives in `tetherly-node/src/runtime/remote_session.rs`.
//!
//! INVARIANTS:
//! - Pure protocol types only (crate root forbids unsafe / target_os cfg).
//! - Session identifiers reuse `InputTunnelValidator::validate_session_id`
//!   (1..=64 chars of `[a-zA-Z0-9_-]`) — one validation surface, no drift.
//!   `sanitize()` blanks an invalid id rather than aliasing it to a sentinel:
//!   the empty string fails `validate_session_id`, so validate-first guards
//!   downstream stay reachable, and no two malformed peers can ever share a
//!   synthetic id.
//! - Reject reasons travel as bounded strings on the wire (`Option<String>`)
//!   but must belong to the `RemoteSessionRejectReason` value domain.
//! - At most one remote session exists per node pair; the state machine has a
//!   single `Idle` sink so every error path collapses to a clean teardown.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_REMOTE_SESSION_CLOSE, TYPE_REMOTE_SESSION_REQUEST, TYPE_REMOTE_SESSION_RESP,
};
use crate::input_tunnel::InputTunnelValidator;

/// Why the viewer initiated a remote session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RemoteSessionReason {
    /// User clicked tray / REST entry on the viewer side.
    #[default]
    UserInitiated,
    /// An API automation triggered the session start.
    ApiTriggered,
}

impl RemoteSessionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserInitiated => "UserInitiated",
            Self::ApiTriggered => "ApiTriggered",
        }
    }
}

impl std::fmt::Display for RemoteSessionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for RemoteSessionReason {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "userinitiated" | "user" | "user_initiated" => Ok(Self::UserInitiated),
            "apitriggered" | "api" | "api_triggered" => Ok(Self::ApiTriggered),
            _ => Err(CoreError::Json(format!(
                "invalid remote session reason: '{s}', expected one of: UserInitiated, ApiTriggered"
            ))),
        }
    }
}

/// Canonical reject reasons the host may answer a request with. Serializes to
/// the exact wire strings the ADR freezes (`PolicyDisabled | ScreenBusy |
/// AllowRequired | Busy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RemoteSessionRejectReason {
    /// `InputTunnelPolicy.allow_inbound_control` is off on the host.
    PolicyDisabled,
    /// Host screen stream is occupied by another pipeline and cannot open.
    ScreenBusy,
    /// Screen streaming is allowed but input requires an explicit local allow.
    AllowRequired,
    /// A remote session already exists on the host (single-session invariant).
    Busy,
}

impl RemoteSessionRejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PolicyDisabled => "PolicyDisabled",
            Self::ScreenBusy => "ScreenBusy",
            Self::AllowRequired => "AllowRequired",
            Self::Busy => "Busy",
        }
    }

    /// Parses a sanitized wire reason back into the canonical domain.
    pub fn from_reason(reason: &str) -> Option<Self> {
        match reason {
            "PolicyDisabled" => Some(Self::PolicyDisabled),
            "ScreenBusy" => Some(Self::ScreenBusy),
            "AllowRequired" => Some(Self::AllowRequired),
            "Busy" => Some(Self::Busy),
            _ => None,
        }
    }
}

/// Viewer → host: open a remote desktop control session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionRequestFrame {
    pub session_id: String,
    pub client_device_id: String,
    pub reason: RemoteSessionReason,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl RemoteSessionRequestFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| String::new());
        self.client_device_id = self.client_device_id.trim().to_string();
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_REMOTE_SESSION_REQUEST, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_REMOTE_SESSION_REQUEST {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Host → viewer: session acceptance carrying the viewport mapping baseline.
///
/// `screen_width`/`screen_height` are the host's physical resolution; the
/// viewer maps window coordinates through normalized units against this pair.
/// `keymap_profile` is the host's suggested preset name (e.g.
/// `"mac_to_windows"`, parseable by `KeyMapProfile::from_preset`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionRespFrame {
    pub session_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    #[serde(default)]
    pub screen_width: u32,
    #[serde(default)]
    pub screen_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap_profile: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl RemoteSessionRespFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| String::new());
        if let Some(ref reason) = self.reject_reason {
            self.reject_reason = Some(InputTunnelValidator::sanitize_reason(reason));
        }
        self.keymap_profile = match self.keymap_profile.as_deref() {
            Some(profile) => {
                let clean = InputTunnelValidator::sanitize_reason(profile);
                // A blank profile name carries no preset; `None` lets the
                // viewer fall back to identity instead of parsing garbage.
                if clean == "unspecified" || clean.is_empty() {
                    None
                } else {
                    Some(clean)
                }
            }
            None => None,
        };
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_REMOTE_SESSION_RESP, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_REMOTE_SESSION_RESP {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Bidirectional: gracefully tear down an active remote session. Sent by the
/// viewer (window closed / Esc / REST stop) or by the host (idle timeout,
/// policy change, stream failure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionCloseFrame {
    pub session_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl RemoteSessionCloseFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| String::new());
        self.reason = InputTunnelValidator::sanitize_reason(&self.reason);
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_REMOTE_SESSION_CLOSE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_REMOTE_SESSION_CLOSE {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Lifecycle of one remote desktop session (ADR-138 §2.3).
///
/// ```text
/// Idle ──request──▶ Requesting ──accept──▶ Streaming ──input_enabled──▶ Controlling
///                      │                      │                            │
///                      └─reject──▶ Idle       └──close/error──▶ Idle ◀──close/error──┘
/// ```
///
/// `Streaming` = screen flow up, input not yet enabled. `Controlling` = input
/// tunnel entered Session mode. The only exit from `Controlling` is `Close`.
/// State is in-memory only: a crash resets to `Idle` by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteSessionPhase {
    Idle,
    Requesting,
    Streaming,
    Controlling,
}

impl RemoteSessionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Requesting => "requesting",
            Self::Streaming => "streaming",
            Self::Controlling => "controlling",
        }
    }
}

/// Pure deterministic session state machine. Owns no I/O; the runtime reads a
/// transition and performs the side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteSessionStateMachine {
    pub phase: RemoteSessionPhase,
}

impl RemoteSessionStateMachine {
    pub fn new() -> Self {
        Self {
            phase: RemoteSessionPhase::Idle,
        }
    }

    /// Viewer side: a request has been sent out.
    pub fn request_sent(&mut self) -> Result<(), CoreError> {
        if self.phase != RemoteSessionPhase::Idle {
            return Err(Self::invalid(self.phase, "request_sent"));
        }
        self.phase = RemoteSessionPhase::Requesting;
        Ok(())
    }

    /// Host side: a valid request arrived.
    pub fn request_received(&mut self) -> Result<(), CoreError> {
        if self.phase != RemoteSessionPhase::Idle {
            return Err(Self::invalid(self.phase, "request_received"));
        }
        self.phase = RemoteSessionPhase::Requesting;
        Ok(())
    }

    /// Host accepted (or viewer saw the accept). Screen flow opens next.
    pub fn accepted(&mut self) -> Result<(), CoreError> {
        if self.phase != RemoteSessionPhase::Requesting {
            return Err(Self::invalid(self.phase, "accepted"));
        }
        self.phase = RemoteSessionPhase::Streaming;
        Ok(())
    }

    /// Input tunnel entered Session mode (`enter_session` succeeded).
    pub fn input_enabled(&mut self) -> Result<(), CoreError> {
        if self.phase != RemoteSessionPhase::Streaming {
            return Err(Self::invalid(self.phase, "input_enabled"));
        }
        self.phase = RemoteSessionPhase::Controlling;
        Ok(())
    }

    /// Reject, close, error, or idle timeout: any phase collapses to `Idle`.
    pub fn close(&mut self) {
        self.phase = RemoteSessionPhase::Idle;
    }

    /// `true` when the session can accept input events (Session mode active).
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            RemoteSessionPhase::Streaming | RemoteSessionPhase::Controlling
        )
    }

    fn invalid(phase: RemoteSessionPhase, event: &str) -> CoreError {
        CoreError::InvalidPayload(format!(
            "remote session state machine: {event} in phase {}",
            phase.as_str()
        ))
    }
}

impl Default for RemoteSessionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TS: u64 = 1_700_000_000_000;

    fn request() -> RemoteSessionRequestFrame {
        RemoteSessionRequestFrame {
            session_id: "rs-test-1".into(),
            client_device_id: "dev-mac-viewer".into(),
            reason: RemoteSessionReason::UserInitiated,
            timestamp_ms: TS,
        }
    }

    fn resp(accepted: bool) -> RemoteSessionRespFrame {
        RemoteSessionRespFrame {
            session_id: "rs-test-1".into(),
            accepted,
            reject_reason: if accepted {
                None
            } else {
                Some("PolicyDisabled".into())
            },
            screen_width: 1920,
            screen_height: 1080,
            keymap_profile: Some("mac_to_windows".into()),
            timestamp_ms: TS + 5,
        }
    }

    #[test]
    fn request_frame_codec_roundtrip() {
        let frame = request().to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_REMOTE_SESSION_REQUEST);
        let decoded = RemoteSessionRequestFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(request(), decoded);
        assert!(
            RemoteSessionRequestFrame::from_frame(&resp(true).to_frame().expect("serializes"))
                .is_err()
        );
    }

    #[test]
    fn resp_frame_codec_roundtrip() {
        let frame = resp(true).to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_REMOTE_SESSION_RESP);
        let decoded = RemoteSessionRespFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(resp(true), decoded);
        let rejected = resp(false);
        let wire = serde_json::to_value(&rejected).expect("json");
        assert_eq!(wire["reject_reason"], "PolicyDisabled");
        assert!(
            RemoteSessionRespFrame::from_frame(&request().to_frame().expect("serializes")).is_err()
        );
    }

    #[test]
    fn close_frame_codec_roundtrip() {
        let close = RemoteSessionCloseFrame {
            session_id: "rs-test-1".into(),
            reason: "viewer window closed".into(),
            timestamp_ms: TS + 10,
        };
        let frame = close.to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_REMOTE_SESSION_CLOSE);
        let decoded = RemoteSessionCloseFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(close, decoded);
    }

    #[test]
    fn request_sanitize_normalizes_fields() {
        let mut req = request();
        req.session_id = " bad;id ".into();
        req.client_device_id = "  dev-mac  ".into();
        req.sanitize();
        // An invalid id is blanked, not aliased to a sentinel: the blank id
        // must fail `validate_session_id` so downstream guards can reject it.
        assert_eq!(req.session_id, "");
        assert!(InputTunnelValidator::validate_session_id(&req.session_id).is_err());
        assert_eq!(req.client_device_id, "dev-mac");
    }

    #[test]
    fn resp_sanitize_clamps_reason_and_profile() {
        let mut frame = resp(true);
        frame.reject_reason = Some("Busy\0\u{7}padding".into());
        frame.keymap_profile = Some("  mac_to_windows\u{1}  ".into());
        frame.sanitize();
        assert_eq!(frame.reject_reason.as_deref(), Some("Busypadding"));
        assert_eq!(frame.keymap_profile.as_deref(), Some("mac_to_windows"));
    }

    #[test]
    fn resp_sanitize_blank_profile_becomes_none() {
        let mut frame = resp(true);
        frame.keymap_profile = Some("   ".into());
        frame.sanitize();
        assert_eq!(frame.keymap_profile, None);
        let mut frame = resp(true);
        frame.keymap_profile = Some("\0\u{7}".into());
        frame.sanitize();
        assert_eq!(frame.keymap_profile, None);
        let mut frame = resp(true);
        frame.keymap_profile = None;
        frame.sanitize();
        assert_eq!(frame.keymap_profile, None);
    }

    #[test]
    fn reject_reason_value_domain() {
        for reason in [
            RemoteSessionRejectReason::PolicyDisabled,
            RemoteSessionRejectReason::ScreenBusy,
            RemoteSessionRejectReason::AllowRequired,
            RemoteSessionRejectReason::Busy,
        ] {
            assert_eq!(
                RemoteSessionRejectReason::from_reason(reason.as_str()),
                Some(reason)
            );
        }
        assert_eq!(RemoteSessionRejectReason::from_reason("whatever"), None);
    }

    #[test]
    fn state_machine_happy_path() {
        let mut sm = RemoteSessionStateMachine::new();
        assert_eq!(sm.phase, RemoteSessionPhase::Idle);
        sm.request_sent().expect("idle → requesting");
        sm.accepted().expect("requesting → streaming");
        assert!(sm.is_active());
        sm.input_enabled().expect("streaming → controlling");
        assert_eq!(sm.phase, RemoteSessionPhase::Controlling);
        sm.close();
        assert_eq!(sm.phase, RemoteSessionPhase::Idle);
        assert!(!sm.is_active());
    }

    #[test]
    fn state_machine_reject_returns_to_idle() {
        let mut sm = RemoteSessionStateMachine::new();
        sm.request_received().expect("idle → requesting");
        sm.close();
        assert_eq!(sm.phase, RemoteSessionPhase::Idle);
    }

    #[test]
    fn state_machine_invalid_transitions_error() {
        let mut sm = RemoteSessionStateMachine::new();
        assert!(sm.accepted().is_err());
        assert!(sm.input_enabled().is_err());
        sm.request_sent().expect("idle → requesting");
        assert!(sm.request_sent().is_err());
        sm.accepted().expect("requesting → streaming");
        assert!(sm.accepted().is_err());
        sm.close();
        // Close from every phase is always legal.
        let mut sm = RemoteSessionStateMachine {
            phase: RemoteSessionPhase::Controlling,
        };
        sm.close();
        assert_eq!(sm.phase, RemoteSessionPhase::Idle);
    }

    #[test]
    fn state_machine_serializes_phase_names() {
        let json = serde_json::to_string(&RemoteSessionPhase::Controlling).expect("json");
        assert_eq!(json, "\"controlling\"");
        let json = serde_json::to_string(&RemoteSessionRejectReason::AllowRequired).expect("json");
        assert_eq!(json, "\"AllowRequired\"");
        let json = serde_json::to_string(&RemoteSessionReason::ApiTriggered).expect("json");
        assert_eq!(json, "\"ApiTriggered\"");
    }
}
