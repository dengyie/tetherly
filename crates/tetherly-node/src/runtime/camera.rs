// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Camera policy, sessions and status.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_camera_policy, CameraPolicy, DeviceType};
use tetherly_core::{
    CameraAnnounceFrame, CameraAttachFrame, CameraCodec, CameraDetachFrame, CameraFrameChunk,
    CameraResolution, CameraRole, CameraValidator, CoreError, DeviceId,
};
use tracing::{debug, warn};

/// Active virtual camera streaming session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveCameraSession {
    pub session_id: String,
    pub peer_id: String,
    pub role: CameraRole,
    pub lens_id: String,
    pub resolution: CameraResolution,
    pub target_fps: u32,
    pub current_fps: u32,
    pub codec: CameraCodec,
    pub flashlight_on: bool,
    pub zoom_ratio: f32,
    pub frames_sent: u64,
    pub frames_received: u64,
    pub bytes_transferred: u64,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Status report for virtual camera streaming and governance policy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CameraStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveCameraSession>,
    pub policy: CameraPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when establishing or terminating a camera session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CameraDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<CameraRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual camera streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CameraPolicyReport {
    pub ok: bool,
    pub policy: CameraPolicy,
    pub message: String,
}

/// Partial update payload for camera policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CameraPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_inbound_stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_capture: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_privacy_shutter_indicator: Option<bool>,
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
    /// Ingest and process an inbound CameraAnnounceFrame from a peer.
    pub fn receive_camera_announce(&self, peer: &DeviceId, frame: CameraAnnounceFrame) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            device = %frame.device_name,
            codecs = ?frame.capabilities.supported_codecs,
            lenses = ?frame.capabilities.lenses,
            "received camera announcement"
        );
    }

    /// Ingest and process an inbound CameraAttachFrame to act as a camera stream source (Lens role).
    pub async fn receive_camera_attach(&self, peer: &DeviceId, mut frame: CameraAttachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if CameraValidator::validate_attach(&frame).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid camera attach frame rejected");
            return;
        }

        let policy = self.camera_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual camera streaming disabled".to_string())
        } else if !policy.allow_remote_capture {
            Some("remote camera capture disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, false) {
            Some("peer ineligible under camera policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = CameraDetachFrame {
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
                "camera_attach_rejected",
                Some(peer_str.clone()),
                format!("Rejected camera attach from {}: {}", peer_str, reason),
                false,
            );
            return;
        }

        let session = ActiveCameraSession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: CameraRole::Lens,
            lens_id: frame.lens_id.clone(),
            resolution: frame.resolution,
            target_fps: frame.target_fps,
            current_fps: frame.target_fps,
            codec: frame.codec,
            flashlight_on: frame.flashlight_on,
            zoom_ratio: frame.zoom_ratio,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self.shared.active_camera.lock().expect("active_camera") = Some(session);

        self.emit(UiEvent::camera_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "lens",
            &frame.lens_id,
            frame.resolution.width,
            frame.resolution.height,
            frame.target_fps,
            frame.codec.as_str(),
            frame.flashlight_on,
            frame.zoom_ratio,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "camera_lens_started",
            Some(peer_str.clone()),
            format!(
                "Camera session {} started as streaming lens ({}x{}@{}fps, codec={}) for host {}",
                frame.session_id,
                frame.resolution.width,
                frame.resolution.height,
                frame.target_fps,
                frame.codec.as_str(),
                peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound CameraDetachFrame.
    pub async fn receive_camera_detach(&self, peer: &DeviceId, mut frame: CameraDetachFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self.shared.active_camera.lock().expect("active_camera");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::camera_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "camera_detached",
                Some(peer_str.clone()),
                format!("Camera detached by peer {}: {:?}", peer_str, frame.reason),
                true,
            );
        }
    }

    /// Ingest streaming camera frame chunk.
    pub fn receive_camera_frame_chunk(&self, peer: &DeviceId, frame: CameraFrameChunk) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        if CameraValidator::validate_chunk_frame(&frame).is_err() {
            return;
        }

        let mut guard = self.shared.active_camera.lock().expect("active_camera");
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

    /// Query status of active virtual camera session and governance policy.
    pub fn camera_status(&self) -> CameraStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.camera_policy();
        let mut guard = self.shared.active_camera.lock().expect("active_camera");

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

                self.emit(UiEvent::camera_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "camera_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Camera session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return CameraStatusReport {
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

        CameraStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a remote peer's camera lens as local virtual camera feed.
    #[allow(clippy::too_many_arguments)]
    pub async fn camera_attach(
        &self,
        peer_query: &str,
        lens_id: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<u32>,
        codec: Option<CameraCodec>,
        flashlight: Option<bool>,
        zoom: Option<f32>,
    ) -> Result<CameraDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.camera_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual camera streaming disabled".to_string(),
            ));
        }
        if !policy.allow_inbound_stream {
            return Err(CoreError::InvalidPayload(
                "attaching remote camera disabled by policy".to_string(),
            ));
        }

        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, true) {
            return Err(CoreError::InvalidPayload(
                "peer is ineligible under camera policy filters".to_string(),
            ));
        }

        let req_lens_id = lens_id.unwrap_or("default");
        let validated_lens_id = CameraValidator::validate_lens_id(req_lens_id)?;

        let req_width = width.unwrap_or(1920).min(policy.max_resolution_width);
        let req_height = height.unwrap_or(1080).min(policy.max_resolution_height);
        let clamped_res = CameraValidator::clamp_resolution(req_width, req_height);

        let req_fps = fps.unwrap_or(30).min(policy.max_fps);
        let clamped_fps = CameraValidator::clamp_fps(req_fps);

        let req_codec = codec.unwrap_or(CameraCodec::Mjpeg);
        let req_flashlight = flashlight.unwrap_or(false);
        let req_zoom = CameraValidator::clamp_zoom(zoom.unwrap_or(1.0));

        let session_id = format!("cam-sess-{}", &now_ms.to_string()[6..]);
        let attach_frame = CameraAttachFrame {
            session_id: session_id.clone(),
            peer_id: self.identity().device_id().to_string(),
            lens_id: validated_lens_id.clone(),
            resolution: clamped_res,
            target_fps: clamped_fps,
            codec: req_codec,
            flashlight_on: req_flashlight,
            zoom_ratio: req_zoom,
            timestamp_ms: now_ms,
        };

        CameraValidator::validate_attach(&attach_frame)?;
        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await?;

        let session = ActiveCameraSession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: CameraRole::Host,
            lens_id: validated_lens_id.clone(),
            resolution: clamped_res,
            target_fps: clamped_fps,
            current_fps: clamped_fps,
            codec: req_codec,
            flashlight_on: req_flashlight,
            zoom_ratio: req_zoom,
            frames_sent: 0,
            frames_received: 0,
            bytes_transferred: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self.shared.active_camera.lock().expect("active_camera") = Some(session);

        self.emit(UiEvent::camera_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            &validated_lens_id,
            clamped_res.width,
            clamped_res.height,
            clamped_fps,
            req_codec.as_str(),
            req_flashlight,
            req_zoom,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "camera_attached",
            Some(peer_str.clone()),
            format!(
                "Attached camera from peer {} (lens '{}', {}x{}@{}fps, codec={:?}, zoom={:.1}x, torch={})",
                peer_str, validated_lens_id, clamped_res.width, clamped_res.height, clamped_fps, req_codec, req_zoom, req_flashlight
            ),
            true,
        );

        Ok(CameraDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(CameraRole::Host),
            message: "virtual camera session established to remote lens".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active camera session.
    pub async fn camera_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<CameraDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = CameraValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self.shared.active_camera.lock().expect("active_camera");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = CameraDetachFrame {
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

            self.emit(UiEvent::camera_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "camera_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Camera session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(CameraDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("camera session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(CameraDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active camera session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current camera governance policy.
    pub fn camera_policy(&self) -> CameraPolicy {
        self.shared
            .camera_policy
            .lock()
            .expect("camera_policy")
            .clone()
    }

    /// Status and configuration report for camera policy.
    pub fn camera_policy_report(&self) -> CameraPolicyReport {
        CameraPolicyReport {
            ok: true,
            policy: self.camera_policy(),
            message: "camera policy report".to_string(),
        }
    }

    /// Update camera policy and persist to disk atomically with 0o600 permissions.
    pub fn set_camera_policy(
        &self,
        mut policy: CameraPolicy,
    ) -> Result<CameraPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_camera_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save camera_policy.json: {e}")))?;
        *self.shared.camera_policy.lock().expect("camera_policy") = policy.clone();

        self.emit(UiEvent::camera_policy_changed(
            policy.enabled,
            policy.allow_inbound_stream,
            policy.allow_remote_capture,
            policy.max_resolution_width,
            policy.max_resolution_height,
            policy.max_fps,
            policy.enforce_privacy_shutter_indicator,
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
            "camera_policy_changed",
            None::<String>,
            format!(
                "Updated camera policy: enabled={}, allow_inbound={}, allow_capture={}, max_res={}x{}, max_fps={}, privacy_shutter={}",
                policy.enabled,
                policy.allow_inbound_stream,
                policy.allow_remote_capture,
                policy.max_resolution_width,
                policy.max_resolution_height,
                policy.max_fps,
                policy.enforce_privacy_shutter_indicator
            ),
            true,
        );

        Ok(CameraPolicyReport {
            ok: true,
            policy,
            message: "camera policy updated".to_string(),
        })
    }

    /// Partially update camera policy and persist to disk.
    pub fn update_camera_policy(
        &self,
        update: CameraPolicyUpdate,
    ) -> Result<CameraPolicyReport, CoreError> {
        let mut policy = self.camera_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_inbound_stream {
            policy.allow_inbound_stream = v;
        }
        if let Some(v) = update.allow_remote_capture {
            policy.allow_remote_capture = v;
        }
        if let Some(v) = update.max_resolution_width {
            policy.max_resolution_width = v;
        }
        if let Some(v) = update.max_resolution_height {
            policy.max_resolution_height = v;
        }
        if let Some(v) = update.max_fps {
            policy.max_fps = v;
        }
        if let Some(v) = update.enforce_privacy_shutter_indicator {
            policy.enforce_privacy_shutter_indicator = v;
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
        self.set_camera_policy(policy)
    }
}
