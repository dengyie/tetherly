use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_CAMERA_ANNOUNCE, TYPE_CAMERA_ATTACH, TYPE_CAMERA_DETACH,
    TYPE_CAMERA_FRAME_CHUNK,
};

/// Role of a node participating in an active virtual camera session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraRole {
    /// Desktop client receiving video feed to expose as local virtual camera.
    Host,
    /// Mobile/Tablet device capturing physical camera feed to stream to remote host.
    Lens,
}

impl CameraRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Lens => "lens",
        }
    }
}

/// Physical facing orientation of the camera lens module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraFacing {
    Back,
    Front,
    External,
}

impl CameraFacing {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Front => "front",
            Self::External => "external",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "front" | "selfie" | "user" => Self::Front,
            "external" | "usb" | "webcam" => Self::External,
            _ => Self::Back,
        }
    }
}

/// Video codec format utilized for camera frame transport over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraCodec {
    Mjpeg,
    ZstdRgb,
    RawNv12,
    H264Light,
}

impl CameraCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mjpeg => "mjpeg",
            Self::ZstdRgb => "zstd_rgb",
            Self::RawNv12 => "raw_nv12",
            Self::H264Light => "h264_light",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "zstd" | "zstd_rgb" => Self::ZstdRgb,
            "nv12" | "raw_nv12" => Self::RawNv12,
            "h264" | "h264_light" => Self::H264Light,
            _ => Self::Mjpeg,
        }
    }
}

/// Resolution configuration for video capture and streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CameraResolution {
    pub width: u32,
    pub height: u32,
}

impl CameraResolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn vga() -> Self {
        Self::new(640, 480)
    }

    pub const fn hd() -> Self {
        Self::new(1280, 720)
    }

    pub const fn fhd() -> Self {
        Self::new(1920, 1080)
    }

    pub const fn uhd_4k() -> Self {
        Self::new(3840, 2160)
    }
}

impl Default for CameraResolution {
    fn default() -> Self {
        Self::fhd()
    }
}

/// Specifications and capabilities of an individual camera lens module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraLens {
    pub id: String,
    pub name: String,
    pub facing: CameraFacing,
    pub supported_resolutions: Vec<CameraResolution>,
    pub max_fps: u32,
    pub has_flashlight: bool,
    pub min_zoom: f32,
    pub max_zoom: f32,
}

impl Default for CameraLens {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "Main Camera".to_string(),
            facing: CameraFacing::Back,
            supported_resolutions: vec![
                CameraResolution::fhd(),
                CameraResolution::hd(),
                CameraResolution::vga(),
            ],
            max_fps: 60,
            has_flashlight: true,
            min_zoom: 1.0,
            max_zoom: 10.0,
        }
    }
}

/// Overall camera capabilities advertised by a provider node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraCapabilities {
    pub supported_codecs: Vec<CameraCodec>,
    pub lenses: Vec<CameraLens>,
}

impl Default for CameraCapabilities {
    fn default() -> Self {
        Self {
            supported_codecs: vec![
                CameraCodec::Mjpeg,
                CameraCodec::ZstdRgb,
                CameraCodec::RawNv12,
            ],
            lenses: vec![CameraLens::default()],
        }
    }
}

/// Announcement frame sent by a mobile/provider node declaring available camera lenses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraAnnounceFrame {
    pub device_id: String,
    pub device_name: String,
    pub capabilities: CameraCapabilities,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl CameraAnnounceFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("camera announce serialize: {e}")))?;
        InnerFrame::new(TYPE_CAMERA_ANNOUNCE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("camera announce deserialize: {e}")))
    }
}

/// Session attach request frame sent by host to initiate camera feed streaming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraAttachFrame {
    pub session_id: String,
    pub peer_id: String,
    pub lens_id: String,
    pub resolution: CameraResolution,
    pub target_fps: u32,
    pub codec: CameraCodec,
    pub flashlight_on: bool,
    pub zoom_ratio: f32,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl CameraAttachFrame {
    pub fn sanitize(&mut self) {
        self.resolution =
            CameraValidator::clamp_resolution(self.resolution.width, self.resolution.height);
        self.target_fps = CameraValidator::clamp_fps(self.target_fps);
        self.zoom_ratio = CameraValidator::clamp_zoom(self.zoom_ratio);
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("camera attach serialize: {e}")))?;
        InnerFrame::new(TYPE_CAMERA_ATTACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("camera attach deserialize: {e}")))
    }
}

/// Session termination frame sent by either peer to release camera feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraDetachFrame {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl CameraDetachFrame {
    pub fn sanitize(&mut self) {
        if let Some(ref r) = self.reason {
            self.reason = Some(CameraValidator::sanitize_reason(r));
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("camera detach serialize: {e}")))?;
        InnerFrame::new(TYPE_CAMERA_DETACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("camera detach deserialize: {e}")))
    }
}

/// Video stream frame chunk carrying encoded frame slices.
/// On the wire `data` rides as a base64 string so the fully encoded inner
/// frame always fits the Noise transport budget (see `crate::wire`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraFrameChunk {
    pub session_id: String,
    pub frame_seq: u64,
    pub chunk_seq: u32,
    pub total_chunks: u32,
    pub payload_bytes: u32,
    #[serde(with = "crate::wire::base64_bytes")]
    pub data: Vec<u8>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl CameraFrameChunk {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("camera chunk serialize: {e}")))?;
        InnerFrame::new(TYPE_CAMERA_FRAME_CHUNK, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("camera chunk deserialize: {e}")))
    }
}

/// Fail-closed validator and boundary guardrails for virtual camera streaming pipeline.
pub struct CameraValidator;

impl CameraValidator {
    /// Validates session identifier (1..=64 chars, ASCII alphanumeric, hyphens, and underscores).
    pub fn validate_session_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::InvalidPayload(
                "session_id length must be between 1 and 64 characters".to_string(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CoreError::InvalidPayload(
                "session_id contains invalid characters".to_string(),
            ));
        }
        Ok(trimmed.to_string())
    }

    /// Validates and sanitizes camera lens ID.
    pub fn validate_lens_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::InvalidPayload(
                "lens_id length must be between 1 and 64 characters".to_string(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CoreError::InvalidPayload(
                "lens_id contains invalid characters".to_string(),
            ));
        }
        Ok(trimmed.to_string())
    }

    /// Sanitizes reason text, truncating to 64 bytes and removing ASCII control characters.
    pub fn sanitize_reason(reason: &str) -> String {
        let clean: String = reason
            .chars()
            .filter(|c| !c.is_ascii_control())
            .take(64)
            .collect();
        clean.trim().to_string()
    }

    /// Clamps resolution width to 320..=7680 and height to 240..=4320.
    pub fn clamp_resolution(width: u32, height: u32) -> CameraResolution {
        let clamped_w = width.clamp(320, 7680);
        let clamped_h = height.clamp(240, 4320);
        CameraResolution::new(clamped_w, clamped_h)
    }

    /// Clamps target FPS to 1..=60.
    pub fn clamp_fps(fps: u32) -> u32 {
        fps.clamp(1, 60)
    }

    /// Clamps camera digital zoom ratio to 0.5..=20.0.
    pub fn clamp_zoom(zoom: f32) -> f32 {
        if zoom.is_nan() || zoom < 0.5 {
            0.5
        } else if zoom > 20.0 {
            20.0
        } else {
            zoom
        }
    }

    /// Validates frame chunk payload bounds.
    pub fn validate_chunk(data: &[u8], chunk_seq: u32, total_chunks: u32) -> Result<(), CoreError> {
        if total_chunks == 0 {
            return Err(CoreError::InvalidPayload(
                "total_chunks must be greater than zero".to_string(),
            ));
        }
        if chunk_seq >= total_chunks {
            return Err(CoreError::InvalidPayload(
                "chunk_seq must be strictly less than total_chunks".to_string(),
            ));
        }
        if data.len() > crate::wire::MAX_WIRE_CHUNK_BYTES {
            return Err(CoreError::PayloadTooLarge);
        }
        Ok(())
    }

    /// Validates an entire camera attach request frame.
    pub fn validate_attach(frame: &CameraAttachFrame) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        Self::validate_lens_id(&frame.lens_id)?;
        if frame.peer_id.trim().is_empty() {
            return Err(CoreError::InvalidPayload(
                "peer_id cannot be empty".to_string(),
            ));
        }
        if frame.resolution.width < 320
            || frame.resolution.width > 7680
            || frame.resolution.height < 240
            || frame.resolution.height > 4320
        {
            return Err(CoreError::InvalidPayload(
                "invalid resolution dimensions".to_string(),
            ));
        }
        if frame.target_fps < 1 || frame.target_fps > 60 {
            return Err(CoreError::InvalidPayload(
                "target_fps must be between 1 and 60".to_string(),
            ));
        }
        if frame.zoom_ratio < 0.5 || frame.zoom_ratio > 20.0 {
            return Err(CoreError::InvalidPayload(
                "zoom_ratio must be between 0.5 and 20.0".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates an incoming camera frame chunk.
    pub fn validate_chunk_frame(frame: &CameraFrameChunk) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        Self::validate_chunk(&frame.data, frame.chunk_seq, frame.total_chunks)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_role_and_codec_lenient() {
        assert_eq!(CameraRole::Host.as_str(), "host");
        assert_eq!(CameraRole::Lens.as_str(), "lens");

        assert_eq!(
            CameraFacing::from_str_lenient("selfie"),
            CameraFacing::Front
        );
        assert_eq!(
            CameraFacing::from_str_lenient("usb"),
            CameraFacing::External
        );
        assert_eq!(CameraFacing::from_str_lenient("other"), CameraFacing::Back);

        assert_eq!(
            CameraCodec::from_str_lenient("zstd_rgb"),
            CameraCodec::ZstdRgb
        );
        assert_eq!(CameraCodec::from_str_lenient("nv12"), CameraCodec::RawNv12);
        assert_eq!(
            CameraCodec::from_str_lenient("h264"),
            CameraCodec::H264Light
        );
        assert_eq!(CameraCodec::from_str_lenient("unknown"), CameraCodec::Mjpeg);
    }

    #[test]
    fn test_camera_validator_guards() {
        assert!(CameraValidator::validate_session_id("cam_sess_123").is_ok());
        assert!(CameraValidator::validate_session_id("bad session!").is_err());
        assert!(CameraValidator::validate_session_id("").is_err());

        assert!(CameraValidator::validate_lens_id("back_wide").is_ok());
        assert!(CameraValidator::validate_lens_id("back/wide").is_err());

        let clean_reason = CameraValidator::sanitize_reason("Meeting finished\x00\x1f\n");
        assert_eq!(clean_reason, "Meeting finished");

        let res = CameraValidator::clamp_resolution(100, 10000);
        assert_eq!(res.width, 320);
        assert_eq!(res.height, 4320);

        assert_eq!(CameraValidator::clamp_fps(0), 1);
        assert_eq!(CameraValidator::clamp_fps(120), 60);

        assert_eq!(CameraValidator::clamp_zoom(0.1), 0.5);
        assert_eq!(CameraValidator::clamp_zoom(30.0), 20.0);
        assert_eq!(CameraValidator::clamp_zoom(2.5), 2.5);

        assert!(CameraValidator::validate_chunk(&[0u8; 1024], 0, 1).is_ok());
        assert!(CameraValidator::validate_chunk(&[0u8; 1024], 1, 1).is_err());
        assert!(CameraValidator::validate_chunk(&[0u8; 1024], 0, 0).is_err());
    }

    #[test]
    fn test_camera_frames_wire_roundtrip() {
        let announce = CameraAnnounceFrame {
            device_id: "dev_phone".to_string(),
            device_name: "Phone 16 Pro".to_string(),
            capabilities: CameraCapabilities::default(),
            timestamp_ms: 1000,
        };
        let frame = announce.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_CAMERA_ANNOUNCE);
        let recovered = CameraAnnounceFrame::from_payload(&frame.payload).unwrap();
        assert_eq!(announce, recovered);

        let attach = CameraAttachFrame {
            session_id: "cam_1".to_string(),
            peer_id: "dev_phone".to_string(),
            lens_id: "back_wide".to_string(),
            resolution: CameraResolution::fhd(),
            target_fps: 30,
            codec: CameraCodec::Mjpeg,
            flashlight_on: true,
            zoom_ratio: 1.5,
            timestamp_ms: 1001,
        };
        let frame = attach.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_CAMERA_ATTACH);
        let recovered = CameraAttachFrame::from_payload(&frame.payload).unwrap();
        assert_eq!(attach, recovered);

        let detach = CameraDetachFrame {
            session_id: "cam_1".to_string(),
            reason: Some("user_stop".to_string()),
            timestamp_ms: 1002,
        };
        let frame = detach.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_CAMERA_DETACH);
        let recovered = CameraDetachFrame::from_payload(&frame.payload).unwrap();
        assert_eq!(detach, recovered);

        let chunk = CameraFrameChunk {
            session_id: "cam_1".to_string(),
            frame_seq: 42,
            chunk_seq: 0,
            total_chunks: 1,
            payload_bytes: 4,
            data: vec![1, 2, 3, 4],
            timestamp_ms: 1003,
        };
        let frame = chunk.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_CAMERA_FRAME_CHUNK);
        let recovered = CameraFrameChunk::from_payload(&frame.payload).unwrap();
        assert_eq!(chunk, recovered);
    }
}
