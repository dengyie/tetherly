use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_AUDIO_ROUTING_ANNOUNCE, TYPE_AUDIO_ROUTING_ATTACH, TYPE_AUDIO_ROUTING_CHUNK,
    TYPE_AUDIO_ROUTING_DETACH,
};

/// Role of a node participating in an active spatial audio routing session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioRoutingRole {
    /// Audio source node capturing and routing outgoing streams.
    Source,
    /// Audio sink node rendering incoming streams to physical speakers/headphones.
    Sink,
}

impl AudioRoutingRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Sink => "sink",
        }
    }
}

/// Channel layout for audio stream spatialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioChannelLayout {
    Mono,
    Stereo,
    Quad,
    Surround5_1,
}

impl AudioChannelLayout {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Stereo => "stereo",
            Self::Quad => "quad",
            Self::Surround5_1 => "surround5_1",
        }
    }

    pub fn default_channels(&self) -> u8 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Quad => 4,
            Self::Surround5_1 => 6,
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "mono" | "1" => Self::Mono,
            "quad" | "4" => Self::Quad,
            "surround5_1" | "5.1" | "6" => Self::Surround5_1,
            _ => Self::Stereo,
        }
    }
}

/// PCM sample format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSampleFormat {
    S16Le,
    F32Le,
}

impl AudioSampleFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::S16Le => "s16le",
            Self::F32Le => "f32le",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "f32" | "f32le" | "float" => Self::F32Le,
            _ => Self::S16Le,
        }
    }
}

/// Codec utilized for audio stream compression over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioStreamCodec {
    RawPcm,
    OpusLight,
    ZstdPcm,
}

impl AudioStreamCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RawPcm => "raw_pcm",
            Self::OpusLight => "opus_light",
            Self::ZstdPcm => "zstd_pcm",
        }
    }
}

/// 3D spatial position vector relative to the listener's head/screen coordinates (-10.0..=10.0).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioSpatialPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for AudioSpatialPosition {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }
}

/// Capabilities advertised by an audio sink node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRoutingCapabilities {
    pub supported_layouts: Vec<AudioChannelLayout>,
    pub supported_formats: Vec<AudioSampleFormat>,
    pub supported_codecs: Vec<AudioStreamCodec>,
    pub max_sample_rate: u32,
    pub max_channels: u8,
    pub spatial_supported: bool,
}

impl Default for AudioRoutingCapabilities {
    fn default() -> Self {
        Self {
            supported_layouts: vec![AudioChannelLayout::Stereo, AudioChannelLayout::Mono],
            supported_formats: vec![AudioSampleFormat::S16Le, AudioSampleFormat::F32Le],
            supported_codecs: vec![AudioStreamCodec::ZstdPcm, AudioStreamCodec::RawPcm],
            max_sample_rate: 48000,
            max_channels: 2,
            spatial_supported: true,
        }
    }
}

/// Announcement frame sent by sink node advertising audio output capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRoutingAnnounceFrame {
    pub device_id: String,
    pub device_name: String,
    pub capabilities: AudioRoutingCapabilities,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl AudioRoutingAnnounceFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_AUDIO_ROUTING_ANNOUNCE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_AUDIO_ROUTING_ANNOUNCE {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Attach frame sent by source node to establish audio streaming route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioRoutingAttachFrame {
    pub session_id: String,
    pub source_id: String,
    pub sink_id: String,
    pub layout: AudioChannelLayout,
    pub sample_rate: u32,
    pub channels: u8,
    pub format: AudioSampleFormat,
    pub codec: AudioStreamCodec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial_pos: Option<AudioSpatialPosition>,
    #[serde(default = "default_volume_gain")]
    pub volume_gain: f32,
    #[serde(default)]
    pub timestamp_ms: u64,
}

fn default_volume_gain() -> f32 {
    1.0
}

impl AudioRoutingAttachFrame {
    pub fn sanitize(&mut self) {
        self.session_id = AudioRoutingValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_audio_session".to_string());
        if self.volume_gain < 0.0 || self.volume_gain > 2.0 || self.volume_gain.is_nan() {
            self.volume_gain = 1.0;
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_AUDIO_ROUTING_ATTACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_AUDIO_ROUTING_ATTACH {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Detach frame tearing down an active audio routing session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRoutingDetachFrame {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl AudioRoutingDetachFrame {
    pub fn sanitize(&mut self) {
        self.session_id = AudioRoutingValidator::validate_session_id(&self.session_id)
            .unwrap_or_else(|_| "invalid_audio_session".to_string());
        if let Some(ref r) = self.reason {
            self.reason = Some(AudioRoutingValidator::sanitize_reason(r));
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_AUDIO_ROUTING_DETACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_AUDIO_ROUTING_DETACH {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Chunk frame containing streaming audio data (max 64 KiB per chunk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRoutingChunkFrame {
    pub session_id: String,
    pub chunk_seq: u64,
    pub payload_bytes: Vec<u8>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl AudioRoutingChunkFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_AUDIO_ROUTING_CHUNK, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_AUDIO_ROUTING_CHUNK {
            return Err(CoreError::TruncatedFrame);
        }
        Self::from_payload(&frame.payload)
    }
}

/// Security validation and parameter bounds engine for spatial audio routing.
pub struct AudioRoutingValidator;

impl AudioRoutingValidator {
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

    /// Validates attach parameters: sample rate (8000..=192000), channels (1..=8),
    /// volume gain (0.0..=2.0) and spatial coordinates (-10.0..=10.0).
    pub fn validate_attach(frame: &AudioRoutingAttachFrame) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        if frame.sample_rate < 8000 || frame.sample_rate > 192000 {
            return Err(CoreError::TruncatedFrame);
        }
        if frame.channels == 0 || frame.channels > 8 {
            return Err(CoreError::TruncatedFrame);
        }
        if frame.volume_gain < 0.0 || frame.volume_gain > 2.0 || frame.volume_gain.is_nan() {
            return Err(CoreError::TruncatedFrame);
        }
        if let Some(ref pos) = frame.spatial_pos {
            if pos.x < -10.0
                || pos.x > 10.0
                || pos.y < -10.0
                || pos.y > 10.0
                || pos.z < -10.0
                || pos.z > 10.0
                || pos.x.is_nan()
                || pos.y.is_nan()
                || pos.z.is_nan()
            {
                return Err(CoreError::TruncatedFrame);
            }
        }
        Ok(())
    }

    /// Validates audio chunk: max 64 KiB payload.
    pub fn validate_chunk(frame: &AudioRoutingChunkFrame) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        if frame.payload_bytes.is_empty() || frame.payload_bytes.len() > 65536 {
            return Err(CoreError::TruncatedFrame);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_routing_announce_codec() {
        let ann = AudioRoutingAnnounceFrame {
            device_id: "dev-speaker-01".into(),
            device_name: "Living Room HiFi".into(),
            capabilities: AudioRoutingCapabilities {
                supported_layouts: vec![AudioChannelLayout::Stereo, AudioChannelLayout::Quad],
                supported_formats: vec![AudioSampleFormat::S16Le, AudioSampleFormat::F32Le],
                supported_codecs: vec![AudioStreamCodec::ZstdPcm, AudioStreamCodec::RawPcm],
                max_sample_rate: 96000,
                max_channels: 4,
                spatial_supported: true,
            },
            timestamp_ms: 1700000000000,
        };
        let frame = ann.to_frame().unwrap();
        assert_eq!(frame.ty, TYPE_AUDIO_ROUTING_ANNOUNCE);
        let decoded = AudioRoutingAnnounceFrame::from_frame(&frame).unwrap();
        assert_eq!(decoded.device_id, "dev-speaker-01");
        assert!(decoded.capabilities.spatial_supported);
        assert_eq!(decoded.capabilities.max_sample_rate, 96000);
    }

    #[test]
    fn test_audio_routing_attach_detach_chunk_codec() {
        let att = AudioRoutingAttachFrame {
            session_id: "audio-sess-001".into(),
            source_id: "dev-host-01".into(),
            sink_id: "dev-speaker-01".into(),
            layout: AudioChannelLayout::Stereo,
            sample_rate: 48000,
            channels: 2,
            format: AudioSampleFormat::S16Le,
            codec: AudioStreamCodec::ZstdPcm,
            spatial_pos: Some(AudioSpatialPosition {
                x: 1.5,
                y: 0.0,
                z: -0.5,
            }),
            volume_gain: 1.0,
            timestamp_ms: 1700000000100,
        };
        let frame_att = att.to_frame().unwrap();
        let decoded_att = AudioRoutingAttachFrame::from_frame(&frame_att).unwrap();
        assert_eq!(decoded_att.session_id, "audio-sess-001");
        assert_eq!(decoded_att.layout, AudioChannelLayout::Stereo);
        assert_eq!(decoded_att.sample_rate, 48000);
        assert_eq!(decoded_att.spatial_pos.unwrap().x, 1.5);

        let det = AudioRoutingDetachFrame {
            session_id: "audio-sess-001".into(),
            reason: Some("user_disconnected".into()),
            timestamp_ms: 1700000000200,
        };
        let frame_det = det.to_frame().unwrap();
        let decoded_det = AudioRoutingDetachFrame::from_frame(&frame_det).unwrap();
        assert_eq!(decoded_det.session_id, "audio-sess-001");
        assert_eq!(decoded_det.reason.as_deref(), Some("user_disconnected"));

        let chunk = AudioRoutingChunkFrame {
            session_id: "audio-sess-001".into(),
            chunk_seq: 1,
            payload_bytes: vec![0u8; 960],
            timestamp_ms: 1700000000300,
        };
        let frame_chunk = chunk.to_frame().unwrap();
        let decoded_chunk = AudioRoutingChunkFrame::from_frame(&frame_chunk).unwrap();
        assert_eq!(decoded_chunk.chunk_seq, 1);
        assert_eq!(decoded_chunk.payload_bytes.len(), 960);
    }

    #[test]
    fn test_audio_routing_validator() {
        assert!(AudioRoutingValidator::validate_session_id("valid-audio_01").is_ok());
        assert!(AudioRoutingValidator::validate_session_id("").is_err());
        assert!(AudioRoutingValidator::validate_session_id("invalid spaces").is_err());

        let mut att = AudioRoutingAttachFrame {
            session_id: "audio-sess-001".into(),
            source_id: "host".into(),
            sink_id: "speaker".into(),
            layout: AudioChannelLayout::Stereo,
            sample_rate: 48000,
            channels: 2,
            format: AudioSampleFormat::S16Le,
            codec: AudioStreamCodec::RawPcm,
            spatial_pos: Some(AudioSpatialPosition {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            volume_gain: 1.0,
            timestamp_ms: 0,
        };
        assert!(AudioRoutingValidator::validate_attach(&att).is_ok());

        att.sample_rate = 4000; // Too low
        assert!(AudioRoutingValidator::validate_attach(&att).is_err());
        att.sample_rate = 48000;

        att.volume_gain = 3.0; // Out of bounds
        assert!(AudioRoutingValidator::validate_attach(&att).is_err());
        att.volume_gain = 1.0;

        att.spatial_pos = Some(AudioSpatialPosition {
            x: 20.0,
            y: 0.0,
            z: 0.0,
        }); // Out of bounds
        assert!(AudioRoutingValidator::validate_attach(&att).is_err());

        let clean_reason = AudioRoutingValidator::sanitize_reason("normal \x00reason\t");
        assert_eq!(clean_reason, "normal reason");
    }
}
