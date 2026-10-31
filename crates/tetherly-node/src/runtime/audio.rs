// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Audio-routing policy, sessions and microphone state.

use super::*;
use crate::events::UiEvent;
use crate::store::{
    save_audio_routing_policy, save_microphone_policy, AudioRoutingPolicy, DeviceType,
    MicrophonePolicy,
};
use tetherly_core::{
    AudioChannelLayout, AudioRoutingAnnounceFrame, AudioRoutingAttachFrame, AudioRoutingChunkFrame,
    AudioRoutingDetachFrame, AudioRoutingRole, AudioRoutingValidator, AudioSampleFormat,
    AudioSpatialPosition, AudioStreamCodec, CoreError, DeviceId, MicAnnounceFrame, MicAttachFrame,
    MicAudioChunkFrame, MicCodec, MicDetachFrame, MicMode, MicRole, MicValidator,
};
use tracing::{debug, warn};

/// Active spatial audio routing session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveAudioRoutingSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: AudioRoutingRole,
    pub layout: AudioChannelLayout,
    pub sample_rate: u32,
    pub channels: u8,
    pub format: AudioSampleFormat,
    pub codec: AudioStreamCodec,
    pub spatial_pos: Option<AudioSpatialPosition>,
    pub volume_gain: f32,
    pub chunks_sent: u64,
    pub chunks_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for spatial audio routing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AudioRoutingStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveAudioRoutingSession>,
    pub policy: AudioRoutingPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating an audio routing route.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AudioRoutingDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<AudioRoutingRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for spatial audio routing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AudioRoutingPolicyReport {
    pub ok: bool,
    pub policy: AudioRoutingPolicy,
    pub message: String,
}

/// Partial update payload for audio routing policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AudioRoutingPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_sink: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_outbound_routing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_volume_limit: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_spatial_rendering: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

/// Active virtual microphone or voice intercom session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveMicrophoneSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: MicRole,
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
    pub privacy_indicator_active: bool,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for virtual microphone, intercom streaming and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophoneStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveMicrophoneSession>,
    pub policy: MicrophonePolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating a microphone session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophoneDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<MicRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual microphone streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MicrophonePolicyReport {
    pub ok: bool,
    pub policy: MicrophonePolicy,
    pub message: String,
}

/// Partial update payload for microphone policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MicrophonePolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_capture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_intercom: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sample_rate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_session_duration_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_privacy_indicator: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_secs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_types: Option<Vec<DeviceType>>,
}

impl Node {
    /// Ingest and process an inbound AudioRoutingAnnounceFrame from a peer.
    pub fn receive_audio_routing_announce(
        &self,
        peer: &DeviceId,
        frame: AudioRoutingAnnounceFrame,
    ) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            capabilities = ?frame.capabilities,
            "received audio routing announcement"
        );
    }

    /// Ingest and process an inbound AudioRoutingAttachFrame to act as an audio sink.
    pub async fn receive_audio_routing_attach(
        &self,
        peer: &DeviceId,
        mut frame: AudioRoutingAttachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if AudioRoutingValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid audio routing attach frame rejected");
            return;
        }

        let policy = self.audio_routing_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("spatial audio routing disabled".to_string())
        } else if !policy.allow_inbound_sink {
            Some("sink audio playback disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, true) {
            Some("peer ineligible under audio routing policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = AudioRoutingDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "audio_routing_attach_rejected",
                Some(peer_str.clone()),
                format!(
                    "Rejected audio routing attach from {}: {}",
                    peer_str, reason
                ),
                false,
            );
            return;
        }

        let max_gain = (policy.max_volume_limit as f32) / 100.0;
        let effective_gain = frame.volume_gain.min(max_gain);

        let session = ActiveAudioRoutingSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: AudioRoutingRole::Sink,
            layout: frame.layout,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            format: frame.format,
            codec: frame.codec,
            spatial_pos: frame.spatial_pos,
            volume_gain: effective_gain,
            chunks_sent: 0,
            chunks_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing") = Some(session);

        self.emit(UiEvent::audio_routing_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "sink",
            frame.layout.as_str(),
            frame.sample_rate,
            frame.channels,
            frame.format.as_str(),
            frame.codec.as_str(),
            effective_gain,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audio_routing_sink_started",
            Some(peer_str.clone()),
            format!(
                "Audio routing session {} established as sink for host {}",
                frame.session_id, peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound AudioRoutingDetachFrame.
    pub async fn receive_audio_routing_detach(
        &self,
        peer: &DeviceId,
        mut frame: AudioRoutingDetachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_audio_routing
                .lock()
                .expect("active_audio_routing");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::audio_routing_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "audio_routing_detached",
                Some(peer_str.clone()),
                format!(
                    "Audio routing detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest streaming audio chunk.
    pub fn receive_audio_routing_chunk(&self, peer: &DeviceId, frame: AudioRoutingChunkFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if AudioRoutingValidator::validate_chunk(&frame).is_err() {
            return;
        }

        let audio_pts_us = if frame.timestamp_ms > 0 {
            frame.timestamp_ms.saturating_mul(1000)
        } else {
            now_ms.saturating_mul(1000)
        };
        if let Ok(mut engine) = self.shared.screen_engine.lock() {
            engine.record_audio_pts(audio_pts_us);
        }

        let mut guard = self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.chunks_received = session.chunks_received.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.payload_bytes.len() as u64);
            }
        }
    }

    /// Query status of active spatial audio routing session and governance policy.
    pub fn audio_routing_status(&self) -> AudioRoutingStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.audio_routing_policy();
        let mut guard = self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing");

        let is_timed_out = if let Some(ref session) = *guard {
            let timeout_ms = (policy.idle_timeout_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > timeout_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::audio_routing_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "audio_routing_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Audio routing session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return AudioRoutingStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        AudioRoutingStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a peer as an audio sink (routing local audio to peer).
    #[allow(clippy::too_many_arguments)]
    pub async fn audio_routing_attach(
        &self,
        peer_query: &str,
        layout: Option<AudioChannelLayout>,
        sample_rate: Option<u32>,
        channels: Option<u8>,
        spatial_x: Option<f32>,
        spatial_y: Option<f32>,
        spatial_z: Option<f32>,
        volume_gain: Option<f32>,
    ) -> Result<AudioRoutingDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.audio_routing_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "spatial audio routing disabled".to_string(),
            ));
        }
        if !policy.allow_outbound_routing {
            return Err(CoreError::InvalidPayload(
                "routing audio to remote peers disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, false) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under audio routing policy filters".to_string(),
            ));
        }

        let req_layout = layout.unwrap_or(AudioChannelLayout::Stereo);
        let req_sample_rate = sample_rate.unwrap_or(48000).clamp(8000, 192000);
        let req_channels = channels
            .unwrap_or_else(|| req_layout.default_channels())
            .clamp(1, 8);
        let req_format = AudioSampleFormat::S16Le;
        let req_codec = AudioStreamCodec::ZstdPcm;
        let spatial_pos = if spatial_x.is_some() || spatial_y.is_some() || spatial_z.is_some() {
            Some(AudioSpatialPosition {
                x: spatial_x.unwrap_or(0.0).clamp(-10.0, 10.0),
                y: spatial_y.unwrap_or(0.0).clamp(-10.0, 10.0),
                z: spatial_z.unwrap_or(0.0).clamp(-10.0, 10.0),
            })
        } else {
            None
        };
        let max_gain = (policy.max_volume_limit as f32) / 100.0;
        let req_gain = volume_gain.unwrap_or(1.0).clamp(0.0, 2.0).min(max_gain);

        let session_id = format!("ar-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = AudioRoutingAttachFrame {
            session_id: session_id.clone(),
            source_id: self.identity().device_id().to_string(),
            sink_id: peer_str.clone(),
            layout: req_layout,
            sample_rate: req_sample_rate,
            channels: req_channels,
            format: req_format,
            codec: req_codec,
            spatial_pos,
            volume_gain: req_gain,
            timestamp_ms: now_ms,
        };

        AudioRoutingValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveAudioRoutingSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: AudioRoutingRole::Source,
            layout: req_layout,
            sample_rate: req_sample_rate,
            channels: req_channels,
            format: req_format,
            codec: req_codec,
            spatial_pos,
            volume_gain: req_gain,
            chunks_sent: 0,
            chunks_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_audio_routing
            .lock()
            .expect("active_audio_routing") = Some(session);

        self.emit(UiEvent::audio_routing_session_started(
            &session_id,
            &peer_str,
            &alias,
            "source",
            req_layout.as_str(),
            req_sample_rate,
            req_channels,
            req_format.as_str(),
            req_codec.as_str(),
            req_gain,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "audio_routing_attached",
            Some(peer_str.clone()),
            format!(
                "Routed audio to peer {} ({:?} @ {}Hz, {} ch, gain={:.2})",
                peer_str, req_layout, req_sample_rate, req_channels, req_gain
            ),
            true,
        );

        Ok(AudioRoutingDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(AudioRoutingRole::Source),
            message: "audio routing established to peer".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active audio routing session.
    pub async fn audio_routing_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<AudioRoutingDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = AudioRoutingValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_audio_routing
                .lock()
                .expect("active_audio_routing");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = AudioRoutingDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::audio_routing_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "audio_routing_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Audio routing session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(AudioRoutingDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("audio routing session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(AudioRoutingDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active audio routing session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current audio routing governance policy.
    pub fn audio_routing_policy(&self) -> AudioRoutingPolicy {
        self.shared
            .audio_routing_policy
            .lock()
            .expect("audio_routing_policy")
            .clone()
    }

    /// Status and configuration report for audio routing policy.
    pub fn audio_routing_policy_report(&self) -> AudioRoutingPolicyReport {
        AudioRoutingPolicyReport {
            ok: true,
            policy: self.audio_routing_policy(),
            message: "audio routing policy report".to_string(),
        }
    }

    /// Update audio routing policy and persist to disk atomically with 0o600 permissions.
    pub fn set_audio_routing_policy(
        &self,
        mut policy: AudioRoutingPolicy,
    ) -> Result<AudioRoutingPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_audio_routing_policy(&self.shared.cfg.data_dir, &policy).map_err(|e| {
            CoreError::File(format!("failed to save audio_routing_policy.json: {e}"))
        })?;
        *self
            .shared
            .audio_routing_policy
            .lock()
            .expect("audio_routing_policy") = policy.clone();

        self.emit(UiEvent::audio_routing_policy_changed(
            policy.enabled,
            policy.allow_inbound_sink,
            policy.allow_outbound_routing,
            policy.max_volume_limit,
            policy.allow_spatial_rendering,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "audio_routing_policy_changed",
            None::<String>,
            format!(
                "Updated audio routing policy: enabled={}, allow_sink={}, allow_outbound={}, max_vol={}, spatial={}",
                policy.enabled,
                policy.allow_inbound_sink,
                policy.allow_outbound_routing,
                policy.max_volume_limit,
                policy.allow_spatial_rendering
            ),
            true,
        );

        Ok(AudioRoutingPolicyReport {
            ok: true,
            policy,
            message: "audio routing policy updated".to_string(),
        })
    }

    /// Partially update audio routing policy and persist to disk.
    pub fn update_audio_routing_policy(
        &self,
        update: AudioRoutingPolicyUpdate,
    ) -> Result<AudioRoutingPolicyReport, CoreError> {
        let mut policy = self.audio_routing_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_sink {
            policy.allow_inbound_sink = v;
        }
        if let Some(v) = update.allow_outbound_routing {
            policy.allow_outbound_routing = v;
        }
        if let Some(v) = update.max_volume_limit {
            policy.max_volume_limit = v;
        }
        if let Some(v) = update.allow_spatial_rendering {
            policy.allow_spatial_rendering = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_audio_routing_policy(policy)
    }

    /// Ingest and process an inbound MicAnnounceFrame from a peer.
    pub fn receive_mic_announce(&self, peer: &DeviceId, frame: MicAnnounceFrame) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            codecs = ?frame.capabilities.supported_codecs,
            devices = ?frame.capabilities.devices,
            "received microphone announcement"
        );
    }

    /// Ingest and process an inbound MicAttachFrame to act as a microphone stream source.
    pub async fn receive_mic_attach(&self, peer: &DeviceId, mut frame: MicAttachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if MicValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid microphone attach frame rejected");
            return;
        }

        let policy = self.microphone_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual microphone streaming disabled".to_string())
        } else if !policy.allow_remote_capture {
            Some("remote microphone capture disabled by policy".to_string())
        } else if frame.mode == MicMode::Intercom && !policy.allow_intercom {
            Some("voice intercom disabled by policy".to_string())
        } else if frame.sample_rate > policy.max_sample_rate {
            Some("sample rate exceeds policy limit".to_string())
        } else if !policy.is_peer_eligible(&profile, false, frame.mode == MicMode::Intercom) {
            Some("peer ineligible under microphone policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = MicDetachFrame {
                session_id: frame.session_id.clone(),
                reason: Some(reason.clone()),
                timestamp_ms: now_ms,
            };
            if let Ok(inner) = detach.to_frame() {
                let _ = self.send_to(peer, inner).await;
            }
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "mic_attach_rejected",
                Some(peer_str.clone()),
                format!("Rejected microphone attach from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        // Fail-closed privacy governance: the capturing side MUST light the privacy
        // indicator whenever the policy demands it, refusing silent capture.
        let privacy_active = frame.enforce_privacy_indicator || policy.enforce_privacy_indicator;

        let session = ActiveMicrophoneSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: MicRole::Source,
            mode: frame.mode,
            mic_id: frame.mic_id.clone(),
            codec: frame.codec,
            sample_rate: frame.sample_rate,
            channels: frame.channels,
            gain: frame.gain,
            muted: frame.muted || policy.default_muted,
            noise_suppression: frame.noise_suppression,
            echo_cancellation: frame.echo_cancellation,
            auto_gain: frame.auto_gain,
            privacy_indicator_active: privacy_active,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone") = Some(session.clone());

        self.emit(UiEvent::microphone_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "source",
            frame.mode.as_str(),
            &frame.mic_id,
            frame.codec.as_str(),
            frame.sample_rate,
            frame.channels,
            frame.gain,
            session.muted,
            privacy_active,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "mic_source_started",
            Some(peer_str.clone()),
            format!(
                "Microphone session {} started as capture source ({}Hz x{}ch, codec={}, mode={}, gain={:.2}, muted={}, privacy_indicator={}) for host {}",
                frame.session_id,
                frame.sample_rate,
                frame.channels,
                frame.codec.as_str(),
                frame.mode.as_str(),
                frame.gain,
                session.muted,
                privacy_active,
                peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound MicDetachFrame.
    pub async fn receive_mic_detach(&self, peer: &DeviceId, mut frame: MicDetachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_microphone
                .lock()
                .expect("active_microphone");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::microphone_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "mic_detached",
                Some(peer_str.clone()),
                format!(
                    "Microphone detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest streaming microphone audio chunk. Only session metadata (sequence,
    /// mute state, RMS dB telemetry) is tracked; raw sample buffers never leave
    /// this function and are never logged, audited, or broadcast (Zero-Plaintext).
    pub fn receive_mic_audio_chunk(&self, peer: &DeviceId, frame: MicAudioChunkFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if MicValidator::validate_chunk_frame(&frame).is_err() {
            return;
        }

        let mut guard = self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone");
        if let Some(ref mut session) = *guard {
            if session.session_id == frame.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.frames_received = session.frames_received.saturating_add(1);
                session.bytes_transferred = session
                    .bytes_transferred
                    .saturating_add(frame.data.len() as u64);
            }
        }
    }

    /// Query status of active microphone/intercom session and governance policy.
    pub fn microphone_status(&self) -> MicrophoneStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.microphone_policy();
        let mut guard = self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone");

        let is_timed_out = if let Some(ref session) = *guard {
            let idle_ms = (policy.idle_timeout_secs as u64) * 1000;
            let max_ms = (policy.max_session_duration_secs as u64) * 1000;
            now_ms.saturating_sub(session.last_activity_ms) > idle_ms
                || now_ms.saturating_sub(session.started_at_ms) > max_ms
        } else {
            false
        };

        if is_timed_out {
            if let Some(session) = guard.take() {
                drop(guard);
                let alias = DeviceId::parse(&session.peer_id)
                    .ok()
                    .and_then(|id| self.peer_alias(&id))
                    .unwrap_or_else(|| session.peer_id.clone());

                self.emit(UiEvent::microphone_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle or duration timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "mic_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Microphone session {} closed due to idle or duration timeout",
                        session.session_id
                    ),
                    true,
                );
                return MicrophoneStatusReport {
                    ok: true,
                    active: false,
                    session: None,
                    policy,
                    timestamp_ms: now_ms,
                };
            }
        }

        let (active, session) = match *guard {
            Some(ref s) => (true, Some(s.clone())),
            None => (false, None),
        };

        MicrophoneStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a remote peer's microphone as local virtual mic / intercom feed.
    #[allow(clippy::too_many_arguments)]
    pub async fn mic_attach(
        &self,
        peer_query: &str,
        mic_id: Option<&str>,
        mode: MicMode,
        codec: Option<MicCodec>,
        sample_rate: Option<u32>,
        channels: Option<u32>,
        gain: Option<f32>,
        muted: Option<bool>,
        noise_suppression: Option<bool>,
        echo_cancellation: Option<bool>,
        auto_gain: Option<bool>,
    ) -> Result<MicrophoneDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.microphone_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual microphone streaming disabled".to_string(),
            ));
        }
        if !policy.allow_inbound_stream {
            return Err(CoreError::InvalidPayload(
                "attaching remote microphone disabled by policy".to_string(),
            ));
        }
        if mode == MicMode::Intercom && !policy.allow_intercom {
            return Err(CoreError::InvalidPayload(
                "voice intercom disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true, mode == MicMode::Intercom) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under microphone policy filters".to_string(),
            ));
        }

        let req_mic_id = MicValidator::validate_mic_id(mic_id.unwrap_or("default"))?;
        let req_codec = codec.unwrap_or(MicCodec::Opus);
        let req_rate = MicValidator::clamp_sample_rate(sample_rate.unwrap_or(48000));
        if req_rate > policy.max_sample_rate {
            return Err(CoreError::InvalidPayload(
                "sample rate exceeds policy limit".to_string(),
            ));
        }
        let req_channels = MicValidator::clamp_channels(channels.unwrap_or(1));
        let req_gain = MicValidator::clamp_gain(gain.unwrap_or(1.0));
        let req_muted = muted.unwrap_or(false) || policy.default_muted;
        let req_ns = noise_suppression.unwrap_or(true);
        let req_aec = echo_cancellation.unwrap_or(true);
        let req_agc = auto_gain.unwrap_or(true);

        let session_id = format!("mic-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = MicAttachFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            mode,
            mic_id: req_mic_id.clone(),
            codec: req_codec,
            sample_rate: req_rate,
            channels: req_channels,
            gain: req_gain,
            muted: req_muted,
            noise_suppression: req_ns,
            echo_cancellation: req_aec,
            auto_gain: req_agc,
            enforce_privacy_indicator: policy.enforce_privacy_indicator,
            timestamp_ms: now_ms,
        };

        MicValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveMicrophoneSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: MicRole::Host,
            mode,
            mic_id: req_mic_id.clone(),
            codec: req_codec,
            sample_rate: req_rate,
            channels: req_channels,
            gain: req_gain,
            muted: req_muted,
            noise_suppression: req_ns,
            echo_cancellation: req_aec,
            auto_gain: req_agc,
            privacy_indicator_active: policy.enforce_privacy_indicator,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_microphone
            .lock()
            .expect("active_microphone") = Some(session);

        self.emit(UiEvent::microphone_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            mode.as_str(),
            &req_mic_id,
            req_codec.as_str(),
            req_rate,
            req_channels,
            req_gain,
            req_muted,
            policy.enforce_privacy_indicator,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "mic_attached",
            Some(peer_str.clone()),
            format!(
                "Attached microphone from peer {} (mic '{}', {}Hz x{}ch, codec={:?}, mode={:?}, gain={:.2}, muted={})",
                peer_str, req_mic_id, req_rate, req_channels, req_codec, mode, req_gain, req_muted
            ),
            true,
        );

        Ok(MicrophoneDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(MicRole::Host),
            message: "virtual microphone session established to remote source".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active microphone/intercom session.
    pub async fn mic_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<MicrophoneDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = MicValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_microphone
                .lock()
                .expect("active_microphone");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = MicDetachFrame {
                session_id: session.session_id.clone(),
                reason: Some(sanitized_reason.clone()),
                timestamp_ms: now_ms,
            };

            if let Some(peer) = peer_id {
                if let Ok(inner) = detach_frame.to_frame() {
                    let _ = self.send_to(&peer, inner).await;
                }
            }

            let alias = DeviceId::parse(&session.peer_id)
                .ok()
                .and_then(|id| self.peer_alias(&id))
                .unwrap_or_else(|| session.peer_id.clone());

            self.emit(UiEvent::microphone_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "mic_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Microphone session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(MicrophoneDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("microphone session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(MicrophoneDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active microphone session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current microphone governance policy.
    pub fn microphone_policy(&self) -> MicrophonePolicy {
        self.shared
            .microphone_policy
            .lock()
            .expect("microphone_policy")
            .clone()
    }

    /// Status and configuration report for microphone policy.
    pub fn microphone_policy_report(&self) -> MicrophonePolicyReport {
        MicrophonePolicyReport {
            ok: true,
            policy: self.microphone_policy(),
            message: "microphone policy report".to_string(),
        }
    }

    /// Update microphone policy and persist to disk atomically with 0o600 permissions.
    pub fn set_microphone_policy(
        &self,
        mut policy: MicrophonePolicy,
    ) -> Result<MicrophonePolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_microphone_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save microphone_policy.json: {e}")))?;
        *self
            .shared
            .microphone_policy
            .lock()
            .expect("microphone_policy") = policy.clone();

        self.emit(UiEvent::microphone_policy_changed(
            policy.enabled,
            policy.allow_inbound_stream,
            policy.allow_remote_capture,
            policy.allow_intercom,
            policy.max_sample_rate,
            policy.enforce_privacy_indicator,
            policy.default_muted,
            policy.idle_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "microphone_policy_changed",
            None::<String>,
            format!(
                "Updated microphone policy: enabled={}, allow_inbound={}, allow_capture={}, allow_intercom={}, max_rate={}Hz, privacy_indicator={}, default_muted={}",
                policy.enabled,
                policy.allow_inbound_stream,
                policy.allow_remote_capture,
                policy.allow_intercom,
                policy.max_sample_rate,
                policy.enforce_privacy_indicator,
                policy.default_muted
            ),
            true,
        );

        Ok(MicrophonePolicyReport {
            ok: true,
            policy,
            message: "microphone policy updated".to_string(),
        })
    }

    /// Partially update microphone policy and persist to disk.
    pub fn update_microphone_policy(
        &self,
        update: MicrophonePolicyUpdate,
    ) -> Result<MicrophonePolicyReport, CoreError> {
        let mut policy = self.microphone_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_stream {
            policy.allow_inbound_stream = v;
        }
        if let Some(v) = update.allow_remote_capture {
            policy.allow_remote_capture = v;
        }
        if let Some(v) = update.allow_intercom {
            policy.allow_intercom = v;
        }
        if let Some(v) = update.max_sample_rate {
            policy.max_sample_rate = v;
        }
        if let Some(v) = update.max_session_duration_secs {
            policy.max_session_duration_secs = v;
        }
        if let Some(v) = update.enforce_privacy_indicator {
            policy.enforce_privacy_indicator = v;
        }
        if let Some(v) = update.default_muted {
            policy.default_muted = v;
        }
        if let Some(v) = update.idle_timeout_secs {
            policy.idle_timeout_secs = v;
        }
        if let Some(v) = update.allowed_groups {
            policy.allowed_groups = v;
        }
        if let Some(v) = update.allowed_tags {
            policy.allowed_tags = v;
        }
        if let Some(v) = update.allowed_types {
            policy.allowed_types = v;
        }
        self.set_microphone_policy(policy)
    }
}
