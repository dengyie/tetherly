// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Bandwidth-profile-driven streaming parameter recommendation.
//!
//! Pure domain logic: maps a measured link throughput (Mbps, receiver-side
//! authoritative figure from ADR-077) onto concrete parameter tiers for the
//! four streaming pipelines (virtual display ADR-073, camera ADR-075, audio
//! routing ADR-074, microphone ADR-076). Every tier field is re-validated by
//! the owning module's Fail-Closed validator, so a recommendation can always
//! be fed straight into the existing attach/negotiation paths.

use serde::{Deserialize, Serialize};

use crate::audio_routing::AudioRoutingValidator;
use crate::camera::{CameraCodec, CameraValidator};
use crate::microphone::{MicCodec, MicValidator};
use crate::virtual_display::VirtualDisplayValidator;

/// Recommended virtual display mode (ADR-073).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayTier {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Recommended virtual camera capture profile (ADR-075).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraTier {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: CameraCodec,
}

/// Recommended spatial audio routing profile (ADR-074).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTier {
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: crate::audio_routing::AudioStreamCodec,
}

/// Recommended virtual microphone profile (ADR-076).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicTier {
    pub sample_rate: u32,
    pub channels: u32,
    pub codec: MicCodec,
}

/// Complete streaming recommendation for one peer link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamingRecommendation {
    /// Receiver-side measured throughput the tiers were derived from (Mbps).
    pub measured_mbps: f64,
    /// Human-readable tier band (flagship / high / standard / basic / minimal).
    pub tier: String,
    pub display: DisplayTier,
    pub camera: CameraTier,
    pub audio: AudioTier,
    pub mic: MicTier,
}

impl StreamingRecommendation {
    /// Fail-closed re-validation through each owning module's validator.
    /// A recommendation that could not survive the attach-path clamps is a
    /// logic bug, so callers can assert this in tests and before applying.
    pub fn validate(&self) -> Result<(), crate::error::CoreError> {
        let display_mode =
            VirtualDisplayValidator::validate_mode(&crate::virtual_display::VirtualDisplayMode {
                width: self.display.width,
                height: self.display.height,
                refresh_rate: self.display.fps,
                scale_factor: 1.0,
                pixel_format: crate::virtual_display::VirtualDisplayPixelFormat::Bgra8,
            })?;
        if display_mode.width != self.display.width
            || display_mode.height != self.display.height
            || display_mode.refresh_rate != self.display.fps
        {
            return Err(crate::error::CoreError::InvalidPayload(
                "display tier outside validator bounds".to_string(),
            ));
        }
        let cam_res = CameraValidator::clamp_resolution(self.camera.width, self.camera.height);
        let cam_fps = CameraValidator::clamp_fps(self.camera.fps);
        if cam_res.width != self.camera.width
            || cam_res.height != self.camera.height
            || cam_fps != self.camera.fps
        {
            return Err(crate::error::CoreError::InvalidPayload(
                "camera tier outside validator bounds".to_string(),
            ));
        }
        let audio_frame = crate::audio_routing::AudioRoutingAttachFrame {
            session_id: "rec-val".to_string(),
            source_id: "rec".to_string(),
            sink_id: "val".to_string(),
            layout: crate::audio_routing::AudioChannelLayout::Stereo,
            sample_rate: self.audio.sample_rate,
            channels: self.audio.channels as u8,
            format: crate::audio_routing::AudioSampleFormat::S16Le,
            codec: self.audio.codec,
            spatial_pos: None,
            volume_gain: 1.0,
            timestamp_ms: 0,
        };
        AudioRoutingValidator::validate_attach(&audio_frame)?;
        let mic_rate = MicValidator::clamp_sample_rate(self.mic.sample_rate);
        let mic_channels = MicValidator::clamp_channels(self.mic.channels);
        if mic_rate != self.mic.sample_rate || mic_channels != self.mic.channels {
            return Err(crate::error::CoreError::InvalidPayload(
                "mic tier outside validator bounds".to_string(),
            ));
        }
        Ok(())
    }
}

/// Maps measured throughput onto parameter tiers. Non-finite or negative
/// inputs degrade to the minimal tier (never panic, never extrapolate).
pub fn recommend_streaming_params(measured_mbps: f64) -> StreamingRecommendation {
    let mbps = if measured_mbps.is_finite() && measured_mbps > 0.0 {
        measured_mbps
    } else {
        0.0
    };

    // Bands are chosen so each tier keeps >=2x headroom below its band floor:
    // real links fluctuate, and sustained encoding costs ride on top of payload.
    let (tier, display, camera, audio_rate, mic_rate) = if mbps >= 500.0 {
        (
            "flagship",
            (3840, 2160, 60),
            (3840, 2160, 30, CameraCodec::H264Light),
            (48000, 2),
            48000,
        )
    } else if mbps >= 100.0 {
        (
            "high",
            (2560, 1440, 60),
            (1920, 1080, 30, CameraCodec::H264Light),
            (48000, 2),
            48000,
        )
    } else if mbps >= 30.0 {
        (
            "standard",
            (1920, 1080, 30),
            (1280, 720, 30, CameraCodec::Mjpeg),
            (48000, 2),
            48000,
        )
    } else if mbps >= 10.0 {
        (
            "basic",
            (1280, 720, 30),
            (1280, 720, 24, CameraCodec::Mjpeg),
            (44100, 2),
            16000,
        )
    } else {
        (
            "minimal",
            (640, 480, 15),
            (640, 480, 15, CameraCodec::Mjpeg),
            (16000, 1),
            16000,
        )
    };

    StreamingRecommendation {
        measured_mbps: mbps,
        tier: tier.to_string(),
        display: DisplayTier {
            width: display.0,
            height: display.1,
            fps: display.2,
        },
        camera: CameraTier {
            width: camera.0,
            height: camera.1,
            fps: camera.2,
            codec: camera.3,
        },
        audio: AudioTier {
            sample_rate: audio_rate.0,
            channels: audio_rate.1,
            codec: crate::audio_routing::AudioStreamCodec::OpusLight,
        },
        mic: MicTier {
            sample_rate: mic_rate,
            channels: 1,
            codec: MicCodec::Opus,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_boundaries() {
        let cases = [
            (0.0, "minimal"),
            (5.0, "minimal"),
            (10.0, "basic"),
            (29.9, "basic"),
            (30.0, "standard"),
            (99.9, "standard"),
            (100.0, "high"),
            (499.9, "high"),
            (500.0, "flagship"),
            (10000.0, "flagship"),
        ];
        for (mbps, tier) in cases {
            assert_eq!(recommend_streaming_params(mbps).tier, tier, "mbps={mbps}");
        }
    }

    #[test]
    fn test_non_finite_and_negative_degrade_to_minimal() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let rec = recommend_streaming_params(bad);
            assert_eq!(rec.tier, "minimal");
            assert_eq!(rec.measured_mbps, 0.0);
        }
    }

    #[test]
    fn test_all_tiers_pass_owning_validators() {
        for mbps in [
            0.0, 9.99, 10.0, 29.99, 30.0, 99.99, 100.0, 499.99, 500.0, 2000.0,
        ] {
            let rec = recommend_streaming_params(mbps);
            rec.validate()
                .unwrap_or_else(|e| panic!("tier {mbps} must validate: {e}"));
        }
    }

    #[test]
    fn test_recommendation_serializes() {
        let rec = recommend_streaming_params(120.0);
        let json = serde_json::to_string(&rec).unwrap();
        let back: StreamingRecommendation = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
    }
}
