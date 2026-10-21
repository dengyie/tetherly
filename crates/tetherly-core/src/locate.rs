use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::CoreError;
use crate::frame::{InnerFrame, TYPE_LOCATE_REQUEST, TYPE_LOCATE_STOP};

pub const DEFAULT_LOCATE_DURATION_SECS: u32 = 15;
pub const MAX_LOCATE_DURATION_SECS: u32 = 60;
pub const MIN_LOCATE_DURATION_SECS: u32 = 1;

/// Acoustic and haptic pattern for the device locator / ringing alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LocatePattern {
    #[default]
    Chime,
    Siren,
    Pulse,
    VibrateOnly,
}

impl LocatePattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chime => "chime",
            Self::Siren => "siren",
            Self::Pulse => "pulse",
            Self::VibrateOnly => "vibrate_only",
        }
    }
}

impl fmt::Display for LocatePattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for LocatePattern {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "chime" | "default" | "bell" => Ok(Self::Chime),
            "siren" | "alarm" => Ok(Self::Siren),
            "pulse" | "beep" => Ok(Self::Pulse),
            "vibrate_only" | "vibrate" | "haptic" => Ok(Self::VibrateOnly),
            _ => Err(CoreError::Json(format!(
                "invalid locate pattern: '{s}', expected one of: chime, siren, pulse, vibrate_only"
            ))),
        }
    }
}

/// Request to trigger an acoustic locator ring on a target mesh device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateRequestFrame {
    pub request_id: String,
    #[serde(default = "default_locate_duration")]
    pub duration_secs: u32,
    #[serde(default)]
    pub pattern: LocatePattern,
    #[serde(default)]
    pub override_silent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

fn default_locate_duration() -> u32 {
    DEFAULT_LOCATE_DURATION_SECS
}

impl LocateRequestFrame {
    pub fn new(
        request_id: impl Into<String>,
        duration_secs: u32,
        pattern: LocatePattern,
        override_silent: bool,
    ) -> Self {
        let mut s = Self {
            request_id: request_id.into(),
            duration_secs,
            pattern,
            override_silent,
            message: None,
            timestamp_ms: 0,
        };
        s.sanitize(MAX_LOCATE_DURATION_SECS);
        s
    }

    pub fn sanitize(&mut self, max_policy_secs: u32) {
        self.request_id = LocateValidator::sanitize_request_id(&self.request_id);
        let max_bound = max_policy_secs.clamp(MIN_LOCATE_DURATION_SECS, MAX_LOCATE_DURATION_SECS);
        self.duration_secs = self
            .duration_secs
            .clamp(MIN_LOCATE_DURATION_SECS, max_bound);
        if let Some(msg) = self.message.as_mut() {
            *msg = LocateValidator::sanitize_message(msg);
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        serde_json::to_vec(self)
            .map_err(|e| CoreError::Json(format!("locate request frame serialize: {e}")))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut parsed: Self = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Json(format!("locate request frame deserialize: {e}")))?;
        parsed.sanitize(MAX_LOCATE_DURATION_SECS);
        Ok(parsed)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Self::decode(payload)
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let json_bytes = self.encode()?;
        InnerFrame::new(TYPE_LOCATE_REQUEST, json_bytes)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_LOCATE_REQUEST {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{:04x}, got 0x{:04x}",
                TYPE_LOCATE_REQUEST, frame.ty
            )));
        }
        Self::decode(&frame.payload)
    }
}

/// Request to cancel/silence an active acoustic locator ring on a target mesh device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateStopFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl LocateStopFrame {
    pub fn new(request_id: Option<String>, reason: impl Into<String>) -> Self {
        let mut s = Self {
            request_id,
            reason: reason.into(),
            timestamp_ms: 0,
        };
        s.sanitize();
        s
    }

    pub fn sanitize(&mut self) {
        if let Some(id) = self.request_id.as_mut() {
            *id = LocateValidator::sanitize_request_id(id);
        }
        self.reason = LocateValidator::sanitize_reason(&self.reason);
    }

    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        serde_json::to_vec(self)
            .map_err(|e| CoreError::Json(format!("locate stop frame serialize: {e}")))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut parsed: Self = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Json(format!("locate stop frame deserialize: {e}")))?;
        parsed.sanitize();
        Ok(parsed)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Self::decode(payload)
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let json_bytes = self.encode()?;
        InnerFrame::new(TYPE_LOCATE_STOP, json_bytes)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_LOCATE_STOP {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{:04x}, got 0x{:04x}",
                TYPE_LOCATE_STOP, frame.ty
            )));
        }
        Self::decode(&frame.payload)
    }
}

/// Input cleaning and validation utilities for device locator frames.
pub struct LocateValidator;

impl LocateValidator {
    /// Clamps locate duration in seconds between MIN_LOCATE_DURATION_SECS and MAX_LOCATE_DURATION_SECS.
    pub fn clamp_duration(secs: u32) -> u32 {
        secs.clamp(MIN_LOCATE_DURATION_SECS, MAX_LOCATE_DURATION_SECS)
    }

    /// Sanitizes a request identifier (strips invalid characters, bounds length to 64).
    pub fn sanitize_request_id(id: &str) -> String {
        let filtered: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        if filtered.is_empty() {
            "req_default".to_string()
        } else {
            filtered
        }
    }

    /// Strips control characters and bounds custom message length to 128 bytes.
    pub fn sanitize_message(msg: &str) -> String {
        msg.chars().filter(|c| !c.is_control()).take(128).collect()
    }

    /// Strips control characters and bounds stop reason length to 64 bytes.
    pub fn sanitize_reason(reason: &str) -> String {
        let cleaned: String = reason
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        if cleaned.is_empty() {
            "user_stopped".to_string()
        } else {
            cleaned
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locate_pattern_parse_and_display() {
        assert_eq!(LocatePattern::Chime.as_str(), "chime");
        assert_eq!(LocatePattern::Siren.as_str(), "siren");
        assert_eq!(LocatePattern::Pulse.as_str(), "pulse");
        assert_eq!(LocatePattern::VibrateOnly.as_str(), "vibrate_only");

        assert_eq!(
            "chime".parse::<LocatePattern>().unwrap(),
            LocatePattern::Chime
        );
        assert_eq!(
            "alarm".parse::<LocatePattern>().unwrap(),
            LocatePattern::Siren
        );
        assert_eq!(
            "pulse".parse::<LocatePattern>().unwrap(),
            LocatePattern::Pulse
        );
        assert_eq!(
            "vibrate".parse::<LocatePattern>().unwrap(),
            LocatePattern::VibrateOnly
        );
        assert!("invalid".parse::<LocatePattern>().is_err());
    }

    #[test]
    fn test_locate_request_wire_roundtrip_and_clamping() {
        let req = LocateRequestFrame::new(
            "loc-test-123",
            120, // should clamp to 60
            LocatePattern::Siren,
            true,
        );
        assert_eq!(req.duration_secs, 60);

        let inner = req.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_LOCATE_REQUEST);

        let recovered = LocateRequestFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.request_id, "loc-test-123");
        assert_eq!(recovered.duration_secs, 60);
        assert_eq!(recovered.pattern, LocatePattern::Siren);
        assert!(recovered.override_silent);
    }

    #[test]
    fn test_locate_stop_wire_roundtrip() {
        let stop = LocateStopFrame::new(Some("loc-test-123".into()), "found_device");
        let inner = stop.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_LOCATE_STOP);

        let recovered = LocateStopFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.request_id, Some("loc-test-123".into()));
        assert_eq!(recovered.reason, "found_device");
    }

    #[test]
    fn test_locate_validator_sanitization() {
        assert_eq!(
            LocateValidator::sanitize_request_id("loc@123#456!"),
            "loc123456"
        );
        assert_eq!(LocateValidator::sanitize_request_id(""), "req_default");
        assert_eq!(
            LocateValidator::sanitize_message("Hello\x00\x1fWorld!"),
            "HelloWorld!"
        );
        assert_eq!(
            LocateValidator::sanitize_reason("Stopped\r\nmanually"),
            "Stoppedmanually"
        );
    }
}
