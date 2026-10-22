use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_INPUT_TUNNEL_HEARTBEAT, TYPE_INPUT_TUNNEL_RELEASE, TYPE_INPUT_TUNNEL_REQUEST,
    TYPE_INPUT_TUNNEL_RESP,
};
use crate::topology::DisplayEdge;

/// Role of a node participating in an active input tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputTunnelRole {
    /// Node whose physical mouse/keyboard drives the remote peer screen.
    Controller,
    /// Node whose virtual cursor/input focus is driven by the controller.
    Target,
}

impl InputTunnelRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Target => "target",
        }
    }
}

/// Handshake request frame sent by controller when entering a remote peer screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTunnelRequestFrame {
    pub session_id: String,
    pub client_device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_edge: Option<DisplayEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_y: Option<i32>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl InputTunnelRequestFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_session".to_string());
        self.client_device_id = self.client_device_id.trim().to_string();
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_INPUT_TUNNEL_REQUEST, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_INPUT_TUNNEL_REQUEST {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Handshake response frame sent by target device accepting or rejecting control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTunnelRespFrame {
    pub session_id: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_height: Option<u32>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl InputTunnelRespFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_session".to_string());
        if let Some(ref r) = self.reject_reason {
            self.reject_reason = Some(InputTunnelValidator::sanitize_reason(r));
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_INPUT_TUNNEL_RESP, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_INPUT_TUNNEL_RESP {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Release frame sent by either controller or target to gracefully tear down tunnel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTunnelReleaseFrame {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_edge: Option<DisplayEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_y: Option<i32>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl InputTunnelReleaseFrame {
    pub fn sanitize(&mut self) {
        self.session_id = InputTunnelValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_session".to_string());
        self.reason = InputTunnelValidator::sanitize_reason(&self.reason);
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_INPUT_TUNNEL_RELEASE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_INPUT_TUNNEL_RELEASE {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Keepalive and sequence synchronization heartbeat for active input tunnels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTunnelHeartbeatFrame {
    pub session_id: String,
    pub seq: u64,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl InputTunnelHeartbeatFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_INPUT_TUNNEL_HEARTBEAT, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_INPUT_TUNNEL_HEARTBEAT {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Security validation and coordinate clamping engine for input tunneling.
pub struct InputTunnelValidator;

impl InputTunnelValidator {
    /// Validates session identifier: 1..=64 characters of `[a-zA-Z0-9_-]`.
    pub fn validate_session_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::TruncatedFrame);
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CoreError::TruncatedFrame);
        }
        Ok(trimmed.to_string())
    }

    /// Sanitizes reason text: strips control characters and clamps to 64 bytes.
    pub fn sanitize_reason(reason: &str) -> String {
        let clean: String = reason
            .chars()
            .filter(|c| !c.is_control() && *c != '\0')
            .take(64)
            .collect();
        if clean.trim().is_empty() {
            "unspecified".to_string()
        } else {
            clean.trim().to_string()
        }
    }

    /// Clamps input coordinates within display boundaries.
    pub fn clamp_coords(x: i32, y: i32, width: u32, height: u32) -> (i32, i32) {
        let max_x = (width.saturating_sub(1)) as i32;
        let max_y = (height.saturating_sub(1)) as i32;
        (x.clamp(0, max_x.max(0)), y.clamp(0, max_y.max(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_tunnel_request_frame_codec() {
        let req = InputTunnelRequestFrame {
            session_id: "tun-test-123".into(),
            client_device_id: "dev-desktop-host".into(),
            requested_edge: Some(DisplayEdge::Right),
            entry_x: Some(0),
            entry_y: Some(540),
            timestamp_ms: 1700000000000,
        };
        let frame = req.to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_INPUT_TUNNEL_REQUEST);
        let decoded = InputTunnelRequestFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_input_tunnel_resp_frame_codec() {
        let resp = InputTunnelRespFrame {
            session_id: "tun-test-123".into(),
            accepted: true,
            reject_reason: None,
            capture_width: Some(1920),
            capture_height: Some(1080),
            timestamp_ms: 1700000000005,
        };
        let frame = resp.to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_INPUT_TUNNEL_RESP);
        let decoded = InputTunnelRespFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn test_input_tunnel_release_frame_codec() {
        let rel = InputTunnelReleaseFrame {
            session_id: "tun-test-123".into(),
            exit_edge: Some(DisplayEdge::Left),
            return_x: Some(1919),
            return_y: Some(540),
            reason: "cursor returned across left edge".into(),
            timestamp_ms: 1700000000100,
        };
        let frame = rel.to_frame().expect("serializes");
        assert_eq!(frame.ty, TYPE_INPUT_TUNNEL_RELEASE);
        let decoded = InputTunnelReleaseFrame::from_frame(&frame).expect("deserializes");
        assert_eq!(rel, decoded);
    }

    #[test]
    fn test_input_tunnel_validator() {
        assert!(InputTunnelValidator::validate_session_id("tun_abc-123").is_ok());
        assert!(InputTunnelValidator::validate_session_id("").is_err());
        assert!(InputTunnelValidator::validate_session_id("bad;inject").is_err());

        let clean = InputTunnelValidator::sanitize_reason("normal exit\r\n\t\0reason");
        assert_eq!(clean, "normal exitreason");

        let (cx, cy) = InputTunnelValidator::clamp_coords(-50, 2000, 1920, 1080);
        assert_eq!(cx, 0);
        assert_eq!(cy, 1079);
    }
}
