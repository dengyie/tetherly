// SPDX-License-Identifier: Apache-2.0 OR MIT

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_ACTION_LIST_REQ, TYPE_ACTION_LIST_RESP, TYPE_ACTION_RESULT,
    TYPE_ACTION_TRIGGER,
};

pub const DEFAULT_ACTION_TIMEOUT_SECS: u32 = 10;
pub const MIN_ACTION_TIMEOUT_SECS: u32 = 1;
pub const MAX_ACTION_TIMEOUT_SECS: u32 = 30;
pub const MAX_ACTION_OUTPUT_LEN: usize = 1024;
pub const MAX_ACTION_ERROR_LEN: usize = 256;
pub const MAX_ACTION_ID_LEN: usize = 64;
pub const MAX_TRIGGER_ID_LEN: usize = 64;
pub const MAX_ACTION_NAME_LEN: usize = 64;
pub const MAX_ACTION_DESC_LEN: usize = 256;
pub const MAX_ACTION_ICON_LEN: usize = 32;

/// Safe public metadata of an action advertised to remote mesh peers.
///
/// Under the ADR-008 security invariant, the actual command string and args
/// are strictly private to the local host and never transmitted over the wire!
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl ActionItem {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            icon: None,
        }
    }

    pub fn sanitize(&mut self) {
        self.id = ActionValidator::clean_identifier(&self.id, MAX_ACTION_ID_LEN);
        self.name = ActionValidator::sanitize_text(&self.name, MAX_ACTION_NAME_LEN);
        if let Some(desc) = &self.description {
            self.description = Some(ActionValidator::sanitize_text(desc, MAX_ACTION_DESC_LEN));
        }
        if let Some(icon) = &self.icon {
            self.icon = Some(ActionValidator::sanitize_text(icon, MAX_ACTION_ICON_LEN));
        }
    }
}

/// Request to list available actions from a target peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionListReqFrame {
    pub request_id: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl ActionListReqFrame {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            timestamp_ms: 0,
        }
    }

    pub fn sanitize(&mut self) {
        self.request_id = ActionValidator::clean_identifier(&self.request_id, MAX_TRIGGER_ID_LEN);
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        InnerFrame::new(TYPE_ACTION_LIST_REQ, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        frame.sanitize();
        Ok(frame)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_ACTION_LIST_REQ {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{TYPE_ACTION_LIST_REQ:04x}, got 0x{:04x}",
                frame.ty
            )));
        }
        Self::from_payload(&frame.payload)
    }
}

/// Response containing the list of safe, advertised actions from a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionListRespFrame {
    pub request_id: String,
    #[serde(default)]
    pub actions: Vec<ActionItem>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl ActionListRespFrame {
    pub fn new(request_id: impl Into<String>, actions: Vec<ActionItem>) -> Self {
        let mut frame = Self {
            request_id: request_id.into(),
            actions,
            timestamp_ms: 0,
        };
        frame.sanitize();
        frame
    }

    pub fn sanitize(&mut self) {
        self.request_id = ActionValidator::clean_identifier(&self.request_id, MAX_TRIGGER_ID_LEN);
        for action in &mut self.actions {
            action.sanitize();
        }
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        InnerFrame::new(TYPE_ACTION_LIST_RESP, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        frame.sanitize();
        Ok(frame)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_ACTION_LIST_RESP {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{TYPE_ACTION_LIST_RESP:04x}, got 0x{:04x}",
                frame.ty
            )));
        }
        Self::from_payload(&frame.payload)
    }
}

/// Request to trigger a specific named action on a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTriggerFrame {
    pub trigger_id: String,
    pub action_id: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl ActionTriggerFrame {
    pub fn new(trigger_id: impl Into<String>, action_id: impl Into<String>) -> Self {
        let mut frame = Self {
            trigger_id: trigger_id.into(),
            action_id: action_id.into(),
            timestamp_ms: 0,
        };
        frame.sanitize();
        frame
    }

    pub fn sanitize(&mut self) {
        self.trigger_id = ActionValidator::clean_identifier(&self.trigger_id, MAX_TRIGGER_ID_LEN);
        self.action_id = ActionValidator::clean_identifier(&self.action_id, MAX_ACTION_ID_LEN);
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        InnerFrame::new(TYPE_ACTION_TRIGGER, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        frame.sanitize();
        Ok(frame)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_ACTION_TRIGGER {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{TYPE_ACTION_TRIGGER:04x}, got 0x{:04x}",
                frame.ty
            )));
        }
        Self::from_payload(&frame.payload)
    }
}

/// Result returned from executing a local action on a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResultFrame {
    pub trigger_id: String,
    pub action_id: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl ActionResultFrame {
    pub fn success(
        trigger_id: impl Into<String>,
        action_id: impl Into<String>,
        exit_code: Option<i32>,
        output: Option<String>,
        duration_ms: u64,
    ) -> Self {
        let mut frame = Self {
            trigger_id: trigger_id.into(),
            action_id: action_id.into(),
            success: true,
            exit_code,
            output_summary: output,
            error_message: None,
            duration_ms,
            timestamp_ms: 0,
        };
        frame.sanitize();
        frame
    }

    pub fn failure(
        trigger_id: impl Into<String>,
        action_id: impl Into<String>,
        exit_code: Option<i32>,
        err: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        let mut frame = Self {
            trigger_id: trigger_id.into(),
            action_id: action_id.into(),
            success: false,
            exit_code,
            output_summary: None,
            error_message: Some(err.into()),
            duration_ms,
            timestamp_ms: 0,
        };
        frame.sanitize();
        frame
    }

    pub fn sanitize(&mut self) {
        self.trigger_id = ActionValidator::clean_identifier(&self.trigger_id, MAX_TRIGGER_ID_LEN);
        self.action_id = ActionValidator::clean_identifier(&self.action_id, MAX_ACTION_ID_LEN);
        if let Some(out) = &self.output_summary {
            self.output_summary = Some(ActionValidator::sanitize_output(out));
        }
        if let Some(err) = &self.error_message {
            self.error_message = Some(ActionValidator::sanitize_text(err, MAX_ACTION_ERROR_LEN));
        }
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)?;
        InnerFrame::new(TYPE_ACTION_RESULT, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        frame.sanitize();
        Ok(frame)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_ACTION_RESULT {
            return Err(CoreError::Json(format!(
                "expected frame type 0x{TYPE_ACTION_RESULT:04x}, got 0x{:04x}",
                frame.ty
            )));
        }
        Self::from_payload(&frame.payload)
    }
}

/// Fail-closed validator and sanitizer for actions and identifiers.
pub struct ActionValidator;

impl ActionValidator {
    /// Validates and returns a sanitized action identifier.
    /// Only allows `[a-zA-Z0-9_-]`, clamped to max_len.
    pub fn validate_action_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidPayload(
                "action_id cannot be empty".into(),
            ));
        }
        if trimmed.len() > MAX_ACTION_ID_LEN {
            return Err(CoreError::InvalidPayload(format!(
                "action_id exceeds maximum length of {MAX_ACTION_ID_LEN} bytes"
            )));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CoreError::InvalidPayload(
                "action_id contains illegal characters (only [a-zA-Z0-9_-] permitted)".into(),
            ));
        }
        Ok(trimmed.to_string())
    }

    /// Cleans an identifier by replacing forbidden chars with underscores and truncating.
    pub fn clean_identifier(s: &str, max_len: usize) -> String {
        let mut res = String::with_capacity(s.len().min(max_len));
        for c in s.chars() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                res.push(c);
            } else {
                res.push('_');
            }
            if res.len() >= max_len {
                break;
            }
        }
        if res.is_empty() {
            "action_default".to_string()
        } else {
            res
        }
    }

    /// Sanitizes human-readable text by stripping control characters and truncating at UTF-8 boundary.
    pub fn sanitize_text(s: &str, max_len: usize) -> String {
        let mut cleaned = String::with_capacity(s.len().min(max_len));
        for c in s.chars() {
            if !c.is_control() {
                cleaned.push(c);
            }
            if cleaned.len() >= max_len {
                break;
            }
        }
        cleaned.trim().to_string()
    }

    /// Sanitizes execution output summary (allows standard newline/tab, caps at 1024 bytes).
    pub fn sanitize_output(s: &str) -> String {
        let mut cleaned = String::with_capacity(s.len().min(MAX_ACTION_OUTPUT_LEN));
        for c in s.chars() {
            if c == '\n' || c == '\r' || c == '\t' || !c.is_control() {
                cleaned.push(c);
            }
            if cleaned.len() >= MAX_ACTION_OUTPUT_LEN {
                break;
            }
        }
        cleaned
    }

    /// Clamps timeout in seconds between 1 and 30 seconds.
    pub fn clamp_timeout(secs: u32) -> u32 {
        secs.clamp(MIN_ACTION_TIMEOUT_SECS, MAX_ACTION_TIMEOUT_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_item_sanitization() {
        let mut item = ActionItem {
            id: "lock-screen_123".into(),
            name: "Lock Screen\x00\x07".into(),
            description: Some("Locks the screen\r\nnow".into()),
            icon: Some("lock\ticon".into()),
        };
        item.sanitize();
        assert_eq!(item.id, "lock-screen_123");
        assert_eq!(item.name, "Lock Screen");
        assert_eq!(item.description.unwrap(), "Locks the screennow");
        assert_eq!(item.icon.unwrap(), "lockicon");
    }

    #[test]
    fn test_action_id_validator() {
        assert!(ActionValidator::validate_action_id("lock_screen").is_ok());
        assert!(ActionValidator::validate_action_id("sleep-now-1").is_ok());
        assert!(ActionValidator::validate_action_id("").is_err());
        assert!(ActionValidator::validate_action_id("bad id with spaces").is_err());
        assert!(ActionValidator::validate_action_id("bad;rm -rf /").is_err());
        assert!(ActionValidator::validate_action_id("bad`whoami`").is_err());
    }

    #[test]
    fn test_action_list_req_wire_roundtrip() {
        let req = ActionListReqFrame::new("req_123");
        let inner = req.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_ACTION_LIST_REQ);

        let recovered = ActionListReqFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.request_id, "req_123");
    }

    #[test]
    fn test_action_list_resp_wire_roundtrip() {
        let resp = ActionListRespFrame::new(
            "req_123",
            vec![
                ActionItem::new("lock_screen", "Lock Screen"),
                ActionItem::new("sleep_display", "Sleep Display"),
            ],
        );
        let inner = resp.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_ACTION_LIST_RESP);

        let recovered = ActionListRespFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.request_id, "req_123");
        assert_eq!(recovered.actions.len(), 2);
        assert_eq!(recovered.actions[0].id, "lock_screen");
        assert_eq!(recovered.actions[1].id, "sleep_display");
    }

    #[test]
    fn test_action_trigger_wire_roundtrip() {
        let trig = ActionTriggerFrame::new("trig_999", "lock_screen");
        let inner = trig.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_ACTION_TRIGGER);

        let recovered = ActionTriggerFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.trigger_id, "trig_999");
        assert_eq!(recovered.action_id, "lock_screen");
    }

    #[test]
    fn test_action_result_wire_roundtrip() {
        let res = ActionResultFrame::success(
            "trig_999",
            "lock_screen",
            Some(0),
            Some("session locked successfully\n".into()),
            45,
        );
        let inner = res.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_ACTION_RESULT);

        let recovered = ActionResultFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(recovered.trigger_id, "trig_999");
        assert_eq!(recovered.action_id, "lock_screen");
        assert!(recovered.success);
        assert_eq!(recovered.exit_code, Some(0));
        assert_eq!(
            recovered.output_summary.as_deref(),
            Some("session locked successfully\n")
        );
        assert_eq!(recovered.duration_ms, 45);
    }

    #[test]
    fn test_action_timeout_clamping() {
        assert_eq!(ActionValidator::clamp_timeout(0), MIN_ACTION_TIMEOUT_SECS);
        assert_eq!(ActionValidator::clamp_timeout(10), 10);
        assert_eq!(ActionValidator::clamp_timeout(100), MAX_ACTION_TIMEOUT_SECS);
    }
}
