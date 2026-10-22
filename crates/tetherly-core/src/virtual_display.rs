use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_VIRTUAL_DISPLAY_ANNOUNCE, TYPE_VIRTUAL_DISPLAY_ATTACH,
    TYPE_VIRTUAL_DISPLAY_DETACH, TYPE_VIRTUAL_DISPLAY_FRAME_ACK,
};
use crate::topology::DisplayEdge;

/// Role of a node participating in an active virtual secondary display session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VirtualDisplayRole {
    /// Host machine streaming extended desktop to remote display.
    Host,
    /// Client device rendering the incoming virtual secondary display buffer.
    Sink,
}

impl VirtualDisplayRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Sink => "sink",
        }
    }
}

/// Compression codec for virtual display streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VirtualDisplayCodec {
    ZstdBgra,
    ZstdRgba,
    RawBgra,
    RawRgba,
}

impl VirtualDisplayCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZstdBgra => "zstd_bgra",
            Self::ZstdRgba => "zstd_rgba",
            Self::RawBgra => "raw_bgra",
            Self::RawRgba => "raw_rgba",
        }
    }
}

/// Pixel format used in virtual display buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VirtualDisplayPixelFormat {
    Bgra8,
    Rgba8,
}

/// Display geometry and refresh rate configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualDisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
    pub scale_factor: f32,
    pub pixel_format: VirtualDisplayPixelFormat,
}

impl Default for VirtualDisplayMode {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            refresh_rate: 60,
            scale_factor: 1.0,
            pixel_format: VirtualDisplayPixelFormat::Bgra8,
        }
    }
}

/// Display capabilities advertised by client/sink node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDisplayCapabilities {
    pub supported_codecs: Vec<VirtualDisplayCodec>,
    pub max_width: u32,
    pub max_height: u32,
    pub touch_supported: bool,
}

impl Default for VirtualDisplayCapabilities {
    fn default() -> Self {
        Self {
            supported_codecs: vec![VirtualDisplayCodec::ZstdBgra, VirtualDisplayCodec::RawBgra],
            max_width: 3840,
            max_height: 2160,
            touch_supported: false,
        }
    }
}

/// Announcement frame sent by sink node advertising capabilities and preferred display mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualDisplayAnnounceFrame {
    pub device_id: String,
    pub display_name: String,
    pub capabilities: VirtualDisplayCapabilities,
    pub preferred_mode: VirtualDisplayMode,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl VirtualDisplayAnnounceFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_VIRTUAL_DISPLAY_ANNOUNCE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_VIRTUAL_DISPLAY_ANNOUNCE {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Attach frame sent by host node establishing a secondary display session with negotiated mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualDisplayAttachFrame {
    pub session_id: String,
    pub peer_id: String,
    pub assigned_edge: DisplayEdge,
    pub mode: VirtualDisplayMode,
    pub codec: VirtualDisplayCodec,
    pub target_fps: u32,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl VirtualDisplayAttachFrame {
    pub fn sanitize(&mut self) {
        self.session_id = VirtualDisplayValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_session".to_string());
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_VIRTUAL_DISPLAY_ATTACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_VIRTUAL_DISPLAY_ATTACH {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Detach frame tearing down an active secondary display session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDisplayDetachFrame {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl VirtualDisplayDetachFrame {
    pub fn sanitize(&mut self) {
        self.session_id = VirtualDisplayValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_session".to_string());
        if let Some(ref r) = self.reason {
            self.reason = Some(VirtualDisplayValidator::sanitize_reason(r));
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_VIRTUAL_DISPLAY_DETACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_VIRTUAL_DISPLAY_DETACH {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Frame acknowledgment telemetry sent back by sink node for adaptive rate control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDisplayFrameAck {
    pub session_id: String,
    pub frame_seq: u64,
    pub render_delay_us: u64,
    pub dropped_frames: u32,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl VirtualDisplayFrameAck {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_VIRTUAL_DISPLAY_FRAME_ACK, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_VIRTUAL_DISPLAY_FRAME_ACK {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Security validation and parameter bounds engine for virtual secondary display.
pub struct VirtualDisplayValidator;

impl VirtualDisplayValidator {
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

    /// Validates virtual display mode: bounds checks resolution (320..=7680, 240..=4320),
    /// refresh rate (10..=120) and scale factor (0.5..=4.0).
    pub fn validate_mode(mode: &VirtualDisplayMode) -> Result<VirtualDisplayMode, CoreError> {
        if mode.width < 320 || mode.width > 7680 || mode.height < 240 || mode.height > 4320 {
            return Err(CoreError::TruncatedFrame);
        }
        if mode.refresh_rate < 10 || mode.refresh_rate > 120 {
            return Err(CoreError::TruncatedFrame);
        }
        if mode.scale_factor < 0.5 || mode.scale_factor > 4.0 || mode.scale_factor.is_nan() {
            return Err(CoreError::TruncatedFrame);
        }
        Ok(mode.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_display_announce_codec() {
        let ann = VirtualDisplayAnnounceFrame {
            device_id: "dev-tablet-01".into(),
            display_name: "Tablet 2K Screen".into(),
            capabilities: VirtualDisplayCapabilities {
                supported_codecs: vec![VirtualDisplayCodec::ZstdBgra, VirtualDisplayCodec::RawBgra],
                max_width: 2560,
                max_height: 1600,
                touch_supported: true,
            },
            preferred_mode: VirtualDisplayMode {
                width: 2560,
                height: 1600,
                refresh_rate: 60,
                scale_factor: 1.5,
                pixel_format: VirtualDisplayPixelFormat::Bgra8,
            },
            timestamp_ms: 1700000000000,
        };
        let frame = ann.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_VIRTUAL_DISPLAY_ANNOUNCE);
        let decoded = VirtualDisplayAnnounceFrame::from_frame(&frame).unwrap();
        assert_eq!(decoded.device_id, "dev-tablet-01");
        assert!(decoded.capabilities.touch_supported);
        assert_eq!(decoded.preferred_mode.width, 2560);
    }

    #[test]
    fn test_virtual_display_attach_detach_ack_codec() {
        let att = VirtualDisplayAttachFrame {
            session_id: "vdisp-sess-001".into(),
            peer_id: "dev-tablet-01".into(),
            assigned_edge: DisplayEdge::Right,
            mode: VirtualDisplayMode::default(),
            codec: VirtualDisplayCodec::ZstdBgra,
            target_fps: 60,
            timestamp_ms: 1700000000100,
        };
        let frame_att = att.to_frame().unwrap();
        let decoded_att = VirtualDisplayAttachFrame::from_frame(&frame_att).unwrap();
        assert_eq!(decoded_att.session_id, "vdisp-sess-001");
        assert_eq!(decoded_att.assigned_edge, DisplayEdge::Right);
        assert_eq!(decoded_att.target_fps, 60);

        let det = VirtualDisplayDetachFrame {
            session_id: "vdisp-sess-001".into(),
            reason: Some("user_closed_display".into()),
            timestamp_ms: 1700000000200,
        };
        let frame_det = det.to_frame().unwrap();
        let decoded_det = VirtualDisplayDetachFrame::from_frame(&frame_det).unwrap();
        assert_eq!(decoded_det.session_id, "vdisp-sess-001");
        assert_eq!(decoded_det.reason.as_deref(), Some("user_closed_display"));

        let ack = VirtualDisplayFrameAck {
            session_id: "vdisp-sess-001".into(),
            frame_seq: 42,
            render_delay_us: 1250,
            dropped_frames: 0,
            timestamp_ms: 1700000000300,
        };
        let frame_ack = ack.to_frame().unwrap();
        let decoded_ack = VirtualDisplayFrameAck::from_frame(&frame_ack).unwrap();
        assert_eq!(decoded_ack.frame_seq, 42);
        assert_eq!(decoded_ack.render_delay_us, 1250);
    }

    #[test]
    fn test_virtual_display_validator() {
        assert!(VirtualDisplayValidator::validate_session_id("valid-sess_01").is_ok());
        assert!(VirtualDisplayValidator::validate_session_id("").is_err());
        assert!(VirtualDisplayValidator::validate_session_id("invalid/id").is_err());

        assert_eq!(
            VirtualDisplayValidator::sanitize_reason("normal reason"),
            "normal reason"
        );
        assert_eq!(
            VirtualDisplayValidator::sanitize_reason("test\u{0000}bad"),
            "testbad"
        );
        assert_eq!(
            VirtualDisplayValidator::sanitize_reason("   "),
            "unspecified"
        );

        let valid_mode = VirtualDisplayMode::default();
        assert!(VirtualDisplayValidator::validate_mode(&valid_mode).is_ok());

        let invalid_mode = VirtualDisplayMode {
            width: 100, // too small (< 320)
            height: 1080,
            refresh_rate: 60,
            scale_factor: 1.0,
            pixel_format: VirtualDisplayPixelFormat::Bgra8,
        };
        assert!(VirtualDisplayValidator::validate_mode(&invalid_mode).is_err());
    }
}
