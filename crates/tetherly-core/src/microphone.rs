use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{
    InnerFrame, TYPE_MIC_ANNOUNCE, TYPE_MIC_ATTACH, TYPE_MIC_AUDIO_CHUNK, TYPE_MIC_DETACH,
};

/// Role of a node participating in an active virtual microphone or voice intercom session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicRole {
    /// Desktop client receiving audio feed to expose as local virtual microphone.
    Host,
    /// Mobile/Tablet device capturing physical microphone feed to stream to remote host.
    Source,
}

impl MicRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Source => "source",
        }
    }
}

/// Operational mode of a microphone session: continuous virtual mic or push-to-talk intercom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicMode {
    /// Continuous low-latency streaming exposing the remote mic as local virtual input.
    VirtualMic,
    /// Push-To-Talk (PTT) half-duplex LAN voice intercom.
    Intercom,
}

impl MicMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::VirtualMic => "virtual_mic",
            Self::Intercom => "intercom",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "intercom" | "ptt" | "push_to_talk" | "talk" => Self::Intercom,
            _ => Self::VirtualMic,
        }
    }
}

/// Audio codec utilized for microphone and intercom streaming over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicCodec {
    Opus,
    Pcm16,
    AacLight,
}

impl MicCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Pcm16 => "pcm16",
            Self::AacLight => "aac_light",
        }
    }

    pub fn from_str_lenient(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "pcm" | "pcm16" | "raw" => Self::Pcm16,
            "aac" | "aac_light" => Self::AacLight,
            _ => Self::Opus,
        }
    }
}

/// Specifications and acoustic features of a physical microphone hardware device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicDevice {
    pub id: String,
    pub name: String,
    pub supported_sample_rates: Vec<u32>,
    pub supported_channels: Vec<u32>,
    pub supports_noise_suppression: bool,
    pub supports_echo_cancellation: bool,
    pub supports_auto_gain: bool,
}

impl Default for MicDevice {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "Built-in Microphone".to_string(),
            supported_sample_rates: vec![48000, 44100, 16000],
            supported_channels: vec![1, 2],
            supports_noise_suppression: true,
            supports_echo_cancellation: true,
            supports_auto_gain: true,
        }
    }
}

/// Overall microphone capabilities advertised by a provider node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicCapabilities {
    pub supported_codecs: Vec<MicCodec>,
    pub devices: Vec<MicDevice>,
}

impl Default for MicCapabilities {
    fn default() -> Self {
        Self {
            supported_codecs: vec![MicCodec::Opus, MicCodec::Pcm16, MicCodec::AacLight],
            devices: vec![MicDevice::default()],
        }
    }
}

/// Announcement frame sent by a provider node declaring available microphone inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicAnnounceFrame {
    pub device_id: String,
    pub device_name: String,
    pub capabilities: MicCapabilities,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl MicAnnounceFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("mic announce serialize: {e}")))?;
        InnerFrame::new(TYPE_MIC_ANNOUNCE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("mic announce deserialize: {e}")))
    }
}

/// Attach/negotiation frame to establish a virtual microphone or voice intercom stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicAttachFrame {
    pub session_id: String,
    pub peer_id: String,
    pub mode: MicMode,
    pub mic_id: String,
    pub codec: MicCodec,
    pub sample_rate: u32,
    pub channels: u32,
    pub gain: f32,
    pub muted: bool,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    pub enforce_privacy_indicator: bool,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl MicAttachFrame {
    pub fn sanitize(&mut self) {
        self.sample_rate = MicValidator::clamp_sample_rate(self.sample_rate);
        self.channels = MicValidator::clamp_channels(self.channels);
        self.gain = MicValidator::clamp_gain(self.gain);
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("mic attach serialize: {e}")))?;
        InnerFrame::new(TYPE_MIC_ATTACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("mic attach deserialize: {e}")))
    }
}

/// Session termination frame sent by either peer to release microphone feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicDetachFrame {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl MicDetachFrame {
    pub fn sanitize(&mut self) {
        if let Some(ref r) = self.reason {
            self.reason = Some(MicValidator::sanitize_reason(r));
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("mic detach serialize: {e}")))?;
        InnerFrame::new(TYPE_MIC_DETACH, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("mic detach deserialize: {e}")))
    }
}

/// Audio stream chunk carrying encoded microphone packet slices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MicAudioChunkFrame {
    pub session_id: String,
    pub frame_seq: u64,
    pub chunk_seq: u32,
    pub total_chunks: u32,
    pub is_muted: bool,
    pub rms_db: f32,
    pub data: Vec<u8>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl MicAudioChunkFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self)
            .map_err(|e| CoreError::InvalidPayload(format!("mic chunk serialize: {e}")))?;
        InnerFrame::new(TYPE_MIC_AUDIO_CHUNK, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload)
            .map_err(|e| CoreError::InvalidPayload(format!("mic chunk deserialize: {e}")))
    }
}

/// Fail-closed validator and acoustic privacy guardrails for the virtual microphone pipeline.
pub struct MicValidator;

impl MicValidator {
    pub const MAX_CHUNK_BYTES: usize = 65536;
    pub const MIN_SAMPLE_RATE: u32 = 8000;
    pub const MAX_SAMPLE_RATE: u32 = 192000;
    pub const MIN_CHANNELS: u32 = 1;
    pub const MAX_CHANNELS: u32 = 2;
    pub const MIN_GAIN: f32 = 0.0;
    pub const MAX_GAIN: f32 = 2.0;
    pub const MIN_RMS_DB: f32 = -120.0;
    pub const MAX_RMS_DB: f32 = 0.0;

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

    /// Validates and sanitizes microphone device ID.
    pub fn validate_mic_id(id: &str) -> Result<String, CoreError> {
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::InvalidPayload(
                "mic_id length must be between 1 and 64 characters".to_string(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CoreError::InvalidPayload(
                "mic_id contains invalid characters".to_string(),
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

    /// Clamps sample rate to 8000..=192000 Hz.
    pub fn clamp_sample_rate(rate: u32) -> u32 {
        rate.clamp(Self::MIN_SAMPLE_RATE, Self::MAX_SAMPLE_RATE)
    }

    /// Clamps channel count to 1..=2.
    pub fn clamp_channels(channels: u32) -> u32 {
        channels.clamp(Self::MIN_CHANNELS, Self::MAX_CHANNELS)
    }

    /// Clamps microphone gain multiplier to 0.0..=2.0.
    pub fn clamp_gain(gain: f32) -> f32 {
        if gain.is_nan() || gain < Self::MIN_GAIN {
            Self::MIN_GAIN
        } else if gain > Self::MAX_GAIN {
            Self::MAX_GAIN
        } else {
            gain
        }
    }

    /// Clamps RMS loudness telemetry to -120.0..=0.0 dBFS.
    pub fn clamp_rms_db(rms: f32) -> f32 {
        if rms.is_nan() || rms < Self::MIN_RMS_DB {
            Self::MIN_RMS_DB
        } else if rms > Self::MAX_RMS_DB {
            Self::MAX_RMS_DB
        } else {
            rms
        }
    }

    /// Validates audio chunk payload bounds to prevent memory exhaustion DoS.
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
        if data.len() > Self::MAX_CHUNK_BYTES {
            return Err(CoreError::PayloadTooLarge);
        }
        Ok(())
    }

    /// Validates an entire microphone attach request frame.
    pub fn validate_attach(frame: &MicAttachFrame) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        Self::validate_mic_id(&frame.mic_id)?;
        if frame.peer_id.trim().is_empty() {
            return Err(CoreError::InvalidPayload(
                "peer_id cannot be empty".to_string(),
            ));
        }
        if frame.sample_rate < Self::MIN_SAMPLE_RATE || frame.sample_rate > Self::MAX_SAMPLE_RATE {
            return Err(CoreError::InvalidPayload(
                "sample_rate must be between 8000 and 192000".to_string(),
            ));
        }
        if frame.channels < Self::MIN_CHANNELS || frame.channels > Self::MAX_CHANNELS {
            return Err(CoreError::InvalidPayload(
                "channels must be between 1 and 2".to_string(),
            ));
        }
        if !(Self::MIN_GAIN..=Self::MAX_GAIN).contains(&frame.gain) || frame.gain.is_nan() {
            return Err(CoreError::InvalidPayload(
                "gain must be between 0.0 and 2.0".to_string(),
            ));
        }
        Ok(())
    }

    /// Validates an incoming microphone audio chunk frame.
    pub fn validate_chunk_frame(frame: &MicAudioChunkFrame) -> Result<(), CoreError> {
        Self::validate_session_id(&frame.session_id)?;
        Self::validate_chunk(&frame.data, frame.chunk_seq, frame.total_chunks)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mic_role_mode_codec_strings() {
        assert_eq!(MicRole::Host.as_str(), "host");
        assert_eq!(MicRole::Source.as_str(), "source");
        assert_eq!(MicMode::VirtualMic.as_str(), "virtual_mic");
        assert_eq!(MicMode::Intercom.as_str(), "intercom");
        assert_eq!(MicMode::from_str_lenient("ptt"), MicMode::Intercom);
        assert_eq!(MicMode::from_str_lenient("other"), MicMode::VirtualMic);

        assert_eq!(MicCodec::Opus.as_str(), "opus");
        assert_eq!(MicCodec::Pcm16.as_str(), "pcm16");
        assert_eq!(MicCodec::AacLight.as_str(), "aac_light");
        assert_eq!(MicCodec::from_str_lenient("pcm"), MicCodec::Pcm16);
        assert_eq!(MicCodec::from_str_lenient("aac"), MicCodec::AacLight);
        assert_eq!(MicCodec::from_str_lenient("unknown"), MicCodec::Opus);
    }

    #[test]
    fn test_mic_validator_clamps_and_cleaning() {
        assert_eq!(MicValidator::clamp_sample_rate(4000), 8000);
        assert_eq!(MicValidator::clamp_sample_rate(48000), 48000);
        assert_eq!(MicValidator::clamp_sample_rate(384000), 192000);
        assert_eq!(MicValidator::clamp_channels(0), 1);
        assert_eq!(MicValidator::clamp_channels(8), 2);
        assert_eq!(MicValidator::clamp_gain(-0.5), 0.0);
        assert_eq!(MicValidator::clamp_gain(3.0), 2.0);
        assert!(MicValidator::clamp_gain(f32::NAN) == 0.0);
        assert_eq!(MicValidator::clamp_rms_db(-150.0), -120.0);
        assert_eq!(MicValidator::clamp_rms_db(10.0), 0.0);
        assert_eq!(MicValidator::sanitize_reason("a\nb\x7fc  "), "abc");

        assert!(MicValidator::validate_session_id("mic_ok-1").is_ok());
        assert!(MicValidator::validate_session_id("").is_err());
        assert!(MicValidator::validate_session_id("bad id!").is_err());
        assert!(MicValidator::validate_mic_id("default").is_ok());
        assert!(MicValidator::validate_mic_id("a b").is_err());
    }

    #[test]
    fn test_mic_validator_chunk_bounds() {
        assert!(MicValidator::validate_chunk(&[0u8; 128], 0, 1).is_ok());
        assert!(MicValidator::validate_chunk(&[0u8; 128], 1, 1).is_err());
        assert!(MicValidator::validate_chunk(&[0u8; 128], 0, 0).is_err());
        let big = [0u8; MicValidator::MAX_CHUNK_BYTES + 1];
        assert!(MicValidator::validate_chunk(&big, 0, 1).is_err());
    }

    #[test]
    fn test_mic_validator_attach_frame() {
        let mut frame = MicAttachFrame {
            session_id: "sess-1".to_string(),
            peer_id: "peer".to_string(),
            mode: MicMode::Intercom,
            mic_id: "default".to_string(),
            codec: MicCodec::Opus,
            sample_rate: 48000,
            channels: 1,
            gain: 1.0,
            muted: false,
            noise_suppression: true,
            echo_cancellation: true,
            auto_gain: true,
            enforce_privacy_indicator: true,
            timestamp_ms: 0,
        };
        assert!(MicValidator::validate_attach(&frame).is_ok());
        frame.sample_rate = 999999;
        frame.channels = 8;
        frame.gain = 42.0;
        frame.sanitize();
        assert_eq!(frame.sample_rate, 192000);
        assert_eq!(frame.channels, 2);
        assert_eq!(frame.gain, 2.0);
        frame.sample_rate = 1;
        frame.channels = 1;
        frame.gain = 1.0;
        assert!(MicValidator::validate_attach(&frame).is_err());
    }

    #[test]
    fn test_mic_frames_wire_roundtrip() {
        let announce = MicAnnounceFrame {
            device_id: "dev_phone".to_string(),
            device_name: "Phone".to_string(),
            capabilities: MicCapabilities::default(),
            timestamp_ms: 1000,
        };
        let decoded =
            MicAnnounceFrame::from_payload(&announce.to_frame().unwrap().payload).unwrap();
        assert_eq!(announce, decoded);

        let attach = MicAttachFrame {
            session_id: "mic_sess_1".to_string(),
            peer_id: "peer_123".to_string(),
            mode: MicMode::VirtualMic,
            mic_id: "default".to_string(),
            codec: MicCodec::Opus,
            sample_rate: 48000,
            channels: 1,
            gain: 1.0,
            muted: false,
            noise_suppression: true,
            echo_cancellation: true,
            auto_gain: true,
            enforce_privacy_indicator: true,
            timestamp_ms: 1001,
        };
        let decoded = MicAttachFrame::from_payload(&attach.to_frame().unwrap().payload).unwrap();
        assert_eq!(attach, decoded);

        let mut detach = MicDetachFrame {
            session_id: "mic_sess_1".to_string(),
            reason: Some("Normal shutdown".to_string()),
            timestamp_ms: 1002,
        };
        detach.sanitize();
        let decoded = MicDetachFrame::from_payload(&detach.to_frame().unwrap().payload).unwrap();
        assert_eq!(detach, decoded);

        let chunk = MicAudioChunkFrame {
            session_id: "mic_sess_1".to_string(),
            frame_seq: 1,
            chunk_seq: 0,
            total_chunks: 1,
            is_muted: false,
            rms_db: -18.2,
            data: vec![1, 2, 3, 4],
            timestamp_ms: 1003,
        };
        assert!(MicValidator::validate_chunk_frame(&chunk).is_ok());
        let decoded = MicAudioChunkFrame::from_payload(&chunk.to_frame().unwrap().payload).unwrap();
        assert_eq!(chunk, decoded);
    }
}
