use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;
use crate::frame::{InnerFrame, TYPE_FOCUS_STATE, TYPE_FOCUS_SYNC_REQ};

pub const MAX_FOCUS_REASON_LEN: usize = 64;
pub const DEFAULT_FOCUS_DURATION_MINS: u32 = 60;
pub const MIN_FOCUS_DURATION_MINS: u32 = 1;
pub const MAX_FOCUS_DURATION_MINS: u32 = 1440; // 24 hours

/// Standardized focus modes for distraction-free operation and disturbance suppression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    #[default]
    Dnd,
    Work,
    Meeting,
    Sleep,
}

impl FocusMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dnd => "dnd",
            Self::Work => "work",
            Self::Meeting => "meeting",
            Self::Sleep => "sleep",
        }
    }
}

impl fmt::Display for FocusMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for FocusMode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "dnd" | "do_not_disturb" | "silent" | "default" => Ok(Self::Dnd),
            "work" | "focus" | "coding" | "study" => Ok(Self::Work),
            "meeting" | "presentation" | "call" => Ok(Self::Meeting),
            "sleep" | "bedtime" | "night" => Ok(Self::Sleep),
            _ => Err(CoreError::Json(format!(
                "invalid focus mode: '{s}', expected one of: dnd, work, meeting, sleep"
            ))),
        }
    }
}

/// Wire protocol frame broadcasting or updating a node's Focus / DND state across the mesh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusStateFrame {
    pub enabled: bool,
    #[serde(default)]
    pub mode: FocusMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl FocusStateFrame {
    pub fn new(
        enabled: bool,
        mode: FocusMode,
        expires_ms: Option<u64>,
        reason: Option<String>,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            enabled,
            mode,
            expires_ms,
            reason: reason.map(|r| FocusValidator::sanitize_reason(&r)),
            timestamp_ms,
        }
    }

    /// Checks if this focus state is currently active given the current timestamp.
    pub fn is_active_at(&self, now_ms: u64) -> bool {
        if !self.enabled {
            return false;
        }
        match self.expires_ms {
            Some(exp) => now_ms < exp,
            None => true,
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        Ok(InnerFrame {
            ty: TYPE_FOCUS_STATE,
            flags: 0,
            msg_id: 0,
            payload,
        })
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        if let Some(r) = frame.reason.as_deref() {
            frame.reason = Some(FocusValidator::sanitize_reason(r));
        }
        Ok(frame)
    }
}

/// Wire protocol frame requesting a remote peer to send its current FocusStateFrame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusSyncReqFrame {
    pub request_id: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl FocusSyncReqFrame {
    pub fn new(request_id: impl Into<String>, timestamp_ms: u64) -> Result<Self, CoreError> {
        let clean_id = FocusValidator::validate_request_id(&request_id.into())?;
        Ok(Self {
            request_id: clean_id,
            timestamp_ms,
        })
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        Ok(InnerFrame {
            ty: TYPE_FOCUS_SYNC_REQ,
            flags: 0,
            msg_id: 0,
            payload,
        })
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let frame: Self = serde_json::from_slice(payload)?;
        FocusValidator::validate_request_id(&frame.request_id)?;
        Ok(frame)
    }
}

/// Zero-dependency fail-closed validation and input sanitization firewall for Focus Mode.
pub struct FocusValidator;

impl FocusValidator {
    /// Sanitizes optional reason text: removes ASCII control characters and clamps to MAX_FOCUS_REASON_LEN.
    pub fn sanitize_reason(reason: &str) -> String {
        let filtered: String = reason
            .chars()
            .filter(|&c| !c.is_ascii_control() || c == ' ')
            .collect();
        let trimmed = filtered.trim();
        if trimmed.len() <= MAX_FOCUS_REASON_LEN {
            trimmed.to_string()
        } else {
            let mut end = MAX_FOCUS_REASON_LEN;
            while !trimmed.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            trimmed[..end].trim_end().to_string()
        }
    }

    /// Clamps duration minutes within safe bounds (1..=1440 mins).
    pub fn clamp_duration_mins(mins: u32) -> u32 {
        mins.clamp(MIN_FOCUS_DURATION_MINS, MAX_FOCUS_DURATION_MINS)
    }

    /// Validates request ID format: must be non-empty, max 64 chars, alphanumeric/underscore/hyphen.
    pub fn validate_request_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidPayload(
                "request_id cannot be empty".into(),
            ));
        }
        if trimmed.len() > 64 {
            return Err(CoreError::InvalidPayload(
                "request_id exceeds 64 chars".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CoreError::InvalidPayload(
                "request_id contains invalid characters (only alphanumeric, _, - allowed)".into(),
            ));
        }
        Ok(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_mode_roundtrip() {
        for mode in [
            FocusMode::Dnd,
            FocusMode::Work,
            FocusMode::Meeting,
            FocusMode::Sleep,
        ] {
            let s = mode.as_str();
            let parsed = FocusMode::from_str(s).unwrap();
            assert_eq!(mode, parsed);
        }
    }

    #[test]
    fn test_focus_state_frame_serialization() {
        let frame = FocusStateFrame::new(
            true,
            FocusMode::Meeting,
            Some(1_000_000),
            Some("Client Demo".to_string()),
            123456,
        );
        let inner = frame.to_frame().unwrap();
        assert_eq!(inner.ty, TYPE_FOCUS_STATE);
        let decoded = FocusStateFrame::from_payload(&inner.payload).unwrap();
        assert_eq!(frame, decoded);
        assert!(decoded.is_active_at(500_000));
        assert!(!decoded.is_active_at(1_000_001));
    }

    #[test]
    fn test_focus_sync_req_frame() {
        let req = FocusSyncReqFrame::new("req_123", 456789).unwrap();
        let inner = req.to_frame().unwrap();
        assert_eq!(inner.ty, TYPE_FOCUS_SYNC_REQ);
        let decoded = FocusSyncReqFrame::from_payload(&inner.payload).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_focus_validator_sanitization() {
        let long_reason = "A".repeat(100);
        let sanitized = FocusValidator::sanitize_reason(&long_reason);
        assert_eq!(sanitized.len(), MAX_FOCUS_REASON_LEN);

        let with_ctrl = "Meeting\nwith\r\0client";
        assert_eq!(
            FocusValidator::sanitize_reason(with_ctrl),
            "Meetingwithclient"
        );

        assert_eq!(FocusValidator::clamp_duration_mins(0), 1);
        assert_eq!(FocusValidator::clamp_duration_mins(2000), 1440);
        assert_eq!(FocusValidator::clamp_duration_mins(45), 45);
    }
}
